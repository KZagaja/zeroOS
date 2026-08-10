# OS security

## Roadmap scope
Cross-cutting across M0–M11 and later maintenance; use the target milestone's live threats, gates, and acceptance state from `ROADMAP.md`.

## Purpose and application
Use for privileged code, untrusted parsing/IPC, identity/permissions, signing/crypto/secrets, recovery, external engines, devices, or destructive operations.

## Inspect first
Read root/scoped rules, `docs/ai/security-review.md`, relevant API/format contract in `ROADMAP.md`, all entry points, logging, persistence, and recovery paths.

## Decisions and invariants
Authenticate/authorize before mutation, validate/bound before allocation/dispatch, fail closed, least privilege, isolated clients/services, non-secret actionable logs, no custom crypto, and explicit destructive confirmation.

## Forbidden
No secret in arguments/environment/logs/artifacts/Git, fail-open verification, stale numeric identity trust, unbounded request/object/retry state, shell injection surface, hidden recovery authority, or ad-hoc cryptography.

## Workflow
1. Fill `docs/ai/templates/threat-model.md` for a new boundary.
2. Inventory assets, actors, entry points, privileges, secrets, persistence, and abuse cases.
3. Place authorization and strict parsing before effects.
4. Bound CPU/memory/FD/time/retries and isolate failure.
5. Add negative/fault/revocation/recovery tests and review logs.

## Review checklist
Check confused deputy, replay/downgrade, TOCTOU/PID reuse, descriptor leaks, path traversal/symlinks, rollback, exhaustion, error oracle, dependency compromise, and cross-client/session isolation.

## Tests, architecture, security, evidence
Use malformed/boundary/property/fuzz tests, fault/crash/power-loss tests, both targets, and independent review at roadmap gates. Retain exact commands, threat model, logs, hashes, and unresolved risks.

## ADR / stop conditions
ADR for trust roots, crypto/signature format, identity/authorization model, destructive recovery, or security exception. Stop for missing threat decision, key handling, revocation/rollback, or unresolved high-impact risk.
