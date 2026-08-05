FROM docker.io/library/rust:1.97.1-trixie@sha256:f1400ab14caacbb8a2c4a9730718a737499d930e9e59cc3d6890ae428b4edf0b

ARG DEBIAN_SNAPSHOT=20250809T000000Z
RUN printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s trixie main\n' "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        clang-19=1:19.1.7-3+b1 \
        lld-19=1:19.1.7-3+b1 \
        musl-tools=1.2.5-3 \
    && rm -rf /var/lib/apt/lists/* \
    && cargo install --locked cargo-deny --version 0.19.4
RUN apt-get update \
    && apt-get install -y --no-install-recommends llvm-19=1:19.1.7-3+b1 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add clippy rustfmt

ENV ZEROOS_BUILD_IMAGE=docker.io/library/rust:1.97.1-trixie@sha256:f1400ab14caacbb8a2c4a9730718a737499d930e9e59cc3d6890ae428b4edf0b
WORKDIR /work
