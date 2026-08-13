use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    io::{BufRead, BufReader, Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const USAGE: &str = "usage: cargo xtask <check|test [--arch <x86_64|aarch64>]|test-release --arch <x86_64|aarch64> --sequence <n> --url <public-tag-url>|build --arch <x86_64|aarch64>|run --arch <x86_64|aarch64>>";
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
    TestRelease {
        arch: Arch,
        sequence: u64,
        url: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BootExpectation {
    Normal,
    Recovery,
    Rejected,
}

struct CleanupDir(PathBuf);

struct CleanupChild(Child);

impl Drop for CleanupChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct AcceptanceBuild<'a> {
    origin: &'a str,
    ca_pem: &'a str,
    release_key: &'a Path,
}

impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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
        [
            command,
            arch_flag,
            arch,
            sequence_flag,
            sequence,
            url_flag,
            url,
        ] if command == "test-release"
            && arch_flag == "--arch"
            && sequence_flag == "--sequence"
            && url_flag == "--url" =>
        {
            let sequence = sequence
                .parse::<u64>()
                .ok()
                .filter(|value| *value != 0)
                .ok_or_else(|| {
                    (
                        2,
                        "zeroOS: release sequence must be a positive integer".into(),
                    )
                })?;
            Ok(Action::TestRelease {
                arch: Arch::parse(arch)?,
                sequence,
                url: url.clone(),
            })
        }
        [] => bad("zeroOS: missing command".into()),
        [command, ..]
            if !matches!(
                command.as_str(),
                "check" | "test" | "test-release" | "build" | "run"
            ) =>
        {
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
        Action::TestRelease {
            arch,
            sequence,
            url,
        } => {
            ensure_native(arch).map_err(failed)?;
            test_release(arch, sequence, &url).map_err(failed)
        }
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
    build_init(arch, &output_dir, None)?;
    build_kernel(arch, &source, &output_dir, false)?;
    build_selector(arch, &output_dir)?;
    package_disk(arch, &output_dir)?;
    verify_production_artifacts(&output_dir)?;
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

fn build_init(
    arch: Arch,
    output_dir: &Path,
    acceptance: Option<&AcceptanceBuild<'_>>,
) -> Result<(), String> {
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
    if acceptance.is_some() {
        cargo.args(["--features", "acceptance"]);
    }
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
    if let Some(acceptance) = acceptance {
        updater
            .args(["--features", "acceptance"])
            .env("ZEROOS_ACCEPTANCE_ORIGIN", acceptance.origin)
            .env("ZEROOS_ACCEPTANCE_CA_PEM", acceptance.ca_pem);
    }
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
    if acceptance.is_none() {
        verify_production_artifact(&output_dir.join("init"))?;
        verify_production_artifact(&output_dir.join("zeroos-update"))?;
    }
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
    if acceptance.is_none() {
        verify_production_artifact(&output_dir.join("zeroos-data"))?;
    }
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
    let current_release_key = acceptance
        .map(|value| value.release_key)
        .unwrap_or_else(|| Path::new("policy/m3-trust/release-current.der"));
    if current_release_key.exists() {
        if !fs::symlink_metadata(current_release_key)
            .map_err(error_string)?
            .file_type()
            .is_file()
        {
            return Err("zeroOS: production release key must be a regular file".into());
        }
        manifest.push_str(
            "dir /etc 0755 0 0\ndir /etc/zeroos 0755 0 0\ndir /etc/zeroos/release-keys 0755 0 0\n",
        );
        manifest.push_str(&format!(
            "file /etc/zeroos/release-keys/{} {} 0444 0 0\n",
            if acceptance.is_some() {
                "acceptance-current.der"
            } else {
                "release-current.der"
            },
            current_release_key
                .canonicalize()
                .map_err(error_string)?
                .display()
        ));
    }
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

fn verify_production_artifact(path: &Path) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(error_string)?;
    let forbidden = [
        b"ZEROOS_ACCEPT".as_slice(),
        b"ZEROOS_ACCEPTANCE_".as_slice(),
        b"https://10.0.2.2:8443".as_slice(),
        b"zeroOS M3 disposable".as_slice(),
        b"acceptance-current".as_slice(),
        b"acceptance-next".as_slice(),
        b"BEGIN PRIVATE KEY".as_slice(),
    ];
    let overlap = forbidden
        .iter()
        .map(|marker| marker.len())
        .max()
        .ok_or_else(|| "zeroOS: production marker list is empty".to_owned())?
        .checked_sub(1)
        .ok_or_else(|| "zeroOS: invalid production marker".to_owned())?;
    let mut carry = Vec::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut file, &mut chunk).map_err(error_string)?;
        if count == 0 {
            break;
        }
        carry.extend_from_slice(&chunk[..count]);
        if forbidden
            .iter()
            .any(|marker| carry.windows(marker.len()).any(|window| window == *marker))
        {
            return Err(format!(
                "zeroOS: production artifact {} contains acceptance hooks",
                path.display()
            ));
        }
        if carry.len() > overlap {
            carry.drain(..carry.len() - overlap);
        }
    }
    Ok(())
}

fn verify_production_artifacts(output_dir: &Path) -> Result<(), String> {
    for name in [
        "init",
        "zeroos-update",
        "zeroos-data",
        "selector.efi",
        "kernel.efi",
        "system-a.efi",
        "system-b.efi",
        "recovery.efi",
        "zeroos.img",
    ] {
        verify_production_artifact(&output_dir.join(name))?;
    }
    Ok(())
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

fn build_kernel(
    arch: Arch,
    source: &Path,
    output_dir: &Path,
    acceptance: bool,
) -> Result<(), String> {
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
    if acceptance {
        merge.arg(
            Path::new("kernel")
                .join(format!("{}-acceptance.config", arch.name()))
                .canonicalize()
                .map_err(error_string)?,
        );
    }
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
    let valid_tail = manifest.lines().next_back().is_some_and(|line| {
        line.ends_with(" 0755 0 0")
            || (line.starts_with("file /etc/zeroos/release-keys/release-current.der ")
                && line.ends_with(" 0444 0 0"))
    });
    if !manifest.starts_with("dir /dev 0755 0 0\nnod /dev/console 0600 0 0 c 5 1\n")
        || !manifest.contains("nod /dev/null 0666 0 0 c 1 3\n")
        || !manifest.contains("file /init ")
        || !manifest.contains("file /zeroos-update ")
        || !manifest.contains("file /zeroos-data ")
        || !valid_tail
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
    command("cargo", &["test", "--workspace", "--locked"])?;
    build(arch)?;
    let output_dir = artifacts(arch);
    let first = output_dir.join("zeroos.first.img");
    fs::copy(output_dir.join("zeroos.img"), &first).map_err(error_string)?;
    package_disk(arch, &output_dir)?;
    identical_files(&first, &output_dir.join("zeroos.img"))?;
    fs::remove_file(first).map_err(error_string)?;
    inspect(arch, &output_dir)?;
    run_qemu(arch, true)?;
    run_secure_boot_scenarios(arch)?;
    run_update_acceptance(arch)
}

fn run_recovery_acceptance(arch: Arch, output_dir: &Path) -> Result<(), String> {
    let image = output_dir.join("recovery-acceptance.img");
    fs::copy(output_dir.join("zeroos.img"), &image).map_err(error_string)?;
    let mut disk = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&image)
        .map_err(error_string)?;
    for sector in [34_816u64, 231_424] {
        disk.seek(std::io::SeekFrom::Start(
            sector
                .checked_mul(zeroos_storage::SECTOR)
                .ok_or_else(|| "zeroOS: recovery slot offset overflow".to_owned())?,
        ))
        .map_err(error_string)?;
        disk.write_all(b"INVALID!").map_err(error_string)?;
    }
    disk.sync_all().map_err(error_string)?;
    let unchanged_before = immutable_partition_hashes(&image)?;
    let (code, vars) = firmware(arch)?;
    run_qemu_scenario(
        arch,
        true,
        "acceptance-recovery",
        &image,
        &code,
        &vars,
        BootExpectation::Recovery,
    )?;
    if immutable_partition_hashes(&image)? != unchanged_before {
        return Err("zeroOS: recovery mutation changed a system or recovery partition".into());
    }
    Ok(())
}

