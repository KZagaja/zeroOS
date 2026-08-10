use ring::{digest, signature};
use std::{
    env, fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use ureq::{Agent, http::StatusCode};
use zeroos_storage::{
    MAX_MANIFEST, MAX_PAYLOAD, Manifest, RELEASE_URL, SIGNATURE_BYTES, SLOT_BYTES,
};

const MAGIC: &[u8; 8] = b"ZEROSLT1";
const BUILD_EPOCH: u64 = 1_785_888_000;
const MAX_CONTAINER: u64 = 12 + MAX_MANIFEST as u64 + SIGNATURE_BYTES as u64 + MAX_PAYLOAD;
const MAX_REDIRECTS: usize = 5;
const METADATA_LIMIT: u64 = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
struct DownloadMetadata {
    url: String,
    etag: String,
    length: u64,
    written: u64,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ZEROOS_UPDATE phase=failed error={error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let check_only = env::args().nth(1).as_deref() == Some("--check");
    let arch = match env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => return Err(format!("unsupported-arch:{other}")),
    };
    let build_epoch = option_env!("ZEROOS_BUILD_EPOCH")
        .and_then(|value| value.parse().ok())
        .unwrap_or(BUILD_EPOCH);
    if SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "invalid-time")?
        .as_secs()
        < build_epoch
    {
        return Err("invalid-time".into());
    }

    let url = release_url(arch);
    let work = PathBuf::from(
        env::var_os("ZEROOS_UPDATE_DIR").unwrap_or_else(|| "/var/lib/zeroos/update".into()),
    );
    fs::create_dir_all(&work).map_err(redact)?;
    let download = work.join(format!("zeroos-{arch}.slot.part"));
    let metadata = work.join(format!("zeroos-{arch}.resume"));
    println!("ZEROOS_UPDATE phase=download");
    download_asset(&url, &download, &metadata, arch)?;

    let current_sequence = env::var("ZEROOS_SEQUENCE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    println!("ZEROOS_UPDATE phase=verify");
    let manifest = verify_container(&download, arch, current_sequence, true)?;
    if check_only {
        println!(
            "ZEROOS_UPDATE phase=complete state=available sequence={}",
            manifest.sequence
        );
        return Ok(());
    }

    let inactive = env::var_os("ZEROOS_INACTIVE_SLOT")
        .map(PathBuf::from)
        .ok_or("missing-inactive-slot")?;
    validate_inactive_target(&inactive, arch)?;
    println!("ZEROOS_UPDATE phase=slot-write");
    write_slot(&download, &inactive)?;
    println!("ZEROOS_UPDATE phase=reread");
    let reread = verify_container(&inactive, arch, current_sequence, false)?;
    if reread != manifest {
        return Err("slot-reread-mismatch".into());
    }
    println!(
        "ZEROOS_UPDATE phase=complete state=staged sequence={} slot={}",
        manifest.sequence,
        inactive.display()
    );
    Ok(())
}

fn release_url(arch: &str) -> String {
    #[cfg(feature = "acceptance")]
    if let Some(origin) = option_env!("ZEROOS_ACCEPTANCE_ORIGIN") {
        return format!("{}/zeroos-{arch}.slot", origin.trim_end_matches('/'));
    }
    format!("{RELEASE_URL}zeroos-{arch}.slot")
}

fn agent() -> Result<Agent, String> {
    let builder = Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .proxy(None)
        .max_response_header_size(32 * 1024)
        .timeout_global(Some(Duration::from_secs(15 * 60)));
    #[cfg(feature = "acceptance")]
    let builder = if let Some(pem) = option_env!("ZEROOS_ACCEPTANCE_CA_PEM") {
        let certificate = ureq::tls::Certificate::from_pem(pem.as_bytes())
            .map_err(|_| "invalid-acceptance-ca")?;
        builder.tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::from([certificate]))
                .build(),
        )
    } else {
        builder
    };
    Ok(builder.build().new_agent())
}

