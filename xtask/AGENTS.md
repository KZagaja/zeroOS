# Build and policy tooling scope

Root `AGENTS.md` applies. Use reproducible-build and dependency-governance skills.

Roadmap scope: the stable operator and acceptance interface spans M0–M11. Resolve the task's live target milestone and commands from `ROADMAP.md`; do not cache them here.

- Preserve the stable `cargo xtask build|run|test|check` interface and existing milestone acceptance. `check` must remain fail closed.
- Treat manifests, policy files, command output, paths, archives, image metadata, and QEMU output as untrusted. Emit file, line, rule, and remediation for source-policy violations.
- Every source/tool/image/action is pinned and checksummed as applicable; generated images/build/source trees remain outside Git.
- Keep target orchestration here, target implementation in `selector/` or `kernel/`. Test policy rules with compliant and injected invalid temporary trees.