fn immutable_partition_hashes(image: &Path) -> Result<Vec<String>, String> {
    let mut disk = fs::File::open(image).map_err(error_string)?;
    zeroos_storage::PARTITIONS[..4]
        .iter()
        .map(|partition| {
            let offset = partition
                .first
                .checked_mul(zeroos_storage::SECTOR)
                .ok_or_else(|| "zeroOS: partition hash offset overflow".to_owned())?;
            let length = partition
                .last
                .checked_sub(partition.first)
                .and_then(|value| value.checked_add(1))
                .and_then(|value| value.checked_mul(zeroos_storage::SECTOR))
                .ok_or_else(|| "zeroOS: partition hash size overflow".to_owned())?;
            disk.seek(std::io::SeekFrom::Start(offset))
                .map_err(error_string)?;
            let mut context = ring::digest::Context::new(&ring::digest::SHA256);
            let mut remaining = length;
            let mut buffer = [0; 64 * 1024];
            while remaining != 0 {
                let wanted = buffer
                    .len()
                    .min(usize::try_from(remaining).map_err(error_string)?);
                disk.read_exact(&mut buffer[..wanted])
                    .map_err(error_string)?;
                context.update(&buffer[..wanted]);
                remaining = remaining
                    .checked_sub(u64::try_from(wanted).map_err(error_string)?)
                    .ok_or_else(|| "zeroOS: partition hash underflow".to_owned())?;
            }
            Ok(hex_digest(context.finish().as_ref()))
        })
        .collect()
}

fn run_update_acceptance(arch: Arch) -> Result<(), String> {
    let root = env::temp_dir().join(format!(
        "zeroos-m3-update-{}-{}",
        std::process::id(),
        arch.name()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(error_string)?;
    }
    fs::create_dir_all(&root).map_err(error_string)?;
    let _cleanup = CleanupDir(root.clone());
    generate_update_fixture(&root)?;
    let release_key = root.join("release.der");
    let ca_pem = fs::read_to_string(root.join("ca.pem")).map_err(error_string)?;
    let output_dir = artifacts(arch).join("acceptance");
    fs::create_dir_all(&output_dir).map_err(error_string)?;
    let source = fetch_linux()?;
    build_init(
        arch,
        &output_dir,
        Some(&AcceptanceBuild {
            origin: "https://10.0.2.2:8443",
            ca_pem: &ca_pem,
            release_key: &release_key,
        }),
    )?;
    build_kernel(arch, &source, &output_dir, true)?;
    fs::copy(
        artifacts(arch).join("selector.efi"),
        output_dir.join("selector.efi"),
    )
    .map_err(error_string)?;
    package_disk(arch, &output_dir)?;
    run_recovery_acceptance(arch, &output_dir)?;
    let raw_slot = root.join("update.slot");
    make_signed_slot(
        arch,
        1,
        arch.name(),
        &output_dir.join("kernel.efi"),
        &raw_slot,
        &root.join("release.key"),
    )?;
    write_http_fixture(
        &raw_slot,
        &root.join(format!("zeroos-{}.slot", arch.name())),
    )?;
    let mut server = Command::new("openssl");
    server
        .args(["s_server", "-quiet", "-HTTP", "-accept", "8443", "-cert"])
        .arg(root.join("server.pem"))
        .arg("-key")
        .arg(root.join("server.key"))
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _server = CleanupChild(server.spawn().map_err(error_string)?);
    wait_for_server()?;
    let image = output_dir.join("zeroos.img");
    let (code, vars) = firmware(arch)?;
    for (name, marker) in [
        ("download", "during-download-persist"),
        ("verification", "before-signature-verification"),
        ("slot-write", "during-slot-write"),
        ("slot-flush", "before-slot-flush"),
        ("reread", "before-reread"),
        ("journal", "before-journal-switch"),
    ] {
        let interrupted = output_dir.join(format!("interrupted-{name}.img"));
        fs::copy(&image, &interrupted).map_err(error_string)?;
        run_qemu_scenario(
            arch,
            true,
            &format!("acceptance-cut-{name}"),
            &interrupted,
            &code,
            &vars,
            BootExpectation::Normal,
        )?;
        run_qemu_scenario(
            arch,
            true,
            &format!("acceptance-restart-{name}"),
            &interrupted,
            &code,
            &vars,
            BootExpectation::Normal,
        )?;
        let log =
            fs::read_to_string(artifacts(arch).join(format!("qemu-acceptance-cut-{name}.log")))
                .map_err(error_string)?;
        if !log.contains(&format!("ZEROOS_ACCEPT phase={marker}")) {
            return Err(format!("zeroOS: interruption did not reach {marker}"));
        }
    }
    run_qemu_scenario(
        arch,
        true,
        "acceptance-update",
        &image,
        &code,
        &vars,
        BootExpectation::Normal,
    )?;
    let rollback = output_dir.join("rollback.img");
    fs::copy(&image, &rollback).map_err(error_string)?;
    run_qemu_scenario(
        arch,
        true,
        "acceptance-interrupt-before-health",
        &rollback,
        &code,
        &vars,
        BootExpectation::Normal,
    )?;
    run_qemu_scenario(
        arch,
        true,
        "acceptance-rollback",
        &rollback,
        &code,
        &vars,
        BootExpectation::Normal,
    )?;
    run_qemu_scenario(
        arch,
        true,
        "acceptance-confirm",
        &image,
        &code,
        &vars,
        BootExpectation::Normal,
    )
}

fn write_http_fixture(input: &Path, output: &Path) -> Result<(), String> {
    let body = fs::read(input).map_err(error_string)?;
    let mut response = fs::File::create(output).map_err(error_string)?;
    write!(
        response,
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nETag: \"zeroos-acceptance-1\"\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .map_err(error_string)?;
    response.write_all(&body).map_err(error_string)?;
    response.sync_all().map_err(error_string)
}

fn generate_update_fixture(root: &Path) -> Result<(), String> {
    let quiet = |command: &mut Command, label| {
        command.stdout(Stdio::null()).stderr(Stdio::null());
        run(command, label)
    };
    let mut release = Command::new("openssl");
    release
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:3072",
            "-out",
        ])
        .arg(root.join("release.key"));
    quiet(&mut release, "generate disposable update key")?;
    let mut public = Command::new("openssl");
    public
        .args(["rsa", "-in"])
        .arg(root.join("release.key"))
        .args(["-RSAPublicKey_out", "-outform", "DER", "-out"])
        .arg(root.join("release.der"));
    quiet(&mut public, "export disposable update public key")?;
    let mut ca = Command::new("openssl");
    ca.args([
        "req", "-new", "-x509", "-newkey", "rsa:3072", "-sha256", "-nodes",
    ])
    .args([
        "-subj",
        "/CN=zeroOS M3 acceptance CA/",
        "-days",
        "1",
        "-keyout",
    ])
    .arg(root.join("ca.key"))
    .arg("-out")
    .arg(root.join("ca.pem"));
    quiet(&mut ca, "generate disposable HTTPS CA")?;
    let mut request = Command::new("openssl");
    request
        .args(["req", "-new", "-newkey", "rsa:3072", "-sha256", "-nodes"])
        .args(["-subj", "/CN=10.0.2.2/", "-keyout"])
        .arg(root.join("server.key"))
        .arg("-out")
        .arg(root.join("server.csr"));
    quiet(&mut request, "generate disposable HTTPS request")?;
    fs::write(
        root.join("server.ext"),
        "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:10.0.2.2\n",
    )
    .map_err(error_string)?;
    let mut certificate = Command::new("openssl");
    certificate
        .args(["x509", "-req", "-sha256", "-days", "1", "-in"])
        .arg(root.join("server.csr"))
        .arg("-CA")
        .arg(root.join("ca.pem"))
        .arg("-CAkey")
        .arg(root.join("ca.key"))
        .arg("-set_serial")
        .arg("2")
        .arg("-extfile")
        .arg(root.join("server.ext"))
        .arg("-out")
        .arg(root.join("server.pem"));
    quiet(&mut certificate, "sign disposable HTTPS certificate")
}

