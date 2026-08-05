# Third-Party Notices

zeroOS-owned source and binaries are proprietary and covered by LICENSE and
the draft EULA.

The build uses Rust, Cargo, LLVM/Clang/LLD, musl, cargo-deny, Debian, QEMU,
EDK2/OVMF, gdisk, mtools, dosfstools, GNU build tools, bc, bison, flex,
OpenSSL, elfutils, pahole, cpio, and the official Rust container image. These
independently licensed components retain their copyrights, licenses,
redistribution rights, and notice/source obligations. zeroOS terms do not
restrict rights their licenses grant.

M1 incorporates Linux 6.18.42 (GPL-2.0-only) and the statically linked musl C
library (MIT) into the boot artifact. The complete corresponding Linux source
archive is pinned in `policy/sources.lock`; embedding `/init` in Linux's
initramfs does not relicense the separately authored zeroOS executable. Legal
review of distribution and corresponding-source delivery remains required
before release.
