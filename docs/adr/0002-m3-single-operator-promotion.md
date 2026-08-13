# ADR: M3 single-operator promotion

- Status: Accepted
- Date / owner: 2026-08-13 / Kordian Zagaja
- Roadmap milestone and acceptance item: M3 key ceremony, production promotion, and achievement evidence
- Supersedes / conflicts: supersedes M3's independent security review and witnessed recovery-test gates; does not supersede ADR 0001

## Context and invariants

M3 cannot use independent review or a second ceremony witness because the repository and key custody have one operator. Keeping either gate would make promotion impossible without adding a nominal account that provides no real independence. Removing them permanently loses separation of duties: one compromised, coerced, or mistaken owner can approve malicious source, operate the signing hardware, and promote the result. Hardware-backed keys limit extraction but do not prevent an authorized bad signature.

No runtime API, `ZEROOSB1`, `ZEROSLT1`, GPT, algorithm, key-custody, update durability, rollback, recovery, cross-architecture, or secret-handling requirement changes. Production and recovery private keys remain outside Git, command arguments, ordinary environment values, artifacts, and logs. CI inputs, runner state, release assets, HSM output, ceremony records, and backup media remain untrusted until verified. A failed or interrupted gate leaves M3 `In progress` and any public candidate a prerelease.

## Options

- Keep independent review and witnessing: preserves separation of duties but cannot be truthfully satisfied by the current single operator.
- Add a nominal second account: creates ceremony theater without an independent person and weakens audit quality.
- Adopt an explicit single-operator exception with compensating evidence: makes the assurance loss visible and relies on reproducible exact-commit checks, non-exporting hardware keys, public verification, and recovery evidence. It does not protect against a malicious owner.
- Do nothing: M3 remains permanently `In progress` despite otherwise complete implementation and hardware custody.

## Decision and consequences

Permanently remove independent security review and second-person ceremony witnessing from M3. Promotion is repository-owner-only and must be dispatched from `main` for a full source SHA identical to `main` after successful exact-SHA native CI. Both `offline-recovery-draft` and `production-release` environments must exist and permit only the `main` branch. Required-reviewer and self-review protection are not gates.

The owner must retain a UTC-timestamped ceremony record with a detached signature made by the offline recovery signer, redacted hash-chained HSM audit exports, public object labels and SHA-256 fingerprints, two encrypted geographically separate backups, and a destructive restore test from one backup. The restore test must reinitialize the still-disposable secondary HSM, reproduce every public fingerprint, and verify one Secure Boot signature and one RSA-PSS/SHA-256 signature. The emergency KEK-authorized current-db removal/dbx rehearsal must reject the revoked artifact while preserving recovery and next-key boot.

Both dedicated Ubuntu 24.04 native runners must use the candidate CI's version- and SHA-256-pinned `zeroos-sign`, local-only non-debug HSM connectors, root-owned configuration, limited per-device authentication, and no unrelated workloads. Production rebuilds and tests both architectures, signs through non-exporting current keys, publishes an immutable public prerelease, and runs unauthenticated native `cargo xtask test-release` installations. Passing installations alone may copy the exact candidate bytes into the immutable final `sequence-N` release and mark it latest. The public release repository's immutable-release setting and all public hashes, signatures, provenance, and availability are evidence gates.

Achievement additionally requires both native acceptance commands on the exact evidence commit, the complete interruption/recovery matrix, redacted backup/restore and audit records, and verification that no secret entered Git, Actions, artifacts, or logs. Roll back or contain suspected owner, runner, token, or HSM compromise by disabling both environments and runners, revoking the release token, and revoking or replacing every affected db/release key through the offline PK/KEK path. Reopen this decision only by a later accepted ADR that restores genuine independent custody or review; do not silently reintroduce a nominal witness.
