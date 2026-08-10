# Testing and fuzzing

## Roadmap scope
Cross-cutting across M0–M11 and later regressions. Derive required techniques and acceptance evidence from the live target milestone in `ROADMAP.md`.

## Purpose and application
Use when planning/reviewing tests, fuzzing, fault injection, Miri, Loom, sanitizers, QEMU, or milestone acceptance. Reference `docs/ai/testing-and-evidence.md` and the affected subsystem skill.

## Inspect first
Read acceptance item/invariants, affected tests and `xtask test` routing, trust inputs, unsafe/concurrency boundaries, CI matrix, and existing evidence.

## Decisions and invariants
Use the smallest runnable check that fails on regression; automate failure/recovery, bound all tests/timeouts, run both targets where relevant, and retain reproducible evidence. One pass is not race proof.

## Forbidden
No screenshot/prose/manual-only completion, deleted/weakened failure test, flaky infinite retry, one-target portability claim, generated corpus/artifacts in Git without accepted boundary, or secret production fixture.

## Workflow
1. Fill `docs/ai/templates/test-plan.md` from invariants/threats.
2. Select unit/state/property/fuzz/Miri/Loom/stress/crash/interruption/QEMU/hardware layer.
3. Add boundary, malformed, exhaustion, cancellation, and rollback cases.
4. Make fixtures deterministic/minimal and failures diagnostic.
5. Run, retain commands/logs/hashes, and state gaps.

## Review checklist
Check test actually crosses the boundary, deterministic oracle, cleanup, timeout, architecture coverage, reduced fuzz repro, fault phase completeness, and no implementation-only assertion masquerading as acceptance.

## Tests, architecture, security, evidence
Use Miri for host unsafe, fuzz/property for parsers/layout, Loom/TSan for concurrency, process crash/power loss for state, and native x86_64/aarch64 CI. Evidence follows milestone template.

## ADR / stop conditions
ADR for acceptance-equivalent substitution, deterministic tolerance, hardware rig, or untestable design. Stop when invariant lacks observable oracle or required target/environment cannot be represented; report gap without completion claim.
