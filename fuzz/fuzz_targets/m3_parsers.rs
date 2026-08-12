#![no_main]

use libfuzzer_sys::fuzz_target;
use zeroos_storage::{
    BootState, Manifest, RECORD_BYTES, container_manifest_size, decode_record, encode_record,
    newest_record,
};

fuzz_target!(|bytes: &[u8]| {
    if let Some(header) = bytes.get(..12) {
        let _ = container_manifest_size(header);
    }
    let _ = Manifest::parse(bytes, "x86_64");
    let valid = encode_record(&BootState::default());
    let mut hostile = [0; RECORD_BYTES];
    for (target, source) in hostile.iter_mut().zip(bytes) {
        *target = *source;
    }
    let _ = decode_record(&hostile);
    let _ = newest_record(&valid, &hostile);
});