fn download_asset(
    url: &str,
    output: &Path,
    metadata_path: &Path,
    arch: &str,
) -> Result<(), String> {
    if !allowed_url(url, arch) {
        return Err("forbidden-url".into());
    }
    let mut metadata = load_metadata(metadata_path).ok().filter(|metadata| {
        metadata.url == url
            && strong_etag(&metadata.etag)
            && metadata.written <= metadata.length
            && metadata.length <= MAX_CONTAINER
    });
    let mut written = metadata.as_ref().map_or(0, |value| value.written);
    let actual = fs::metadata(output).map(|value| value.len()).unwrap_or(0);
    if actual < written {
        written = 0;
        metadata = None;
        truncate_and_sync(output, 0)?;
    } else if actual > written {
        truncate_and_sync(output, written)?;
    }

    let agent = agent()?;
    let mut current = url.to_owned();
    for redirects in 0..=MAX_REDIRECTS {
        if !allowed_url(&current, arch) {
            return Err("forbidden-redirect".into());
        }
        let mut request = agent.get(&current).header("Accept-Encoding", "identity");
        if let Some(saved) = metadata.as_ref().filter(|_| written != 0) {
            request = request
                .header("Range", format!("bytes={written}-"))
                .header("If-Range", &saved.etag);
        }
        let mut response = request.call().map_err(|_| "download-failed")?;
        if response.status().is_redirection() {
            if redirects == MAX_REDIRECTS {
                return Err("too-many-redirects".into());
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .ok_or("bad-redirect")?;
            if !allowed_url(location, arch) {
                return Err("forbidden-redirect".into());
            }
            current = location.to_owned();
            continue;
        }

        let etag = response_header(&response, "etag")?;
        if !strong_etag(&etag) {
            return Err("weak-etag".into());
        }
        match response.status() {
            StatusCode::PARTIAL_CONTENT if written != 0 => {
                let saved = metadata.as_ref().ok_or("missing-resume-metadata")?;
                if etag != saved.etag
                    || response_header(&response, "content-range")?
                        != expected_content_range(written, saved.length)?
                    || content_length(&response)? != saved.length - written
                {
                    return Err("inconsistent-resume".into());
                }
            }
            StatusCode::OK => {
                let length = content_length(&response)?;
                if length == 0 || length > MAX_CONTAINER {
                    return Err("bad-content-length".into());
                }
                truncate_and_sync(output, 0)?;
                metadata = Some(DownloadMetadata {
                    url: url.into(),
                    etag,
                    length,
                    written: 0,
                });
                store_metadata(metadata_path, metadata.as_ref().ok_or("metadata-failed")?)?;
            }
            _ => return Err("bad-download-status".into()),
        }

        let saved = metadata.as_mut().ok_or("missing-download-metadata")?;
        let mut reader = response.body_mut().as_reader();
        persist_body(&mut reader, output, metadata_path, saved)?;
        return Ok(());
    }
    Err("too-many-redirects".into())
}

fn persist_body(
    reader: &mut impl Read,
    output: &Path,
    metadata_path: &Path,
    metadata: &mut DownloadMetadata,
) -> Result<(), String> {
    let mut destination = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(output)
        .map_err(redact)?;
    destination
        .seek(SeekFrom::Start(metadata.written))
        .map_err(redact)?;
    let mut buffer = [0; 256 * 1024];
    while metadata.written < metadata.length {
        let remaining = metadata
            .length
            .checked_sub(metadata.written)
            .ok_or("bad-content-length")?;
        let wanted = buffer
            .len()
            .min(usize::try_from(remaining).map_err(|_| "bad-content-length")?);
        let count = reader.read(&mut buffer[..wanted]).map_err(redact)?;
        if count == 0 {
            return Err("short-download".into());
        }
        destination.write_all(&buffer[..count]).map_err(redact)?;
        destination.sync_all().map_err(redact)?;
        metadata.written = metadata
            .written
            .checked_add(u64::try_from(count).map_err(|_| "bad-content-length")?)
            .ok_or("bad-content-length")?;
        store_metadata(metadata_path, metadata)?;
    }
    Ok(())
}

fn allowed_url(url: &str, arch: &str) -> bool {
    if url.contains('#') || url.contains('@') || !url.starts_with("https://") {
        return false;
    }
    let Some((authority, path)) = url[8..].split_once('/') else {
        return false;
    };
    if authority.is_empty() || path.contains("..") {
        return false;
    }
    let asset = format!("zeroos-{arch}.slot");
    let production = !authority.contains(':')
        && match authority {
            "github.com" => {
                path == format!("KZagaja/zeroOS-releases/releases/latest/download/{asset}")
                    || (path.starts_with("KZagaja/zeroOS-releases/releases/download/")
                        && path.ends_with(&format!("/{asset}")))
            }
            "release-assets.githubusercontent.com" => !path.is_empty(),
            _ => false,
        };
    #[cfg(feature = "acceptance")]
    let acceptance = option_env!("ZEROOS_ACCEPTANCE_ORIGIN").is_some_and(|origin| {
        origin
            .strip_prefix("https://")
            .is_some_and(|allowed| authority == allowed.trim_end_matches('/') && path == asset)
    });
    #[cfg(not(feature = "acceptance"))]
    let acceptance = false;
    production || acceptance
}

fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 256
        && value.starts_with('"')
        && value.ends_with('"')
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn expected_content_range(start: u64, total: u64) -> Result<String, String> {
    let end = total.checked_sub(1).ok_or("bad-content-range")?;
    if start > end {
        return Err("bad-content-range".into());
    }
    Ok(format!("bytes {start}-{end}/{total}"))
}

fn response_header(
    response: &ureq::http::Response<ureq::Body>,
    name: &str,
) -> Result<String, String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 1024)
        .map(str::to_owned)
        .ok_or_else(|| "missing-response-header".into())
}

