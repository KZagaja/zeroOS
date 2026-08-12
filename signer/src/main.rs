use ring::digest::{Context, SHA256};
use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

const VERSION: &str = "zeroos-sign 0.1.0";
const CONFIG: &str = "/etc/zeroos-sign/operator.conf";

#[derive(Debug, Eq, PartialEq)]
enum Action {
    Version,
    VerifyRecovery {
        arch: String,
        payload: PathBuf,
    },
    SignEfi {
        role: String,
        input: PathBuf,
        output: PathBuf,
    },
    Package {
        arch: String,
        sequence: u64,
        payload: PathBuf,
        output: PathBuf,
    },
    Provenance {
        source: String,
        arch: String,
        sequence: u64,
        hashes: PathBuf,
        output: PathBuf,
    },
}

struct Config {
    engine: String,
    selector_key: String,
    production_key: String,
    release_key: String,
    selector_cert: PathBuf,
    production_cert: PathBuf,
    recovery_cert: PathBuf,
    release_signer: String,
    fingerprints: PathBuf,
}

fn main() -> ExitCode {
    match parse(env::args().skip(1).collect()).and_then(execute) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zeroos-sign: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse(args: Vec<String>) -> Result<Action, String> {
    match args.as_slice() {
        [flag] if flag == "--version" => Ok(Action::Version),
        [command, arch_flag, arch, payload_flag, payload]
            if command == "verify-recovery"
                && arch_flag == "--arch"
                && payload_flag == "--payload" =>
        {
            validate_arch(arch)?;
            Ok(Action::VerifyRecovery {
                arch: arch.clone(),
                payload: payload.into(),
            })
        }
        [
            command,
            role_flag,
            role,
            input_flag,
            input,
            output_flag,
            output,
        ] if command == "sign-efi"
            && role_flag == "--role"
            && input_flag == "--input"
            && output_flag == "--output"
            && matches!(role.as_str(), "selector" | "production") =>
        {
            Ok(Action::SignEfi {
                role: role.clone(),
                input: input.into(),
                output: output.into(),
            })
        }
        [
            command,
            arch_flag,
            arch,
            sequence_flag,
            sequence,
            payload_flag,
            payload,
            output_flag,
            output,
        ] if command == "package"
            && arch_flag == "--arch"
            && sequence_flag == "--sequence"
            && payload_flag == "--payload"
            && output_flag == "--output" =>
        {
            validate_arch(arch)?;
            Ok(Action::Package {
                arch: arch.clone(),
                sequence: positive_sequence(sequence)?,
                payload: payload.into(),
                output: output.into(),
            })
        }
        [
            command,
            source_flag,
            source,
            arch_flag,
            arch,
            sequence_flag,
            sequence,
            hashes_flag,
            hashes,
            output_flag,
            output,
        ] if command == "provenance"
            && source_flag == "--source"
            && arch_flag == "--arch"
            && sequence_flag == "--sequence"
            && hashes_flag == "--hashes"
            && output_flag == "--output" =>
        {
            validate_source(source)?;
            validate_arch(arch)?;
            Ok(Action::Provenance {
                source: source.clone(),
                arch: arch.clone(),
                sequence: positive_sequence(sequence)?,
                hashes: hashes.into(),
                output: output.into(),
            })
        }
        _ => Err("invalid arguments".into()),
    }
}

fn execute(action: Action) -> Result<(), String> {
    if action == Action::Version {
        println!("{VERSION}");
        return Ok(());
    }
    let config = load_config(config_path())?;
    match action {
        Action::Version => Ok(()),
        Action::VerifyRecovery { arch, payload } => {
            validate_efi_name(&payload, "zeroos-recovery-", &arch)?;
            run(
                Command::new("sbverify")
                    .arg("--cert")
                    .arg(config.recovery_cert)
                    .arg(payload),
                "recovery signature verification failed",
            )
        }
        Action::SignEfi {
            role,
            input,
            output,
        } => {
            regular_input(&input, zeroos_storage::SLOT_BYTES)?;
            let (key, cert) = if role == "selector" {
                (config.selector_key, config.selector_cert)
            } else {
                (config.production_key, config.production_cert)
            };
            run(
                Command::new("sbsign")
                    .arg("--engine")
                    .arg(config.engine)
                    .arg("--key")
                    .arg(key)
                    .arg("--cert")
                    .arg(cert)
                    .arg("--output")
                    .arg(&output)
                    .arg(input),
                "EFI signing failed",
            )?;
            regular_input(&output, zeroos_storage::SLOT_BYTES).map(|_| ())
        }
        Action::Package {
            arch,
            sequence,
            payload,
            output,
        } => package(&config, &arch, sequence, &payload, &output),
        Action::Provenance {
            source,
            arch,
            sequence,
            hashes,
            output,
        } => provenance(&config, &source, &arch, sequence, &hashes, &output),
    }
}

fn config_path() -> PathBuf {
    #[cfg(feature = "test-config")]
    if let Some(path) = env::var_os("ZEROOS_SIGN_TEST_CONFIG") {
        return path.into();
    }
    PathBuf::from(CONFIG)
}

fn load_config(path: PathBuf) -> Result<Config, String> {
    let metadata = fs::symlink_metadata(&path).map_err(|_| "operator configuration unavailable")?;
    if !metadata.file_type().is_file() || metadata.len() > 16 * 1024 {
        return Err("invalid operator configuration".into());
    }
    let input = fs::read_to_string(path).map_err(|_| "operator configuration unavailable")?;
    let mut fields = BTreeMap::new();
    for line in input.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or("invalid operator configuration")?;
        if value.is_empty() || fields.insert(key, value).is_some() {
            return Err("invalid operator configuration".into());
        }
    }
    let required = [
        "engine",
        "selector-key",
        "production-key",
        "release-key",
        "selector-cert",
        "production-cert",
        "recovery-cert",
        "release-signer",
        "fingerprints",
    ];
    if fields.len() != required.len() || required.iter().any(|key| !fields.contains_key(key)) {
        return Err("invalid operator configuration".into());
    }
    for key in ["selector-key", "production-key", "release-key"] {
        let value = fields[key];
        if !value.starts_with("pkcs11:")
            || ["pin-value", "pin-source", "secret", "password"]
                .iter()
                .any(|marker| value.to_ascii_lowercase().contains(marker))
        {
            return Err("private-key URI must contain only a PKCS#11 object selector".into());
        }
    }
    let path_field = |name: &str| -> Result<PathBuf, String> {
        let value = PathBuf::from(fields[name]);
        if !value.is_absolute() {
            return Err("operator paths must be absolute".into());
        }
        let metadata =
            fs::symlink_metadata(&value).map_err(|_| "operator public file unavailable")?;
        if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 {
            return Err("invalid operator public file".into());
        }
        Ok(value)
    };
    let signer = fields["release-signer"];
    if signer.len() > 128
        || !signer
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("invalid release signer".into());
    }
    Ok(Config {
        engine: fields["engine"].into(),
        selector_key: fields["selector-key"].into(),
        production_key: fields["production-key"].into(),
        release_key: fields["release-key"].into(),
        selector_cert: path_field("selector-cert")?,
        production_cert: path_field("production-cert")?,
        recovery_cert: path_field("recovery-cert")?,
        release_signer: signer.into(),
        fingerprints: path_field("fingerprints")?,
    })
}

