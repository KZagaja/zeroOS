# UEFI selector scope

Root `AGENTS.md` applies. Use UEFI, unsafe Rust, storage, and both architecture skills.

Roadmap scope: M1 UEFI bootstrap, M3 slot/recovery selection, M10 recovery media, and M11 certification. Read live status and accepted boot contracts from `ROADMAP.md`; do not infer the current target from this file.

- This is the explicit architecture/firmware boundary. Preserve `BOOTX64.EFI`/`BOOTAA64.EFI`, accepted partition starts, `ZEROOSB1` journal, one-boot trials, rollback, and recovery selection.
- Validate every firmware pointer/protocol/media/block size, checked offset/rounding, allocation, and container field before dereference or load. Every pool allocation is freed on every failure path.
- Keep unsafe operations in minimal blocks with full `SAFETY` proofs; do not weaken workspace lint policy.
- Build both UEFI targets and run both QEMU acceptance paths for boot-state or ABI changes.
