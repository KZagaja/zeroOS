# Dependency review

Use `templates/dependency-admission.md` for every direct, transitive, build, image, firmware, protocol-engine, or test dependency. Verify the exact pin/checksum, source, license obligations, maintainer/vulnerability process, both targets, base-image presence, privilege and input surface, owner/update path, and `Retain`/`Replace` exit criteria. Reconcile `Cargo.lock`, `Dockerfile`, `policy/dependencies.csv`, `policy/sources.lock`, and `THIRD_PARTY_NOTICES.md`; then run `cargo xtask check`.

Reject convenience-only additions and small helper crates. Do not reject a mature security-critical implementation merely to replace it with unreviewed local protocol/crypto code.