fn package(
    config: &Config,
    arch: &str,
    sequence: u64,
    payload: &Path,
    output: &Path,
) -> Result<(), String> {
    let payload_size = regular_input(payload, zeroos_storage::MAX_PAYLOAD)?;
    let digest = hash_file(payload)?;
    let manifest = format!(
        "version=1\narch={arch}\nsequence={sequence}\npayload-size={payload_size}\nsha256={digest}\nsigner={}\n",
        config.release_signer
    );
    zeroos_storage::Manifest::parse(manifest.as_bytes(), arch)
        .map_err(|_| "generated manifest is invalid")?;
    let temporary = temporary_path(output, "slot")?;
    let manifest_path = temporary_path(output, "manifest")?;
    let signature_path = temporary_path(output, "signature")?;
    let result = (|| {
        fs::write(&manifest_path, manifest.as_bytes()).map_err(|_| "manifest write failed")?;
        run(
            Command::new("openssl")
                .args(["dgst", "-sha256", "-engine"])
                .arg(&config.engine)
                .args(["-keyform", "engine", "-sign"])
                .arg(&config.release_key)
                .args([
                    "-sigopt",
                    "rsa_padding_mode:pss",
                    "-sigopt",
                    "rsa_pss_saltlen:32",
                    "-out",
                ])
                .arg(&signature_path)
                .arg(&manifest_path),
            "manifest signing failed",
        )?;
        let signature = fs::read(&signature_path).map_err(|_| "signature read failed")?;
        if signature.len() != zeroos_storage::SIGNATURE_BYTES {
            return Err("HSM did not produce an RSA-3072 signature".into());
        }
        let mut file = fs::File::create(&temporary).map_err(|_| "package write failed")?;
        file.write_all(b"ZEROSLT1")
            .map_err(|_| "package write failed")?;
        let manifest_size = u32::try_from(manifest.len()).map_err(|_| "manifest too large")?;
        file.write_all(&manifest_size.to_le_bytes())
            .map_err(|_| "package write failed")?;
        file.write_all(manifest.as_bytes())
            .map_err(|_| "package write failed")?;
        file.write_all(&signature)
            .map_err(|_| "package write failed")?;
        std::io::copy(
            &mut fs::File::open(payload).map_err(|_| "payload unavailable")?,
            &mut file,
        )
        .map_err(|_| "package write failed")?;
        file.sync_all().map_err(|_| "package flush failed")?;
        fs::rename(&temporary, output).map_err(|_| "package activation failed")?;
        sync_parent(output)
    })();
    let _ = fs::remove_file(&temporary);
    let _ = fs::remove_file(manifest_path);
    let _ = fs::remove_file(signature_path);
    result
}

