# Change protocol

1. Read root and nearest scoped `AGENTS.md`, then the relevant `ROADMAP.md` section.
2. State milestone, exact acceptance item, inspected files, invariants, trust/privilege inputs, architecture effects, recovery path, and planned tests.
3. If a required decision is absent, stop implementation and use `templates/architecture-decision.md`; do not invent it.
4. Trace all callers and state transitions. Reuse an existing boundary; avoid unrelated cleanup.
5. Implement the smallest complete change with bounded inputs, typed errors, RAII, checked arithmetic, and safe rollback.
6. Inspect the full diff, run `cargo xtask check`, applicable host/architecture acceptance, and record exact results.
7. Update behavior docs; update milestone evidence only under the ledger rules.

Rollback the code change when its new format/API cannot safely coexist with the prior version. Never roll back or erase historical evidence.
