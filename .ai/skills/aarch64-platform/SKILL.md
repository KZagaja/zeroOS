# aarch64 platform

## Roadmap scope
Cross-architecture acceptance in M0–M11, with primary boot ownership in M1/M3 and hardware/certification ownership in M4/M11. Resolve live status from `ROADMAP.md`.

## Purpose and application
Use for ARM64 kernel, UEFI, image, QEMU `virt`, atomics, timers, cache/DMA, firmware/device tree, or target builds. Reference `linux-userspace-os`, `uefi-and-boot`, and cross-architecture review.

## Inspect first
Inspect `kernel/aarch64.config`, `selector/`, ARM branches in `xtask/`, CI, and the matching roadmap acceptance.

## Decisions and invariants
UEFI fallback is `EFI/BOOT/BOOTAA64.EFI`; targets are `aarch64-unknown-linux-musl` / `aarch64-unknown-uefi`; ARM64 weak ordering and hardware variation are first-class.

## Forbidden
No assumed ACPI over device tree, PSCI/timer behavior, cache coherency/DMA, 4 KiB page, unaligned access, atomic availability, QEMU `virt` equivalence, or board-generic firmware.

## Workflow
1. State capability and board/firmware discovery source.
2. Keep target code in approved boundary behind typed capability.
3. Prove alignment, arithmetic, DMA/cache, timer, and ordering behavior.
4. Define unsupported/fallback/recovery result.
5. Build, inspect, boot-test, and compare with x86_64.

## Review checklist
Check fallback filename, PE/COFF target, ACPI/DT, PSCI/generic timer, cache/DMA, page size/alignment, atomics, QEMU/physical differences, and kernel config.

## Tests, architecture, security, evidence
Run `cargo xtask test --arch aarch64` on native ARM CI plus generic checks; run x86_64 when shared code changed. Record hashes and physical-hardware gap.

## ADR / stop conditions
ADR for board-specific base behavior, reference device, firmware discovery contract, or target-only feature. Stop if weak-order/alignment/cache or fallback behavior is unproved.
