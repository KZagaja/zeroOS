# ADR: YubiHSM 2 PKCS#11 protected production signer

- Status: Accepted
- Date / owner: 2026-08-11 / Kordian Zagaja
- Roadmap milestone and acceptance item: M3 production signing, rotation/revocation, and protected-environment promotion gate
- Supersedes / conflicts: none

## Context and invariants

M3 requires RSA-3072-PSS/SHA-256 release signatures and Secure Boot signing without exporting production private keys. Authentication secrets may not enter Git, artifacts, command arguments, environment values, or logs. The operator interface must run on dedicated native x86_64 and aarch64 Linux runners without changing `ZEROSLT1`, Secure Boot trust, or device APIs. Failure must leave no published or partially activated output.

YubiHSM 2 exposes PKCS#11 2.40/3.0, non-exporting sensitive private objects, RSA-3072, RSA-PSS, and SHA-256. Its PKCS#11 mapping grants RSA signing objects `sign-pkcs` and `sign-pss` capabilities. References: [product capabilities](https://docs.yubico.com/hardware/yubihsm-2/hsm-2-user-guide/hsm2-intro-overview.html), [PKCS#11 object and login behavior](https://docs.yubico.com/hardware/yubihsm-2/hsm-2-user-guide/hsm2-tools-pkcs11.html), and [OpenSSL/libp11 integration](https://docs.yubico.com/hardware/yubihsm-2/hsm-2-user-guide/hsm2-openssl-libp11.html).

## Options

- YubiHSM 2 through its PKCS#11 module: satisfies the algorithms and non-exporting-key boundary, supports both Linux runner architectures, and reuses admitted OpenSSL/sbsigntool interfaces. The HSM, connector, PKCS#11 module, and libp11 engine remain runner infrastructure rather than build/runtime image content.
- Cloud KMS: introduces a remote vendor/API, network availability, and credential model not selected by the roadmap ceremony.
- Offline files or software keystore: exports production keys and fails the protected-interface requirement.
- Custom signing or PKCS#11 implementation: duplicates mature cryptographic/FFI boundaries.
- Do nothing: production release and M3 achievement remain impossible.

## Decision and consequences

Use YubiHSM 2 through Yubico's PKCS#11 module and the OpenSSL libp11 engine. Generate RSA-3072 production db and release keys on the HSM with only the required signing capabilities. Runner-owned OpenSSL/PKCS#11 configuration supplies connector and authentication outside repository state; key URIs contain object selectors only. `zeroos-sign` rejects PIN/password-bearing URIs, suppresses external-tool output, uses fixed commands for EFI signing and RSA-PSS/SHA-256 packaging, and atomically installs output only after successful signing and flush.

Build and hash `zeroos-sign` separately on native x86_64 and aarch64 runners. Validate its parser and output handling in workspace tests, then exercise the same PKCS#11 commands with disposable SoftHSM RSA-3072 objects before HSM provisioning. Roll back by disabling the protected environments and retaining the prior HSM objects; no device format changes. Reopen this decision only if YubiHSM 2 loses supported algorithms/maintenance, native runner support fails, or an accepted roadmap decision changes the custody boundary.
