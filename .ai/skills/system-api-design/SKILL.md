# System API design

## Roadmap scope
M2 establishes the core API; M3–M9 extend system/application APIs; M10/M11 validate reliability and compatibility. Resolve the live owning milestone from `ROADMAP.md`.

## Purpose and application
Use for PID 1 or future service sockets, protocols, stable error codes, object/session APIs, and compatibility changes. Reference Rust, concurrency, security, and `docs/ai/system-api-review.md`.

## Inspect first
Read the M2 core API contract and later relevant roadmap section; inspect dispatch/parser, all clients/tests, authorization/session ownership, and recovery console mapping.

## Decisions and invariants
Version negotiation precedes mutation; v1 core socket is root-only, newline-terminated, bounded to 4096 bytes, one request/connection, and returns `OK/ERR ZEROOS/1`. Compatible additions do not change existing meaning; incompatible change uses a new major/socket.

## Forbidden
No unbounded messages/objects, ambiguous strings where callers need policy, engine API leakage, authorization after mutation, silent semantic change, client monopoly, descriptor/identity confusion, or secret-bearing error/log.

## Workflow
1. Define actor, authority, state, version, request/response limits, timeout/cancellation, and resource ownership.
2. Define stable error codes and recovery classification.
3. Parse/validate/version/authenticate before dispatch.
4. Make mutations idempotent or define duplicate/retry behavior.
5. Test malformed/incompatible/unauthorized/disconnected/restarted clients.

## Review checklist
Check compatibility, trailing fields, IDs/PID reuse, FD ownership/CLOEXEC, replay/order, per-client quotas/fairness, logging owner, cancellation, engine restart, and revocation.

## Tests, architecture, security, evidence
Run parser/state tests, malformed/boundary/fuzz cases, failure isolation, and both targets for system runtime APIs. Record protocol examples, exact commands, and behavior on rejection.

## ADR / stop conditions
ADR for new public major version, durable error/identity model, cross-subsystem authority, or transport change. Stop when compatibility, authorization, limits, or recovery semantics are unspecified.