fn make_signed_slot(
    arch: Arch,
    sequence: u64,
    manifest_arch: &str,
    payload: &Path,
    output: &Path,
    key: &Path,
) -> Result<(), String> {
    let payload_size = fs::metadata(payload).map_err(error_string)?.len();
    let digest = hash_file(payload)?;
    let manifest = format!(
        "version=1\narch={manifest_arch}\nsequence={sequence}\npayload-size={payload_size}\nsha256={digest}\nsigner=acceptance-current\n"
    );
    if manifest_arch == arch.name() {
        zeroos_storage::Manifest::parse(manifest.as_bytes(), arch.name()).map_err(str::to_owned)?;
    }
    let manifest_path = output.with_extension("manifest");
    let signature_path = output.with_extension("signature");
    fs::write(&manifest_path, manifest.as_bytes()).map_err(error_string)?;
    let mut sign = Command::new("openssl");
    sign.args(["dgst", "-sha256", "-sign"])
        .arg(key)
        .args([
            "-sigopt",
            "rsa_padding_mode:pss",
            "-sigopt",
            "rsa_pss_saltlen:32",
            "-out",
        ])
        .arg(&signature_path)
        .arg(&manifest_path);
    run(&mut sign, "sign disposable update manifest")?;
    let signature = fs::read(&signature_path).map_err(error_string)?;
    if signature.len() != zeroos_storage::SIGNATURE_BYTES {
        return Err("zeroOS: disposable update signature is not RSA-3072".into());
    }
    let mut file = fs::File::create(output).map_err(error_string)?;
    file.write_all(b"ZEROSLT1").map_err(error_string)?;
    file.write_all(
        &u32::try_from(manifest.len())
            .map_err(error_string)?
            .to_le_bytes(),
    )
    .map_err(error_string)?;
    file.write_all(manifest.as_bytes()).map_err(error_string)?;
    file.write_all(&signature).map_err(error_string)?;
    std::io::copy(
        &mut fs::File::open(payload).map_err(error_string)?,
        &mut file,
    )
    .map_err(error_string)?;
    file.sync_all().map_err(error_string)?;
    fs::remove_file(manifest_path).map_err(error_string)?;
    fs::remove_file(signature_path).map_err(error_string)
}

fn wait_for_server() -> Result<(), String> {
    for _ in 0..100 {
        if std::net::TcpStream::connect("127.0.0.1:8443").is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err("zeroOS: disposable HTTPS server did not start".into())
}

fn interruption_marker(name: &str) -> Option<&'static str> {
    match name {
        "acceptance-cut-download" => Some("during-download-persist"),
        "acceptance-cut-verification" => Some("before-signature-verification"),
        "acceptance-cut-slot-write" => Some("during-slot-write"),
        "acceptance-cut-slot-flush" => Some("before-slot-flush"),
        "acceptance-cut-reread" => Some("before-reread"),
        "acceptance-cut-journal" => Some("before-journal-switch"),
        _ => None,
    }
}

fn test_release(arch: Arch, sequence: u64, public_tag_url: &str) -> Result<(), String> {
    let expected_tag =
        format!("https://github.com/KZagaja/zeroOS-releases/releases/tag/sequence-{sequence}");
    if public_tag_url.trim_end_matches('/') != expected_tag {
        return Err(format!(
            "zeroOS: public release URL must be exactly {expected_tag}"
        ));
    }
    let root = env::temp_dir().join(format!(
        "zeroos-m3-release-{}-{}-{sequence}",
        std::process::id(),
        arch.name()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(error_string)?;
    }
    fs::create_dir_all(&root).map_err(error_string)?;
    let _cleanup = CleanupDir(root.clone());
    let prefix = format!("zeroos-{}", arch.name());
    let names = [
        format!("{prefix}.slot"),
        format!("{prefix}.sha256"),
        format!("{prefix}.provenance.json"),
        format!("zeroos-selector-{}.efi", arch.name()),
        format!("zeroos-system-{}.efi", arch.name()),
        format!("zeroos-recovery-{}.efi", arch.name()),
    ];
    let download_base =
        format!("https://github.com/KZagaja/zeroOS-releases/releases/download/sequence-{sequence}");
    for name in &names {
        download_public_asset(&format!("{download_base}/{name}"), &root.join(name))?;
    }

    let expected_hashes =
        verify_release_hash_manifest(&root.join(format!("{prefix}.sha256")), arch, &root)?;
    let trust = Path::new("policy/m3-trust");
    verify_trust_manifest(trust)?;
    verify_release_provenance(
        &root.join(format!("{prefix}.provenance.json")),
        arch,
        sequence,
        trust,
        &expected_hashes,
    )?;
    verify_release_slot(
        &root.join(format!("{prefix}.slot")),
        &root.join(format!("zeroos-system-{}.efi", arch.name())),
        arch,
        sequence,
        trust,
    )?;
    verify_pe_signature(
        &root.join(format!("zeroos-selector-{}.efi", arch.name())),
        &trust.join("db.pem"),
    )?;
    verify_pe_signature(
        &root.join(format!("zeroos-system-{}.efi", arch.name())),
        &trust.join("db.pem"),
    )?;
    verify_pe_signature(
        &root.join(format!("zeroos-recovery-{}.efi", arch.name())),
        &trust.join("recovery.pem"),
    )?;

    build(arch)?;
    let normal = install_release_disk(arch, &root, sequence, false)?;
    let recovery = install_release_disk(arch, &root, sequence, true)?;
    let (code, vars_template) = secure_firmware(arch)?;
    let enrolled_vars = root.join("production-vars.fd");
    let owner = "5A45524F-4F53-4D33-8000-000000000007";
    let mut enroll = Command::new("virt-fw-vars");
    enroll
        .args(["--input"])
        .arg(vars_template)
        .args(["--output"])
        .arg(&enrolled_vars)
        .args(["--set-pk", owner])
        .arg(trust.join("pk.pem"))
        .args(["--add-kek", owner])
        .arg(trust.join("kek.pem"))
        .args(["--add-db", owner])
        .arg(trust.join("db.pem"))
        .args(["--add-db", owner])
        .arg(trust.join("recovery.pem"))
        .arg("--sb");
    run(
        &mut enroll,
        "enroll committed production Secure Boot certificates",
    )?;
    run_qemu_scenario(
        arch,
        true,
        &format!("release-{sequence}-normal"),
        &normal,
        &code,
        &enrolled_vars,
        BootExpectation::Normal,
    )?;
    run_qemu_scenario(
        arch,
        true,
        &format!("release-{sequence}-recovery"),
        &recovery,
        &code,
        &enrolled_vars,
        BootExpectation::Recovery,
    )
}

fn download_public_asset(url: &str, output: &Path) -> Result<(), String> {
    let partial = output.with_extension("download");
    let mut curl = Command::new("curl");
    curl.args([
        "--fail",
        "--location",
        "--proto",
        "=https",
        "--tlsv1.2",
        "--max-filesize",
        "100663296",
        "--output",
    ])
    .arg(&partial)
    .arg(url);
    run(&mut curl, "download unauthenticated public release asset")?;
    fs::rename(partial, output).map_err(error_string)
}

fn verify_release_hash_manifest(
    manifest: &Path,
    arch: Arch,
    root: &Path,
) -> Result<BTreeMap<String, String>, String> {
    let input = fs::read_to_string(manifest).map_err(error_string)?;
    if input.len() > 4096 {
        return Err("zeroOS: release hash manifest is oversized".into());
    }
    let expected_names: HashSet<String> = [
        format!("zeroos-{}.slot", arch.name()),
        format!("zeroos-selector-{}.efi", arch.name()),
        format!("zeroos-system-{}.efi", arch.name()),
        format!("zeroos-recovery-{}.efi", arch.name()),
    ]
    .into_iter()
    .collect();
    let mut hashes = BTreeMap::new();
    for line in input.lines() {
        let (digest, name) = line
            .split_once("  ")
            .ok_or_else(|| "zeroOS: malformed release hash manifest".to_owned())?;
        if digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !expected_names.contains(name)
            || hashes
                .insert(name.into(), digest.to_ascii_lowercase())
                .is_some()
        {
            return Err("zeroOS: malformed release hash manifest".into());
        }
        if hash_file(&root.join(name))? != digest.to_ascii_lowercase() {
            return Err(format!("zeroOS: public asset hash mismatch for {name}"));
        }
    }
    if hashes.len() != expected_names.len() {
        return Err("zeroOS: incomplete release hash manifest".into());
    }
    Ok(hashes)
}

fn verify_trust_manifest(trust: &Path) -> Result<(), String> {
    let certificate_names = ["pk.pem", "kek.pem", "db.pem", "next-db.pem", "recovery.pem"];
    for name in certificate_names {
        if !trust.join(name).is_file() {
            return Err(format!(
                "zeroOS: committed production trust file policy/m3-trust/{name} is missing"
            ));
        }
    }
    let fingerprint_path = trust.join("fingerprints.sha256");
    if !fs::symlink_metadata(&fingerprint_path)
        .map_err(error_string)?
        .file_type()
        .is_file()
    {
        return Err("zeroOS: production fingerprint manifest must be a regular file".into());
    }
    let input = fs::read_to_string(fingerprint_path).map_err(error_string)?;
    if input.len() > 8192 {
        return Err("zeroOS: production fingerprint manifest is oversized".into());
    }
    let mut count = 0usize;
    let mut der_count = 0usize;
    let mut names = HashSet::new();
    for line in input.lines() {
        let (digest, name) = line
            .split_once("  ")
            .ok_or_else(|| "zeroOS: malformed production fingerprint manifest".to_owned())?;
        let permitted = certificate_names.contains(&name)
            || matches!(name, "release-current.der" | "release-next.der");
        let metadata = fs::symlink_metadata(trust.join(name)).map_err(error_string)?;
        let size_limit = if name.ends_with(".der") {
            2048
        } else {
            16 * 1024
        };
        if digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || name.contains('/')
            || name.contains("..")
            || name.is_empty()
            || !permitted
            || !metadata.file_type().is_file()
            || metadata.len() > size_limit
            || !names.insert(name)
            || hash_file(&trust.join(name))? != digest.to_ascii_lowercase()
        {
            return Err("zeroOS: production trust fingerprint mismatch".into());
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| "zeroOS: too many production fingerprints".to_owned())?;
        if name.ends_with(".der") {
            der_count = der_count
                .checked_add(1)
                .ok_or_else(|| "zeroOS: too many production release keys".to_owned())?;
        }
    }
    if count != 7 || der_count != 2 {
        return Err("zeroOS: unexpected production fingerprint count".into());
    }
    for certificate in certificate_names {
        if fs::read_to_string(trust.join(certificate))
            .map_err(error_string)?
            .contains("PRIVATE KEY")
        {
            return Err("zeroOS: production trust policy contains private key material".into());
        }
    }
    Ok(())
}

fn verify_release_provenance(
    provenance: &Path,
    arch: Arch,
    sequence: u64,
    trust: &Path,
    artifact_hashes: &BTreeMap<String, String>,
) -> Result<(), String> {
    let input = fs::read_to_string(provenance).map_err(error_string)?;
    if input.len() > 64 * 1024 {
        return Err("zeroOS: release provenance is oversized".into());
    }
    let lowercase = input.to_ascii_lowercase();
    if [
        "private-key",
        "private_key",
        "private key",
        "private-material",
        "private_material",
        "private material",
        "secret-key",
        "secret_key",
        "secret key",
        "secret-material",
        "secret_material",
        "secret material",
    ]
    .iter()
    .any(|label| lowercase.contains(label))
    {
        return Err("zeroOS: release provenance contains a secret-material label".into());
    }
    let source = output("git", &["rev-parse", "HEAD"])?;
    let mut validate = Command::new("jq");
    validate
        .args(["-e", "--arg", "source", &source, "--arg", "arch", arch.name()])
        .args(["--argjson", "sequence", &sequence.to_string()])
        .arg(
            ".source == $source and .arch == $arch and .sequence == $sequence and (.public_fingerprints | type == \"array\")",
        )
        .arg(provenance);
    run(&mut validate, "validate release provenance fields")?;
    for digest in artifact_hashes.values() {
        if !lowercase.contains(digest) {
            return Err("zeroOS: release provenance is missing an artifact hash".into());
        }
    }
    let fingerprints =
        fs::read_to_string(trust.join("fingerprints.sha256")).map_err(error_string)?;
    let expected: HashSet<String> = fingerprints
        .lines()
        .filter_map(|line| {
            line.split_once("  ")
                .map(|(digest, _)| digest.to_ascii_lowercase())
        })
        .collect();
    let fingerprint_output = Command::new("jq")
        .args(["-e", "-r", ".public_fingerprints[]"])
        .arg(provenance)
        .output()
        .map_err(error_string)?;
    if !fingerprint_output.status.success() {
        return Err("zeroOS: invalid public fingerprints in release provenance".into());
    }
    let actual: HashSet<String> = String::from_utf8(fingerprint_output.stdout)
        .map_err(error_string)?
        .lines()
        .map(str::to_ascii_lowercase)
        .collect();
    if actual != expected {
        return Err("zeroOS: release provenance public fingerprints differ from policy".into());
    }
    Ok(())
}

fn verify_release_slot(
    slot: &Path,
    system: &Path,
    arch: Arch,
    sequence: u64,
    trust: &Path,
) -> Result<(), String> {
    let mut file = fs::File::open(slot).map_err(error_string)?;
    let mut header = [0; 12];
    file.read_exact(&mut header).map_err(error_string)?;
    let manifest_size = zeroos_storage::container_manifest_size(&header).map_err(str::to_owned)?;
    let mut manifest_bytes = vec![0; manifest_size];
    file.read_exact(&mut manifest_bytes).map_err(error_string)?;
    let manifest =
        zeroos_storage::Manifest::parse(&manifest_bytes, arch.name()).map_err(str::to_owned)?;
    if manifest.sequence != sequence || manifest.signer != "release-current" {
        return Err("zeroOS: public slot sequence mismatch".into());
    }
    let mut signature = [0; zeroos_storage::SIGNATURE_BYTES];
    file.read_exact(&mut signature).map_err(error_string)?;
    let key_path = trust.join(format!("{}.der", manifest.signer));
    let key = fs::read(&key_path).map_err(|_| {
        format!(
            "zeroOS: public slot signer {} is not committed",
            manifest.signer
        )
    })?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PSS_2048_8192_SHA256, key)
        .verify(&manifest_bytes, &signature)
        .map_err(|_| "zeroOS: public slot manifest signature is invalid".to_owned())?;
    let payload_offset = 12u64
        .checked_add(u64::try_from(manifest_size).map_err(error_string)?)
        .and_then(|value| value.checked_add(zeroos_storage::SIGNATURE_BYTES as u64))
        .ok_or_else(|| "zeroOS: public slot size overflow".to_owned())?;
    let expected_size = payload_offset
        .checked_add(manifest.payload_size)
        .ok_or_else(|| "zeroOS: public slot size overflow".to_owned())?;
    if file.metadata().map_err(error_string)?.len() != expected_size
        || fs::metadata(system).map_err(error_string)?.len() != manifest.payload_size
        || hash_file(system)? != hex_digest(&manifest.sha256)
    {
        return Err("zeroOS: public slot payload does not match the signed system EFI".into());
    }
    file.seek(std::io::SeekFrom::Start(payload_offset))
        .map_err(error_string)?;
    let mut system_file = fs::File::open(system).map_err(error_string)?;
    let mut slot_buffer = [0; 64 * 1024];
    let mut system_buffer = [0; 64 * 1024];
    loop {
        let slot_count = file.read(&mut slot_buffer).map_err(error_string)?;
        let system_count = system_file.read(&mut system_buffer).map_err(error_string)?;
        if slot_count != system_count || slot_buffer[..slot_count] != system_buffer[..system_count]
        {
            return Err("zeroOS: public slot embeds a different system EFI".into());
        }
        if slot_count == 0 {
            break;
        }
    }
    Ok(())
}

