use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const USAGE: &str = "usage: cargo xtask <check|test [--arch <x86_64|aarch64>]|build --arch <x86_64|aarch64>|run --arch <x86_64|aarch64>>";
const LINUX_VERSION: &str = "6.18.42";
const READY: &str = "zeroOS init: READY";
const ESP_OFFSET: u64 = 1_048_576;
const ESP_SECTORS: &str = "32768";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    fn parse(value: &str) -> Result<Self, (u8, String)> {
        match value {
            "x86_64" => Ok(Self::X86_64),
            "aarch64" => Ok(Self::Aarch64),
            _ => Err((2, format!("zeroOS: unsupported architecture '{value}'"))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }

    fn rust_target(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-linux-musl",
            Self::Aarch64 => "aarch64-unknown-linux-musl",
        }
    }

    fn uefi_target(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-uefi",
            Self::Aarch64 => "aarch64-unknown-uefi",
        }
    }

    fn kernel_arch(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "arm64",
        }
    }

    fn kernel_image(self) -> &'static str {
        match self {
            Self::X86_64 => "arch/x86/boot/bzImage",
            Self::Aarch64 => "arch/arm64/boot/Image",
        }
    }

    fn kernel_target(self) -> &'static str {
        match self {
            Self::X86_64 => "bzImage",
            Self::Aarch64 => "Image",
        }
    }

    fn fallback(self) -> &'static str {
        match self {
            Self::X86_64 => "BOOTX64.EFI",
            Self::Aarch64 => "BOOTAA64.EFI",
        }
    }

    fn qemu(self) -> &'static str {
        match self {
            Self::X86_64 => "qemu-system-x86_64",
            Self::Aarch64 => "qemu-system-aarch64",
        }
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Check,
    Test(Option<Arch>),
    Build(Arch),
    Run(Arch),
}

fn main() -> ExitCode {
    match parse(env::args().skip(1).collect()).and_then(execute) {
        Ok(()) => ExitCode::SUCCESS,
        Err((2, message)) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::from(2)
        }
        Err((_, message)) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: Vec<String>) -> Result<Action, (u8, String)> {
    let bad = |message: String| Err((2, message));
    match args.as_slice() {
        [command] if command == "check" => Ok(Action::Check),
        [command] if command == "test" => Ok(Action::Test(None)),
        [command, flag, arch] if flag == "--arch" => match command.as_str() {
            "test" => Ok(Action::Test(Some(Arch::parse(arch)?))),
            "build" => Ok(Action::Build(Arch::parse(arch)?)),
            "run" => Ok(Action::Run(Arch::parse(arch)?)),
            _ => bad(format!("zeroOS: unsupported command '{command}'")),
        },
        [] => bad("zeroOS: missing command".into()),
        [command, ..] if !matches!(command.as_str(), "check" | "test" | "build" | "run") => {
            bad(format!("zeroOS: unsupported command '{command}'"))
        }
        _ => bad("zeroOS: invalid arguments".into()),
    }
}

fn execute(action: Action) -> Result<(), (u8, String)> {
    match action {
        Action::Check => check().map_err(failed),
        Action::Test(None) => {
            command("cargo", &["test", "--workspace", "--locked"]).map_err(failed)
        }
        Action::Build(arch) => build(arch).map_err(failed),
        Action::Run(arch) => {
            ensure_native(arch).map_err(failed)?;
            if !["zeroos.img", "kernel.efi", "init"]
                .iter()
                .all(|name| artifacts(arch).join(name).is_file())
            {
                build(arch).map_err(failed)?;
            }
            run_qemu(arch, false).map_err(failed)
        }
        Action::Test(Some(arch)) => test_arch(arch).map_err(failed),
    }
}

fn failed(message: String) -> (u8, String) {
    (1, message)
}

fn ensure_native(arch: Arch) -> Result<(), String> {
    ensure_native_for(arch, env::consts::ARCH)
}

fn ensure_native_for(arch: Arch, host_name: &str) -> Result<(), String> {
    let host = native_arch(host_name)
        .ok_or_else(|| format!("zeroOS: unsupported host architecture {}", host_name))?;
    if host == arch {
        Ok(())
    } else {
        Err(format!(
            "zeroOS: requested {} on {} host; M1 is native-only",
            arch.name(),
            host.name()
        ))
    }
}

fn native_arch(value: &str) -> Option<Arch> {
    match value {
        "x86_64" => Some(Arch::X86_64),
        "aarch64" => Some(Arch::Aarch64),
        _ => None,
    }
}

fn artifacts(arch: Arch) -> PathBuf {
    Path::new("target/m3").join(arch.name())
}

fn build(arch: Arch) -> Result<(), String> {
    ensure_native(arch)?;
    let output_dir = artifacts(arch);
    fs::create_dir_all(&output_dir).map_err(error_string)?;
    let source = fetch_linux()?;
    build_init(arch, &output_dir)?;
    build_kernel(arch, &source, &output_dir)?;
    build_selector(arch, &output_dir)?;
    package_disk(arch, &output_dir)?;
    inspect(arch, &output_dir)?;
    print_hashes(arch, &output_dir)
}

fn fetch_linux() -> Result<PathBuf, String> {
    let (url, expected) =
        linux_source(&fs::read_to_string("policy/sources.lock").map_err(error_string)?)?;
    let cache = Path::new("target/m3/sources");
    fs::create_dir_all(cache).map_err(error_string)?;
    let archive = cache.join(format!("linux-{LINUX_VERSION}.tar.xz"));
    if !archive.is_file() {
        let partial = archive.with_extension("tar.xz.part");
        let mut curl = Command::new("curl");
        curl.args(["--fail", "--location", "--output"])
            .arg(&partial)
            .arg(&url);
        run(&mut curl, "download Linux")?;
        fs::rename(partial, &archive).map_err(error_string)?;
    }
    verify_checksum(&archive, &expected)?;
    let source = cache.join(format!("linux-{LINUX_VERSION}"));
    if !source.join("Makefile").is_file()
        || !source.join("scripts/Kbuild.include").is_file()
        || !source.join("usr/Kconfig").is_file()
        || !source.join("drivers/cpuidle/Kconfig").is_file()
    {
        if source.exists() {
            let quarantine = (1..100)
                .map(|number| cache.join(format!("linux-{LINUX_VERSION}.incomplete-{number}")))
                .find(|path| !path.exists())
                .ok_or_else(|| "zeroOS: too many incomplete Linux source trees".to_owned())?;
            fs::rename(&source, quarantine).map_err(error_string)?;
        }
        let mut tar = Command::new("tar");
        tar.args(["-xf"]).arg(&archive).args(["-C"]).arg(cache);
        run(&mut tar, "extract Linux")?;
    }
    source.canonicalize().map_err(error_string)
}

