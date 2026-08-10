# Concurrency and locking

## Roadmap scope
Primarily M2 and all later concurrent services, with cross-cutting application wherever shared state exists. Resolve live status from `ROADMAP.md`.

## Purpose and application
Use for threads, channels, locks, atomics, signals, fork/exec, callbacks, event loops, shutdown, or shared state. Reference `rust-systems-engineering` and `docs/ai/concurrency-review.md`.

## Inspect first
Trace every reader/writer, lock acquisition, callback, channel, atomic, signal handler, child lifecycle, timeout, and shutdown path. Inspect `init/` first for PID 1 work.

## Decisions and invariants
Correct on ARM64 weak ordering and x86_64; critical sections are short; multi-lock subsystems publish a hierarchy and poisoning policy; shutdown/cancellation are bounded.

## Forbidden
No I/O, child wait, callbacks/engines, blocking channel, sleep, notifications, or `.await` under synchronous locks; no inverse rank; no mutex/log reacquisition; no mutex/allocation/logging in signal handlers; no unproved `Relaxed` or reflexive `SeqCst`.

## Workflow
1. Map state ownership and interleavings.
2. Define lock ranks/poisoning or atomic publisher-observer happens-before.
3. Extract state under lock, release, perform slow work, validate and commit.
4. Use atomic signal notification or event-loop dispatch.
5. Add deterministic failure/interleaving checks.

## Review checklist
Check deadlock, starvation, priority inversion, duplicate/reordered events, PID reuse, fork-after-threads, cancellation, time changes, crash/restart, and repeated signals.

## Tests, architecture, security, evidence
Use state-machine tests, Loom reduced models, stress/fault/process-crash/timeout tests, repeated target CI, and TSan where supported. Record ordering proof and exact results for both targets or explicit host limitation.

## ADR / stop conditions
ADR for global concurrency model, lock hierarchy crossing subsystems, or manual `Send`/`Sync`. Stop when happens-before, poison recovery, cancellation, or fork safety is unprovable.
