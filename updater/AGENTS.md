# Updater scope

Root `AGENTS.md` and `storage/AGENTS.md` apply. Use `.ai/skills/storage-updates-and-recovery/SKILL.md` and `.ai/skills/os-security/SKILL.md`.

Roadmap scope: M3 update foundations and M11 release certification. Read live status, gates, and accepted formats from `ROADMAP.md`; do not infer the current target from this file.

- Verify URL/redirect, architecture, complete manifest, monotonic sequence, RSA-PSS signature over exact manifest bytes, payload size/hash, and inactive target before mutation.
- Fail closed and redact secrets/paths where disclosure matters. Private keys never enter Git, arguments, environment, artifacts, or logs.
- Define and test partial download, short read/write, verification failure, interruption, flush, reread, staging, and rollback outcomes.
- OpenSSL/curl are admitted `Replace` engines behind this narrow adapter; do not expose their interface as zeroOS policy.