fn provenance(
    config: &Config,
    source: &str,
    arch: &str,
    sequence: u64,
    hashes: &Path,
    output: &Path,
) -> Result<(), String> {
    let artifacts = parse_hashes(hashes)?;
    let fingerprints = parse_fingerprints(&config.fingerprints)?;
    let artifacts = artifacts
        .iter()
        .map(|(name, digest)| format!("    \"{}\": \"{}\"", json(name), digest))
        .collect::<Vec<_>>()
        .join(",\n");
    let fingerprints = fingerprints
        .iter()
        .map(|digest| format!("\"{digest}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let document = format!(
        "{{\n  \"source\": \"{}\",\n  \"arch\": \"{arch}\",\n  \"sequence\": {sequence},\n  \"signer\": \"{}\",\n  \"artifacts\": {{\n{artifacts}\n  }},\n  \"public_fingerprints\": [{fingerprints}]\n}}\n",
        json(source),
        json(&config.release_signer)
    );
    atomic_write(output, document.as_bytes())
}

fn parse_hashes(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let metadata = fs::metadata(path).map_err(|_| "hash manifest unavailable")?;
    if metadata.len() > 16 * 1024 {
        return Err("hash manifest is oversized".into());
    }
    let mut result = BTreeMap::new();
    for line in fs::read_to_string(path)
        .map_err(|_| "hash manifest unavailable")?
        .lines()
    {
        let (digest, name) = line.split_once("  ").ok_or("malformed hash manifest")?;
        if !valid_digest(digest)
            || name.is_empty()
            || name.contains(['/', '\\'])
            || name.contains("..")
            || result
                .insert(name.into(), digest.to_ascii_lowercase())
                .is_some()
        {
            return Err("malformed hash manifest".into());
        }
    }
    if result.is_empty() || result.len() > 16 {
        return Err("unexpected hash manifest size".into());
    }
    Ok(result)
}

fn parse_fingerprints(path: &Path) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut names = HashSet::new();
    for line in fs::read_to_string(path)
        .map_err(|_| "fingerprint manifest unavailable")?
        .lines()
    {
        let (digest, name) = line
            .split_once("  ")
            .ok_or("malformed fingerprint manifest")?;
        if !valid_digest(digest) || name.is_empty() || name.contains('/') || !names.insert(name) {
            return Err("malformed fingerprint manifest".into());
        }
        result.push(digest.to_ascii_lowercase());
    }
    if result.is_empty() || result.len() > 16 {
        return Err("unexpected fingerprint count".into());
    }
    result.sort();
    Ok(result)
}

