# Dependency governance

## Roadmap scope
Cross-cutting across M0–M11 and later maintenance. Resolve the dependency's owning subsystem and live milestone constraints from `ROADMAP.md`.

## Purpose and application
Use before adding/updating/removing any Cargo, build-image, kernel, firmware, engine, action, test, or transitive dependency. Reference reproducible builds, security, and `docs/ai/dependency-review.md`.

## Inspect first
Inspect `Cargo.toml`/`Cargo.lock`, Dockerfile, workflows, policy ledgers/locks, notices/licenses, existing code/stdlib/native alternatives, both target support, and applicable roadmap boundary.

## Decisions and invariants
Every dependency has a concrete current need, owner, maintenance/vulnerability process, license, exact reproducible pin/checksum, transitive review, target evidence, attack/base-image surface, update path, and `Retain`/`Replace` exit criteria.

## Forbidden
No convenience-only helper crate, wildcard/floating pin, invisible transitive, missing record/checksum/license, unsupported target without fallback, direct compatibility-engine policy, or local reimplementation of mature security protocol solely to avoid review.

## Workflow
1. Test stdlib, existing dependency, and Linux/UEFI native alternatives.
2. Fill dependency template and threat/target review.
3. Pin acquisition, update ledger/locks/notices, isolate behind narrow API if an engine.
4. Run policy/license/advisory and both-target checks.
5. Record owner/update/exit path.

## Review checklist
Check source integrity, yanked/advisory response, maintainer activity, license obligations, features/defaults, build scripts/proc macros/native code, transitives, privileges/network/files, base image, and replacement trigger.

## Tests, architecture, security, evidence
Run `cargo xtask check`, `cargo deny check`, locked clean resolution, and target builds. Record exact versions/checksums/licenses/advisories and adapter isolation tests.

## ADR / stop conditions
ADR for retained engine, security-critical protocol/crypto, incompatible license, target exception, or dependency direction. Stop for missing owner/pin/checksum/license/vulnerability path/target evidence.
