use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub const IMAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const SECTOR: u64 = 512;
pub const SLOT_BYTES: u64 = 96 * 1024 * 1024;
pub const JOURNAL_BYTES: u64 = 1024 * 1024;
pub const RECORD_BYTES: usize = 4096;
pub const SIGNATURE_BYTES: usize = 384;
pub const MAX_MANIFEST: usize = 4096;
pub const MAX_PAYLOAD: u64 = SLOT_BYTES - MAX_MANIFEST as u64 - SIGNATURE_BYTES as u64 - 16;
pub const RELEASE_URL: &str =
    "https://github.com/KZagaja/zeroOS-releases/releases/latest/download/";
const CONTAINER_MAGIC: &[u8; 8] = b"ZEROSLT1";

pub fn container_manifest_size(header: &[u8]) -> Result<usize, &'static str> {
    if header.len() != 12 || &header[..8] != CONTAINER_MAGIC {
        return Err("BAD_CONTAINER_HEADER");
    }
    let size = u32::from_le_bytes(
        header[8..12]
            .try_into()
            .map_err(|_| "BAD_CONTAINER_HEADER")?,
    ) as usize;
    if size == 0 || size > MAX_MANIFEST {
        return Err("BAD_MANIFEST_SIZE");
    }
    Ok(size)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Partition {
    pub number: u8,
    pub name: &'static str,
    pub first: u64,
    pub last: u64,
    pub type_code: &'static str,
    pub unique_guid: &'static str,
}

pub const PARTITIONS: [Partition; 6] = [
    Partition {
        number: 1,
        name: "ZEROOS-ESP",
        first: 2048,
        last: 34815,
        type_code: "EF00",
        unique_guid: "5A45524F-4F53-4D31-8000-000000000002",
    },
    Partition {
        number: 2,
        name: "ZEROOS-A",
        first: 34816,
        last: 231423,
        type_code: "8300",
        unique_guid: "5A45524F-4F53-4D33-8000-000000000002",
    },
    Partition {
        number: 3,
        name: "ZEROOS-B",
        first: 231424,
        last: 428031,
        type_code: "8300",
        unique_guid: "5A45524F-4F53-4D33-8000-000000000003",
    },
    Partition {
        number: 4,
        name: "ZEROOS-RECOVERY",
        first: 428032,
        last: 624639,
        type_code: "8300",
        unique_guid: "5A45524F-4F53-4D33-8000-000000000004",
    },
    Partition {
        number: 5,
        name: "ZEROOS-STATE",
        first: 624640,
        last: 626687,
        type_code: "8300",
        unique_guid: "5A45524F-4F53-4D33-8000-000000000005",
    },
    Partition {
        number: 6,
        name: "ZEROOS-DATA",
        first: 626688,
        last: 1048542,
        type_code: "8309",
        unique_guid: "5A45524F-4F53-4D33-8000-000000000006",
    },
];

pub fn partition(name: &str) -> Option<&'static Partition> {
    PARTITIONS.iter().find(|partition| partition.name == name)
}

pub fn validate_partition_device(path: &Path, expected_name: &str) -> io::Result<()> {
    validate_partition_at(
        path,
        expected_name,
        Path::new("/sys/class/block"),
        Path::new("/dev"),
    )
}

fn validate_partition_at(
    path: &Path,
    expected_name: &str,
    sysfs: &Path,
    devices: &Path,
) -> io::Result<()> {
    let expected = partition(expected_name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unknown partition"))?;
    if path.parent() != Some(devices) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid device path",
        ));
    }
    let device = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid device name"))?;
    let root = sysfs.join(device);
    let uevent = std::fs::read_to_string(root.join("uevent"))?;
    let field = |key: &str| uevent.lines().find_map(|line| line.strip_prefix(key));
    let start: u64 = std::fs::read_to_string(root.join("start"))?
        .trim()
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid partition start"))?;
    let sectors: u64 = std::fs::read_to_string(root.join("size"))?
        .trim()
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid partition size"))?;
    let expected_sectors = expected
        .last
        .checked_sub(expected.first)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid partition bounds"))?;
    if field("PARTNAME=") != Some(expected.name)
        || field("PARTUUID=").map(str::to_ascii_uppercase).as_deref() != Some(expected.unique_guid)
        || field("PARTN=").and_then(|value| value.parse().ok()) != Some(expected.number)
        || start != expected.first
        || sectors != expected_sectors
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "partition identity mismatch",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Slot {
    A = 1,
    B = 2,
    Recovery = 3,
}

