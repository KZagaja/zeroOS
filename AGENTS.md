# zeroOS agent operating contract

This contract applies to every change across M0–M11 and later maintenance. `ROADMAP.md` is the authoritative zeroOS 1.0 specification and live milestone ledger; read the relevant section completely before editing. Never cache milestone status, the currently targeted milestone, or acceptance state in agent instructions. zeroOS uses Linux mechanisms and owns native Rust userspace policy; do not create a competing roadmap or turn the project into a custom-kernel effort.

## Roadmap scope

This root contract is cross-cutting and applies to every milestone. At the start of each task, resolve from `ROADMAP.md` which milestone owns the subsystem, its live status, the acceptance criterion being advanced or preserved, and all accepted decisions. An achieved milestone remains in scope for regression and maintenance; a not-started milestone does not authorize speculative implementation.

## Source of truth

Resolve conflicts in this order:

1. `ROADMAP.md`, the authoritative zeroOS specification;
2. accepted architecture decisions;
3. security and on-disk/API format specifications;
4. the nearest scoped `AGENTS.md`;
5. this file;
6. implementation documentation;
7. code comments;
8. agent assumptions.

An assumption never outranks a documented decision. If code, documentation, or tests contradict a higher source, identify exact files, stop treating the behavior as accepted, and propose a code correction or explicit architecture decision. Never rewrite historical milestone evidence.

## Before changing anything

Record in the change or working notes:

1. current milestone and acceptance criterion advanced;
2. relevant source, tests, instructions, and accepted decisions inspected;
3. invariants that must remain true;
4. privilege boundaries and untrusted inputs;
5. x86_64 and aarch64 implications;
6. rollback, interruption, and recovery behavior;
7. tests required before implementation.

Avoid unrelated refactoring. Reuse repository code, then the standard library, then native Linux/UEFI facilities, then admitted dependencies. Add only the minimum complete change.

## After changing code

1. inspect the complete diff and remove accidental scope expansion;
2. run `cargo fmt --all -- --check`;
3. run Clippy and `cargo xtask check` in the pinned build image;
4. run workspace unit tests and applicable integration tests;
5. build/test both architectures when behavior is target-sensitive;
6. run the milestone acceptance commands when required;
7. update documentation only for changed behavior;
8. update milestone evidence only after every acceptance condition passes reproducibly.

## No false completion

Never claim an unrun command passed, infer both architectures from one, weaken/delete a failing test, convert failure into vague prose, silently edit the specification to fit code, or mark achievement from prose, screenshots, partial coverage, or manual local testing. Preserve earlier evidence; regress an achieved milestone to `In progress` with a dated entry when required by `ROADMAP.md`.

## Scope discipline

Changes must be minimal enough to review, complete enough to be correct, tied to a concrete requirement, free of speculative abstraction, and free of unrelated cleanup. Do not redesign another subsystem for the current implementation's convenience.

## Rust policy

This applies to production crates, `xtask`, tests, examples, fixtures, build tools, and architecture tooling.

- Do not introduce `.unwrap()`, `.expect()`, `panic!`, `todo!`, `unimplemented!`, `unreachable!`, or `unreachable_unchecked`. Tests return `Result` or use explicit assertions.
- Use `?`, explicit matches, fallible constructors, or typed invariant helpers. Panics are not ordinary error handling.
- Preserve actionable error context without secrets. API boundaries need stable codes and retry/permanent/invalid/unauthorized/unavailable/internal classification where callers make policy decisions. One layer owns logging.
- Use checked arithmetic, checked conversions, explicit bounds, and allocation limits for external sizes, offsets, counts, partitions, timestamps, protocol fields, and device data. Validate signed-to-unsigned conversions and never assume an external size fits `usize`.
- Own descriptors, sockets, children, mappings, temporary files, mounts, locks, handles, buffers, and transactions with RAII and deterministic partial-failure cleanup. Use atomic `CLOEXEC` creation where supported.
- Workspace lints deny `unsafe_op_in_unsafe_fn` and undocumented unsafe blocks. Do not weaken them globally.

## Unsafe and FFI

Unsafe is limited to necessary syscall, firmware, FFI, mapping, ABI, device, raw-pointer, or measured low-level boundaries. Keep it private behind a safe validated API.

Every unsafe block must have an immediately preceding falsifiable `SAFETY` comment covering the operation, all preconditions and how they hold, provenance, initialization, aliasing, alignment, lifetime, thread safety, and partial-failure cleanup. For inapplicable dimensions, say why. Focused tests are mandatory; use Miri when host-executable and fuzz/property tests for parsing or layout.

