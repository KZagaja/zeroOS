# PID 1 and supervision

## Roadmap scope
M1 bootstrap, M2 resident runtime, M3 health/recovery integration, and later service maintenance. Resolve live status and acceptance criteria from `ROADMAP.md`.

## Purpose and application
Use for `init/` runtime, root socket, recovery console, signals, children, service graph/restarts, logging, shutdown, health confirmation, or recovery. Reference concurrency, unsafe Rust, system API, security, and storage skills.

## Inspect first
Read M2/M3 contracts, `init/AGENTS.md`, all of `init/src/main.rs`, `storage` boot state calls, `xtask` runtime acceptance, and service/API tests.

## Decisions and invariants
PID 1 stays alive, reaps all children/orphans, bounds 4096-byte requests and 256 logs, preserves dependency order/failure isolation/three-in-ten-second restart budget/two-second shutdown grace, rejects incompatible versions before mutation, and exposes only allowlisted recovery commands.

## Forbidden
No unwrap/panic on failure, shell, arbitrary recovery execution, unvalidated/stale PID identity, unbounded client work/logs/retries/blocking, secret logs, signal allocation/locks/logging, or service failure terminating PID 1.

## Workflow
1. Trace the service/API/signal state transition and all callers.
2. State liveness, ordering, bounds, identity, cancellation, and recovery invariants.
3. Keep signal context atomic/minimal and effects in the loop.
4. Validate version/request/authorization before mutation; bound slow work.
5. Add deterministic failure/shutdown acceptance and inspect logs.

## Review checklist
Check child RAII/reaping, PID reuse, EINTR, orphan adoption, hung/crashing dependency, restart clearing, socket fairness, shutdown mutation rejection/order, log capacity, full buffers, repeated signals, and state flush.

## Tests, architecture, security, evidence
Run workspace tests and both QEMU target acceptance; exercise healthy/hung/crash/dependency/orphan/full-log/EINTR/repeated-signal shutdown. Record exact log markers and target artifacts.

## ADR / stop conditions
ADR for service graph/public API major version, event-loop model, process identity mechanism, authorization, or health-confirmation contract. Stop for unbounded wait/input, unsafe post-fork work, ambiguous identity, or inability to keep PID 1 alive.