impl Slot {
    pub fn other(self) -> Self {
        if self == Self::A { Self::B } else { Self::A }
    }
    fn decode(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::A),
            2 => Some(Self::B),
            3 => Some(Self::Recovery),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::Recovery => "recovery",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootState {
    pub generation: u64,
    pub sequence: u64,
    pub confirmed: Slot,
    pub pending: Option<Slot>,
    pub booting: Option<Slot>,
    pub failed: u8,
    pub recovery: bool,
}

impl Default for BootState {
    fn default() -> Self {
        Self {
            generation: 0,
            sequence: 0,
            confirmed: Slot::A,
            pending: None,
            booting: None,
            failed: 0,
            recovery: false,
        }
    }
}

impl BootState {
    fn bump_generation(&mut self) -> Result<(), &'static str> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("GENERATION_EXHAUSTED")?;
        Ok(())
    }

    pub fn select(&mut self) -> Result<Slot, &'static str> {
        self.bump_generation()?;
        if let Some(previous) = self.booting.take()
            && previous != self.confirmed
        {
            self.failed |= 1 << (previous as u8 - 1);
        }
        let chosen = if self.recovery || self.failed & 0b11 == 0b11 {
            Slot::Recovery
        } else if let Some(pending) = self.pending.take() {
            pending
        } else if self.failed & (1 << (self.confirmed as u8 - 1)) == 0 {
            self.confirmed
        } else {
            self.confirmed.other()
        };
        self.recovery = false;
        self.booting = Some(chosen);
        Ok(chosen)
    }

    pub fn stage(&mut self, slot: Slot, sequence: u64) -> Result<(), &'static str> {
        if !matches!(slot, Slot::A | Slot::B) || slot == self.confirmed {
            return Err("ACTIVE_SLOT");
        }
        if sequence <= self.sequence {
            return Err("DOWNGRADE");
        }
        self.bump_generation()?;
        self.pending = Some(slot);
        self.sequence = sequence;
        self.failed &= !(1 << (slot as u8 - 1));
        Ok(())
    }

    pub fn confirm(&mut self) -> Result<(), &'static str> {
        let slot = self.booting.ok_or("NO_TRIAL")?;
        if slot == Slot::Recovery {
            return Err("RECOVERY_MODE");
        }
        self.bump_generation()?;
        self.booting = None;
        self.confirmed = slot;
        self.failed &= !(1 << (slot as u8 - 1));
        Ok(())
    }

    pub fn request_recovery(&mut self) -> Result<(), &'static str> {
        self.bump_generation()?;
        self.recovery = true;
        Ok(())
    }
    pub fn mark_failed(&mut self, slot: Slot) -> Result<(), &'static str> {
        self.bump_generation()?;
        if slot != Slot::Recovery {
            self.failed |= 1 << (slot as u8 - 1);
        }
        Ok(())
    }
    pub fn repair(&mut self, confirmed: Slot) -> Result<(), &'static str> {
        if confirmed == Slot::Recovery {
            return Err("BAD_SLOT");
        }
        self.bump_generation()?;
        self.confirmed = confirmed;
        self.pending = None;
        self.booting = None;
        self.failed = 0;
        self.recovery = false;
        Ok(())
    }
    pub fn reset_trials(&mut self) -> Result<(), &'static str> {
        self.bump_generation()?;
        self.pending = None;
        self.booting = None;
        self.failed = 0;
        self.recovery = false;
        Ok(())
    }
}

pub fn encode_record(state: &BootState) -> [u8; RECORD_BYTES] {
    let mut out = [0u8; RECORD_BYTES];
    out[..8].copy_from_slice(b"ZEROOSB1");
    out[8..16].copy_from_slice(&state.generation.to_le_bytes());
    out[16..24].copy_from_slice(&state.sequence.to_le_bytes());
    out[24] = state.confirmed as u8;
    out[25] = state.pending.map_or(0, |v| v as u8);
    out[26] = state.booting.map_or(0, |v| v as u8);
    out[27] = state.failed;
    out[28] = state.recovery as u8;
    let crc = crc32(&out[..RECORD_BYTES - 4]);
    out[RECORD_BYTES - 4..].copy_from_slice(&crc.to_le_bytes());
    out
}

