# Reproducible builds

## Roadmap scope
M0 establishes reproducibility; M1/M3 add boot and update artifacts; every later milestone preserves it; M11 certifies releases. Resolve live acceptance from `ROADMAP.md`.

## Purpose and application
Use for `xtask`, Dockerfile, toolchain, source locks, kernel/image packaging, CI/release, timestamps, downloads, or artifact hashes. Reference dependency governance and both platform skills.

## Inspect first
Inspect `rust-toolchain.toml`, `Dockerfile`, `Cargo.lock`, `deny.toml`, `policy/{build-image.lock,sources.lock,dependencies.csv}`, `xtask`, workflows, kernel configs, and evidence for the live target milestone.

## Decisions and invariants
Exact Rust/build-image/source pins, checksums, LLVM/Clang/LLD where supported, static musl, deterministic GPT/ESP inputs, generated artifacts/third-party sources outside Git, and clean builders.

## Forbidden
No floating image/action/source/tool download, missing checksum, undeclared host dependency, committed target/image/vendor output, ambient timestamp/user/host metadata, network fetch bypassing source lock, or false byte-identical claim.

## Workflow
1. Inventory every input and acquisition path.
2. Pin version/source/digest/checksum and admit dependency.
3. Normalize declared nondeterminism and isolate caches outside Git.
4. Build twice clean and compare exact outputs or tested normalization.
5. Run both native architecture CI and record hashes.

## Review checklist
Check lockfiles, Docker digest/packages, action SHAs, source URLs/hashes, tool versions, env/time/locale, archive order/metadata, filesystem IDs, signing nondeterminism, and notices/licenses.

## Tests, architecture, security, evidence
Run `cargo xtask check`, clean double build, both architecture acceptance, image inspection, and hash comparison. Signing evidence distinguishes reproducible unsigned payload from randomized signature where applicable.

## ADR / stop conditions
ADR for unavoidable nondeterminism, new acquisition channel, build-image/toolchain change, or signing normalization. Stop on unpinned input, checksum gap, undeclared tool, or unexplained artifact difference.
