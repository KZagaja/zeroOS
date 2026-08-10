#![cfg_attr(target_os = "uefi", no_std)]
#![cfg_attr(target_os = "uefi", no_main)]
#![cfg_attr(not(target_os = "uefi"), allow(dead_code, unused_imports))]

use core::{ffi::c_void, panic::PanicInfo, ptr};

type Status = usize;
type Handle = *mut c_void;
const SUCCESS: Status = 0;
const BY_PROTOCOL: u32 = 2;
const LOADER_DATA: u32 = 2;
const RECORD: usize = 4096;
const SLOT_BYTES: usize = 96 * 1024 * 1024;
const SLOT_SECTORS: u64 = 196_608;
const STATE_SECTORS: u64 = 2_048;
const A_GUID: [u8; 16] = [
    0x4f, 0x52, 0x45, 0x5a, 0x53, 0x4f, 0x33, 0x4d, 0x80, 0, 0, 0, 0, 0, 0, 2,
];
const B_GUID: [u8; 16] = [
    0x4f, 0x52, 0x45, 0x5a, 0x53, 0x4f, 0x33, 0x4d, 0x80, 0, 0, 0, 0, 0, 0, 3,
];
const RECOVERY_GUID: [u8; 16] = [
    0x4f, 0x52, 0x45, 0x5a, 0x53, 0x4f, 0x33, 0x4d, 0x80, 0, 0, 0, 0, 0, 0, 4,
];
const STATE_GUID: [u8; 16] = [
    0x4f, 0x52, 0x45, 0x5a, 0x53, 0x4f, 0x33, 0x4d, 0x80, 0, 0, 0, 0, 0, 0, 5,
];

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}
const BLOCK_IO_GUID: Guid = Guid {
    a: 0x964e5b21,
    b: 0x6459,
    c: 0x11d2,
    d: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};