Do not introduce `static mut`, unjustified manual `Send`/`Sync`, avoidable `transmute`, invalid uninitialized/zeroed values, unaligned references, MMIO references, aliased mutable references, lifetime extension, unproven self-reference, or raw-pointer leakage. Manual `Send`/`Sync` needs a dedicated proof and concurrency review.

FFI uses correct ABI types and `repr(C)` where required, verifies size/alignment/discriminants, defines nullability/encoding/ownership/reentrancy, prevents unwinding, translates errors, and keeps cleanup correct.

## Concurrency, signals, and processes

Correctness must hold on ARM64's weaker ordering, not merely x86_64. Document nontrivial atomics: state, publisher, observer, happens-before edge, and ordering sufficiency. `Relaxed` needs proof; `SeqCst` needs a stated simplicity/correctness reason.

Subsystems with multiple locks define and follow a hierarchy. Never hold a lock across disk/network I/O, child waits, external callbacks/engines, blocking channels, sleep, synchronous-guard `.await`, notification, or lower-ranked acquisition. Define poisoning recovery; never unwrap a poisoned lock.

Signal handlers only perform async-signal-safe work or atomic notification. Prefer `signalfd`, self-pipe, or event-loop dispatch. After `fork` in a multithreaded process, perform only async-signal-safe operations before `exec` unless an accepted design proves otherwise. Use deterministic models, Loom, stress/fault/crash/timeout tests, and sanitizers where applicable.

## Architecture and Linux boundary

Review every owned component for `x86_64` and `aarch64`. Do not assume 4 KiB pages, ACPI, alignment tolerance, cache/order behavior, atomic support, timer/firmware/device enumeration, boot conventions beyond accepted UEFI, kernel configuration, code generation, or graphics/storage hardware equivalence.

Target code belongs in `kernel/*.config`, `selector/`, or a future explicit architecture module; generic code consumes typed capabilities. `xtask` may orchestrate targets but must not hide target behavior in generic policy. Review x86_64 for `BOOTX64.EFI`, CPUID/TSC/ACPI, unaligned access, ordering, and QEMU differences; review ARM64 for `BOOTAA64.EFI`, ACPI/device tree, PSCI/timers, DMA/coherency, alignment/page size, ordering, board firmware, and QEMU `virt` differences.

Linux owns scheduling, VM, drivers, filesystems, networking primitives, DRM/KMS, and input. zeroOS Rust services own product policy. Do not reimplement kernel mechanisms or outsource policy to shell commands/compatibility engines without an accepted decision.

## High-risk subsystem rules

- PID 1: stay alive, reap all children/orphans, bound memory/logs/messages/time, validate process identity and API version before mutation, isolate failures, cap restarts, and shut down deterministically. No ordinary shell execution or arbitrary recovery commands. Test healthy, hung/crashing/dependency/orphan/full-buffer/EINTR/repeated-signal shutdown.
- Storage/update/recovery: follow the live accepted storage milestone contracts in `ROADMAP.md`. Mutations are transactional and power-loss aware; `rename` alone is not atomic. Verify signed metadata and payload before inactive-slot activation, fail closed, flush every durability boundary, preserve data, account boot trials/health/rollback, and inject interruption at every roadmap phase. Do not invent cryptography, filesystem, TPM, verity, or boot-count decisions.
- Networking: when introduced, use bounded validated explicit state machines with timeout, cancellation, retry/backoff, authorization, secret-safe logs, netlink multipart/truncation/sequence/index-reuse handling, and suspend/restart recovery. No command-output parsing or ordinary shell configuration.
- Wayland: zeroOS owns the wire server; do not add Smithay/foundation frameworks. Treat clients as hostile; validate object ownership/version/lifecycle/IDs, all wire data and descriptors, resource limits, buffer release, and focus/input/clipboard/DnD isolation.

## Dependencies and evidence

Every new dependency requires a record using `docs/ai/templates/dependency-admission.md` and an entry in `policy/dependencies.csv`. Record the current need, alternatives, maintenance/vulnerability handling, license, both targets, exact pin/checksum acquisition, transitives, owner/update path, attack/base-image surface, and `Retain`/`Replace` exit criteria. Convenience is insufficient; do not hand-roll mature security protocols to avoid a reviewed isolated dependency.

Use `docs/ai/change-protocol.md` and `docs/ai/review-protocol.md`. Evidence follows `docs/ai/testing-and-evidence.md`; milestone changes use `.ai/skills/milestone-evidence/SKILL.md`.