fn verify_pe_signature(payload: &Path, certificate: &Path) -> Result<(), String> {
    let mut verify = Command::new("sbverify");
    verify.arg("--cert").arg(certificate).arg(payload);
    run(&mut verify, "verify production EFI signature")
}

fn install_release_disk(
    arch: Arch,
    root: &Path,
    sequence: u64,
    force_recovery: bool,
) -> Result<PathBuf, String> {
    let image = root.join(format!(
        "release-{sequence}-{}.img",
        if force_recovery { "recovery" } else { "normal" }
    ));
    fs::copy(artifacts(arch).join("zeroos.img"), &image).map_err(error_string)?;
    let spec = format!("{}@@{ESP_OFFSET}", image.display());
    let guest_path = format!("::/EFI/BOOT/{}", arch.fallback());
    let mut copy = Command::new("mcopy");
    copy.env("MTOOLS_SKIP_CHECK", "1")
        .args(["-o", "-i", &spec])
        .arg(root.join(format!("zeroos-selector-{}.efi", arch.name())))
        .arg(&guest_path);
    run(&mut copy, "install production selector")?;
    let slot = root.join(format!("zeroos-{}.slot", arch.name()));
    for seek in ["34816", "231424"] {
        let mut dd = Command::new("dd");
        dd.arg(format!("if={}", slot.display()))
            .arg(format!("of={}", image.display()))
            .args([
                "bs=512",
                &format!("seek={seek}"),
                "conv=notrunc",
                "status=none",
            ]);
        run(&mut dd, "install production system slot")?;
    }
    let recovery_slot = root.join("production-recovery.slot");
    make_development_slot(
        arch,
        &root.join(format!("zeroos-recovery-{}.efi", arch.name())),
        &recovery_slot,
    )?;
    let mut recovery_dd = Command::new("dd");
    recovery_dd
        .arg(format!("if={}", recovery_slot.display()))
        .arg(format!("of={}", image.display()))
        .args(["bs=512", "seek=428032", "conv=notrunc", "status=none"]);
    run(&mut recovery_dd, "install production recovery slot")?;
    if force_recovery {
        let mut disk = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&image)
            .map_err(error_string)?;
        for sector in [34_816u64, 231_424] {
            disk.seek(std::io::SeekFrom::Start(
                sector
                    .checked_mul(zeroos_storage::SECTOR)
                    .ok_or_else(|| "zeroOS: release slot offset overflow".to_owned())?,
            ))
            .map_err(error_string)?;
            disk.write_all(b"INVALID!").map_err(error_string)?;
        }
        disk.sync_all().map_err(error_string)?;
    }
    Ok(image)
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(error_string)?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(error_string)?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }
    Ok(hex_digest(context.finish().as_ref()))
}

