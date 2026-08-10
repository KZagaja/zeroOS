# x86_64 platform

## Roadmap scope
Cross-architecture acceptance in M0–M11, with primary boot ownership in M1/M3 and certification in M11. Resolve live status from `ROADMAP.md`.

## Purpose and application
Use for x86_64 kernel, UEFI, image, QEMU, atomics, timers, CPU, firmware, device, or target build work. Reference `linux-userspace-os`, `uefi-and-boot`, and cross-architecture review.

## Inspect first
Inspect `kernel/x86_64.config`, `selector/`, x86 branches in `xtask/`, `.github/workflows/check.yml`, and the matching roadmap acceptance.

## Decisions and invariants
UEFI fallback is `EFI/BOOT/BOOTX64.EFI`; target is `x86_64-unknown-linux-musl` / `x86_64-unknown-uefi`; generic behavior remains shared and portable to aarch64.

## Forbidden
No assumed ACPI presence, stable TSC, universal CPUID feature, tolerated unaligned access, x86 memory ordering, legacy boot/base-system creep, or QEMU-as-hardware claim.

## Workflow
1. State required x86 capability and generic interface.
2. Keep implementation in explicit approved boundary.
3. Detect features/firmware inputs and define fallback/error.
4. Review alignment/order/page/timer assumptions.
5. Build, inspect, boot-test, and compare with aarch64 behavior.

## Review checklist
Check fallback filename, PE/COFF target, ACPI/CPUID/TSC, q35/physical differences, kernel config, descriptors/DMA, and recovery boot.

## Tests, architecture, security, evidence
Run `cargo xtask test --arch x86_64` in the pinned image plus generic checks; run aarch64 when generic code changed. Record artifact hashes and note missing physical-device evidence.

## ADR / stop conditions
ADR for legacy boot, x86-only base behavior, reference hardware, or generic capability change. Stop if firmware/hardware assumption lacks an accepted fallback or both-target impact is unknown.
