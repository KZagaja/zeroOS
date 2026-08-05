use std::{
    collections::HashSet,
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};

const USAGE: &str = "usage: cargo xtask <check|test [--arch <x86_64|aarch64>]|build --arch <x86_64|aarch64>|run --arch <x86_64|aarch64>>";
const M1_MESSAGE: &str = "zeroOS: build and run are available in M1";

#[derive(Debug, PartialEq)]
enum Action {
    Check,
    Test(Option<String>),
    M1(&'static str, String),
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
        [command, flag, arch] if command == "test" && flag == "--arch" => {
            validate_arch(arch)?;
            Ok(Action::Test(Some(arch.clone())))
        }
        [command, flag, arch] if (command == "build" || command == "run") && flag == "--arch" => {
            validate_arch(arch)?;
            Ok(Action::M1(
                if command == "build" { "build" } else { "run" },
                arch.clone(),
            ))
        }
        [] => bad("zeroOS: missing command".into()),
        [command, ..] if !matches!(command.as_str(), "check" | "test" | "build" | "run") => {
            bad(format!("zeroOS: unsupported command '{command}'"))
        }
        _ => bad("zeroOS: invalid arguments".into()),
    }
}

fn validate_arch(arch: &str) -> Result<(), (u8, String)> {
    if matches!(arch, "x86_64" | "aarch64") {
        Ok(())
    } else {
        Err((2, format!("zeroOS: unsupported architecture '{arch}'")))
    }
}

fn execute(action: Action) -> Result<(), (u8, String)> {
    match action {
        Action::Check => check().map_err(failed),
        Action::Test(arch) => {
            if let Some(arch) = arch {
                println!("zeroOS: testing architecture context {arch}");
            }
            command("cargo", &["test", "--workspace", "--locked"]).map_err(failed)
        }
        Action::M1(command, arch) => Err((1, format!("{M1_MESSAGE} ({command} --arch {arch})"))),
    }
}

fn failed(message: String) -> (u8, String) {
    (1, message)
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

fn command(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| format!("zeroOS: failed to start {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "zeroOS: {program} {} failed with {status}",
            args.join(" ")
        ))
    }
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

fn check_tools() -> Result<(), String> {
    for (program, args, expected) in [
        ("rustc", &["--version"][..], "rustc 1.97.1"),
        ("cargo", &["--version"][..], "cargo 1.97.1"),
        ("clang-19", &["--version"][..], "19.1.7"),
        ("ld.lld-19", &["--version"][..], "19.1.7"),
        ("dpkg-query", &["-W", "llvm-19"][..], "19.1.7"),
        ("cargo", &["deny", "--version"][..], "0.19.4"),
        ("dpkg-query", &["-W", "musl-tools"][..], "1.2.5"),
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
            output("sha256sum", &[&first_binary.to_string_lossy()])?
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
        Err("zeroOS: clean release builds differ".into())
    }
}

fn build_into(target: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--locked", "--package", "xtask"])
        .env("CARGO_TARGET_DIR", target)
        .status()
        .map_err(error_string)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("zeroOS: clean release build failed with {status}"))
    }
}

fn validate_repository(root: &Path) -> Result<(), String> {
    validate_manifest(&fs::read_to_string(root.join("Cargo.toml")).map_err(error_string)?)?;
    let ledger = fs::read_to_string(root.join("policy/dependencies.csv")).map_err(error_string)?;
    let admitted = validate_ledger(&ledger)?;
    let lock = fs::read_to_string(root.join("Cargo.lock")).map_err(error_string)?;
    validate_cargo_lock(&lock, &admitted)?;
    validate_source_lock(
        &fs::read_to_string(root.join("policy/sources.lock")).map_err(error_string)?,
    )
}

fn validate_manifest(manifest: &str) -> Result<(), String> {
    for required in [
        "members = [\"xtask\"]",
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
    fn parses_interface_and_invalid_architectures() {
        assert_eq!(parse(vec!["check".into()]), Ok(Action::Check));
        assert_eq!(
            parse(vec!["test".into(), "--arch".into(), "aarch64".into()]),
            Ok(Action::Test(Some("aarch64".into())))
        );
        assert!(parse(vec!["test".into(), "--arch".into(), "mips".into()]).is_err());
        assert!(parse(vec!["build".into()]).is_err());
    }

    #[test]
    fn validates_sources_locks_and_license_policy() {
        assert!(validate_source_lock("name,version,url,sha256\n").is_ok());
        assert!(validate_source_lock("name,version,url,sha256\nlinux,1,url,nope\n").is_err());
        let sum = "a".repeat(64);
        assert!(
            validate_source_lock(&format!(
                "name,version,url,sha256\nlinux,1,url,{sum}\nlinux,1,url,{sum}\n"
            ))
            .is_err()
        );
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
    fn propagates_command_failure() {
        assert!(command("rustc", &["--definitely-not-a-rustc-option"]).is_err());
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