fn run_secure_boot_scenarios(arch: Arch) -> Result<(), String> {
    let output_dir = artifacts(arch);
    let root = env::temp_dir().join(format!(
        "zeroos-m3-secure-boot-{}-{}",
        std::process::id(),
        arch.name()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(error_string)?;
    }
    fs::create_dir_all(&root).map_err(error_string)?;
    let _cleanup = CleanupDir(root.clone());
    for (name, serial) in [
        ("pk", "1"),
        ("kek", "2"),
        ("db", "3"),
        ("recovery", "4"),
        ("next-db", "5"),
    ] {
        generate_test_certificate(&root, name, serial)?;
    }
    let (code, vars_template) = secure_firmware(arch)?;
    let enrolled_vars = root.join("enrolled-vars.fd");
    let owner = "5A45524F-4F53-4D33-8000-000000000007";
    let mut enroll = Command::new("virt-fw-vars");
    enroll
        .args(["--input"])
        .arg(&vars_template)
        .args(["--output"])
        .arg(&enrolled_vars)
        .args(["--set-pk", owner])
        .arg(root.join("pk.pem"))
        .args(["--add-kek", owner])
        .arg(root.join("kek.pem"))
        .args(["--add-db", owner])
        .arg(root.join("db.pem"))
        .args(["--add-db", owner])
        .arg(root.join("recovery.pem"))
        .arg("--sb");
    run(&mut enroll, "enroll disposable Secure Boot keys")?;

    let signed_selector = root.join("selector.efi");
    let signed_system = root.join("system.efi");
    let signed_recovery = root.join("recovery.efi");
    let next_selector = root.join("next-selector.efi");
    let next_system = root.join("next-system.efi");
    sign_efi(
        &root,
        "db",
        &output_dir.join("selector.efi"),
        &signed_selector,
    )?;
    sign_efi(&root, "db", &output_dir.join("kernel.efi"), &signed_system)?;
    sign_efi(
        &root,
        "recovery",
        &output_dir.join("kernel.efi"),
        &signed_recovery,
    )?;
    sign_efi(
        &root,
        "next-db",
        &output_dir.join("selector.efi"),
        &next_selector,
    )?;
    sign_efi(
        &root,
        "next-db",
        &output_dir.join("kernel.efi"),
        &next_system,
    )?;

    let signed = secure_disk(
        arch,
        &root,
        "signed",
        &signed_selector,
        &signed_system,
        &signed_recovery,
    )?;
    let unsigned_selector = secure_disk(
        arch,
        &root,
        "unsigned-selector",
        &output_dir.join("selector.efi"),
        &signed_system,
        &signed_recovery,
    )?;
    let altered_selector_file = root.join("altered-selector.efi");
    fs::copy(&signed_selector, &altered_selector_file).map_err(error_string)?;
    corrupt_signed_body(&altered_selector_file)?;
    let altered_selector = secure_disk(
        arch,
        &root,
        "altered-selector",
        &altered_selector_file,
        &signed_system,
        &signed_recovery,
    )?;
    let unsigned_normal = secure_disk(
        arch,
        &root,
        "unsigned-normal",
        &signed_selector,
        &output_dir.join("kernel.efi"),
        &signed_recovery,
    )?;
    let altered_system_file = root.join("altered-system.efi");
    fs::copy(&signed_system, &altered_system_file).map_err(error_string)?;
    corrupt_signed_body(&altered_system_file)?;
    let altered_normal = secure_disk(
        arch,
        &root,
        "altered-normal",
        &signed_selector,
        &altered_system_file,
        &signed_recovery,
    )?;
    let unsigned_recovery = secure_disk(
        arch,
        &root,
        "unsigned-recovery",
        &signed_selector,
        &output_dir.join("kernel.efi"),
        &output_dir.join("kernel.efi"),
    )?;
    let altered_recovery_file = root.join("altered-recovery.efi");
    fs::copy(&signed_recovery, &altered_recovery_file).map_err(error_string)?;
    corrupt_signed_body(&altered_recovery_file)?;
    let altered_recovery = secure_disk(
        arch,
        &root,
        "altered-recovery",
        &signed_selector,
        &output_dir.join("kernel.efi"),
        &altered_recovery_file,
    )?;

    for (name, image, expectation) in [
        ("secure-signed", signed.clone(), BootExpectation::Normal),
        (
            "secure-unsigned-selector",
            unsigned_selector,
            BootExpectation::Rejected,
        ),
        (
            "secure-altered-selector",
            altered_selector,
            BootExpectation::Rejected,
        ),
        (
            "secure-unsigned-normal",
            unsigned_normal,
            BootExpectation::Recovery,
        ),
        (
            "secure-altered-normal",
            altered_normal,
            BootExpectation::Recovery,
        ),
        (
            "secure-unsigned-recovery",
            unsigned_recovery,
            BootExpectation::Rejected,
        ),
        (
            "secure-altered-recovery",
            altered_recovery,
            BootExpectation::Rejected,
        ),
    ] {
        run_qemu_scenario(arch, true, name, &image, &code, &enrolled_vars, expectation)?;
    }

    let overlap_vars = root.join("overlap-vars.fd");
    let mut overlap = Command::new("virt-fw-vars");
    overlap
        .args(["--input"])
        .arg(&enrolled_vars)
        .args(["--output"])
        .arg(&overlap_vars)
        .args(["--add-db", owner])
        .arg(root.join("next-db.pem"))
        .arg("--sb");
    run(&mut overlap, "enroll disposable next db key")?;
    let transition = secure_disk(
        arch,
        &root,
        "rotation-transition",
        &signed_selector,
        &next_system,
        &signed_recovery,
    )?;
    run_qemu_scenario(
        arch,
        true,
        "secure-rotation-transition",
        &transition,
        &code,
        &overlap_vars,
        BootExpectation::Normal,
    )?;

    let retired_vars = root.join("retired-vars.fd");
    let mut retire = Command::new("virt-fw-vars");
    retire
        .args(["--input"])
        .arg(&overlap_vars)
        .args(["--output"])
        .arg(&retired_vars)
        .args(["--delete", "db", "--add-db", owner])
        .arg(root.join("next-db.pem"))
        .args(["--add-db", owner])
        .arg(root.join("recovery.pem"))
        .arg("--sb");
    run(&mut retire, "remove disposable old db key")?;
    let next = secure_disk(
        arch,
        &root,
        "rotation-next",
        &next_selector,
        &next_system,
        &signed_recovery,
    )?;
    run_qemu_scenario(
        arch,
        true,
        "secure-rotation-next",
        &next,
        &code,
        &retired_vars,
        BootExpectation::Normal,
    )?;
    run_qemu_scenario(
        arch,
        true,
        "secure-rotation-old-rejected",
        &signed,
        &code,
        &retired_vars,
        BootExpectation::Rejected,
    )?;
    Ok(())
}

fn generate_test_certificate(root: &Path, name: &str, serial: &str) -> Result<(), String> {
    let mut openssl = Command::new("openssl");
    openssl
        .args([
            "req", "-new", "-x509", "-newkey", "rsa:3072", "-sha256", "-nodes",
        ])
        .args(["-subj", &format!("/CN=zeroOS M3 disposable {name}/")])
        .args(["-days", "1", "-set_serial", serial, "-keyout"])
        .arg(root.join(format!("{name}.key")))
        .arg("-out")
        .arg(root.join(format!("{name}.pem")))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    run(&mut openssl, "generate disposable Secure Boot certificate")
}

fn sign_efi(root: &Path, key: &str, input: &Path, output: &Path) -> Result<(), String> {
    let mut sign = Command::new("sbsign");
    sign.arg("--key")
        .arg(root.join(format!("{key}.key")))
        .arg("--cert")
        .arg(root.join(format!("{key}.pem")))
        .arg("--output")
        .arg(output)
        .arg(input);
    run(&mut sign, "sign disposable EFI payload")
}

fn secure_disk(
    arch: Arch,
    root: &Path,
    name: &str,
    selector: &Path,
    system: &Path,
    recovery: &Path,
) -> Result<PathBuf, String> {
    let image = root.join(format!("{name}.img"));
    fs::copy(artifacts(arch).join("zeroos.img"), &image).map_err(error_string)?;
    let spec = format!("{}@@{ESP_OFFSET}", image.display());
    let guest_path = format!("::/EFI/BOOT/{}", arch.fallback());
    let mut copy = Command::new("mcopy");
    copy.env("MTOOLS_SKIP_CHECK", "1")
        .args(["-o", "-i", &spec])
        .arg(selector)
        .arg(&guest_path);
    run(&mut copy, "replace Secure Boot selector")?;
    for (payload, slot_name, seek) in [
        (system, "a", "34816"),
        (system, "b", "231424"),
        (recovery, "recovery", "428032"),
    ] {
        let slot = root.join(format!("{name}-{slot_name}.slot"));
        make_development_slot(arch, payload, &slot)?;
        let mut dd = Command::new("dd");
        dd.arg(format!("if={}", slot.display()))
            .arg(format!("of={}", image.display()))
            .args([
                "bs=512",
                &format!("seek={seek}"),
                "conv=notrunc",
                "status=none",
            ]);
        run(&mut dd, "populate Secure Boot slot")?;
    }
    Ok(image)
}

fn corrupt_signed_body(path: &Path) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(error_string)?;
    let length = file.metadata().map_err(error_string)?.len();
    if length <= 4096 {
        return Err("zeroOS: EFI payload too small for signed-body corruption".into());
    }
    let offset = 4096;
    file.seek(std::io::SeekFrom::Start(offset))
        .map_err(error_string)?;
    let mut byte = [0];
    std::io::Read::read_exact(&mut file, &mut byte).map_err(error_string)?;
    byte[0] ^= 1;
    file.seek(std::io::SeekFrom::Start(offset))
        .map_err(error_string)?;
    file.write_all(&byte).map_err(error_string)?;
    file.sync_all().map_err(error_string)
}

