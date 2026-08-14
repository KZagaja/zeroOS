# ADR: M3 single signing host with native hosted builders

- Status: Accepted
- Date / owner: 2026-08-14 / Kordian Zagaja
- Roadmap milestone and acceptance item: M3 protected production signing and both-architecture release acceptance
- Supersedes / conflicts: supersedes ADR 0001's requirement that the operator interface run on both native runner architectures and ADR 0002 only where it requires separate self-hosted x86_64 and aarch64 signing runners; preserves their non-exporting signer boundary and every other gate

## Context and invariants

Production RSA and Secure Boot signing is architecture-independent, while building and boot-testing zeroOS is not. Requiring an HSM-connected self-hosted machine for each target duplicates the signing boundary without improving native execution evidence. The available infrastructure supports one x86_64 Linux signing host; GitHub supplies native Ubuntu 24.04 x86_64 and aarch64 runners already used by exact-SHA CI.

The signing host, GitHub artifact transport, unsigned EFI inputs, recovery assets, source SHA, sequence, and public trust are untrusted until verified. Production private keys remain non-exportable and authentication remains outside Git, arguments, ordinary environment values, artifacts, and logs. `ZEROOSB1`, `ZEROSLT1`, GPT, `ZEROOS/1`, algorithms, native acceptance, public installation, offline custody, backup/restore, revocation, and achievement gates do not change.

## Options

- Two HSM-connected native signing runners: duplicates target-independent signing and requires unavailable dedicated aarch64 hardware.
- One protected signer plus native hosted builders/testers: keeps one hardware-key boundary while retaining native build and boot evidence for both targets.
- Cross-build and emulate everything on one host: loses the required native architecture evidence.
- Software or cloud signing: changes the accepted non-exporting custody boundary.
- Do nothing: leaves production promotion impossible with the available infrastructure.

## Decision and consequences

Use GitHub-hosted Ubuntu 24.04 x86_64 and aarch64 jobs to rebuild the exact source SHA, run `cargo xtask test --arch <arch>`, and upload only unsigned selector/system EFI artifacts plus SHA-256 manifests. Record the Actions artifact digests. A single isolated self-hosted x86_64 Linux runner, labelled `zeroos-production`, verifies both manifests, the pinned x86_64 `zeroos-sign` version/hash, both offline recovery hashes/signatures, source SHA, sequence, and committed public trust before signing and packaging both architectures through the YubiHSM.

After signing, GitHub-hosted native x86_64 and aarch64 jobs perform unauthenticated public `cargo xtask test-release` installation and Secure Boot tests. Only both passing jobs permit copying the immutable candidate bytes to the final release. The signing host performs no build or native acceptance policy; the hosted builders never receive HSM authentication or production private-key access.

The assurance tradeoff is explicit: GitHub artifact transport now crosses into the signer boundary, and one signing-host compromise can authorize both target artifacts. Exact workflow-run artifact provenance, SHA-256 verification, minimal runner scope, non-exporting keys, HSM audit, and public native verification detect substitution but do not protect against a malicious authorized owner or a fully compromised signing host. Roll back by disabling the environment and single runner, revoking its token, and revoking or replacing affected keys. Reopen if GitHub-hosted native runners cease to provide required evidence or signing becomes target-dependent.
