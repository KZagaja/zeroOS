use std::{
    collections::HashSet,
    env, fs,
    fs::File,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

const USAGE: &str = "usage: cargo xtask <check|test [--arch <x86_64|aarch64>]|build --arch <x86_64|aarch64>|run --arch <x86_64|aarch64>>";
const LINUX_VERSION: &str = "6.18.42";
const READY: &str = "zeroOS init: READY";
const ESP_OFFSET: u64 = 1_048_576;
const ESP_SECTORS: &str = "126976";

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
    Path::new("target/m1").join(arch.name())
}

fn build(arch: Arch) -> Result<(), String> {
    ensure_native(arch)?;
    let output_dir = artifacts(arch);
    fs::create_dir_all(&output_dir).map_err(error_string)?;
    let source = fetch_linux()?;
    build_init(arch, &output_dir)?;
    build_kernel(arch, &source, &output_dir)?;
    package_disk(arch, &output_dir)?;
    inspect(arch, &output_dir)?;
    print_hashes(arch, &output_dir)
}

fn fetch_linux() -> Result<PathBuf, String> {
    let (url, expected) =
        linux_source(&fs::read_to_string("policy/sources.lock").map_err(error_string)?)?;
    let cache = Path::new("target/m1/sources");
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
    if !source.join("Makefile").is_file() {
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
    verify_static(&output_dir.join("init"))?;
    fs::write(
        output_dir.join("initramfs.list"),
        format!(
            "dir /dev 0755 0 0\nnod /dev/console 0600 0 0 c 5 1\nnod /dev/null 0666 0 0 c 1 3\nfile /init {} 0755 0 0\n",
            output_dir
                .join("init")
                .canonicalize()
                .map_err(error_string)?
                .display()
        ),
    )
    .map_err(error_string)
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
    truncate.args(["-s", "64M"]).arg(&image);
    run(&mut truncate, "allocate disk image")?;

    let mut gpt = Command::new("sgdisk");
    gpt.args([
        "--clear",
        "--disk-guid=5A45524F-4F53-4D31-8000-000000000001",
        "--new=1:2048:129023",
        "--typecode=1:EF00",
        "--partition-guid=1:5A45524F-4F53-4D31-8000-000000000002",
    ])
    .arg(&image);
    run(&mut gpt, "create GPT")?;

    let spec = format!("{}@@{ESP_OFFSET}", image.display());
    let mut format = Command::new("mformat");
    format.env("MTOOLS_SKIP_CHECK", "1").args([
        "-i",
        &spec,
        "-F",
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
    fs::copy(output_dir.join("kernel.efi"), boot.join(arch.fallback())).map_err(error_string)?;
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
    run(&mut copy, "populate ESP")
}

fn inspect(arch: Arch, output_dir: &Path) -> Result<(), String> {
    let image = output_dir.join("zeroos.img");
    let bytes = fs::read(&image).map_err(error_string)?;
    validate_signatures(&bytes)?;
    let mut verify = Command::new("sgdisk");
    verify.arg("--verify").arg(&image);
    run(&mut verify, "verify GPT")?;
    let info = output_path("sgdisk", &[Path::new("--info=1"), &image])?;
    if !info.contains("EFI system partition") {
        return Err("zeroOS: partition 1 is not an EFI System Partition".into());
    }
    let esp = output_dir.join("esp.fat");
    let mut dd = Command::new("dd");
    dd.arg(format!("if={}", image.display()))
        .arg(format!("of={}", esp.display()))
        .args(["bs=512", "skip=2048", "count=126976", "status=none"]);
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
    identical_files(&extracted, &output_dir.join("kernel.efi"))?;
    verify_static(&output_dir.join("init"))?;
    let manifest = fs::read_to_string(output_dir.join("initramfs.list")).map_err(error_string)?;
    if !manifest.starts_with("dir /dev 0755 0 0\nnod /dev/console 0600 0 0 c 5 1\n")
        || !manifest.contains("nod /dev/null 0666 0 0 c 1 3\nfile /init ")
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
    let log_path = output_dir.join("qemu.log");
    let log = File::create(&log_path).map_err(error_string)?;
    qemu.stdout(Stdio::from(log.try_clone().map_err(error_string)?))
        .stderr(Stdio::from(log));
    let status = run_with_timeout(&mut qemu, Duration::from_secs(90))?;
    if !status.success() {
        return Err(format!("zeroOS: QEMU failed with {status}"));
    }
    let output = fs::read_to_string(&log_path).map_err(error_string)?;
    verify_readiness(&output)
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
    for name in ["kernel.efi", "init", "zeroos.img"] {
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
    validate_manifest(&fs::read_to_string(root.join("Cargo.toml")).map_err(error_string)?)?;
    let ledger = fs::read_to_string(root.join("policy/dependencies.csv")).map_err(error_string)?;
    let admitted = validate_ledger(&ledger)?;
    let lock = fs::read_to_string(root.join("Cargo.lock")).map_err(error_string)?;
    validate_cargo_lock(&lock, &admitted)?;
    let sources = fs::read_to_string(root.join("policy/sources.lock")).map_err(error_string)?;
    validate_source_lock(&sources)?;
    linux_source(&sources).map(|_| ())
}

fn validate_manifest(manifest: &str) -> Result<(), String> {
    for required in [
        "members = [\"xtask\", \"init\"]",
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
    let header = "package,version,need,owner,maintenance,license,architectures,attack_surface,classification";
    let mut lines = input.lines();
    if lines.next() != Some(header) {
        return Err("zeroOS: invalid dependency ledger header".into());
    }
    let mut admitted = HashSet::new();
    for (index, line) in lines.filter(|line| !line.is_empty()).enumerate() {
        let fields: Vec<_> = line.split(',').collect();
        if fields.len() != 9
            || fields.iter().any(|field| field.trim().is_empty())
            || !matches!(fields[8], "Retain" | "Replace")
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
    fn rejects_different_build_outputs() {
        let root = env::temp_dir().join(format!("zeroos-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        assert!(identical_files(&first, &second).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