fn secure_firmware(arch: Arch) -> Result<(PathBuf, PathBuf), String> {
    let (code, vars) = match arch {
        Arch::X86_64 => (
            "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd",
            "/usr/share/OVMF/OVMF_VARS_4M.fd",
        ),
        Arch::Aarch64 => (
            "/usr/share/AAVMF/AAVMF_CODE.secboot.fd",
            "/usr/share/AAVMF/AAVMF_VARS.fd",
        ),
    };
    if Path::new(code).is_file() && Path::new(vars).is_file() {
        Ok((code.into(), vars.into()))
    } else {
        Err(format!(
            "zeroOS: {} Secure Boot firmware not found",
            arch.name()
        ))
    }
}

fn run_qemu(arch: Arch, capture: bool) -> Result<(), String> {
    let output_dir = artifacts(arch);
    let (code, vars_template) = firmware(arch)?;
    run_qemu_scenario(
        arch,
        capture,
        "runtime",
        &output_dir.join("zeroos.img"),
        &code,
        &vars_template,
        BootExpectation::Normal,
    )
}

fn run_qemu_scenario(
    arch: Arch,
    capture: bool,
    name: &str,
    image: &Path,
    code: &Path,
    vars_template: &Path,
    expectation: BootExpectation,
) -> Result<(), String> {
    let output_dir = artifacts(arch);
    let log = output_dir.join(if name == "runtime" {
        "qemu.log".to_owned()
    } else {
        format!("qemu-{name}.log")
    });
    let vars = output_dir.join(format!("uefi-vars-{name}.fd"));
    fs::copy(vars_template, &vars).map_err(error_string)?;
    print_scenario_hashes(name, image, &vars)?;
    let mut qemu = Command::new(arch.qemu());
    qemu.args(["-no-reboot", "-nographic", "-m", "512"]);
    match arch {
        Arch::X86_64 => {
            qemu.args(["-machine", "q35,accel=tcg", "-cpu", "max"]);
            if code.to_string_lossy().contains("secboot") {
                qemu.args(["-global", "driver=cfi.pflash01,property=secure,value=on"]);
            }
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
    if name.starts_with("acceptance-") {
        qemu.args([
            "-netdev",
            "user,id=zeroos-net",
            "-device",
            "virtio-net-pci,netdev=zeroos-net",
        ]);
    }

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
    let mut sent_status = false;
    let mut sent_logs = false;
    let cut_marker = interruption_marker(name);
    let update_install = name == "acceptance-update" || cut_marker.is_some();
    let confirm_update = name == "acceptance-confirm";
    let interrupt_before_health = name == "acceptance-interrupt-before-health";
    let confirm_rollback = name == "acceptance-rollback";
    let recovery_acceptance = name == "acceptance-recovery";
    let restart_acceptance = name.starts_with("acceptance-restart-");
    let mut sent_shutdown = false;
    let mut recovery_code = None;
    let status = loop {
        while let Ok(line) = received.try_recv() {
            output.push_str(&line);
            output.push('\n');
            if line.contains("credential-request=new-passphrase")
                || line.contains("credential-request=repeat-passphrase")
                || line.contains("credential-request=credential")
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
            if line.trim_end_matches('\r') == READY && restart_acceptance && !sent_shutdown {
                writeln!(input, "status\nshutdown").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_shutdown = true;
            } else if line.trim_end_matches('\r') == READY && recovery_acceptance && !sent_selftest
            {
                writeln!(input, "factory-reset\nrepair-boot\nrepair-data").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_selftest = true;
            } else if line.trim_end_matches('\r') == READY && confirm_rollback && !sent_shutdown {
                writeln!(input, "status\nshutdown").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_shutdown = true;
            } else if line.trim_end_matches('\r') == READY && update_install && !sent_selftest {
                writeln!(input, "update install").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_selftest = true;
            } else if line.trim_end_matches('\r') == READY
                && !confirm_update
                && !interrupt_before_health
                && !recovery_acceptance
                && !restart_acceptance
                && !sent_selftest
            {
                if !sent_status {
                    writeln!(input, "status").map_err(error_string)?;
                    input.flush().map_err(error_string)?;
                    sent_status = true;
                }
                writeln!(input, "selftest").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_selftest = true;
            }
            if line.contains("SELFTEST PASS") && !sent_logs {
                writeln!(input, "logs").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_logs = true;
            }
            if confirm_update
                && line.contains("ZEROOS_ACCEPT phase=after-health-confirmation")
                && !sent_shutdown
            {
                writeln!(input, "status\nshutdown").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_shutdown = true;
            }
            if interrupt_before_health
                && line.contains("ZEROOS_ACCEPT phase=before-health-confirmation")
            {
                child.kill().map_err(error_string)?;
                child.wait().map_err(error_string)?;
                fs::write(&log, redact_recovery_codes(&output)).map_err(error_string)?;
                print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
                return Ok(());
            }
            if recovery_acceptance && line.contains("ZEROOS_ACCEPT phase=after-repair-data") {
                writeln!(input, "factory-reset ERASE-USER-DATA").map_err(error_string)?;
                input.flush().map_err(error_string)?;
            }
            if recovery_acceptance
                && line.contains("ZEROOS_ACCEPT phase=after-factory-reset")
                && !sent_shutdown
            {
                writeln!(input, "status\nshutdown").map_err(error_string)?;
                input.flush().map_err(error_string)?;
                sent_shutdown = true;
            }
            if cut_marker
                .is_some_and(|marker| line.contains(&format!("ZEROOS_ACCEPT phase={marker}")))
            {
                child.kill().map_err(error_string)?;
                child.wait().map_err(error_string)?;
                fs::write(&log, redact_recovery_codes(&output)).map_err(error_string)?;
                print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
                return Ok(());
            }
        }
        if let Some(status) = child.try_wait().map_err(error_string)? {
            break status;
        }
        let timeout = if expectation == BootExpectation::Rejected {
            Duration::from_secs(12)
        } else {
            Duration::from_secs(180)
        };
        if start.elapsed() >= timeout {
            child.kill().map_err(error_string)?;
            child.wait().map_err(error_string)?;
            thread::sleep(Duration::from_millis(50));
            for line in received.try_iter() {
                output.push_str(&line);
                output.push('\n');
            }
            fs::write(&log, redact_recovery_codes(&output)).map_err(error_string)?;
            if expectation == BootExpectation::Rejected && !output.contains(READY) {
                print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
                return Ok(());
            }
            return Err(format!(
                "zeroOS: QEMU acceptance timed out after {}s; output ended with {:?}",
                timeout.as_secs(),
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
    fs::write(log, redact_recovery_codes(&output)).map_err(error_string)?;
    if update_install && cut_marker.is_none() {
        for marker in [
            "ZEROOS_ACCEPT phase=after-download-persist",
            "ZEROOS_ACCEPT phase=after-signature-verification",
            "ZEROOS_ACCEPT phase=after-slot-flush",
            "ZEROOS_ACCEPT phase=before-reread",
            "ZEROOS_ACCEPT phase=after-journal-switch",
            "ZEROOS_ACCEPT phase=before-reboot",
        ] {
            if !output.contains(marker) {
                return Err(format!("zeroOS: update acceptance missing {marker}"));
            }
        }
        print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
        return Ok(());
    }
    if restart_acceptance {
        if output.contains("confirmed=a")
            && output.contains("pending=none")
            && output.contains("sequence=0")
        {
            print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
            return Ok(());
        }
        return Err("zeroOS: interrupted update did not restart the confirmed slot A".into());
    }
    if confirm_update {
        if output.contains("confirmed=b")
            && output.contains("sequence=1")
            && output.contains("ZEROOS_ACCEPT phase=after-health-confirmation")
        {
            print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
            return Ok(());
        }
        return Err("zeroOS: installed update was not health-confirmed".into());
    }
    if confirm_rollback {
        if output.contains("confirmed=a")
            && output.contains("pending=none")
            && output.contains("sequence=1")
        {
            print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
            return Ok(());
        }
        return Err("zeroOS: reboot-before-confirmation did not roll back to slot A".into());
    }
    if recovery_acceptance {
        for evidence in [
            "ERR ZEROOS/1 BAD_COMMAND",
            "ZEROOS_ACCEPT phase=after-repair-boot",
            "ZEROOS_ACCEPT phase=after-repair-data",
            "ZEROOS_ACCEPT phase=after-factory-reset",
            "mode=recovery",
            "pending=none",
            "data=mounted",
        ] {
            if !output.contains(evidence) {
                return Err(format!("zeroOS: recovery acceptance missing {evidence}"));
            }
        }
        print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
        return Ok(());
    }
    if expectation == BootExpectation::Rejected {
        print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
        return if output.contains(READY) {
            Err(format!("zeroOS: rejected scenario {name} reached init"))
        } else {
            Ok(())
        };
    }
    if !status.success() {
        return Err(format!("zeroOS: QEMU failed with {status}"));
    }
    verify_runtime_acceptance(&output)?;
    print_scenario_hashes(&format!("{name}-after"), image, &vars)?;
    let mode = match expectation {
        BootExpectation::Normal => "mode=normal",
        BootExpectation::Recovery => "mode=recovery",
        BootExpectation::Rejected => return Ok(()),
    };
    if output.contains(mode) {
        Ok(())
    } else {
        Err(format!("zeroOS: scenario {name} missing {mode}"))
    }
}

fn print_scenario_hashes(name: &str, image: &Path, vars: &Path) -> Result<(), String> {
    let mut disk = fs::File::open(image).map_err(error_string)?;
    let mut buffer = [0; 64 * 1024];
    for partition in zeroos_storage::PARTITIONS {
        let offset = partition
            .first
            .checked_mul(zeroos_storage::SECTOR)
            .ok_or_else(|| "zeroOS: partition hash offset overflow".to_owned())?;
        let sectors = partition
            .last
            .checked_sub(partition.first)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| "zeroOS: partition hash size overflow".to_owned())?;
        let mut remaining = sectors
            .checked_mul(zeroos_storage::SECTOR)
            .ok_or_else(|| "zeroOS: partition hash size overflow".to_owned())?;
        disk.seek(std::io::SeekFrom::Start(offset))
            .map_err(error_string)?;
        let mut context = ring::digest::Context::new(&ring::digest::SHA256);
        while remaining != 0 {
            let wanted = buffer
                .len()
                .min(usize::try_from(remaining).map_err(|_| "zeroOS: partition too large")?);
            std::io::Read::read_exact(&mut disk, &mut buffer[..wanted]).map_err(error_string)?;
            context.update(&buffer[..wanted]);
            remaining = remaining
                .checked_sub(u64::try_from(wanted).map_err(|_| "zeroOS: partition too large")?)
                .ok_or_else(|| "zeroOS: partition hash underflow".to_owned())?;
        }
        println!(
            "zeroOS scenario {name} partition {} sha256={}",
            partition.name,
            hex_digest(context.finish().as_ref())
        );
    }
    println!(
        "zeroOS scenario {name} variables sha256={}",
        hex_digest(
            ring::digest::digest(
                &ring::digest::SHA256,
                &fs::read(vars).map_err(error_string)?
            )
            .as_ref()
        )
    );
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
    signer_smoke()?;
    command("cargo", &["deny", "check"])?;
    fuzz_smoke()?;
    reproducible_build()
}

fn signer_smoke() -> Result<(), String> {
    let root = env::temp_dir().join(format!("zeroos-signer-smoke-{}", std::process::id()));
    if root.exists() {
        fs::remove_dir_all(&root).map_err(error_string)?;
    }
    fs::create_dir_all(root.join("tokens")).map_err(error_string)?;
    let _cleanup = CleanupDir(root.clone());
    let module = package_file("libsofthsm2", "/libsofthsm2.so")?;
    let engine = package_file("libengine-pkcs11-openssl", "/engines-3/pkcs11.so")?;
    let softhsm = root.join("softhsm2.conf");
    fs::write(
        &softhsm,
        format!(
            "directories.tokendir = {}\nobjectstore.backend = file\nlog.level = ERROR\nslots.removable = false\n",
            root.join("tokens").display()
        ),
    )
    .map_err(error_string)?;
    let pin = "000100000000";
    let mut initialize = Command::new("softhsm2-util");
    initialize.env("SOFTHSM2_CONF", &softhsm).args([
        "--init-token",
        "--free",
        "--label",
        "zeroos-test",
        "--so-pin",
        "00000000",
        "--pin",
        pin,
    ]);
    run(&mut initialize, "initialize disposable PKCS#11 token")?;
    let mut generate = Command::new("pkcs11-tool");
    generate
        .env("SOFTHSM2_CONF", &softhsm)
        .args(["--module"])
        .arg(&module)
        .args([
            "--login",
            "--pin",
            pin,
            "--token-label",
            "zeroos-test",
            "--keypairgen",
            "--key-type",
            "rsa:3072",
            "--label",
            "release",
            "--id",
            "03",
        ]);
    run(&mut generate, "generate disposable RSA-3072 PKCS#11 object")?;
    let public_der = root.join("release.der");
    let mut export = Command::new("pkcs11-tool");
    export
        .env("SOFTHSM2_CONF", &softhsm)
        .args(["--module"])
        .arg(&module)
        .args([
            "--login",
            "--pin",
            pin,
            "--token-label",
            "zeroos-test",
            "--read-object",
            "--type",
            "pubkey",
            "--label",
            "release",
            "--output-file",
        ])
        .arg(&public_der);
    run(&mut export, "export disposable PKCS#11 public key")?;
    let public_pem = root.join("release.pem");
    let mut convert = Command::new("openssl");
    convert
        .args(["pkey", "-pubin", "-inform", "DER", "-in"])
        .arg(&public_der)
        .args(["-out"])
        .arg(&public_pem);
    run(&mut convert, "convert disposable public key")?;
    let openssl = root.join("openssl.cnf");
    fs::write(
        &openssl,
        format!(
            "openssl_conf = zeroos_init\n[zeroos_init]\nengines = zeroos_engines\n[zeroos_engines]\npkcs11 = zeroos_pkcs11\n[zeroos_pkcs11]\nengine_id = pkcs11\ndynamic_path = {}\nMODULE_PATH = {}\nPIN = {}\ninit = 0\n",
            engine.display(), module.display(), pin
        ),
    )
    .map_err(error_string)?;
    let fingerprint = root.join("fingerprints.sha256");
    fs::write(
        &fingerprint,
        "0000000000000000000000000000000000000000000000000000000000000000  release-current.der\n",
    )
    .map_err(error_string)?;
    let operator = root.join("operator.conf");
    fs::write(
        &operator,
        format!(
            "engine=pkcs11\nselector-key=pkcs11:token=zeroos-test;object=release;type=private\nproduction-key=pkcs11:token=zeroos-test;object=release;type=private\nrelease-key=pkcs11:token=zeroos-test;object=release;type=private\nselector-cert={}\nproduction-cert={}\nrecovery-cert={}\nrelease-signer=release-current\nfingerprints={}\n",
            public_pem.display(), public_pem.display(), public_pem.display(), fingerprint.display()
        ),
    )
    .map_err(error_string)?;
    let mut build = Command::new("cargo");
    build.args([
        "build",
        "--locked",
        "--package",
        "zeroos-sign",
        "--features",
        "test-config",
    ]);
    run(&mut build, "build disposable signer adapter")?;
    let payload = root.join("system.efi");
    fs::write(&payload, vec![0x5a; 4096]).map_err(error_string)?;
    let slot = root.join("system.slot");
    let mut package = Command::new("target/debug/zeroos-sign");
    package
        .env("SOFTHSM2_CONF", &softhsm)
        .env("OPENSSL_CONF", &openssl)
        .env("ZEROOS_SIGN_TEST_CONFIG", &operator)
        .args(["package", "--arch", env::consts::ARCH, "--sequence", "1"])
        .arg("--payload")
        .arg(&payload)
        .arg("--output")
        .arg(&slot);
    run(
        &mut package,
        "sign package through disposable PKCS#11 token",
    )?;
    verify_signer_slot(&slot, &public_pem, env::consts::ARCH)
}

fn package_file(package: &str, suffix: &str) -> Result<PathBuf, String> {
    output("dpkg-query", &["-L", package])?
        .lines()
        .find(|line| line.ends_with(suffix))
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("zeroOS: {package} did not install {suffix}"))
}

fn verify_signer_slot(slot: &Path, public_key: &Path, arch: &str) -> Result<(), String> {
    let mut file = fs::File::open(slot).map_err(error_string)?;
    let mut header = [0; 12];
    file.read_exact(&mut header).map_err(error_string)?;
    let manifest_size = zeroos_storage::container_manifest_size(&header).map_err(str::to_owned)?;
    let mut manifest = vec![0; manifest_size];
    file.read_exact(&mut manifest).map_err(error_string)?;
    zeroos_storage::Manifest::parse(&manifest, arch).map_err(str::to_owned)?;
    let mut signature = vec![0; zeroos_storage::SIGNATURE_BYTES];
    file.read_exact(&mut signature).map_err(error_string)?;
    let root = slot
        .parent()
        .ok_or_else(|| "zeroOS: slot has no parent".to_owned())?;
    let manifest_path = root.join("signed-manifest");
    let signature_path = root.join("signed-manifest.sig");
    fs::write(&manifest_path, manifest).map_err(error_string)?;
    fs::write(&signature_path, signature).map_err(error_string)?;
    let mut verify = Command::new("openssl");
    verify
        .args(["dgst", "-sha256", "-verify"])
        .arg(public_key)
        .args(["-signature"])
        .arg(signature_path)
        .args([
            "-sigopt",
            "rsa_padding_mode:pss",
            "-sigopt",
            "rsa_pss_saltlen:32",
        ])
        .arg(manifest_path);
    run(&mut verify, "verify disposable PKCS#11 package signature")
}

fn fuzz_smoke() -> Result<(), String> {
    let mut fuzz = Command::new("cargo");
    fuzz.env("RUSTC_BOOTSTRAP", "1").args([
        "fuzz",
        "run",
        "m3_parsers",
        "--fuzz-dir",
        "fuzz",
        "--",
        "-runs=1000",
        "-seed=1785888000",
        "-max_len=8192",
    ]);
    run(&mut fuzz, "run bounded M3 parser fuzz smoke test")
}

fn check_tools() -> Result<(), String> {
    for (program, args, expected) in [
        ("rustc", &["--version"][..], "rustc 1.97.1"),
        ("cargo", &["--version"][..], "cargo 1.97.1"),
        ("clang-19", &["--version"][..], "19.1.7"),
        ("ld.lld-19", &["--version"][..], "19.1.7"),
        ("dpkg-query", &["-W", "llvm-19"][..], "19.1.7"),
        ("cargo", &["deny", "--version"][..], "0.19.4"),
        ("cargo", &["fuzz", "--version"][..], "0.12.0"),
        ("dpkg-query", &["-W", "musl-tools"][..], "1.2.5"),
        ("dpkg-query", &["-W", "jq"][..], "1.7.1-6+deb13u1"),
        (
            "dpkg-query",
            &["-W", "libengine-pkcs11-openssl"][..],
            "0.4.13-1",
        ),
        ("dpkg-query", &["-W", "opensc"][..], "0.26.1-2"),
        ("dpkg-query", &["-W", "softhsm2"][..], "2.6.1-3"),
        ("dpkg-query", &["-W", "sbsigntool"][..], "0.9.4-3.2"),
        ("dpkg-query", &["-W", "efitools"][..], "1.9.2-3.5"),
        (
            "dpkg-query",
            &["-W", "python3-virt-firmware"][..],
            "24.11-2",
        ),
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
        for binary in ["xtask", "zeroos-sign"] {
            let first_binary = first.join("release").join(binary);
            let second_binary = second.join("release").join(binary);
            identical_files(&first_binary, &second_binary)?;
            println!(
                "zeroOS release {binary}: {}",
                output_path("sha256sum", &[&first_binary])?
            );
        }
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
        .args([
            "build",
            "--release",
            "--locked",
            "--package",
            "xtask",
            "--package",
            "zeroos-sign",
        ])
        .env("CARGO_TARGET_DIR", target);
    run(&mut cargo, "build clean release tools")
}

fn validate_repository(root: &Path) -> Result<(), String> {
    validate_ai_policy(root)?;
    validate_build_inputs(root)?;
    validate_kernel_network_boundary(root)?;
    validate_manifest(&fs::read_to_string(root.join("Cargo.toml")).map_err(error_string)?)?;
    let ledger = fs::read_to_string(root.join("policy/dependencies.csv")).map_err(error_string)?;
    let admitted = validate_ledger(&ledger)?;
    let lock = fs::read_to_string(root.join("Cargo.lock")).map_err(error_string)?;
    validate_cargo_lock(&lock, &admitted)?;
    let fuzz_lock = fs::read_to_string(root.join("fuzz/Cargo.lock")).map_err(error_string)?;
    validate_cargo_lock(&fuzz_lock, &admitted)?;
    let sources = fs::read_to_string(root.join("policy/sources.lock")).map_err(error_string)?;
    validate_source_lock(&sources)?;
    linux_source(&sources).map(|_| ())
}

fn validate_kernel_network_boundary(root: &Path) -> Result<(), String> {
    for arch in ["x86_64", "aarch64"] {
        let production =
            fs::read_to_string(root.join(format!("kernel/{arch}.config"))).map_err(error_string)?;
        let acceptance = fs::read_to_string(root.join(format!("kernel/{arch}-acceptance.config")))
            .map_err(error_string)?;
        if !production
            .lines()
            .any(|line| line == "# CONFIG_IP_PNP is not set")
            || production.contains("CONFIG_IP_PNP_DHCP=y")
            || !acceptance.lines().any(|line| line == "CONFIG_IP_PNP=y")
            || !acceptance
                .lines()
                .any(|line| line == "CONFIG_IP_PNP_DHCP=y")
            || !acceptance.lines().any(|line| line == "CONFIG_VIRTIO_NET=y")
            || !acceptance.contains("ip=dhcp")
        {
            return Err(format!(
                "zeroOS: kernel DHCP must be disabled in production and enabled only for {arch} acceptance"
            ));
        }
    }
    Ok(())
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
        || !docker.contains("cargo install --locked cargo-fuzz --version 0.12.0")
        || !docker.contains("jq=1.7.1-6+deb13u1")
        || !docker.contains("libengine-pkcs11-openssl=0.4.13-1")
        || !docker.contains("opensc=0.26.1-2")
        || !docker.contains("softhsm2=2.6.1-3")
    {
        return Err(
            "zeroOS: Docker build inputs must pin image digest, Debian snapshot, cargo-deny, cargo-fuzz, and jq"
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
        "members = [\"xtask\", \"init\", \"storage\", \"updater\", \"data\", \"selector\", \"signer\"]",
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
        assert_eq!(
            parse(vec![
                "test-release".into(),
                "--arch".into(),
                "x86_64".into(),
                "--sequence".into(),
                "1".into(),
                "--url".into(),
                "https://github.com/KZagaja/zeroOS-releases/releases/tag/sequence-1".into(),
            ]),
            Ok(Action::TestRelease {
                arch: Arch::X86_64,
                sequence: 1,
                url: "https://github.com/KZagaja/zeroOS-releases/releases/tag/sequence-1".into(),
            })
        );
        assert!(
            parse(vec![
                "test-release".into(),
                "--arch".into(),
                "x86_64".into(),
                "--sequence".into(),
                "0".into(),
                "--url".into(),
                "https://example.test".into(),
            ])
            .is_err()
        );
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
    fn production_scan_rejects_split_acceptance_markers() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = env::temp_dir().join(format!("zeroos-artifact-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        let clean = root.join("clean");
        fs::write(&clean, vec![b'x'; 128 * 1024])?;
        assert!(verify_production_artifact(&clean).is_ok());

        let contaminated = root.join("contaminated");
        let mut bytes = vec![b'x'; 64 * 1024 - 5];
        bytes.extend_from_slice(b"ZEROOS_ACCEPT phase=before-download");
        fs::write(&contaminated, bytes)?;
        assert!(verify_production_artifact(&contaminated).is_err());
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
