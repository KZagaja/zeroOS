# Storage format scope

Root `AGENTS.md` applies. Also use `.ai/skills/storage-updates-and-recovery/SKILL.md`.

Roadmap scope: M3 storage/update foundations, M10 installation/recovery, and M11 certification. Read their live status, formats, and acceptance criteria from `ROADMAP.md`; do not infer the current target from this file.

- Preserve the currently accepted GPT, `ZEROOSB1`, and `ZEROSLT1` roadmap contracts; do not alter them without an ADR and version/migration plan.
- Treat journal, manifest, redirect, file size, offset, sequence, credential, and interruption point as untrusted. Use checked conversions/arithmetic and explicit allocation bounds.
- Preserve alternating-record torn-write recovery, monotonic update sequence, inactive-slot rule, exact signed bytes, fail-closed verification, and user-data preservation.
- Unit/property tests must cover malformed formats, boundary sizes, corruption, downgrade, and every state transition affected; target-sensitive changes require both architecture acceptance.