pub fn decode_record(input: &[u8]) -> Option<BootState> {
    if input.len() != RECORD_BYTES || &input[..8] != b"ZEROOSB1" {
        return None;
    }
    let stored = u32::from_le_bytes(input[RECORD_BYTES - 4..].try_into().ok()?);
    if crc32(&input[..RECORD_BYTES - 4]) != stored || input[27] & !0b111 != 0 || input[28] > 1 {
        return None;
    }
    Some(BootState {
        generation: u64::from_le_bytes(input[8..16].try_into().ok()?),
        sequence: u64::from_le_bytes(input[16..24].try_into().ok()?),
        confirmed: Slot::decode(input[24])?,
        pending: if input[25] == 0 {
            None
        } else {
            Some(Slot::decode(input[25])?)
        },
        booting: if input[26] == 0 {
            None
        } else {
            Some(Slot::decode(input[26])?)
        },
        failed: input[27],
        recovery: input[28] != 0,
    })
}

pub fn newest_record(first: &[u8], second: &[u8]) -> Option<BootState> {
    [decode_record(first), decode_record(second)]
        .into_iter()
        .flatten()
        .max_by_key(|state| state.generation)
}

pub fn write_journal(path: &Path, state: &BootState) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let offset = (state.generation & 1) * RECORD_BYTES as u64;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&encode_record(state))?;
    file.sync_all()
}

pub fn reconstruct_journal(path: &Path, state: &BootState) -> io::Result<BootState> {
    let mut first = *state;
    first.generation = first.generation.checked_add(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "boot-state generation exhausted",
        )
    })?;
    write_journal(path, &first)?;
    let mut second = first;
    second.generation = second.generation.checked_add(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "boot-state generation exhausted",
        )
    })?;
    write_journal(path, &second)?;
    Ok(second)
}

pub fn read_journal(path: &Path) -> io::Result<BootState> {
    let mut records = [0; RECORD_BYTES * 2];
    File::open(path)?.read_exact(&mut records)?;
    newest_record(&records[..RECORD_BYTES], &records[RECORD_BYTES..])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no valid boot-state record"))
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    pub version: u8,
    pub arch: String,
    pub sequence: u64,
    pub payload_size: u64,
    pub sha256: [u8; 32],
    pub signer: String,
}

impl Manifest {
    pub fn parse(input: &[u8], expected_arch: &str) -> Result<Self, &'static str> {
        if input.is_empty() || input.len() > MAX_MANIFEST {
            return Err("BAD_MANIFEST_SIZE");
        }
        let text = std::str::from_utf8(input).map_err(|_| "BAD_MANIFEST_UTF8")?;
        let mut fields = std::collections::BTreeMap::new();
        for line in text.lines() {
            let (key, value) = line.split_once('=').ok_or("BAD_MANIFEST_FIELD")?;
            if !matches!(
                key,
                "version" | "arch" | "sequence" | "payload-size" | "sha256" | "signer"
            ) || value.is_empty()
                || fields.insert(key, value).is_some()
            {
                return Err("BAD_MANIFEST_FIELD");
            }
        }
        if fields.len() != 6 {
            return Err("MISSING_MANIFEST_FIELD");
        }
        let version = fields["version"].parse().map_err(|_| "BAD_VERSION")?;
        if version != 1 {
            return Err("BAD_VERSION");
        }
        if fields["arch"] != expected_arch {
            return Err("WRONG_ARCH");
        }
        let sequence = fields["sequence"].parse().map_err(|_| "BAD_SEQUENCE")?;
        let payload_size = fields["payload-size"]
            .parse()
            .map_err(|_| "BAD_PAYLOAD_SIZE")?;
        if payload_size == 0 || payload_size > MAX_PAYLOAD {
            return Err("BAD_PAYLOAD_SIZE");
        }
        let sha256 = hex32(fields["sha256"])?;
        let signer = fields["signer"];
        if signer.len() > 128
            || !signer
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err("BAD_SIGNER");
        }
        Ok(Self {
            version,
            arch: expected_arch.into(),
            sequence,
            payload_size,
            sha256,
            signer: signer.into(),
        })
    }
}

fn hex32(value: &str) -> Result<[u8; 32], &'static str> {
    if value.len() != 64 {
        return Err("BAD_SHA256");
    }
    let mut out = [0; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        out[index] = (hex(chunk[0])? << 4) | hex(chunk[1])?;
    }
    Ok(out)
}
fn hex(byte: u8) -> Result<u8, &'static str> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("BAD_SHA256"),
    }
}