fn content_length(response: &ureq::http::Response<ureq::Body>) -> Result<u64, String> {
    response_header(response, "content-length")?
        .parse()
        .map_err(|_| "bad-content-length".into())
}

fn load_metadata(path: &Path) -> Result<DownloadMetadata, String> {
    if fs::metadata(path).map_err(redact)?.len() > METADATA_LIMIT {
        return Err("bad-resume-metadata".into());
    }
    parse_metadata(&fs::read_to_string(path).map_err(redact)?)
}

fn parse_metadata(input: &str) -> Result<DownloadMetadata, String> {
    let mut lines = input.lines();
    let url = lines.next().and_then(|line| line.strip_prefix("url="));
    let etag = lines.next().and_then(|line| line.strip_prefix("etag="));
    let length = lines
        .next()
        .and_then(|line| line.strip_prefix("length="))
        .and_then(|value| value.parse().ok());
    let written = lines
        .next()
        .and_then(|line| line.strip_prefix("written="))
        .and_then(|value| value.parse().ok());
    if lines.next().is_some() {
        return Err("bad-resume-metadata".into());
    }
    Ok(DownloadMetadata {
        url: url.ok_or("bad-resume-metadata")?.into(),
        etag: etag.ok_or("bad-resume-metadata")?.into(),
        length: length.ok_or("bad-resume-metadata")?,
        written: written.ok_or("bad-resume-metadata")?,
    })
}

fn store_metadata(path: &Path, metadata: &DownloadMetadata) -> Result<(), String> {
    if metadata.url.contains(['\n', '\r']) || metadata.etag.contains(['\n', '\r']) {
        return Err("bad-resume-metadata".into());
    }
    let temporary = path.with_extension("resume.new");
    let mut file = fs::File::create(&temporary).map_err(redact)?;
    write!(
        file,
        "url={}\netag={}\nlength={}\nwritten={}\n",
        metadata.url, metadata.etag, metadata.length, metadata.written
    )
    .map_err(redact)?;
    file.sync_all().map_err(redact)?;
    fs::rename(&temporary, path).map_err(redact)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(redact)?;
    }
    Ok(())
}

fn truncate_and_sync(path: &Path, length: u64) -> Result<(), String> {
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(redact)?;
    file.set_len(length).map_err(redact)?;
    file.sync_all().map_err(redact)
}

fn verify_container(
    path: &Path,
    arch: &str,
    current_sequence: u64,
    exact_size: bool,
) -> Result<Manifest, String> {
    verify_container_with_keys(
        path,
        arch,
        current_sequence,
        exact_size,
        Path::new("/etc/zeroos/release-keys"),
    )
}

