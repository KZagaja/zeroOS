use std::{
    fs::File,
    io::{self, Read, Write},
    ops::Deref,
    os::fd::{AsRawFd, RawFd},
    process::{Command, ExitCode},
};
#[cfg(target_os = "linux")]
use std::{
    io::{Seek, SeekFrom},
    os::fd::FromRawFd,
};
use zeroize::{Zeroize, Zeroizing};
use zeroos_storage::{valid_passphrase, validate_partition_device};

const MAX_SECRET: usize = 1024;
const CRYPTSETUP: &str = "/usr/sbin/cryptsetup";
const E2FSCK: &str = "/usr/sbin/e2fsck";
const MKFS_EXT4: &str = "/usr/sbin/mkfs.ext4";

struct Secret {
    bytes: Zeroizing<Vec<u8>>,
    length: usize,
}

impl Secret {
    fn locked(length: usize) -> Result<Self, String> {
        let mut bytes = Zeroizing::new(vec![0; length]);
        // SAFETY: `bytes` is initialized, uniquely owned storage valid and aligned for `len`
        // bytes and remains at a stable address until paired `munlock` in Drop. The kernel keeps
        // no Rust reference and creates no alias. Construction is single-threaded; failure wipes
        // the allocation and acquires no resource requiring cleanup.
        if unsafe { libc::mlock(bytes.as_ptr().cast(), bytes.len()) } != 0 {
            bytes.zeroize();
            return Err("unable to lock secret memory".into());
        }
        Ok(Self { bytes, length: 0 })
    }
}

impl Deref for Secret {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes[..self.length]
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.bytes.zeroize();
        // SAFETY: `bytes` is the same initialized, uniquely owned allocation successfully locked
        // by `Secret::locked`; it remains valid and aligned through this call. No pointer escaped,
        // no alias or cross-thread access exists, and bytes are wiped before unlock. Failure needs
        // no additional cleanup because the memory contents are already zeroized.
        unsafe {
            libc::munlock(self.bytes.as_ptr().cast(), self.bytes.len());
        }
    }
}

struct TerminalEcho {
    original: libc::termios,
    original_flags: libc::c_int,
    restored: bool,
}

impl TerminalEcho {
    fn disable() -> Result<Self, String> {
        // SAFETY: STDIN_FILENO is a valid scalar descriptor; F_GETFL/F_SETFL retain no pointer
        // and touch no Rust memory. The interactive data tool owns terminal input while running,
        // and the original flags are restored by this guard on success or partial failure.
        let original_flags = unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_GETFL) };
        if original_flags < 0 {
            return Err("unable to acquire terminal input".into());
        }
        // SAFETY: `original_flags` came from F_GETFL for this descriptor; only O_NONBLOCK is
        // cleared, no pointer is passed or retained, and the guard restores the exact value.
        if unsafe {
            libc::fcntl(
                libc::STDIN_FILENO,
                libc::F_SETFL,
                original_flags & !libc::O_NONBLOCK,
            )
        } != 0
        {
            return Err("unable to acquire terminal input".into());
        }
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `original` points to correctly aligned writable storage large enough for one
        // `termios`; `tcgetattr` initializes it on success and retains no pointer. No alias,
        // lifetime, or thread-shared state crosses the call; failure initializes nothing and owns
        // no resource requiring cleanup.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, original.as_mut_ptr()) } != 0 {
            return Err("unable to read terminal state".into());
        }
        // SAFETY: the successful `tcgetattr` immediately above initialized every byte required for
        // a valid `termios`. The value is copied into private Rust storage; no pointer or alias is
        // retained, and there is no partial-failure resource at this step.
        let original = unsafe { original.assume_init() };
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        // SAFETY: `hidden` is a valid initialized `termios` derived from `tcgetattr` and borrowed
        // immutably for this call. The pointer is aligned and live, not retained or aliased
        // mutably, and terminal mutation is serialized by this single-threaded interactive tool.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &hidden) } != 0 {
            return Err("unable to hide terminal input".into());
        }
        Ok(Self {
            original,
            original_flags,
            restored: false,
        })
    }

    fn restore(mut self) -> Result<(), String> {
        self.restore_inner()?;
        self.restored = true;
        Ok(())
    }

    fn restore_inner(&self) -> Result<(), String> {
        // SAFETY: `original` is the initialized `termios` obtained for this same terminal and is
        // valid, aligned, and immutably borrowed for the synchronous call. The kernel retains no
        // pointer; no mutable alias or cross-thread access exists, and a failure leaves Drop able
        // to retry restoration without other cleanup obligations.
        let terminal_restored = {
            // SAFETY: `original` is initialized, aligned, live, and uniquely owned by this guard;
            // the synchronous call retains no pointer and failure leaves Drop able to retry.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &self.original) }
        } == 0;
        // SAFETY: `original_flags` was read from this descriptor before mutation. F_SETFL reads
        // only the scalar, retains nothing, and restores descriptor state after terminal use.
        let flags_restored = {
            // SAFETY: this scalar came from F_GETFL for the same live descriptor; F_SETFL retains
            // no pointer, transfers no ownership, and restores the pre-prompt state.
            unsafe { libc::fcntl(libc::STDIN_FILENO, libc::F_SETFL, self.original_flags) }
        } == 0;
        if terminal_restored && flags_restored {
            Ok(())
        } else {
            Err("unable to restore terminal input".into())
        }
    }
}

