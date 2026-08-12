FROM docker.io/library/rust:1.97.1-trixie@sha256:f1400ab14caacbb8a2c4a9730718a737499d930e9e59cc3d6890ae428b4edf0b

ARG DEBIAN_SNAPSHOT=20250809T000000Z
RUN printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        bc \
        binutils \
        bison \
        build-essential \
        clang-19=1:19.1.7-3+b1 \
        cpio \
        cryptsetup-bin=2:2.7.5-2 \
        e2fsprogs=1.47.2-3+b3 \
        efitools=1.9.2-3.5 \
        curl=8.14.1-2+deb13u4 \
        dosfstools \
        dwarves \
        flex \
        gdisk \
        jq=1.7.1-6+deb13u1 \
        libengine-pkcs11-openssl=0.4.13-1 \
        libelf-dev \
        libssl-dev \
        lld-19=1:19.1.7-3+b1 \
        mtools \
        musl-tools=1.2.5-3 \
        openssl=3.5.6-1~deb13u2 \
        opensc=0.26.1-2 \
        ovmf \
        python3-virt-firmware=24.11-2 \
        qemu-efi-aarch64 \
        qemu-system-arm \
        qemu-system-x86 \
        sbsigntool=0.9.4-3.2 \
        softhsm2=2.6.1-3 \
        xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install --locked cargo-deny --version 0.19.4 \
    && cargo install --locked cargo-fuzz --version 0.12.0
RUN apt-get update \
    && apt-get install -y --no-install-recommends llvm-19=1:19.1.7-3+b1 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add clippy rustfmt \
    && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-unknown-uefi aarch64-unknown-uefi

ENV ZEROOS_BUILD_IMAGE=docker.io/library/rust:1.97.1-trixie@sha256:f1400ab14caacbb8a2c4a9730718a737499d930e9e59cc3d6890ae428b4edf0b
ENV PATH=/usr/lib/llvm-19/bin:$PATH
WORKDIR /work
