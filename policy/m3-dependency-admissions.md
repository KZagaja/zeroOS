# M3 dependency admissions

All packages below are acquired only from the dated Debian snapshot configured in `Dockerfile`.
APT verifies the snapshot metadata signature and the package checksum recorded by that metadata;
the exact Debian versions are pinned in both `Dockerfile` and `policy/dependencies.csv`. Updates
require changing the snapshot or exact version, rebuilding the image, and rerunning both native
acceptance jobs. Kordian Zagaja owns review of Debian security notices and upstream advisories.

## cryptsetup-bin 2:2.7.5-2 — Retain

- Requirement: M3 standard LUKS2 formatting, keyslots, and dm-crypt activation.
- Alternatives: custom cryptography is prohibited; no existing dependency supplies LUKS2.
- License/distribution: GPL-2.0-or-later; package notices and runtime libraries are shipped from Debian.
- Targets/transitives: Debian x86_64 and aarch64 package closures; included in the initramfs by `xtask`.
- Boundary: root-only fixed arguments, secret key material through CLOEXEC memfd descriptors, redacted errors.

## e2fsprogs 1.47.2-3+b3 — Retain

- Requirement: M3 ext4 creation and explicit recovery repair.
- Alternatives: the kernel mounts ext4 but does not provide `mkfs.ext4` or `e2fsck`.
- License/distribution: GPL-2.0-only; package notices and runtime libraries are shipped from Debian.
- Targets/transitives: Debian x86_64 and aarch64 package closures; fixed commands and device arguments only.

## ureq 3.3.0 with rustls and static webpki roots — Retain

- Requirement: bounded HTTPS download, manual redirect validation, strong-ETag Range resume, and certificate-time validation without runtime shell tools.
- Alternatives: Rust std has no TLS client; curl delegated redirect/resume policy to a CLI and shipped an unnecessary runtime closure; custom HTTP/TLS is prohibited.
- Maintenance/security: exact workspace and lockfile pin; review RustSec, ureq, rustls, and Mozilla trust-store changes before updates.
- License/distribution: ureq and most utility transitives are MIT/Apache-2.0; webpki-roots is CDLA-Permissive-2.0. Exact transitive versions and notices are in `Cargo.lock`, `policy/dependencies.csv`, and `THIRD_PARTY_NOTICES.md`.
- Targets/transitives: static-musl x86_64 and aarch64; rustls/ring, webpki, ureq-proto, HTTP parsing, and utility closure reviewed in the machine ledger.
- Boundary: HTTPS-only, proxy disabled, five manually validated redirects, fixed repository/CDN origins, bounded headers/body/time, and no credentials.

## ring 0.17.14 — Retain

- Requirement: direct RSA-3072-PSS/SHA-256 manifest verification and the admitted rustls crypto provider.
- Alternatives: OpenSSL CLI required temporary files and subprocess policy; custom cryptography is prohibited.
- Maintenance/security: exact workspace and lockfile pin; review RustSec and ring security releases before updates.
- License/distribution: Apache-2.0 AND ISC; its `cc`, entropy, libc, and parsing closure is recorded in the machine ledger.
- Targets/boundary: static-musl x86_64 and aarch64; verifies exact manifest bytes against one of at most two installed DER PKCS#1 release keys.

## libc 0.2.184 and zeroize 1.8.2 — Retain

- Requirement: termios, mlock, memfd/fcntl RAII, and deterministic credential wiping.
- Alternatives: std does not expose the required Linux APIs; the previous hand-written FFI and `stty` subprocess widened the boundary; ordinary fills alone do not guarantee zeroization.
- Maintenance/security: exact workspace and lockfile pins; review RustSec and upstream releases before updates.
- Licenses/targets: MIT/Apache-2.0; static-musl x86_64 and aarch64. Unsafe calls remain private and documented; secrets use locked buffers and CLOEXEC descriptors.

## sbsigntool 0.9.4-3.2 — Retain

- Requirement: CI PE/COFF signing and signature inspection; never present in the runtime image.
- Alternatives: no existing admitted Secure Boot signing tool.
- License/distribution: GPL-3.0-only; build-image use only.
- Targets/transitives: signs both target artifacts; private keys remain behind the protected signer interface.