fn linux_source(input: &str) -> Result<(String, String), String> {
    input
        .lines()
        .skip(1)
        .find_map(|line| {
            let fields: Vec<_> = line.split(',').collect();
            (fields.len() == 4 && fields[0] == "linux" && fields[1] == LINUX_VERSION)
                .then(|| (fields[2].to_owned(), fields[3].to_owned()))
        })
        .ok_or_else(|| format!("zeroOS: Linux {LINUX_VERSION} is not pinned"))
}

fn verify_checksum(path: &Path, expected: &str) -> Result<(), String> {
    let actual = output_path("sha256sum", &[path])?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    if checksum_matches(&actual, expected) {
        Ok(())
    } else {
        Err(format!(
            "zeroOS: checksum mismatch for {}: expected {expected}, got {actual}",
            path.display()
        ))
    }
}

fn checksum_matches(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

fn build_init(arch: Arch, output_dir: &Path) -> Result<(), String> {
    let mut cargo = Command::new("cargo");
    cargo.args([
        "build",
        "--release",
        "--locked",
        "--package",
        "zeroos-init",
        "--target",
        arch.rust_target(),
    ]);
    run(&mut cargo, "build /init")?;
    let built = Path::new("target")
        .join(arch.rust_target())
        .join("release/init");
    fs::copy(&built, output_dir.join("init")).map_err(error_string)?;
    let mut updater = Command::new("cargo");
    updater.args([
        "build",
        "--release",
        "--locked",
        "--package",
        "zeroos-updater",
        "--target",
        arch.rust_target(),
    ]);
    run(&mut updater, "build updater")?;
    fs::copy(
        Path::new("target")
            .join(arch.rust_target())
            .join("release/zeroos-update"),
        output_dir.join("zeroos-update"),
    )
    .map_err(error_string)?;
    verify_static(&output_dir.join("init"))?;
    verify_static(&output_dir.join("zeroos-update"))?;
    let mut data = Command::new("cargo");
    data.args([
        "build",
        "--release",
        "--locked",
        "--package",
        "zeroos-data",
        "--target",
        arch.rust_target(),
    ]);
    run(&mut data, "build data tool")?;
    fs::copy(
        Path::new("target")
            .join(arch.rust_target())
            .join("release/zeroos-data"),
        output_dir.join("zeroos-data"),
    )
    .map_err(error_string)?;
    verify_static(&output_dir.join("zeroos-data"))?;
    let mut manifest = format!(
        "dir /dev 0755 0 0\nnod /dev/console 0600 0 0 c 5 1\nnod /dev/null 0666 0 0 c 1 3\ndir /proc 0555 0 0\nfile /init {} 0755 0 0\nfile /zeroos-update {} 0755 0 0\nfile /zeroos-data {} 0755 0 0\n",
        output_dir
            .join("init")
            .canonicalize()
            .map_err(error_string)?
            .display(),
        output_dir
            .join("zeroos-update")
            .canonicalize()
            .map_err(error_string)?
            .display(),
        output_dir
            .join("zeroos-data")
            .canonicalize()
            .map_err(error_string)?
            .display()
    );
    manifest.push_str(&runtime_files_manifest()?);
    fs::write(output_dir.join("initramfs.list"), manifest).map_err(error_string)
}

fn runtime_files_manifest() -> Result<String, String> {
    let mut files = std::collections::BTreeSet::new();
    for program in ["cryptsetup", "mkfs.ext4", "e2fsck"] {
        let path = ["/usr/bin", "/usr/sbin", "/bin", "/sbin"]
            .into_iter()
            .map(|dir| Path::new(dir).join(program))
            .find(|path| path.is_file())
            .ok_or_else(|| format!("zeroOS: runtime tool {program} not found"))?;
        files.insert(path.clone());
        for word in output_path("ldd", &[&path])?.split_whitespace() {
            if word.starts_with('/') {
                files.insert(PathBuf::from(word));
            }
        }
    }
    let mut directories = std::collections::BTreeSet::new();
    for path in &files {
        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir == Path::new("/") {
                break;
            }
            directories.insert(dir.to_path_buf());
            parent = dir.parent();
        }
    }
    let mut manifest = String::new();
    for directory in directories {
        manifest.push_str(&format!("dir {} 0755 0 0\n", directory.display()));
    }
    for path in files {
        manifest.push_str(&format!(
            "file {} {} 0755 0 0\n",
            path.display(),
            path.display()
        ));
    }
    Ok(manifest)
}