pub fn valid_passphrase(secret: &[u8]) -> bool {
    (12..=1024).contains(&secret.len())
}
pub fn factory_reset_allowed(
    mode: Slot,
    confirmation: &str,
    passphrase: &[u8],
    repeated: &[u8],
    code: &str,
    repeated_code: &str,
) -> bool {
    mode == Slot::Recovery
        && confirmation == "ERASE-USER-DATA"
        && valid_passphrase(passphrase)
        && passphrase == repeated
        && code.len() == 44
        && code == repeated_code
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_state_journal_and_torn_writes() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            PARTITIONS[0].last - PARTITIONS[0].first + 1,
            16 * 1024 * 1024 / SECTOR
        );
        for slot in &PARTITIONS[1..4] {
            assert_eq!(slot.last - slot.first + 1, SLOT_BYTES / SECTOR);
        }
        assert_eq!(
            PARTITIONS[4].last - PARTITIONS[4].first + 1,
            JOURNAL_BYTES / SECTOR
        );
        assert!(PARTITIONS.windows(2).all(|p| p[0].last + 1 == p[1].first));
        assert!(
            PARTITIONS
                .last()
                .is_some_and(|partition| partition.last < IMAGE_BYTES / SECTOR - 33)
        );
        let mut state = BootState::default();
        assert_eq!(state.select()?, Slot::A);
        let clean = encode_record(&state);
        for boundary in 0..RECORD_BYTES {
            let mut torn = clean;
            torn[boundary..].fill(0);
            if boundary < RECORD_BYTES {
                assert_eq!(newest_record(&clean, &torn), Some(state));
            }
        }
        state.stage(Slot::B, 2)?;
        assert_eq!(state.select()?, Slot::B);
        assert_eq!(state.select()?, Slot::A);
        state.mark_failed(Slot::A)?;
        assert_eq!(state.select()?, Slot::Recovery);
        let mut exhausted = BootState {
            generation: u64::MAX,
            ..BootState::default()
        };
        let unchanged = exhausted;
        assert_eq!(exhausted.select(), Err("GENERATION_EXHAUSTED"));
        assert_eq!(exhausted, unchanged);
        Ok(())
    }
    #[test]
    fn manifest_and_reset_are_strict() -> Result<(), Box<dyn std::error::Error>> {
        let mut header = *b"ZEROSLT1\0\0\0\0";
        header[8..].copy_from_slice(&128u32.to_le_bytes());
        assert_eq!(container_manifest_size(&header), Ok(128));
        header[0] ^= 1;
        assert!(container_manifest_size(&header).is_err());
        let good = b"version=1\narch=x86_64\nsequence=2\npayload-size=10\nsha256=0000000000000000000000000000000000000000000000000000000000000000\nsigner=release-1\n";
        assert_eq!(Manifest::parse(good, "x86_64")?.sequence, 2);
        assert!(Manifest::parse(good, "aarch64").is_err());
        assert!(Manifest::parse(&[good.as_slice(), b"arch=x86_64\n"].concat(), "x86_64").is_err());
        assert!(!factory_reset_allowed(
            Slot::A,
            "ERASE-USER-DATA",
            b"twelve-bytes!",
            b"twelve-bytes!",
            &"x".repeat(44),
            &"x".repeat(44)
        ));
        assert!(factory_reset_allowed(
            Slot::Recovery,
            "ERASE-USER-DATA",
            b"twelve-bytes!",
            b"twelve-bytes!",
            &"x".repeat(44),
            &"x".repeat(44)
        ));
        Ok(())
    }

    #[test]
    fn destructive_partition_identity_is_exact() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("zeroos-partition-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sysfs = root.join("sys");
        let devices = root.join("dev");
        let device = devices.join("vda6");
        std::fs::create_dir_all(sysfs.join("vda6"))?;
        std::fs::create_dir_all(&devices)?;
        std::fs::write(&device, [])?;
        std::fs::write(
            sysfs.join("vda6/uevent"),
            "PARTN=6\nPARTNAME=ZEROOS-DATA\nPARTUUID=5a45524f-4f53-4d33-8000-000000000006\n",
        )?;
        std::fs::write(sysfs.join("vda6/start"), "626688\n")?;
        std::fs::write(sysfs.join("vda6/size"), "421855\n")?;
        assert!(validate_partition_at(&device, "ZEROOS-DATA", &sysfs, &devices).is_ok());
        std::fs::write(sysfs.join("vda6/size"), "421854\n")?;
        assert!(validate_partition_at(&device, "ZEROOS-DATA", &sysfs, &devices).is_err());
        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }
}
