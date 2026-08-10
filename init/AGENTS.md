# PID 1 scope

Root `AGENTS.md` applies. Also use `.ai/skills/pid1-and-supervision/SKILL.md`.

Roadmap scope: M1 bootstrap, M2 resident runtime, M3 health/recovery integration, and later maintenance. Read their live status and accepted contracts from `ROADMAP.md`; do not infer the current target from this file.

- Preserve the live accepted PID 1/API contract in `ROADMAP.md`, including service order, restart budget, bounded logs/request, recovery allowlist, failure isolation, and shutdown behavior.
- Treat the root socket, serial recovery console, fixture messages, PIDs, signals, and child exits as privileged or untrusted boundaries. Validate API version before state mutation and never trust a stale numeric PID.
- Signal handlers only set atomics. Bound every client/read/wait; PID 1 must stay alive and reap every child.
- No shell, arbitrary command execution, secret logs, or blocking operation without cancellation/timeout.
- Changes require host tests plus both `cargo xtask test --arch ...` runtime acceptance when target behavior is affected.
