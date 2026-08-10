# UEFI and boot

## Roadmap scope
M1 bootstrap, M3 selection/recovery, M10 installation/recovery media, and M11 release certification. Read each section's live status and contract from `ROADMAP.md`.

## Purpose and application
Use for `selector/`, ESP/image construction, kernel EFI artifacts, boot journal/slot selection, recovery selection, or Secure Boot work. Reference `unsafe-rust`, both platform skills, storage/recovery, and security.

## Inspect first
Read M1 and M3 contracts in `ROADMAP.md`; inspect `selector/src/main.rs`, `storage/src/lib.rs`, `xtask` packaging/inspection, kernel configs, source locks, and both architecture tests.

## Decisions and invariants
UEFI fallback names and boot state follow the accepted contracts in the applicable M1/M3/M10/M11 sections exactly; never substitute a cached copy for the live roadmap text.

## Forbidden
No GRUB, alternate layout/record format, unverified activation, numeric/overflow assumptions, firmware pointer trust without ABI validation, one-architecture evidence, or production key material.

## Workflow
1. Map firmware/block inputs and boot-state transition including crash result.
2. Validate ABI/layout, block size/alignment, bounds, checked offset/rounding, and ownership.
3. Persist journal record then flush at the accepted boundary.
4. Load only validated selected payload; mark failure and isolate recovery.
5. Inspect image and boot both targets with failure injection.

## Review checklist
Check pool cleanup every exit, protocol/media validity, partition identity, torn records, generation wrap policy, slot size, manifest/signature/payload ordering, flush, attempt limits, and independent recovery.

## Tests, architecture, security, evidence
Unit/property test journal/container parsing; test corruption, torn writes, both slot failures, target fallback files, QEMU x86_64/aarch64, disposable Secure Boot keys when implemented, and hashes.

## ADR / stop conditions
ADR for boot-count persistence, Secure Boot keys, signature/trust changes, layout/format revision, non-UEFI path, or hardware-specific boot. Stop on unchecked firmware ABI, missing crash state, absent signer decision, or one-target regression.
