# ADR: Defer hardware-backed signing to the 1.0 gate

- Status: Accepted
- Date / owner: 2026-08-14 / Kordian Zagaja
- Roadmap milestone and acceptance item: M3 release promotion and M11 signed 1.0 release
- Supersedes / conflicts: supersedes ADR 0001 and ADR 0003 only where they require YubiHSM, hardware-non-exportable keys, HSM audit, and HSM restore evidence for M3

## Context and invariants

YubiHSM hardware will not be available during M3. Keeping it as an M3 gate would block storage, update, rollback, recovery, and public-install evidence without improving those implementations. Signed artifacts, Secure Boot, RSA-3072-PSS/SHA-256 manifests, offline PK/KEK and recovery custody, exact-SHA native CI, immutable public releases, secret exclusion, rotation/revocation rehearsal, and both-architecture acceptance remain required.

A software PKCS#11 token does not provide hardware-enforced non-exportability. A compromised signing host or root operator may copy its backing store and authentication material. PKCS#11 `sensitive` or `non-extractable` attributes do not remove that risk. M3 releases are therefore development releases and cannot be represented as zeroOS 1.0 production releases.

## Options

- Keep YubiHSM at M3: preserves the strongest custody boundary but blocks unrelated M3 acceptance for unavailable hardware.
- Use the existing SoftHSM/libp11 interface until M11: changes no artifact, algorithm, signer CLI, or device format and leaves one replaceable infrastructure boundary.
- Store PEM private keys directly: bypasses the accepted PKCS#11 interface and expands secret-handling paths.
- Use cloud KMS: adds a vendor, network, credential, and availability boundary without removing the later hardware migration.
- Do nothing: leaves M3 unable to progress.

## Decision and consequences

M3 may use a SoftHSM token whose backing store is on encrypted storage on the single protected x86_64 signing host. Its backing store, OpenSSL configuration, and authentication material must be readable only by the signing account, must remain outside Git, command arguments, ordinary environment values, Actions artifacts, and logs, and must be removed from connected systems after promotion. Retain two encrypted geographically separate backups, verify one restore into a disposable token, record public fingerprints and signed/timestamped ceremony evidence, and rehearse current-to-next key rotation and revocation. Hash-chained host signing logs replace HSM audit exports for M3.

M11 must freshly admit the then-current YubiHSM SDK and replace every M3 software signing key with a newly generated YubiHSM-resident key before a 1.0 release. It must exercise the overlap transition, revoke or remove the software keys, verify two encrypted hardware backups and a destructive restore, export redacted HSM audit evidence, and repeat public installation on both native architectures. M11 cannot be achieved and no release may be named or marked 1.0 while any release or Secure Boot signing private key remains software-backed.

Rollback disables the protected environment and runner, revokes the release token, removes the affected public trust through the offline PK/KEK path, and rotates every potentially exposed software key. Reopen this decision if suitable hardware becomes available before M11 or SoftHSM cannot preserve the existing PKCS#11 signing interface.
