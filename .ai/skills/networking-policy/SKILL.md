# Networking policy

## Roadmap scope
M4 owns networking policy; M7–M11 consume and validate it. Read live status, accepted decisions, and acceptance criteria from `ROADMAP.md`.

## Purpose and application
Use for networking service/adapters and their later integrations. If implementation is absent, do not scaffold it without a concrete live roadmap acceptance target. Reference Rust, Linux userspace, concurrency, system API, security, architecture, and dependency skills.

## Inspect first
Read M4 and dependency decisions, then the actual new service/API/tests, kernel capability use, retained WPA adapter record, and session/permission integration.

## Decisions and invariants
zeroOS owns public networking policy; a mature WPA engine may remain behind a narrow restartable adapter. Operations are explicit bounded state machines with state/event/result/timeout/cancel/retry/backoff/user error/log/secret rules.

## Forbidden
No human-readable command parsing, ordinary shell configuration, public WPA-engine API, credential/auth-exchange logs, lock held while waiting, infinite retry, link-up-as-Internet, or unbounded messages.

## Workflow
1. Define states/events including interface loss, duplicate/reordered netlink, engine restart, suspend, clock, revocation, DHCP/DNS/captive failure.
2. Bound and validate netlink multipart/truncation/sequence/index reuse/buffer pressure.
3. Release locks before kernel/engine/network waits.
4. Isolate credentials and authorize every mutation.
5. Add deterministic/fault/reconnect tests.

## Review checklist
Check stale state, cancellation races, backoff exhaustion, namespace/session isolation, engine crash, secret redaction, client quotas, and user-visible distinction between link/DHCP/DNS/Internet.

## Tests, architecture, security, evidence
Use state-machine/netlink fixture/fault/stress/suspend tests, hostile messages, engine restart, both target builds, and reference hardware when selected. Retain event traces without secrets.

## ADR / stop conditions
ADR for retained WPA engine, public API, credential storage, connectivity definition, or namespace ownership. Stop when the live roadmap does not authorize the requested scope or any state/timeout/authorization is undefined.
