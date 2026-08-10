# Encrypted data scope

Root `AGENTS.md` and `storage/AGENTS.md` apply. Use unsafe and security skills.

Roadmap scope: M3 encrypted data/recovery, M10 installation/reset, and M11 certification. Read live status and accepted storage decisions from `ROADMAP.md`; do not infer the current target from this file.

- This privileged tool handles user credentials and destructive storage engines. Never log secrets; keep key material in locked memory and anonymous non-inheritable FDs, wipe it on every exit, and define partial-failure cleanup.
- Preserve accepted LUKS2 AES-XTS-plain64/512-bit/Argon2id, two user-held keyslots, ext4, and recovery/factory-reset behavior. Any format change needs an ADR.
- Exact device identity and destructive confirmation must precede mutation. Engine failure must remain actionable without echoing secret material.
- Focused tests cover credential bounds/redaction/cleanup; destructive flows use disposable devices and interruption tests.
