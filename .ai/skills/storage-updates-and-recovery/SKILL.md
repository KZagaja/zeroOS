# Storage, updates, and recovery

## Roadmap scope
M3 owns storage/update/recovery foundations; M10 owns installation and consumer recovery; M11 owns certification. Read live status, formats, gates, and acceptance criteria from `ROADMAP.md`.

## Purpose and application
Use for `storage/`, `updater/`, `data/`, selector state, image layout, update transport/signing, LUKS/ext4, recovery, reset, or M3/M10/M11 evidence. Reference UEFI, security, dependency, concurrency, testing, and milestone skills.

## Inspect first
Read the complete M3 contract/gates; inspect scoped instructions and all affected storage/updater/data/selector/init/xtask paths, release workflow, dependency/source locks, and tests.

## Decisions and invariants
Exact deterministic partition layout, `ZEROOSB1` journal, `ZEROSLT1` manifest/signature format, RSA-3072-PSS/SHA-256 accepted boundary, inactive-slot-only writes, monotonic sequence, verified-before-activation, flush/rollback, independent recovery, and data-preserving reset semantics are fixed.

## Forbidden
No “atomic because rename”, active-slot write, pre-verification activation, downgrade, custom crypto, secret in Git/args/env/logs, invented verity/filesystem/TPM/boot-count scheme, silent data deletion, or premature milestone achievement/tag.

## Workflow
1. Enumerate each state transition and crash result.
2. Bound/parse manifest, sizes, offsets, sequences, redirects, and architecture before effects.
3. Verify exact manifest signature and payload hash, then write inactive slot.
4. Flush artifact/block/journal boundaries, stage trial, confirm only after accepted health, rollback otherwise.
5. Test every interruption and recovery/reset path.

## Review checklist
Check signed metadata completeness, signer/rotation/revocation, range resume prefix, redirects, short writes/rereads, block/fs sync, journal torn writes/generation, attempts, recovery independence, credential wiping, exact target confirmation, and user-data preservation.

## Tests, architecture, security, evidence
Inject before/after download, verification, slot write, metadata sync, selection, health, and rollback; corrupt signature/payload/state; run both architecture commands. Apply every human or independent-review gate currently recorded in the target roadmap section.

## ADR / stop conditions
ADR for any format/crypto/filesystem/TPM/boot-count/recovery-hosting change. Stop for absent production trust/rotation decision, missing crash outcome, inability to fail closed, destructive ambiguity, failed target check, or incomplete human gate.
