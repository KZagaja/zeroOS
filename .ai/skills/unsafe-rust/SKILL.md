# Unsafe Rust

## Roadmap scope
Cross-cutting across any milestone containing unsafe Rust, currently including the M1/M3 boot and storage boundaries; live scope comes from `ROADMAP.md`.

## Purpose and application
Use for syscall, FFI, UEFI ABI, mappings, devices, raw pointers, manual `Send`/`Sync`, or any unsafe edit/review. Reference `rust-systems-engineering` and `docs/ai/unsafe-rust-review.md`.

## Inspect first
Inspect the complete private module, all safe callers, ABI definitions, ownership/cleanup paths, target docs already encoded in the repository, and focused tests. Current unsafe boundaries are `init` libc calls, `data` secret-memory/memfd calls, and `selector` UEFI.

## Decisions and invariants
Workspace denies unsafe operations in unsafe functions and undocumented blocks. Unsafe is necessary, minimal, private, validated before entry, and exposed through a safe API.

## Forbidden
No convenience unsafe, raw-pointer leakage, `static mut`, undocumented manual `Send`/`Sync`, avoidable transmute, invalid uninit/zero, unaligned/MMIO references, aliasing, lifetime extension, self-reference without pin proof, or `unreachable_unchecked`.

## Workflow
1. Prove a safe alternative is insufficient.
2. List every unsafe operation and precondition.
3. Validate external parameters before the boundary.
4. Add immediately preceding `SAFETY` proof covering operation, preconditions, provenance, initialization, aliasing, alignment, lifetime, threads, and partial-failure cleanup.
5. Encapsulate cleanup and expose safe types/functions.
6. Add focused validity/failure tests and run review.

## Review checklist
Verify `repr(C)`, size/alignment/discriminants, ABI types, nullability, strings, ownership transfer, callbacks/reentrancy, no unwind across FFI, error translation, and every exit cleanup.

## Tests, architecture, security, evidence
Run Clippy, focused tests, Miri when host-executable, fuzz/property tests for byte/layout handling, and both UEFI targets for firmware code. Evidence includes safety template, commands, target, and uncovered hardware behavior.

## ADR / stop conditions
ADR for manual thread-safety, novel pinning/ownership, durable ABI, or measured unsafe optimization. Stop when provenance/lifetime/cleanup cannot be proven, target ABI is unknown, or Miri/fuzz-required coverage cannot be designed.