fn verify_static(path: &Path) -> Result<(), String> {
    let headers = output_path("readelf", &[Path::new("-l"), path])?;
    if headers.contains("INTERP") || headers.contains("program interpreter") {
        Err(format!(
            "zeroOS: {} has a dynamic interpreter",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn build_selector(arch: Arch, output_dir: &Path) -> Result<(), String> {
    let mut cargo = Command::new("cargo");
    cargo.args([
        "build",
        "--release",
        "--locked",
        "--package",
        "zeroos-selector",
        "--target",
        arch.uefi_target(),
    ]);
    run(&mut cargo, "build UEFI selector")?;
    fs::copy(
        Path::new("target")
            .join(arch.uefi_target())
            .join("release/zeroos-selector.efi"),
        output_dir.join("selector.efi"),
    )
    .map_err(error_string)?;
    Ok(())
}

fn build_kernel(arch: Arch, source: &Path, output_dir: &Path) -> Result<(), String> {
    let build_dir = output_dir.join("linux-build");
    fs::create_dir_all(&build_dir).map_err(error_string)?;
    let build_dir = build_dir.canonicalize().map_err(error_string)?;
    let fragment = Path::new("kernel")
        .join(format!("{}.config", arch.name()))
        .canonicalize()
        .map_err(error_string)?;
    let manifest = output_dir
        .join("initramfs.list")
        .canonicalize()
        .map_err(error_string)?;

    kernel_make(source, arch, &build_dir, &["defconfig"])?;
    let config = build_dir.join(".config");
    let mut merge = Command::new(source.join("scripts/kconfig/merge_config.sh"));
    merge
        .env("ARCH", arch.kernel_arch())
        .args(["-m", "-O"])
        .arg(&build_dir)
        .arg(&config)
        .arg(fragment)
        .current_dir(source);
    run(&mut merge, "merge Linux configuration")?;

    let mut config_tool = Command::new(source.join("scripts/config"));
    config_tool
        .args(["--file"])
        .arg(&config)
        .args(["--set-str", "INITRAMFS_SOURCE"])
        .arg(&manifest);
    run(&mut config_tool, "set initramfs source")?;
    kernel_make(source, arch, &build_dir, &["olddefconfig"])?;
    kernel_make(
        source,
        arch,
        &build_dir,
        &[
            &format!(
                "-j{}",
                thread::available_parallelism().map_or(1, usize::from)
            ),
            arch.kernel_target(),
        ],
    )?;
    fs::copy(
        build_dir.join(arch.kernel_image()),
        output_dir.join("kernel.efi"),
    )
    .map_err(error_string)?;
    let compiler =
        fs::read_to_string(build_dir.join("include/generated/compile.h")).map_err(error_string)?;
    if !compiler.contains("clang version 19.1.7") || !compiler.contains("LLD 19.1.7") {
        return Err("zeroOS: Linux was not built with pinned Clang and LLD".into());
    }
    Ok(())
}

fn kernel_make(source: &Path, arch: Arch, build_dir: &Path, args: &[&str]) -> Result<(), String> {
    let mut make = Command::new("make");
    make.arg("-C")
        .arg(source)
        .arg(format!("O={}", build_dir.display()))
        .args([
            "LLVM=1",
            "KBUILD_BUILD_USER=zeroos",
            "KBUILD_BUILD_HOST=zeroos",
            "KBUILD_BUILD_VERSION=1",
        ])
        .args([
            "KBUILD_BUILD_TIMESTAMP=2026-08-05 00:00:00 UTC",
            "LOCALVERSION=-zeroos",
        ])
        .env("ARCH", arch.kernel_arch())
        .env("SOURCE_DATE_EPOCH", "1785888000")
        .env("KCONFIG_NOTIMESTAMP", "1")
        .args(args);
    run(&mut make, "build Linux")
}

fn package_disk(arch: Arch, output_dir: &Path) -> Result<(), String> {
    let image = output_dir.join("zeroos.img");
    if image.exists() {
        fs::remove_file(&image).map_err(error_string)?;
    }
    let mut truncate = Command::new("truncate");
    truncate.args(["-s", "512M"]).arg(&image);
    run(&mut truncate, "allocate disk image")?;

    let mut gpt = Command::new("sgdisk");
    gpt.args([
        "--clear",
        "--disk-guid=5A45524F-4F53-4D31-8000-000000000001",
        "--new=1:2048:34815",
        "--typecode=1:EF00",
        "--change-name=1:ZEROOS-ESP",
        "--partition-guid=1:5A45524F-4F53-4D31-8000-000000000002",
        "--new=2:34816:231423",
        "--typecode=2:8300",
        "--change-name=2:ZEROOS-A",
        "--partition-guid=2:5A45524F-4F53-4D33-8000-000000000002",
        "--new=3:231424:428031",
        "--typecode=3:8300",
        "--change-name=3:ZEROOS-B",
        "--partition-guid=3:5A45524F-4F53-4D33-8000-000000000003",
        "--new=4:428032:624639",
        "--typecode=4:8300",
        "--change-name=4:ZEROOS-RECOVERY",
        "--partition-guid=4:5A45524F-4F53-4D33-8000-000000000004",
        "--new=5:624640:626687",
        "--typecode=5:8300",
        "--change-name=5:ZEROOS-STATE",
        "--partition-guid=5:5A45524F-4F53-4D33-8000-000000000005",
        "--new=6:626688:1048542",
        "--typecode=6:8309",
        "--change-name=6:ZEROOS-DATA",
        "--partition-guid=6:5A45524F-4F53-4D33-8000-000000000006",
    ])
    .arg(&image);
    run(&mut gpt, "create GPT")?;

    let spec = format!("{}@@{ESP_OFFSET}", image.display());
    let mut format = Command::new("mformat");
    format.env("MTOOLS_SKIP_CHECK", "1").args([
        "-i",
        &spec,
        "-T",
        ESP_SECTORS,
        "-N",
        "5A45524F",
        "::",
    ]);
    run(&mut format, "format ESP")?;

    let staging = output_dir.join("esp-root");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(error_string)?;
    }
    let boot = staging.join("EFI/BOOT");
    fs::create_dir_all(&boot).map_err(error_string)?;
    fs::copy(output_dir.join("selector.efi"), boot.join(arch.fallback())).map_err(error_string)?;
    let mut touch = Command::new("touch");
    touch
        .args(["-t", "202601010000"])
        .arg(&staging)
        .arg(staging.join("EFI"))
        .arg(&boot)
        .arg(boot.join(arch.fallback()));
    run(&mut touch, "normalize ESP timestamps")?;
    let mut copy = Command::new("mcopy");
    copy.env("MTOOLS_SKIP_CHECK", "1")
        .args(["-m", "-s", "-i", &spec])
        .arg(staging.join("EFI"))
        .arg("::/");
    run(&mut copy, "populate ESP")?;
    for (name, seek) in [
        ("system-a.efi", "34816"),
        ("system-b.efi", "231424"),
        ("recovery.efi", "428032"),
    ] {
        make_development_slot(arch, &output_dir.join("kernel.efi"), &output_dir.join(name))?;
        let mut dd = Command::new("dd");
        dd.arg(format!("if={}", output_dir.join(name).display()))
            .arg(format!("of={}", image.display()))
            .args([
                "bs=512",
                &format!("seek={seek}"),
                "conv=notrunc",
                "status=none",
            ]);
        run(&mut dd, "populate raw boot slot")?;
    }
    Ok(())
}

fn make_development_slot(arch: Arch, payload: &Path, output_file: &Path) -> Result<(), String> {
    let size = fs::metadata(payload).map_err(error_string)?.len();
    let digest = output_path("sha256sum", &[payload])?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    let manifest = format!(
        "version=1\narch={}\nsequence=0\npayload-size={size}\nsha256={digest}\nsigner=development-unsigned\n",
        arch.name()
    );
    let mut output = fs::File::create(output_file).map_err(error_string)?;
    output.write_all(b"ZEROSLT1").map_err(error_string)?;
    output
        .write_all(&(manifest.len() as u32).to_le_bytes())
        .map_err(error_string)?;
    output
        .write_all(manifest.as_bytes())
        .map_err(error_string)?;
    output
        .write_all(&[0; zeroos_storage::SIGNATURE_BYTES])
        .map_err(error_string)?;
    std::io::copy(
        &mut fs::File::open(payload).map_err(error_string)?,
        &mut output,
    )
    .map_err(error_string)?;
    output.sync_all().map_err(error_string)
}

fn inspect(arch: Arch, output_dir: &Path) -> Result<(), String> {
    let image = output_dir.join("zeroos.img");
    if fs::metadata(&image).map_err(error_string)?.len() != zeroos_storage::IMAGE_BYTES {
        return Err("zeroOS: disk image is not exactly 512 MiB".into());
    }
    let bytes = fs::read(&image).map_err(error_string)?;
    validate_signatures(&bytes)?;
    let mut verify = Command::new("sgdisk");
    verify.arg("--verify").arg(&image);
    run(&mut verify, "verify GPT")?;
    for partition in zeroos_storage::PARTITIONS {
        let flag = format!("--info={}", partition.number);
        let info = output_path("sgdisk", &[Path::new(&flag), &image])?;
        if !info.contains(&format!("First sector: {}", partition.first))
            || !info.contains(&format!("Last sector: {}", partition.last))
            || !info.contains(&format!("Partition name: '{}'", partition.name))
            || (partition.number == 1 && !info.contains("EFI system partition"))
        {
            return Err(format!(
                "zeroOS: invalid GPT partition {}",
                partition.number
            ));
        }
    }
    let esp = output_dir.join("esp.fat");
    let mut dd = Command::new("dd");
    dd.arg(format!("if={}", image.display()))
        .arg(format!("of={}", esp.display()))
        .args(["bs=512", "skip=2048", "count=32768", "status=none"]);
    run(&mut dd, "extract ESP for inspection")?;
    let mut fat = Command::new("fsck.fat");
    fat.args(["-n"]).arg(&esp);
    run(&mut fat, "verify FAT")?;
    fs::remove_file(esp).map_err(error_string)?;

    let extracted = output_dir.join("packaged-kernel.efi");
    if extracted.exists() {
        fs::remove_file(&extracted).map_err(error_string)?;
    }
    let spec = format!("{}@@{ESP_OFFSET}", image.display());
    let guest_path = format!("::/EFI/BOOT/{}", arch.fallback());
    let mut copy = Command::new("mcopy");
    copy.env("MTOOLS_SKIP_CHECK", "1")
        .args(["-i", &spec, &guest_path])
        .arg(&extracted);
    run(&mut copy, "extract fallback EFI file")?;
    identical_files(&extracted, &output_dir.join("selector.efi"))?;
    verify_static(&output_dir.join("init"))?;
    let manifest = fs::read_to_string(output_dir.join("initramfs.list")).map_err(error_string)?;
    if !manifest.starts_with("dir /dev 0755 0 0\nnod /dev/console 0600 0 0 c 5 1\n")
        || !manifest.contains("nod /dev/null 0666 0 0 c 1 3\n")
        || !manifest.contains("file /init ")
        || !manifest.contains("file /zeroos-update ")
        || !manifest.contains("file /zeroos-data ")
        || !manifest.ends_with(" 0755 0 0\n")
    {
        return Err("zeroOS: invalid initramfs manifest".into());
    }
    Ok(())
}

fn validate_signatures(bytes: &[u8]) -> Result<(), String> {
    if bytes.get(512..520) != Some(b"EFI PART") {
        return Err("zeroOS: corrupt GPT signature".into());
    }
    let fat_signature = ESP_OFFSET as usize + 510;
    if bytes.get(fat_signature..fat_signature + 2) != Some(&[0x55, 0xaa]) {
        return Err("zeroOS: corrupt FAT signature".into());
    }
    Ok(())
}

fn test_arch(arch: Arch) -> Result<(), String> {
    build(arch)?;
    let output_dir = artifacts(arch);
    let first = output_dir.join("zeroos.first.img");
    fs::copy(output_dir.join("zeroos.img"), &first).map_err(error_string)?;
    package_disk(arch, &output_dir)?;
    identical_files(&first, &output_dir.join("zeroos.img"))?;
    fs::remove_file(first).map_err(error_string)?;
    inspect(arch, &output_dir)?;
    run_qemu(arch, true)
}

fn run_qemu(arch: Arch, capture: bool) -> Result<(), String> {
    let output_dir = artifacts(arch);
    let (code, vars_template) = firmware(arch)?;
    let vars = output_dir.join("uefi-vars.fd");
    fs::copy(vars_template, &vars).map_err(error_string)?;
    let image = output_dir.join("zeroos.img");
    let mut qemu = Command::new(arch.qemu());
    qemu.args(["-no-reboot", "-nographic", "-m", "512"]);
    match arch {
        Arch::X86_64 => {
            qemu.args(["-machine", "q35,accel=tcg", "-cpu", "max"]);
        }
        Arch::Aarch64 => {
            qemu.args(["-machine", "virt,accel=tcg", "-cpu", "max"]);
        }
    }
    qemu.arg("-drive")
        .arg(format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        ))
        .arg("-drive")
        .arg(format!("if=pflash,format=raw,file={}", vars.display()))
        .arg("-drive")
        .arg(format!(
            "if=none,format=raw,id=zeroos,file={}",
            image.display()
        ))
        .args(["-device", "virtio-blk-pci,drive=zeroos"]);

    if !capture {
        return run(&mut qemu, "run QEMU");
    }
    qemu.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = qemu.spawn().map_err(error_string)?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| "zeroOS: piped QEMU stdin unavailable".to_owned())?;
    let (lines, received) = mpsc::channel();
    let readers = vec![
        qemu_reader(
            child
                .stdout
                .take()
                .ok_or_else(|| "zeroOS: piped QEMU stdout unavailable".to_owned())?,
            lines.clone(),
        ),
        qemu_reader(
            child
                .stderr
                .take()
                .ok_or_else(|| "zeroOS: piped QEMU stderr unavailable".to_owned())?,
            lines.clone(),
        ),
    ];
    drop(lines);
    let start = Instant::now();
    let mut output = String::new();
    let mut sent_selftest = false;
    let mut sent_logs = false;
    let mut recovery_code = None;
    let status = loop {
        while let Ok(line) = received.try_recv() {
            output.push_str(&line);
            output.push('\n');
            if line.contains("credential-request=new-passphrase")
                || line.contains("credential-request=repeat-passphrase")
            {
                writeln!(input, "zeroos-test-passphrase").map_err(error_string)?;
                input.flush().map_err(error_string)?;
            }
            if let Some(code) = line.strip_prefix("Recovery code (store offline): ") {
                recovery_code = Some(code.trim().to_owned());
            }
            if line.contains("credential-request=repeat-recovery-code") {
                let code = recovery_code.as_deref().ok_or_else(|| {
                    "zeroOS: recovery code prompt preceded generated code".to_owned()
                })?;
                writeln!(input, "{code}").map_err(error_string)?;
                input.flush().map_err(error_string)?;
            }
            if line.trim_end_matches('\r') == READY && !sent_selftest {
                writeln!(input, "selftest").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_selftest = true;
            }
            if line.contains("SELFTEST PASS") && !sent_logs {
                writeln!(input, "logs").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_logs = true;
            }
        }
        if let Some(status) = child.try_wait().map_err(error_string)? {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(90) {
            child.kill().map_err(error_string)?;
            child.wait().map_err(error_string)?;
            thread::sleep(Duration::from_millis(50));
            for line in received.try_iter() {
                output.push_str(&line);
                output.push('\n');
            }
            fs::write(output_dir.join("qemu.log"), redact_recovery_codes(&output))
                .map_err(error_string)?;
            return Err(format!(
                "zeroOS: QEMU acceptance timed out after 90s; output ended with {:?}",
                output.lines().rev().take(8).collect::<Vec<_>>()
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    for reader in readers {
        reader
            .join()
            .map_err(|_| "zeroOS: QEMU reader failed".to_owned())?;
    }
    for line in received.try_iter() {
        output.push_str(&line);
        output.push('\n');
    }
    fs::write(output_dir.join("qemu.log"), redact_recovery_codes(&output)).map_err(error_string)?;
    if !status.success() {
        return Err(format!("zeroOS: QEMU failed with {status}"));
    }
    verify_runtime_acceptance(&output)
}

fn redact_recovery_codes(output: &str) -> String {
    output
        .lines()
        .map(|line| {
            if line.contains("Recovery code (store offline): ") {
                "Recovery code (store offline): [REDACTED]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn qemu_reader(
    stream: impl std::io::Read + Send + 'static,
    lines: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let _ = lines.send(line);
        }
    })
}

fn verify_runtime_acceptance(output: &str) -> Result<(), String> {
    verify_readiness(output)?;
    for evidence in [
        "\tselftest\trestart-before-dependent\tpass",
        "\tbase\tfixture-online\tgeneration=1",
        "\tcore\torphan-reaped\tpid=",
        "\tflaky\tpermanent-failure\trestart-limit=3 window-seconds=10",
        "\tselftest\tv2-rejected\tERR ZEROOS/1 UNSUPPORTED_VERSION supported=1 unchanged=true",
        "\tselftest\tfailure-isolation\tindependent-running=true",
        "\tselftest\tadministrative-recovery\tpass",
        "OK ZEROOS/1 LOGS count=",
        "SELFTEST PASS",
        "\tcore\tshutdown-complete\tstate-synced=true",
    ] {
        if !output.contains(evidence) {
            return Err(format!("zeroOS: missing QEMU evidence {evidence:?}"));
        }
    }
    let shutdown = output
        .rfind("\tcore\tshutdown-started\t")
        .ok_or_else(|| "zeroOS: missing shutdown start".to_owned())?;
    let tail = &output[shutdown..];
    let mut previous = 0;
    for service in ["independent", "dependent", "flaky", "base"] {
        let position = tail
            .find(&format!("\t{service}\tstop-sent\t"))
            .ok_or_else(|| format!("zeroOS: shutdown did not stop {service}"))?;
        if position < previous {
            return Err("zeroOS: services did not stop in reverse dependency order".into());
        }
        previous = position;
    }
    Ok(())
}

fn firmware(arch: Arch) -> Result<(PathBuf, PathBuf), String> {
    let candidates: &[(&str, &str)] = match arch {
        Arch::X86_64 => &[
            (
                "/usr/share/OVMF/OVMF_CODE.fd",
                "/usr/share/OVMF/OVMF_VARS.fd",
            ),
            (
                "/usr/share/OVMF/OVMF_CODE_4M.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.fd",
            ),
        ],
        Arch::Aarch64 => &[
            (
                "/usr/share/AAVMF/AAVMF_CODE.fd",
                "/usr/share/AAVMF/AAVMF_VARS.fd",
            ),
            (
                "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
                "/usr/share/AAVMF/AAVMF_VARS.fd",
            ),
        ],
    };
    candidates
        .iter()
        .find(|(code, vars)| Path::new(code).is_file() && Path::new(vars).is_file())
        .map(|(code, vars)| (PathBuf::from(code), PathBuf::from(vars)))
        .ok_or_else(|| format!("zeroOS: {} UEFI firmware not found", arch.name()))
}

#[cfg(test)]
fn run_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<std::process::ExitStatus, String> {
    let mut child = command.spawn().map_err(error_string)?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(error_string)? {
            return Ok(status);
        }
        if start.elapsed() >= timeout {
            child.kill().map_err(error_string)?;
            child.wait().map_err(error_string)?;
            return Err(format!(
                "zeroOS: command timed out after {}s",
                timeout.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn verify_readiness(output: &str) -> Result<(), String> {
    let count = output.lines().filter(|line| *line == READY).count();
    if count == 1 {
        Ok(())
    } else {
        Err(format!(
            "zeroOS: expected one complete readiness line, found {count}"
        ))
    }
}

fn print_hashes(arch: Arch, output_dir: &Path) -> Result<(), String> {
    for name in [
        "selector.efi",
        "system-a.efi",
        "system-b.efi",
        "recovery.efi",
        "init",
        "zeroos-update",
        "zeroos.img",
    ] {
        println!(
            "zeroOS {} {name}: {}",
            arch.name(),
            output_path("sha256sum", &[&output_dir.join(name)])?
        );
    }
    Ok(())
}

fn run(command: &mut Command, description: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("zeroOS: failed to {description}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("zeroOS: failed to {description}: {status}"))
    }
}

fn command(program: &str, args: &[&str]) -> Result<(), String> {
    run(Command::new(program).args(args), &format!("run {program}"))
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let result = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("zeroOS: failed to start {program}: {error}"))?;
    if !result.status.success() {
        return Err(format!("zeroOS: {program} {} failed", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn output_path(program: &str, args: &[&Path]) -> Result<String, String> {
    let result = Command::new(program)
        .args(args)
        .output()
        .map_err(error_string)?;
    if !result.status.success() {
        return Err(format!("zeroOS: {program} failed"));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn check() -> Result<(), String> {
    validate_repository(Path::new("."))?;
    check_tools()?;
    command("cargo", &["fmt", "--all", "--", "--check"])?;
    command(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    command("cargo", &["test", "--workspace", "--locked"])?;
    command("cargo", &["deny", "check"])?;
    reproducible_build()
}

fn check_tools() -> Result<(), String> {
    for (program, args, expected) in [
        ("rustc", &["--version"][..], "rustc 1.97.1"),
        ("cargo", &["--version"][..], "cargo 1.97.1"),
        ("clang-19", &["--version"][..], "19.1.7"),
        ("ld.lld-19", &["--version"][..], "19.1.7"),
        ("dpkg-query", &["-W", "llvm-19"][..], "19.1.7"),
        ("cargo", &["deny", "--version"][..], "0.19.4"),
        ("dpkg-query", &["-W", "musl-tools"][..], "1.2.5"),
        ("qemu-system-x86_64", &["--version"][..], "QEMU emulator"),
        ("qemu-system-aarch64", &["--version"][..], "QEMU emulator"),
        ("sgdisk", &["--version"][..], "GPT fdisk"),
        ("mformat", &["-V"][..], "mtools"),
    ] {
        let actual = output(program, args)?;
        if !actual.contains(expected) {
            return Err(format!(
                "zeroOS: expected {program} {expected}, got {actual}"
            ));
        }
    }
    let image = env::var("ZEROOS_BUILD_IMAGE")
        .map_err(|_| "zeroOS: run check in the pinned build image".to_owned())?;
    let expected = fs::read_to_string("policy/build-image.lock").map_err(error_string)?;
    if image != expected.trim() {
        return Err(format!("zeroOS: build image mismatch: {image}"));
    }
    println!("zeroOS build image: {image}");
    Ok(())
}

fn reproducible_build() -> Result<(), String> {
    let root = env::temp_dir().join(format!("zeroos-repro-{}", std::process::id()));
    let first = root.join("first");
    let second = root.join("second");
    let result = (|| {
        build_into(&first)?;
        build_into(&second)?;
        let binary = if cfg!(windows) { "xtask.exe" } else { "xtask" };
        let first_binary = first.join("release").join(binary);
        let second_binary = second.join("release").join(binary);
        identical_files(&first_binary, &second_binary)?;
        println!(
            "zeroOS release xtask: {}",
            output_path("sha256sum", &[&first_binary])?
        );
        Ok(())
    })();
    let _ = fs::remove_dir_all(root);
    result
}

fn identical_files(first: &Path, second: &Path) -> Result<(), String> {
    if fs::read(first).map_err(error_string)? == fs::read(second).map_err(error_string)? {
        Ok(())
    } else {
        Err(format!(
            "zeroOS: {} and {} differ",
            first.display(),
            second.display()
        ))
    }
}

fn build_into(target: &Path) -> Result<(), String> {
    let mut cargo = Command::new("cargo");
    cargo
        .args(["build", "--release", "--locked", "--package", "xtask"])
        .env("CARGO_TARGET_DIR", target);
    run(&mut cargo, "build clean release xtask")
}

fn validate_repository(root: &Path) -> Result<(), String> {
    validate_ai_policy(root)?;
    validate_build_inputs(root)?;
    validate_manifest(&fs::read_to_string(root.join("Cargo.toml")).map_err(error_string)?)?;
    let ledger = fs::read_to_string(root.join("policy/dependencies.csv")).map_err(error_string)?;
    let admitted = validate_ledger(&ledger)?;
    let lock = fs::read_to_string(root.join("Cargo.lock")).map_err(error_string)?;
    validate_cargo_lock(&lock, &admitted)?;
    let sources = fs::read_to_string(root.join("policy/sources.lock")).map_err(error_string)?;
    validate_source_lock(&sources)?;
    linux_source(&sources).map(|_| ())
}

fn validate_build_inputs(root: &Path) -> Result<(), String> {
    let docker = fs::read_to_string(root.join("Dockerfile")).map_err(error_string)?;
    let image = fs::read_to_string(root.join("policy/build-image.lock")).map_err(error_string)?;
    let expected_from = format!("FROM {}", image.trim());
    if docker.lines().next() != Some(expected_from.as_str())
        || !docker.lines().any(|line| {
            line.strip_prefix("ARG DEBIAN_SNAPSHOT=")
                .is_some_and(|value| {
                    value.len() == 16
                        && value.ends_with('Z')
                        && value.as_bytes().get(8) == Some(&b'T')
                        && value[..8].bytes().all(|byte| byte.is_ascii_digit())
                        && value[9..15].bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        || !docker.contains("cargo install --locked cargo-deny --version 0.19.4")
    {
        return Err(
            "zeroOS: Docker build inputs must pin image digest, Debian snapshot, and cargo-deny"
                .into(),
        );
    }
    for workflow in [
        ".github/workflows/check.yml",
        ".github/workflows/release.yml",
    ] {
        let input = fs::read_to_string(root.join(workflow)).map_err(error_string)?;
        for (index, line) in input.lines().enumerate() {
            let Some(action) = line.trim().strip_prefix("- uses: ") else {
                continue;
            };
            let Some((_, revision)) = action.split_once('@') else {
                return Err(format!("zeroOS: {workflow}:{} unpinned action", index + 1));
            };
            let revision = revision.split_whitespace().next().unwrap_or_default();
            if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!(
                    "zeroOS: {workflow}:{} action must use a 40-hex commit pin",
                    index + 1
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
struct AiPolicy {
    rust_roots: Vec<String>,
    architecture_roots: HashSet<String>,
    excluded_directories: HashSet<String>,
    generated_extensions: Vec<String>,
}

fn validate_ai_policy(root: &Path) -> Result<(), String> {
    let config = parse_ai_policy(
        &fs::read_to_string(root.join("policy/ai-policy.conf"))
            .map_err(|error| format!("zeroOS: cannot read policy/ai-policy.conf: {error}"))?,
    )?;
    let mut rust_files = Vec::new();
    for path in &config.rust_roots {
        let directory = root.join(path);
        if !directory.is_dir() {
            return Err(format!("zeroOS: policy rust root does not exist: {path}"));
        }
        collect_rust_files(&directory, &config.excluded_directories, &mut rust_files)?;
    }
    let mut violations = Vec::new();
    for path in rust_files {
        scan_rust(root, &path, &config, &mut violations)?;
    }
    validate_generated_artifacts(root, &config, &mut violations)?;
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "zeroOS: AI policy violations:\n{}",
            violations.join("\n")
        ))
    }
}

fn parse_ai_policy(input: &str) -> Result<AiPolicy, String> {
    let mut values = BTreeMap::new();
    for (index, line) in input.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!(
                "zeroOS: invalid AI policy line {}: expected key=value",
                index + 1
            )
        })?;
        if key.is_empty() || value.is_empty() || values.insert(key, value).is_some() {
            return Err(format!("zeroOS: invalid AI policy line {}", index + 1));
        }
    }
    let expected = [
        "architecture-roots",
        "excluded-directories",
        "generated-extensions",
        "rust-roots",
        "version",
    ];
    if values.keys().copied().collect::<Vec<_>>() != expected {
        return Err("zeroOS: AI policy has missing or unknown keys".into());
    }
    if values["version"] != "1" {
        return Err("zeroOS: unsupported AI policy version".into());
    }
    let list = |key: &str| -> Result<Vec<String>, String> {
        let fields: Vec<_> = values[key].split(',').map(str::to_owned).collect();
        if fields.iter().any(|field| {
            field.is_empty()
                || field.starts_with('/')
                || field.contains("..")
                || field.contains(char::is_whitespace)
        }) {
            return Err(format!("zeroOS: invalid AI policy value for {key}"));
        }
        Ok(fields)
    };
    let rust_roots = list("rust-roots")?;
    let architecture_roots: HashSet<String> = list("architecture-roots")?.into_iter().collect();
    if !architecture_roots
        .iter()
        .all(|path| rust_roots.contains(path))
    {
        return Err("zeroOS: architecture roots must be Rust roots".into());
    }
    Ok(AiPolicy {
        rust_roots,
        architecture_roots,
        excluded_directories: list("excluded-directories")?.into_iter().collect(),
        generated_extensions: list("generated-extensions")?,
    })
}

fn collect_rust_files(
    directory: &Path,
    excluded: &HashSet<String>,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(error_string)? {
        let entry = entry.map_err(error_string)?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !excluded.contains(name.as_ref()) {
                collect_rust_files(&path, excluded, output)?;
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            output.push(path);
        }
    }
    Ok(())
}

fn scan_rust(
    root: &Path,
    path: &Path,
    config: &AiPolicy,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(error_string)?;
    let code = rust_code_only(&source)?;
    let relative = path.strip_prefix(root).map_err(error_string)?;
    let root_name = relative
        .components()
        .next()
        .and_then(|part| part.as_os_str().to_str())
        .ok_or_else(|| format!("zeroOS: invalid policy path {}", relative.display()))?;
    let rules = [
        (
            ".unwrap()",
            "RUST_NO_UNWRAP",
            "propagate with ? or match explicitly",
        ),
        (
            ".expect(",
            "RUST_NO_EXPECT",
            "propagate with ? or match explicitly",
        ),
        (
            "todo!(",
            "RUST_NO_TODO",
            "implement the required behavior or report a blocker",
        ),
        (
            "unimplemented!(",
            "RUST_NO_UNIMPLEMENTED",
            "implement the required behavior or report a blocker",
        ),
        ("panic!(", "RUST_NO_PANIC", "return a structured error"),
        (
            "unreachable!(",
            "RUST_NO_UNREACHABLE",
            "handle the state explicitly",
        ),
        (
            "unreachable_unchecked",
            "UNSAFE_UNREACHABLE",
            "remove the unchecked unreachable assumption",
        ),
        (
            "static mut",
            "UNSAFE_STATIC_MUT",
            "use owned state, a lock, or an atomic",
        ),
    ];
    let original_lines: Vec<_> = source.lines().collect();
    for (index, line) in code.lines().enumerate() {
        for (needle, rule, remediation) in rules {
            if line.contains(needle) {
                violations.push(policy_diagnostic(relative, index + 1, rule, remediation));
            }
        }
        if line.contains("unsafe {") || line.contains("unsafe{") {
            let documented = original_lines[..index]
                .iter()
                .rev()
                .take_while(|line| line.trim_start().starts_with("//"))
                .any(|line| line.trim_start().starts_with("// SAFETY:"));
            if !documented {
                violations.push(policy_diagnostic(
                    relative,
                    index + 1,
                    "UNSAFE_DOCUMENTATION",
                    "add an immediately preceding falsifiable SAFETY comment",
                ));
            }
        }
        if line.contains("target_arch") && !config.architecture_roots.contains(root_name) {
            violations.push(policy_diagnostic(
                relative,
                index + 1,
                "ARCHITECTURE_BOUNDARY",
                "move target-specific code to selector/ or expose a typed capability",
            ));
        }
    }
    Ok(())
}

fn policy_diagnostic(path: &Path, line: usize, rule: &str, remediation: &str) -> String {
    format!("{}:{line}: {rule}: {remediation}", path.display())
}

fn rust_code_only(source: &str) -> Result<String, String> {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut block_depth = 0usize;
    let mut string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if block_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                block_depth += 1;
                output.extend_from_slice(b"  ");
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                block_depth -= 1;
                output.extend_from_slice(b"  ");
                index += 2;
            } else {
                output.push(if byte == b'\n' { b'\n' } else { b' ' });
                index += 1;
            }
        } else if string {
            output.push(if byte == b'\n' { b'\n' } else { b' ' });
            index += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                string = false;
            }
        } else if bytes.get(index..index + 2) == Some(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                output.push(b' ');
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            block_depth = 1;
            output.extend_from_slice(b"  ");
            index += 2;
        } else if byte == b'"'
            && !(index > 0 && bytes[index - 1] == b'\'' && bytes.get(index + 1) == Some(&b'\''))
        {
            string = true;
            output.push(b' ');
            index += 1;
        } else {
            output.push(byte);
            index += 1;
        }
    }
    if block_depth != 0 || string {
        return Err("zeroOS: policy scanner could not parse Rust comments/strings".into());
    }
    String::from_utf8(output).map_err(error_string)
}

fn validate_generated_artifacts(
    root: &Path,
    config: &AiPolicy,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    let safe = root.canonicalize().map_err(error_string)?;
    let safe = safe
        .to_str()
        .ok_or_else(|| "zeroOS: repository path is not UTF-8".to_owned())?;
    let result = Command::new("git")
        .args(["-c", &format!("safe.directory={safe}")])
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(error_string)?;
    if !result.status.success() {
        return Err("zeroOS: git ls-files failed during generated-artifact policy".into());
    }
    for bytes in result
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = String::from_utf8(bytes.to_vec()).map_err(error_string)?;
        let generated = path.split('/').any(|part| part == "target")
            || config
                .generated_extensions
                .iter()
                .any(|extension| path.ends_with(extension));
        if generated {
            violations.push(format!(
                "{path}:1: GENERATED_ARTIFACT: remove generated build/image/source output from Git"
            ));
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &str) -> Result<(), String> {
    for required in [
        "members = [\"xtask\", \"init\", \"storage\", \"updater\", \"data\", \"selector\"]",
        "edition = \"2024\"",
        "license = \"LicenseRef-zeroOS-Proprietary\"",
        "publish = false",
        "rust-version = \"1.97.1\"",
    ] {
        if !manifest.contains(required) {
            return Err(format!("zeroOS: workspace policy missing '{required}'"));
        }
    }
    Ok(())
}

fn validate_ledger(input: &str) -> Result<HashSet<(String, String)>, String> {
    let header = "package,version,need,alternatives,owner,maintenance,vulnerability_response,license,architectures,version_pin,acquisition,transitives,update_path,attack_surface,base_image,classification,replacement_trigger";
    let mut lines = input.lines();
    if lines.next() != Some(header) {
        return Err("zeroOS: invalid dependency ledger header".into());
    }
    let mut admitted = HashSet::new();
    for (index, line) in lines.filter(|line| !line.is_empty()).enumerate() {
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 17
            || fields.iter().any(|field| field.trim().is_empty())
            || !matches!(fields[14], "yes" | "no")
            || !matches!(fields[15], "Retain" | "Replace")
            || (fields[15] == "Replace" && fields[16] == "n/a")
            || (fields[15] == "Retain" && fields[16] != "n/a")
        {
            return Err(format!(
                "zeroOS: invalid dependency ledger row {}",
                index + 2
            ));
        }
        if !admitted.insert((fields[0].into(), fields[1].into())) {
            return Err(format!(
                "zeroOS: duplicate dependency {} {}",
                fields[0], fields[1]
            ));
        }
    }
    Ok(admitted)
}

fn validate_cargo_lock(lock: &str, admitted: &HashSet<(String, String)>) -> Result<(), String> {
    let mut package = (String::new(), String::new());
    let mut external = false;
    for line in lock.lines().chain(["[[package]]"]) {
        if line == "[[package]]" {
            if external && !admitted.contains(&package) {
                return Err(format!(
                    "zeroOS: unrecorded dependency {} {}",
                    package.0, package.1
                ));
            }
            package = (String::new(), String::new());
            external = false;
        } else if let Some(value) = quoted_value(line, "name = ") {
            package.0 = value;
        } else if let Some(value) = quoted_value(line, "version = ") {
            package.1 = value;
        } else if line.starts_with("source = ") {
            external = true;
        }
    }
    Ok(())
}

fn quoted_value(line: &str, prefix: &str) -> Option<String> {
    line.strip_prefix(prefix)?
        .strip_prefix('"')?
        .strip_suffix('"')
        .map(str::to_owned)
}

fn validate_source_lock(input: &str) -> Result<(), String> {
    let mut lines = input.lines();
    if lines.next() != Some("name,version,url,sha256") {
        return Err("zeroOS: invalid source lock header".into());
    }
    let mut sources = HashSet::new();
    for (index, line) in lines.filter(|line| !line.is_empty()).enumerate() {
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 4 || fields[..3].iter().any(|field| field.trim().is_empty()) {
            return Err(format!("zeroOS: invalid source lock row {}", index + 2));
        }
        if fields[3].len() != 64 || !fields[3].bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("zeroOS: malformed SHA-256 for {}", fields[0]));
        }
        if !sources.insert(fields[0]) {
            return Err(format!("zeroOS: duplicate source {}", fields[0]));
        }
    }
    Ok(())
}

fn error_string(error: impl std::fmt::Display) -> String {
    format!("zeroOS: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interface_and_architecture_mapping() {
        assert_eq!(parse(vec!["check".into()]), Ok(Action::Check));
        assert_eq!(
            parse(vec!["test".into(), "--arch".into(), "aarch64".into()]),
            Ok(Action::Test(Some(Arch::Aarch64)))
        );
        assert_eq!(native_arch("x86_64"), Some(Arch::X86_64));
        assert!(ensure_native_for(Arch::Aarch64, "x86_64").is_err());
        assert!(parse(vec!["build".into(), "--arch".into(), "mips".into()]).is_err());
        assert!(parse(vec!["build".into()]).is_err());
    }

    #[test]
    fn validates_sources_locks_and_license_policy() {
        assert!(validate_source_lock("name,version,url,sha256\n").is_ok());
        assert!(validate_source_lock("name,version,url,sha256\nlinux,1,url,nope\n").is_err());
        assert!(!checksum_matches("a", "b"));
        assert!(
            validate_cargo_lock(
                "[[package]]\nname = \"serde\"\nversion = \"1\"\nsource = \"registry+x\"\n",
                &HashSet::new()
            )
            .is_err()
        );
        assert!(validate_manifest(
            "members = [\"xtask\"]\nedition = \"2024\"\nlicense = \"MIT\"\npublish = false\nrust-version = \"1.97.1\""
        )
        .is_err());
    }

    #[test]
    fn propagates_command_failure_and_timeout() {
        assert!(command("rustc", &["--definitely-not-a-rustc-option"]).is_err());
        let mut sleep = Command::new("sh");
        sleep.args(["-c", "sleep 1"]);
        assert!(run_with_timeout(&mut sleep, Duration::from_millis(10)).is_err());
    }

    #[test]
    fn validates_readiness_and_disk_signatures() {
        assert!(verify_readiness(&format!("firmware\n{READY}\n")).is_ok());
        assert!(verify_readiness(&format!("{READY}\n{READY}\n")).is_err());
        assert!(verify_readiness(&format!("prefix {READY}\n")).is_err());
        let mut disk = vec![0; ESP_OFFSET as usize + 512];
        disk[512..520].copy_from_slice(b"EFI PART");
        disk[ESP_OFFSET as usize + 510..ESP_OFFSET as usize + 512].copy_from_slice(&[0x55, 0xaa]);
        assert!(validate_signatures(&disk).is_ok());
        disk[512] = 0;
        assert!(validate_signatures(&disk).is_err());
        disk[512] = b'E';
        disk[ESP_OFFSET as usize + 510] = 0;
        assert!(validate_signatures(&disk).is_err());
        assert_eq!(Arch::X86_64.fallback(), "BOOTX64.EFI");
        assert_eq!(Arch::Aarch64.fallback(), "BOOTAA64.EFI");
    }

    #[test]
    fn rejects_different_build_outputs() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("zeroos-test-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let first = root.join("first");
        let second = root.join("second");
        fs::write(&first, b"first")?;
        fs::write(&second, b"second")?;
        assert!(identical_files(&first, &second).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn policy_rejects_temporary_source_unsafe_and_dependency_violations()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("zeroos-policy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("owned"))?;
        let source = root.join("owned/source.rs");
        let config = AiPolicy {
            rust_roots: vec!["owned".into()],
            architecture_roots: HashSet::new(),
            excluded_directories: HashSet::new(),
            generated_extensions: vec![".img".into()],
        };
        fs::write(&source, "fn compliant() -> Result<(), ()> { Ok(()) }\n")?;
        let mut violations = Vec::new();
        scan_rust(&root, &source, &config, &mut violations)?;
        assert!(violations.is_empty());

        fs::write(&source, "fn bad(value: Option<u8>) { value.unwrap(); }\n")?;
        scan_rust(&root, &source, &config, &mut violations)?;
        assert!(
            violations
                .iter()
                .any(|item| item.contains("RUST_NO_UNWRAP"))
        );

        violations.clear();
        fs::write(
            &source,
            "fn bad() { unsafe { core::ptr::read(1 as *const u8); } }\n",
        )?;
        scan_rust(&root, &source, &config, &mut violations)?;
        assert!(
            violations
                .iter()
                .any(|item| item.contains("UNSAFE_DOCUMENTATION"))
        );

        let lock = root.join("Cargo.lock");
        fs::write(
            &lock,
            "[[package]]\nname = \"unadmitted\"\nversion = \"1.0.0\"\nsource = \"registry+x\"\n",
        )?;
        assert!(validate_cargo_lock(&fs::read_to_string(lock)?, &HashSet::new()).is_err());
        assert!(parse_ai_policy("version=1\nunknown=value\n").is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