impl Drop for TerminalEcho {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore_inner();
        }
    }
}

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("zeroOS data: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, device] if command == "provision" => provision_checked(device),
        [command, device] if command == "unlock" => unlock_checked(device),
        [command, mapper] if command == "repair" && mapper == "/dev/mapper/zeroos-data" => {
            repair_ext4(mapper)
        }
        [command, device, confirmation]
            if command == "reset" && confirmation == "ERASE-USER-DATA" =>
        {
            validate_partition_device(std::path::Path::new(device), "ZEROOS-DATA")
                .map_err(redact)?;
            run(Command::new(CRYPTSETUP).args(["close", "zeroos-data"]), &[])?;
            provision(device)
        }
        _ => Err("usage: zeroos-data <provision|unlock|repair|reset> <device>".into()),
    }
}

fn repair_ext4(mapper: &str) -> Result<(), String> {
    let code = Command::new(E2FSCK)
        .args(["-p", "-f", mapper])
        .status()
        .map_err(|_| "unable to start data engine".to_owned())?
        .code();
    if e2fsck_succeeded(code) {
        Ok(())
    } else {
        Err("engine failed".into())
    }
}

fn e2fsck_succeeded(code: Option<i32>) -> bool {
    matches!(code, Some(0 | 1))
}

fn provision_checked(device: &str) -> Result<(), String> {
    validate_partition_device(std::path::Path::new(device), "ZEROOS-DATA").map_err(redact)?;
    provision(device)
}

fn unlock_checked(device: &str) -> Result<(), String> {
    validate_partition_device(std::path::Path::new(device), "ZEROOS-DATA").map_err(redact)?;
    unlock(device)
}

fn provision(device: &str) -> Result<(), String> {
    let passphrase = prompt("New passphrase: ")?;
    let repeated = prompt("Repeat passphrase: ")?;
    if !valid_passphrase(&passphrase) || *passphrase != *repeated {
        return Err("passphrases do not match or are outside 12..=1024 bytes".into());
    }
    let recovery = recovery_code()?;
    println!(
        "Recovery code (store offline): {}",
        String::from_utf8_lossy(&recovery)
    );
    let recovery_repeat = prompt("Repeat recovery code: ")?;
    if *recovery != *recovery_repeat {
        return Err("recovery code mismatch".into());
    }
    let pass_file = secret_file(&passphrase)?;
    let recovery_file = secret_file(&recovery)?;
    let pass_path = format!("/proc/self/fd/{}", pass_file.as_raw_fd());
    let recovery_path = format!("/proc/self/fd/{}", recovery_file.as_raw_fd());
    run(
        Command::new(CRYPTSETUP).args([
            "luksFormat",
            "--batch-mode",
            "--type",
            "luks2",
            "--cipher",
            "aes-xts-plain64",
            "--key-size",
            "512",
            "--pbkdf",
            "argon2id",
            "--key-file",
            &pass_path,
            device,
        ]),
        &[&pass_file],
    )?;
    run(
        Command::new(CRYPTSETUP).args([
            "luksAddKey",
            "--key-file",
            &pass_path,
            device,
            &recovery_path,
        ]),
        &[&pass_file, &recovery_file],
    )?;
    run(
        Command::new(CRYPTSETUP).args(["open", "--key-file", &pass_path, device, "zeroos-data"]),
        &[&pass_file],
    )?;
    run(
        Command::new(MKFS_EXT4).args(["-F", "-L", "ZEROOS-DATA", "/dev/mapper/zeroos-data"]),
        &[],
    )
}

fn unlock(device: &str) -> Result<(), String> {
    let secret = prompt("Passphrase or recovery code: ")?;
    if !valid_passphrase(&secret) {
        return Err("invalid credential length".into());
    }
    let file = secret_file(&secret)?;
    let path = format!("/proc/self/fd/{}", file.as_raw_fd());
    run(
        Command::new(CRYPTSETUP).args(["open", "--key-file", &path, device, "zeroos-data"]),
        &[&file],
    )
}

