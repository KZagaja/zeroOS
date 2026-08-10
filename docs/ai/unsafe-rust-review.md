# Unsafe Rust review

Apply `.ai/skills/unsafe-rust/SKILL.md`. Reject unsafe used for convenience. Verify a small private boundary with safe validated API, `repr(C)` and ABI layout where needed, and one immediately preceding falsifiable `SAFETY` proof covering operation, preconditions, provenance, initialization, aliasing, alignment, lifetime, threads, and cleanup.

Reject raw-pointer leakage, `static mut`, `unreachable_unchecked`, avoidable transmute, invalid zero/uninit, unaligned/MMIO references, aliasing, lifetime extension, or unexplained manual `Send`/`Sync`. Require focused tests plus Miri/fuzzing where executable/applicable.