fn verify_container_with_keys(
    path: &Path,
    arch: &str,
    current_sequence: u64,
    exact_size: bool,
    key_directory: &Path,
) -> Result<Manifest, String> {
    let mut file = fs::File::open(path).map_err(redact)?;
    let mut header = [0; 12];
    file.read_exact(&mut header).map_err(redact)?;
    if &header[..8] != MAGIC {
        return Err("bad-container-magic".into());
    }
    let manifest_size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    if manifest_size == 0 || manifest_size > MAX_MANIFEST {
        return Err("bad-manifest-size".into());
    }
    let mut manifest_bytes = vec![0; manifest_size];
    file.read_exact(&mut manifest_bytes).map_err(redact)?;
    let manifest = Manifest::parse(&manifest_bytes, arch).map_err(str::to_owned)?;
    if manifest.sequence <= current_sequence {
        return Err("downgrade".into());
    }
    let mut signature_bytes = [0; SIGNATURE_BYTES];
    file.read_exact(&mut signature_bytes).map_err(redact)?;
    let expected = 12u64
        .checked_add(manifest_size as u64)
        .and_then(|value| value.checked_add(SIGNATURE_BYTES as u64))
        .and_then(|value| value.checked_add(manifest.payload_size))
        .ok_or("bad-container-size")?;
    let actual_size = file.metadata().map_err(redact)?.len();
    if (exact_size && actual_size != expected) || (!exact_size && actual_size < expected) {
        return Err("bad-container-size".into());
    }

    let public_key = release_key(&manifest.signer, key_directory)?;
    signature::UnparsedPublicKey::new(&signature::RSA_PSS_2048_8192_SHA256, public_key)
        .verify(&manifest_bytes, &signature_bytes)
        .map_err(|_| "bad-signature")?;

    let payload_offset = 12 + manifest_size as u64 + SIGNATURE_BYTES as u64;
    file.seek(SeekFrom::Start(payload_offset)).map_err(redact)?;
    let mut context = digest::Context::new(&digest::SHA256);
    let mut remaining = manifest.payload_size;
    let mut buffer = [0; 64 * 1024];
    while remaining != 0 {
        let wanted = buffer
            .len()
            .min(usize::try_from(remaining).map_err(|_| "bad-payload-size")?);
        let count = file.read(&mut buffer[..wanted]).map_err(redact)?;
        if count == 0 {
            return Err("short-payload".into());
        }
        context.update(&buffer[..count]);
        remaining -= count as u64;
    }
    if context.finish().as_ref() != manifest.sha256 {
        return Err("corrupt-payload".into());
    }
    Ok(manifest)
}

fn release_key(signer: &str, directory: &Path) -> Result<Vec<u8>, String> {
    let mut count = 0usize;
    for entry in fs::read_dir(directory).map_err(redact)? {
        let entry = entry.map_err(redact)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or("bad-release-key-name")?;
        if !name.ends_with(".der")
            || !name[..name.len() - 4]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !entry.file_type().map_err(redact)?.is_file()
        {
            return Err("bad-release-key-entry".into());
        }
        count += 1;
    }
    if !(1..=2).contains(&count) {
        return Err("bad-release-key-count".into());
    }
    let path = directory.join(format!("{signer}.der"));
    let metadata = fs::symlink_metadata(&path).map_err(redact)?;
    if !metadata.file_type().is_file() || metadata.len() > 2048 {
        return Err("bad-release-key".into());
    }
    fs::read(path).map_err(redact)
}

fn validate_inactive_target(path: &Path, arch: &str) -> Result<(), String> {
    let expected = env::var("ZEROOS_INACTIVE_SLOT_NAME").map_err(|_| "missing-slot-identity")?;
    if !matches!(expected.as_str(), "ZEROOS-A" | "ZEROOS-B") {
        return Err("bad-slot-target".into());
    }
    if arch != "x86_64" && arch != "aarch64" {
        return Err("bad-slot-target".into());
    }
    let size = fs::metadata(path).map_err(redact)?.len();
    if size != SLOT_BYTES {
        return Err("bad-slot-capacity".into());
    }
    zeroos_storage::validate_partition_device(path, &expected).map_err(redact)
}