fn prompt(label: &str) -> Result<Secret, String> {
    let terminal = TerminalEcho::disable()?;
    let prompt = match label {
        "New passphrase: " => "new-passphrase",
        "Repeat passphrase: " => "repeat-passphrase",
        "Repeat recovery code: " => "repeat-recovery-code",
        _ => "credential",
    };
    println!("zeroOS data: credential-request={prompt}");
    print!("{label}");
    io::stdout()
        .flush()
        .map_err(|_| "unable to flush secret prompt".to_owned())?;
    let mut secret = Secret::locked(MAX_SECRET + 1)?;
    let mut byte = [0];
    loop {
        match io::stdin()
            .read(&mut byte)
            .map_err(|_| "unable to read secret".to_owned())?
        {
            0 => break,
            1 if matches!(byte[0], b'\n' | b'\r') => break,
            1 if secret.length < MAX_SECRET => {
                secret.bytes[secret.length] = byte[0];
                secret.length += 1;
            }
            1 => return Err("credential exceeds 1024 bytes".into()),
            _ => return Err("unable to read credential".into()),
        }
    }
    terminal.restore()?;
    println!();
    Ok(secret)
}

fn recovery_code() -> Result<Secret, String> {
    let mut random = Zeroizing::new([0; 32]);
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(random.as_mut()))
        .map_err(|_| "unable to generate recovery code".to_owned())?;
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = Secret::locked(44)?;
    for (index, chunk) in random.chunks(3).enumerate() {
        let value = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for (part, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            let offset = index * 4 + part;
            if offset < out.bytes.len() {
                out.bytes[offset] = TABLE[((value >> shift) & 63) as usize];
            }
        }
    }
    out.length = 44;
    Ok(out)
}

fn secret_file(secret: &[u8]) -> Result<File, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = secret;
        Err("secret descriptors require Linux".into())
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: the name is a static initialized NUL-terminated C string with valid provenance,
        // alignment, and process lifetime; the kernel does not retain it. No aliasing or shared Rust
        // state crosses the call. `MFD_CLOEXEC` makes the fresh descriptor non-inheritable by default;
        // a negative return owns nothing requiring cleanup.
        let fd = unsafe { libc::memfd_create(c"zeroos-secret".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err("unable to allocate secret file".into());
        }
        // SAFETY: successful `memfd_create` returned a fresh initialized descriptor uniquely owned by
        // this function. `from_raw_fd` transfers ownership exactly once; pointer provenance, alignment,
        // and aliasing do not apply to the scalar descriptor. Every later failure drops `File` and
        // closes it deterministically.
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(secret)
            .map_err(|_| "unable to store secret".to_owned())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "unable to rewind secret".to_owned())?;
        Ok(file)
    }
}

fn run(command: &mut Command, secrets: &[&File]) -> Result<(), String> {
    let mut flags = Vec::with_capacity(secrets.len());
    for secret in secrets {
        let fd = secret.as_raw_fd();
        let original = fd_flags(fd)?;
        if let Err(error) = set_fd_flags(fd, original & !libc::FD_CLOEXEC) {
            let _ = flags
                .into_iter()
                .try_for_each(|(changed, saved)| set_fd_flags(changed, saved));
            return Err(error);
        }
        flags.push((fd, original));
    }
    let status = command.status();
    let restored = flags
        .into_iter()
        .try_for_each(|(fd, original)| set_fd_flags(fd, original));
    restored?;
    if status
        .map_err(|_| "unable to start data engine".to_owned())?
        .success()
    {
        Ok(())
    } else {
        Err("engine failed".into())
    }
}

fn fd_flags(fd: RawFd) -> Result<i32, String> {
    // SAFETY: `fd` is borrowed from a live owned `File`; `F_GETFD` reads descriptor flags only,
    // retains no pointer, creates no alias, and transfers no ownership. Failure leaves the file
    // owned and closed by RAII with no partial cleanup.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        Err("unable to inspect secret descriptor".into())
    } else {
        Ok(flags)
    }
}

fn set_fd_flags(fd: RawFd, flags: i32) -> Result<(), String> {
    // SAFETY: `fd` is borrowed from a live owned `File` and `flags` came from `F_GETFD` with only
    // `FD_CLOEXEC` changed. The call retains no pointer, does not change ownership or alias Rust
    // memory, is serialized in this single-threaded tool, and failure leaves RAII cleanup intact.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags) } == 0 {
        Ok(())
    } else {
        Err("unable to configure secret descriptor".into())
    }
}

fn redact(error: impl std::fmt::Display) -> String {
    let _ = error;
    "I/O failed".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_is_printable_fixed_size_and_locked() -> Result<(), String> {
        let code = recovery_code()?;
        assert_eq!(code.len(), 44);
        assert!(code.iter().all(u8::is_ascii_graphic));
        Ok(())
    }

    #[test]
    fn echo_bit_is_reversible() {
        let original = libc::ECHO | libc::ICANON;
        let hidden = original & !libc::ECHO;
        assert_eq!(hidden & libc::ECHO, 0);
        assert_eq!(hidden & libc::ICANON, libc::ICANON);
        assert_eq!(original & libc::ECHO, libc::ECHO);
    }

    #[test]
    fn ext4_repair_accepts_clean_or_corrected_only() {
        assert!(e2fsck_succeeded(Some(0)));
        assert!(e2fsck_succeeded(Some(1)));
        assert!(!e2fsck_succeeded(Some(2)));
        assert!(!e2fsck_succeeded(None));
    }
}
