# Rust systems engineering

## Roadmap scope
Cross-cutting across M0–M11 and later maintenance; read the owning milestone and live acceptance state from `ROADMAP.md`.

## Purpose and application
Use for all owned Rust: `init`, `storage`, `updater`, `data`, `selector`, `xtask`, tests, fixtures, and tools.

## Inspect first
Read root/scoped `AGENTS.md`, relevant callers/tests, workspace manifests/lints, and the applicable unsafe/concurrency/security skill.

## Decisions and invariants
Static-musl native userspace, explicit ownership, bounded input, checked sizes, actionable non-secret errors, deterministic cleanup, atomic CLOEXEC creation, and one logging owner. The no-unwrap/panic policy applies to tests and tools too.

## Forbidden
No `.unwrap()`, `.expect()`, panic/todo/unimplemented/unreachable macros, unchecked external arithmetic/casts, secret-bearing errors, resource leaks, speculative abstractions, or new convenience dependency.

## Workflow
1. Trace callers, inputs, ownership, cleanup, and error consumers.
2. Write bounds/invariants and target implications.
3. Reuse code/stdlib/native facilities.
4. Implement with `Result`, typed/stable errors where policy is consumed, checked conversions, RAII, and bounded allocation.
5. Add the smallest failure-focused test and validate.

## Review checklist
Check partial failure, double logging, descriptor inheritance, PID reuse, cancellation/timeouts, secret redaction, release overflow, and every unsafe call.

## Tests, architecture, security, evidence
Run format, Clippy, workspace tests, policy checks, and both target builds when relevant. Add Miri/fuzz/Loom/sanitizers only where the boundary warrants them. Record exact commands/results and limitations.

## ADR / stop conditions
ADR for public/durable error/API format, new dependency, or cross-subsystem ownership. Stop for missing bounds/authorization/recovery decision, unreviewable unsafe, or a failing required check.
