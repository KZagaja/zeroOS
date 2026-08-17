# Third-Party Notices

zeroOS-owned source and binaries are licensed under the MIT License. See
[LICENSE](LICENSE).

The build uses Rust, Cargo, LLVM/Clang/LLD, musl, cargo-deny, Debian, QEMU,
EDK2/OVMF, gdisk, mtools, dosfstools, GNU build tools, bc, bison, flex,
OpenSSL, elfutils, pahole, cpio, and the official Rust container image. These
independently licensed components retain their copyrights, licenses,
redistribution rights, and notice/source obligations. zeroOS terms do not
restrict rights their licenses grant.

M3 additionally statically links ureq/rustls, ring, libc, zeroize, and their
locked transitive closure. The closure is predominantly MIT, Apache-2.0, ISC,
BSD-3-Clause, and CDLA-Permissive-2.0 licensed. M3 packages cryptsetup
(GPL-2.0-or-later) and e2fsprogs (GPL-2.0-only), but no curl, OpenSSL CLI,
`sha256sum`, `stty`, or system CA bundle in the runtime image. Its build image
retains those build/test prerequisites plus sbsigntool (GPL-3.0-only),
efitools (GPL-2.0-only), python3-virt-firmware (GPL-2.0-or-later), jq (MIT),
SoftHSM 2 (BSD-2-Clause), OpenSC (LGPL-2.1-or-later), libp11
(LGPL-2.1-or-later), and cargo-fuzz/libfuzzer-sys (MIT/Apache-2.0). The
protected runners pin Yubico's YubiHSM PKCS#11 module (Apache-2.0). Exact versions and acquisition
details are recorded in `Cargo.lock`, `policy/dependencies.csv`, and
`policy/m3-dependency-admissions.md`.

M1 incorporates Linux 6.18.42 (GPL-2.0-only) and the statically linked musl C
library (MIT) into the boot artifact. The complete corresponding Linux source
archive is pinned in `policy/sources.lock`; embedding `/init` in Linux's
initramfs does not relicense the separately authored zeroOS executable. Legal
review of distribution and corresponding-source delivery remains required
before release.
