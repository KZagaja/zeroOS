# Test plan: M3 storage, updates, and recovery

- Acceptance/invariants: all six M3 automated criteria; fixed formats/layout; inactive-only staging; rejection before activation; interruption safety; trial rollback/confirmation; recovery repair; explicit data-only reset.
- Trust limits: 4096-byte manifest/request/record; 384-byte signature; slot-capacity payload; 32 KiB response headers; 15-minute update call; five redirects; 2048-byte resume metadata; 16 KiB child progress; 12–1024-byte credentials; at most two release keys.
- Host cases: GPT identity/capacity; every torn journal boundary; strict manifest and ring RSA-PSS; signer rotation/count; downgrade/corruption; redirect authority/userinfo/fragment/path; strong ETag and exact Content-Range; metadata mismatch; reread verification; terminal restoration model; secret redaction; API/status/update/health transitions.
- Fault cases: interruption during download/write/flush/reread/journal/reboot/health; short and oversized response; `200` restart; inconsistent `206`; engine failure; invalid LUKS bytes; both slot failures; malformed repair/reset grammar.
- Unsafe coverage: warnings-denied Clippy and focused termios/mlock/memfd/UEFI tests; Miri only for host-executable private boundaries where libc calls are mocked; parser fuzzing remains required before achievement evidence.
- x86_64: pinned image `cargo xtask test --arch x86_64`; retain QEMU log and selector/system/recovery/init/update/disk hashes.
- aarch64: pinned image `cargo xtask test --arch aarch64`; retain equivalent hashes and note QEMU `virt` is not physical-device evidence.
- Environment: Rust 1.97.1 pinned image digest from `policy/build-image.lock`; disposable keys and local HTTPS fixture only; no production secrets. Timeout every child/QEMU run and retain redacted logs.
- Out of scope: human ceremony/review/release gates cannot be automated or inferred; M4 hardware certification is not M3 evidence.