## efitools 1.9.2-3.5 — Retain

- Requirement: disposable QEMU key enrollment and authenticated-variable rotation/revocation tests.
- Alternatives: no existing admitted authenticated-variable tooling.
- License/distribution: GPL-2.0-only; build-image use only.
- Targets/transitives: used for both firmware targets with non-production keys only.

## python3-virt-firmware 24.11-2 — Retain

- Requirement: deterministic offline editing and inspection of disposable EDK2 variable-store copies for Secure Boot and rotation tests.
- Alternatives: efitools creates authenticated updates but cannot apply them to an offline variable store; interactive firmware enrollment is not deterministic.
- License/distribution: GPL-2.0-or-later; Debian supplies the Python cryptography, importlib-resources, and pefile closure.
- Targets/transitives: host tooling on both native runners; never included in the runtime or release artifacts.
- Boundary: reads and writes only per-scenario temporary variable-store copies containing disposable public test material.

## cargo-fuzz 0.12.0 and libfuzzer-sys 0.4.9 — Retain

- Requirement: fixed-seed bounded smoke fuzzing of the hostile M3 journal, container-header, and manifest parsers.
- Alternatives: unit cases cannot explore arbitrary parser byte streams; a second custom mutator would duplicate libFuzzer.
- Maintenance/security: exact Cargo install version and independent fuzz lockfile; review Rust fuzz project releases before updates.
- License/distribution: cargo-fuzz and libfuzzer-sys are MIT/Apache-2.0; the locked helper closure is build/test-only.
- Targets/boundary: host CI for both native runners; no fuzzing code or dependency enters a zeroOS runtime artifact.

## jq 1.7.1-6+deb13u1 — Retain

- Requirement: parse untrusted production provenance with exact JSON types and values during protected release and public installation verification.
- Alternatives: substring matching is ambiguous; a custom JSON parser would duplicate a mature security boundary; adding a runtime Rust JSON dependency would expand the production closure unnecessarily.
- Maintenance/security: exact Debian snapshot version; review Debian and upstream jq advisories before updates.
- License/distribution: MIT; pinned build/release image use only.
- Targets/boundary: native x86_64 and aarch64 release runners; parses only bounded public provenance and never enters runtime artifacts.

## YubiHSM PKCS#11 2.8.0 and libp11 0.4.13-1 — Retain

- Requirement: non-exporting production RSA-3072 Secure Boot and RSA-PSS/SHA-256 release signing through the accepted PKCS#11 boundary.
- Alternatives: software keys violate M3 custody; cloud KMS changes the accepted ceremony; custom PKCS#11/crypto is prohibited.
- Maintenance/security: Yubico's signed 2.8.0 source is SHA-256 pinned in `policy/sources.lock`; libp11 is pinned through the Debian snapshot. Review Yubico, OpenSSL, libp11, and Debian advisories before updates.
- License/distribution: Yubico components are Apache-2.0 and libp11 is LGPL-2.1-or-later; protected-runner infrastructure only.
- Targets/boundary: isolated x86_64 Linux signing host. PKCS#11 URIs select objects only; connector/authentication configuration stays root-owned outside Git, arguments, environment values, artifacts, and logs. Native x86_64 and aarch64 hosted builders never load this production module.

## SoftHSM 2.6.1-3 and OpenSC 0.26.1-2 — Retain

- Requirement: exercise the protected signer's real PKCS#11 RSA-3072/PSS command path with disposable test objects before production hardware.
- Alternatives: mocks do not cross the PKCS#11 engine boundary; production HSM tests would risk protected state.
- Maintenance/security: exact Debian snapshot versions; review Debian, SoftHSM, OpenSC, Botan, and libp11 advisories before updates.
- License/distribution: SoftHSM is BSD-2-Clause; OpenSC is LGPL-2.1-or-later; build-image use only.
- Targets/boundary: native x86_64 and aarch64 test hosts; fixed disposable credentials and objects are process-local, deleted after the bounded smoke test, and never enter runtime or release artifacts.