fn write_slot(container: &Path, slot: &Path) -> Result<(), String> {
    let input = fs::File::open(container).map_err(redact)?;
    let expected = input.metadata().map_err(redact)?.len();
    if expected == 0 || expected > MAX_CONTAINER {
        return Err("bad-container-size".into());
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .open(slot)
        .map_err(redact)?;
    output.seek(SeekFrom::Start(0)).map_err(redact)?;
    let copied = std::io::copy(&mut input.take(expected), &mut output).map_err(redact)?;
    if copied != expected {
        return Err("short-slot-write".into());
    }
    output.flush().map_err(redact)?;
    output.sync_all().map_err(redact)
}

fn redact(error: impl std::fmt::Display) -> String {
    let _ = error;
    "io-failure".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn redirect_resume_metadata_and_redaction_are_strict() -> Result<(), String> {
        assert!(allowed_url(
            "https://github.com/KZagaja/zeroOS-releases/releases/latest/download/zeroos-x86_64.slot",
            "x86_64"
        ));
        assert!(allowed_url(
            "https://github.com/KZagaja/zeroOS-releases/releases/download/v1/zeroos-x86_64.slot",
            "x86_64"
        ));
        assert!(allowed_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1?x=2",
            "x86_64"
        ));
        for bad in [
            "http://github.com/KZagaja/zeroOS-releases/releases/latest/download/zeroos-x86_64.slot",
            "https://github.com@evil.test/KZagaja/zeroOS-releases/releases/latest/download/zeroos-x86_64.slot",
            "https://github.com/KZagaja/zeroOS-releases/releases/latest/download/zeroos-x86_64.slot#fragment",
            "https://objects.githubusercontent.com/asset",
        ] {
            assert!(!allowed_url(bad, "x86_64"));
        }
        assert!(strong_etag("\"strong\""));
        assert!(!strong_etag("W/\"weak\""));
        assert_eq!(expected_content_range(10, 20)?, "bytes 10-19/20");
        let metadata = parse_metadata("url=https://x\netag=\"a\"\nlength=20\nwritten=10\n")?;
        assert_eq!(metadata.written, 10);
        assert!(parse_metadata("url=x\netag=y\nlength=2\nwritten=1\nextra=x\n").is_err());
        assert_eq!(redact("passphrase=hunter2"), "io-failure");

        let root = std::env::temp_dir().join(format!("zeroos-resume-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).map_err(redact)?;
        let output = root.join("asset.part");
        let metadata_path = root.join("asset.resume");
        let mut durable = DownloadMetadata {
            url: "https://x".into(),
            etag: "\"strong\"".into(),
            length: 6,
            written: 0,
        };
        assert_eq!(
            persist_body(
                &mut std::io::Cursor::new(b"abc"),
                &output,
                &metadata_path,
                &mut durable
            ),
            Err("short-download".into())
        );
        assert_eq!(load_metadata(&metadata_path)?.written, 3);
        persist_body(
            &mut std::io::Cursor::new(b"def"),
            &output,
            &metadata_path,
            &mut durable,
        )?;
        assert_eq!(fs::read(&output).map_err(redact)?, b"abcdef");
        fs::remove_dir_all(root).map_err(redact)?;
        Ok(())
    }

    #[test]
    fn ring_verifies_pss_and_rejects_corruption_downgrade_and_unknown_signer()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("zeroos-ring-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let keys = root.join("keys");
        fs::create_dir_all(&keys)?;
        let private = root.join("private.pem");
        assert!(
            Command::new("openssl")
                .args([
                    "genpkey",
                    "-quiet",
                    "-algorithm",
                    "RSA",
                    "-pkeyopt",
                    "rsa_keygen_bits:3072",
                    "-out"
                ])
                .arg(&private)
                .status()?
                .success()
        );
        assert!(
            Command::new("openssl")
                .args(["rsa", "-in"])
                .arg(&private)
                .args(["-RSAPublicKey_out", "-outform", "DER", "-out"])
                .arg(keys.join("release-1.der"))
                .status()?
                .success()
        );
        let payload = b"signed EFI payload";
        let hash = digest::digest(&digest::SHA256, payload);
        let hash = hash
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let manifest = format!(
            "version=1\narch=x86_64\nsequence=7\npayload-size={}\nsha256={hash}\nsigner=release-1\n",
            payload.len()
        );
        let manifest_path = root.join("manifest");
        let signature_path = root.join("signature");
        fs::write(&manifest_path, &manifest)?;
        assert!(
            Command::new("openssl")
                .args(["dgst", "-sha256", "-sign"])
                .arg(&private)
                .args([
                    "-sigopt",
                    "rsa_padding_mode:pss",
                    "-sigopt",
                    "rsa_pss_saltlen:-1",
                    "-out"
                ])
                .arg(&signature_path)
                .arg(&manifest_path)
                .status()?
                .success()
        );
        let mut container = Vec::from(MAGIC.as_slice());
        container.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        container.extend_from_slice(manifest.as_bytes());
        container.extend_from_slice(&fs::read(signature_path)?);
        container.extend_from_slice(payload);
        let path = root.join("release.slot");
        fs::write(&path, &container)?;
        assert_eq!(
            verify_container_with_keys(&path, "x86_64", 6, true, &keys)?.sequence,
            7
        );
        assert!(verify_container_with_keys(&path, "x86_64", 7, true, &keys).is_err());
        let last = container.last_mut().ok_or("empty container")?;
        *last ^= 1;
        fs::write(&path, &container)?;
        assert!(verify_container_with_keys(&path, "x86_64", 6, true, &keys).is_err());
        fs::rename(keys.join("release-1.der"), keys.join("release-2.der"))?;
        assert!(verify_container_with_keys(&path, "x86_64", 6, true, &keys).is_err());
        let _ = fs::remove_dir_all(root);
        Ok(())
    }
}