const DEVICE_PATH_GUID: Guid = Guid {
    a: 0x09576e91,
    b: 0x6d3f,
    c: 0x11d2,
    d: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

#[repr(C)]
struct TableHeader {
    signature: u64,
    revision: u32,
    size: u32,
    crc: u32,
    reserved: u32,
}
type AllocatePool = unsafe extern "efiapi" fn(u32, usize, *mut *mut c_void) -> Status;
type FreePool = unsafe extern "efiapi" fn(*mut c_void) -> Status;
type HandleProtocol = unsafe extern "efiapi" fn(Handle, *const Guid, *mut *mut c_void) -> Status;
type LocateHandle =
    unsafe extern "efiapi" fn(u32, *const Guid, *mut c_void, *mut usize, *mut Handle) -> Status;
type LoadImage = unsafe extern "efiapi" fn(
    bool,
    Handle,
    *const c_void,
    *const c_void,
    usize,
    *mut Handle,
) -> Status;
type StartImage = unsafe extern "efiapi" fn(Handle, *mut usize, *mut *mut u16) -> Status;
type UnloadImage = unsafe extern "efiapi" fn(Handle) -> Status;

#[repr(C)]
struct BootServices {
    header: TableHeader,
    raise_tpl: usize,
    restore_tpl: usize,
    allocate_pages: usize,
    free_pages: usize,
    get_memory_map: usize,
    allocate_pool: AllocatePool,
    free_pool: FreePool,
    create_event: usize,
    set_timer: usize,
    wait_for_event: usize,
    signal_event: usize,
    close_event: usize,
    check_event: usize,
    install_protocol_interface: usize,
    reinstall_protocol_interface: usize,
    uninstall_protocol_interface: usize,
    handle_protocol: HandleProtocol,
    reserved: usize,
    register_protocol_notify: usize,
    locate_handle: LocateHandle,
    locate_device_path: usize,
    install_configuration_table: usize,
    load_image: LoadImage,
    start_image: StartImage,
    exit: usize,
    unload_image: UnloadImage,
}
#[repr(C)]
struct SystemTable {
    header: TableHeader,
    firmware_vendor: *mut u16,
    firmware_revision: u32,
    console_in_handle: Handle,
    con_in: *mut c_void,
    console_out_handle: Handle,
    con_out: *mut c_void,
    stderr_handle: Handle,
    stderr: *mut c_void,
    runtime_services: *mut c_void,
    boot_services: *mut BootServices,
}
#[repr(C)]
struct BlockMedia {
    media_id: u32,
    removable: u8,
    present: u8,
    logical: u8,
    read_only: u8,
    write_caching: u8,
    block_size: u32,
    io_align: u32,
    last_block: u64,
}
type ReadBlocks = unsafe extern "efiapi" fn(*mut BlockIo, u32, u64, usize, *mut c_void) -> Status;
type WriteBlocks =
    unsafe extern "efiapi" fn(*mut BlockIo, u32, u64, usize, *const c_void) -> Status;
type FlushBlocks = unsafe extern "efiapi" fn(*mut BlockIo) -> Status;
#[repr(C)]
struct BlockIo {
    revision: u64,
    media: *mut BlockMedia,
    reset: usize,
    read: ReadBlocks,
    write: WriteBlocks,
    flush: FlushBlocks,
}

#[derive(Clone, Copy)]
struct State {
    generation: u64,
    sequence: u64,
    confirmed: u8,
    pending: u8,
    booting: u8,
    failed: u8,
    recovery: u8,
}
impl State {
    fn default() -> Self {
        Self {
            generation: 0,
            sequence: 0,
            confirmed: 1,
            pending: 0,
            booting: 0,
            failed: 0,
            recovery: 0,
        }
    }
    fn bump_generation(&mut self) -> bool {
        let Some(next) = self.generation.checked_add(1) else {
            return false;
        };
        self.generation = next;
        true
    }
    fn select(&mut self) -> Option<u8> {
        if !self.bump_generation() {
            return None;
        }
        if self.booting != 0 && self.booting != self.confirmed {
            self.failed |= 1 << (self.booting - 1);
        }
        self.booting = 0;
        let chosen = if self.recovery != 0 || self.failed & 3 == 3 {
            3
        } else if self.pending != 0 {
            let value = self.pending;
            self.pending = 0;
            value
        } else if self.failed & (1 << (self.confirmed - 1)) == 0 {
            self.confirmed
        } else {
            3 - self.confirmed
        };
        self.recovery = 0;
        self.booting = chosen;
        Some(chosen)
    }
    fn fail(&mut self, slot: u8) -> bool {
        if !self.bump_generation() {
            return false;
        }
        if slot < 3 {
            self.failed |= 1 << (slot - 1);
        }
        self.booting = 0;
        true
    }
}

#[unsafe(no_mangle)]
#[cfg(target_os = "uefi")]
extern "efiapi" fn efi_main(image: Handle, table: *mut SystemTable) -> Status {
    if table.is_null() {
        return 1;
    }
    // SAFETY: UEFI invokes this entry point with an initialized, correctly aligned SystemTable
    // whose BootServices pointer remains valid for the call. Firmware owns both allocations; this
    // code takes one temporary mutable view during single-threaded boot, creates no competing alias,
    // and never retains a reference. `boot` owns and frees its pool allocations on handled exits;
    // firmware status reports partial failure. No Rust unwind crosses the EFI ABI.
    unsafe {
        let services = (*table).boot_services;
        if services.is_null() {
            return 1;
        }
        boot(image, &mut *services)
    }
}

unsafe fn boot(image: Handle, bs: &mut BootServices) -> Status {
    // SAFETY: `efi_main` establishes that `bs` and all invoked BootServices function pointers are
    // initialized, ABI-correct, aligned, and valid until StartImage. Firmware owns protocol/media
    // objects and keeps them alive; this single-threaded routine does not create concurrent or
    // overlapping mutable references. Protocol results and nulls are checked before dereference,
    // buffer sizes come from validated media/container bounds, and pool allocations are freed on
    // each handled partial-failure path. Firmware calls do not unwind across the EFI ABI.
    unsafe {
        let mut size = 0usize;
        (bs.locate_handle)(
            BY_PROTOCOL,
            &BLOCK_IO_GUID,
            ptr::null_mut(),
            &mut size,
            ptr::null_mut(),
        );
        if size == 0 || !size.is_multiple_of(core::mem::size_of::<Handle>()) {
            return 1;
        }
        let mut handles = ptr::null_mut();
        if (bs.allocate_pool)(LOADER_DATA, size, &mut handles) != SUCCESS || handles.is_null() {
            return 1;
        }
        if (bs.locate_handle)(
            BY_PROTOCOL,
            &BLOCK_IO_GUID,
            ptr::null_mut(),
            &mut size,
            handles.cast(),
        ) != SUCCESS
        {
            (bs.free_pool)(handles);
            return 1;
        }
        let list = core::slice::from_raw_parts(
            handles.cast::<Handle>(),
            size / core::mem::size_of::<Handle>(),
        );
        let mut slots: [*mut BlockIo; 3] = [ptr::null_mut(); 3];
        let mut journal = ptr::null_mut();
        for handle in list {
            let mut raw = ptr::null_mut();
            if (bs.handle_protocol)(*handle, &BLOCK_IO_GUID, &mut raw) != SUCCESS {
                continue;
            }
            let io = raw.cast::<BlockIo>();
            if io.is_null() || (*io).media.is_null() || (*(*io).media).logical == 0 {
                continue;
            }
            let mut path_raw = ptr::null_mut();
            if (bs.handle_protocol)(*handle, &DEVICE_PATH_GUID, &mut path_raw) != SUCCESS {
                continue;
            }
            if path_raw.is_null() {
                continue;
            }
            if let Some((number, start, sectors, guid)) = partition_identity(path_raw.cast()) {
                match (number, start, sectors, guid) {
                    (2, 34816, SLOT_SECTORS, A_GUID) => slots[0] = io,
                    (3, 231424, SLOT_SECTORS, B_GUID) => slots[1] = io,
                    (4, 428032, SLOT_SECTORS, RECOVERY_GUID) => slots[2] = io,
                    (5, 624640, STATE_SECTORS, STATE_GUID) => journal = io,
                    _ => {}
                }
            }
        }
        if journal.is_null() || slots.iter().any(|slot| slot.is_null()) {
            (bs.free_pool)(handles);
            return 1;
        }
        let mut records = [0u8; RECORD * 2];
        let media = &*(*journal).media;
        if ((*journal).read)(
            journal,
            media.media_id,
            0,
            records.len(),
            records.as_mut_ptr().cast(),
        ) != SUCCESS
        {
            (bs.free_pool)(handles);
            return 1;
        }
        let mut state =
            newest(&records[..RECORD], &records[RECORD..]).unwrap_or_else(State::default);
        for _ in 0..3 {
            let Some(selected) = state.select() else {
                (bs.free_pool)(handles);
                return 1;
            };
            if write_state(journal, &state) != SUCCESS {
                (bs.free_pool)(handles);
                return 1;
            }
            let slot = slots[(selected - 1) as usize];
            let mut head = [0u8; 8192];
            let media = &*(*slot).media;
            if ((*slot).read)(
                slot,
                media.media_id,
                0,
                head.len(),
                head.as_mut_ptr().cast(),
            ) != SUCCESS
            {
                if !state.fail(selected) {
                    (bs.free_pool)(handles);
                    return 1;
                }
                continue;
            }
            let Some((offset, payload)) = container(&head) else {
                if !state.fail(selected) {
                    (bs.free_pool)(handles);
                    return 1;
                }
                continue;
            };
            let Some(container_size) = offset.checked_add(payload) else {
                if !state.fail(selected) {
                    (bs.free_pool)(handles);
                    return 1;
                }
                continue;
            };
            if container_size > SLOT_BYTES || media.block_size == 0 {
                if !state.fail(selected) {
                    (bs.free_pool)(handles);
                    return 1;
                }
                continue;
            }
            let block_size = media.block_size as usize;
            let Some(rounded) = container_size
                .checked_add(block_size - 1)
                .map(|value| value / block_size * block_size)
            else {
                if !state.fail(selected) {
                    (bs.free_pool)(handles);
                    return 1;
                }
                continue;
            };
            let mut buffer = ptr::null_mut();
            if (bs.allocate_pool)(LOADER_DATA, rounded, &mut buffer) != SUCCESS {
                (bs.free_pool)(handles);
                return 1;
            }
            if ((*slot).read)(slot, media.media_id, 0, rounded, buffer) != SUCCESS {
                (bs.free_pool)(buffer);
                if !state.fail(selected) {
                    (bs.free_pool)(handles);
                    return 1;
                }
                continue;
            }
            let mut child = ptr::null_mut();
            let status = (bs.load_image)(
                false,
                image,
                ptr::null(),
                buffer.cast::<u8>().add(offset).cast(),
                payload,
                &mut child,
            );
            if status == SUCCESS {
                let started = (bs.start_image)(child, ptr::null_mut(), ptr::null_mut());
                let _ = (bs.unload_image)(child);
                (bs.free_pool)(buffer);
                if started == SUCCESS {
                    (bs.free_pool)(handles);
                    return SUCCESS;
                }
            } else {
                (bs.free_pool)(buffer);
            }
            if !state.fail(selected) {
                (bs.free_pool)(handles);
                return 1;
            }
            let _ = write_state(journal, &state);
        }
        (bs.free_pool)(handles);
        1
    }
}

unsafe fn partition_identity(mut node: *const u8) -> Option<(u32, u64, u64, [u8; 16])> {
    // SAFETY: the caller obtained `node` from UEFI Device Path Protocol, whose initialized nodes
    // are aligned for byte access and valid through this synchronous call. Reads use byte pointers,
    // so no unaligned reference is formed; the 64-node and length checks bound traversal. Firmware
    // owns the immutable bytes, no aliasing/thread mutation is introduced, no pointer escapes, and
    // failure returns `None` without acquiring resources or requiring cleanup.
    unsafe {
        for _ in 0..64 {
            let kind = *node;
            let subtype = *node.add(1);
            let len = u16::from_le_bytes([*node.add(2), *node.add(3)]) as usize;
            if len < 4 {
                return None;
            }
            if kind == 4 && subtype == 1 && len >= 42 && *node.add(40) == 2 && *node.add(41) == 2 {
                let number = u32::from_le_bytes(
                    core::slice::from_raw_parts(node.add(4), 4)
                        .try_into()
                        .ok()?,
                );
                let start = u64::from_le_bytes(
                    core::slice::from_raw_parts(node.add(8), 8)
                        .try_into()
                        .ok()?,
                );
                let sectors = u64::from_le_bytes(
                    core::slice::from_raw_parts(node.add(16), 8)
                        .try_into()
                        .ok()?,
                );
                let guid = core::slice::from_raw_parts(node.add(24), 16)
                    .try_into()
                    .ok()?;
                return Some((number, start, sectors, guid));
            }
            if kind == 0x7f {
                return None;
            }
            node = node.add(len);
        }
        None
    }
}
fn container(head: &[u8]) -> Option<(usize, usize)> {
    if head.get(..8)? != b"ZEROSLT1" {
        return None;
    }
    let manifest = u32::from_le_bytes(head.get(8..12)?.try_into().ok()?) as usize;
    if manifest == 0 || manifest > 4096 {
        return None;
    }
    let manifest_end = 12usize.checked_add(manifest)?;
    let text = core::str::from_utf8(head.get(12..manifest_end)?).ok()?;
    let payload = text
        .lines()
        .find_map(|line| line.strip_prefix("payload-size=")?.parse().ok())?;
    Some((manifest_end.checked_add(384)?, payload))
}
unsafe fn write_state(io: *mut BlockIo, state: &State) -> Status {
    // SAFETY: `io` was returned by the validated UEFI Block I/O protocol and its initialized media
    // remains alive for boot. `record` is initialized, aligned byte storage borrowed immutably for
    // exactly RECORD bytes; firmware retains no pointer. Calls are single-threaded and create no
    // Rust alias, the LBA is within the accepted 1 MiB journal, and FlushBlocks completes durability;
    // an error transfers no ownership and leaves the prior alternating record recoverable.
    unsafe {
        let mut record = [0u8; RECORD];
        encode(state, &mut record);
        let media = &*(*io).media;
        let status = ((*io).write)(
            io,
            media.media_id,
            (state.generation & 1) * 8,
            RECORD,
            record.as_ptr().cast(),
        );
        if status == SUCCESS {
            ((*io).flush)(io)
        } else {
            status
        }
    }
}
fn encode(s: &State, out: &mut [u8; RECORD]) {
    out[..8].copy_from_slice(b"ZEROOSB1");
    out[8..16].copy_from_slice(&s.generation.to_le_bytes());
    out[16..24].copy_from_slice(&s.sequence.to_le_bytes());
    out[24] = s.confirmed;
    out[25] = s.pending;
    out[26] = s.booting;
    out[27] = s.failed;
    out[28] = s.recovery;
    let crc = crc32(&out[..RECORD - 4]);
    out[RECORD - 4..].copy_from_slice(&crc.to_le_bytes())
}
fn decode(input: &[u8]) -> Option<State> {
    if input.len() != RECORD
        || &input[..8] != b"ZEROOSB1"
        || crc32(&input[..RECORD - 4]) != u32::from_le_bytes(input[RECORD - 4..].try_into().ok()?)
    {
        return None;
    }
    let s = State {
        generation: u64::from_le_bytes(input[8..16].try_into().ok()?),
        sequence: u64::from_le_bytes(input[16..24].try_into().ok()?),
        confirmed: input[24],
        pending: input[25],
        booting: input[26],
        failed: input[27],
        recovery: input[28],
    };
    if !(1..=2).contains(&s.confirmed)
        || s.pending > 2
        || s.booting > 3
        || s.failed & !7 != 0
        || s.recovery > 1
    {
        None
    } else {
        Some(s)
    }
}
fn newest(a: &[u8], b: &[u8]) -> Option<State> {
    match (decode(a), decode(b)) {
        (Some(x), Some(y)) => Some(if x.generation >= y.generation { x } else { y }),
        (x @ Some(_), None) | (None, x @ Some(_)) => x,
        _ => None,
    }
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
#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop()
    }
}

#[cfg(not(target_os = "uefi"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_bounds_and_state_fallback_are_strict() {
        let manifest = b"payload-size=1024\n";
        let mut input = Vec::from(&b"ZEROSLT1"[..]);
        input.extend_from_slice(&(manifest.len() as u32).to_le_bytes());
        input.extend_from_slice(manifest);
        input.resize(8192, 0);
        assert_eq!(container(&input), Some((12 + manifest.len() + 384, 1024)));
        input[8..12].copy_from_slice(&4097u32.to_le_bytes());
        assert_eq!(container(&input), None);

        let mut first = [0; RECORD];
        let state = State::default();
        encode(&state, &mut first);
        let mut torn = first;
        torn[64] ^= 1;
        assert_eq!(newest(&first, &torn).map(|value| value.generation), Some(0));
    }
}