fn validate_arch(arch: &str) -> Result<(), String> {
    matches!(arch, "x86_64" | "aarch64")
        .then_some(())
        .ok_or_else(|| "unsupported architecture".into())
}

fn positive_sequence(value: &str) -> Result<u64, String> {
    value
        .parse()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| "release sequence must be positive".into())
}

fn validate_source(source: &str) -> Result<(), String> {
    (source.len() == 40
        && source
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(())
    .ok_or_else(|| "source must be a full lowercase commit SHA".into())
}

fn validate_efi_name(path: &Path, prefix: &str, arch: &str) -> Result<(), String> {
    let expected = format!("{prefix}{arch}.efi");
    (path.file_name().and_then(|name| name.to_str()) == Some(expected.as_str()))
        .then_some(())
        .ok_or_else(|| "unexpected EFI payload name".into())
}

fn regular_input(path: &Path, maximum: u64) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "input unavailable")?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err("invalid input file".into());
    }
    Ok(metadata.len())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|_| "input unavailable")?;
    let mut context = Context::new(&SHA256);
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "input read failed")?;
        if count == 0 {
            break;
        }
        context.update(&buffer[..count]);
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn json(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| character.escape_default())
        .collect()
}

fn temporary_path(output: &Path, suffix: &str) -> Result<PathBuf, String> {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid output path")?;
    Ok(output.with_file_name(format!(".{name}.{}.{suffix}", std::process::id())))
}

fn atomic_write(output: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = temporary_path(output, "new")?;
    let result = (|| {
        let mut file = fs::File::create(&temporary).map_err(|_| "output write failed")?;
        file.write_all(bytes).map_err(|_| "output write failed")?;
        file.sync_all().map_err(|_| "output flush failed")?;
        fs::rename(&temporary, output).map_err(|_| "output activation failed")?;
        sync_parent(output)
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn sync_parent(path: &Path) -> Result<(), String> {
    fs::File::open(path.parent().ok_or("output has no parent")?)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "output directory flush failed".into())
}

fn run(command: &mut Command, failure: &str) -> Result<(), String> {
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| failure.to_owned())?;
    if status.success() {
        Ok(())
    } else {
        Err(failure.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_is_exact() {
        assert_eq!(parse(vec!["--version".into()]), Ok(Action::Version));
        assert!(parse(vec!["package".into()]).is_err());
        assert!(
            parse(vec![
                "package".into(),
                "--arch".into(),
                "mips".into(),
                "--sequence".into(),
                "1".into(),
                "--payload".into(),
                "system.efi".into(),
                "--output".into(),
                "system.slot".into(),
            ])
            .is_err()
        );
    }

    #[test]
    fn manifests_are_strict_and_secret_free() -> Result<(), Box<dyn std::error::Error>> {
        let root = env::temp_dir().join(format!("zeroos-sign-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root)?;
        for name in [
            "selector.pem",
            "production.pem",
            "recovery.pem",
            "fingerprints.sha256",
        ] {
            fs::write(root.join(name), b"public")?;
        }
        let configuration = |key: &str| {
            format!(
                "engine=pkcs11\nselector-key={key}\nproduction-key=pkcs11:object=production;type=private\nrelease-key=pkcs11:object=release;type=private\nselector-cert={}\nproduction-cert={}\nrecovery-cert={}\nrelease-signer=release-current\nfingerprints={}\n",
                root.join("selector.pem").display(),
                root.join("production.pem").display(),
                root.join("recovery.pem").display(),
                root.join("fingerprints.sha256").display()
            )
        };
        let path = root.join("operator.conf");
        fs::write(&path, configuration("pkcs11:object=selector;type=private"))?;
        assert_eq!(load_config(path.clone())?.release_signer, "release-current");
        fs::write(
            &path,
            configuration("pkcs11:object=selector;pin-value=secret"),
        )?;
        assert!(load_config(path).is_err());
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
