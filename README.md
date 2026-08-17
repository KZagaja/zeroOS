# zeroOS

[![check](https://github.com/KZagaja/zeroOS/actions/workflows/check.yml/badge.svg)](https://github.com/KZagaja/zeroOS/actions/workflows/check.yml)

zeroOS is an experimental Linux-based operating system for `x86_64` and
`aarch64`. Linux provides the kernel, drivers, filesystems, and networking
mechanisms; zeroOS owns native userspace policy, system services, the desktop,
and first-party applications in Rust.

This is a pre-1.0 engineering project, not a general-purpose distribution or
a daily-driver operating system. The authoritative specification, milestone
status, accepted decisions, and evidence are in [ROADMAP.md](ROADMAP.md).

## Design

```text
UEFI
  └─ signed selector
      ├─ recovery image
      └─ immutable system slot A/B
          └─ Linux + embedded initramfs
              └─ zeroOS PID 1 (static musl Rust)
                  ├─ supervision, logging, shutdown, recovery
                  ├─ versioned policy-service APIs
                  └─ zeroOS Wayland compositor and desktop (planned)
```

Core constraints:

- UEFI boot on both supported architectures.
- Static musl linkage for zeroOS-owned native userspace.
- Immutable signed system images with A/B updates, rollback, and independent recovery.
- No systemd, GRUB, GNU userspace, glibc, or package manager in the base image.
- Linux owns kernel mechanisms; narrow Rust services own product policy.
- The Wayland server is implemented from the wire protocol upward without a compositor framework.
- Build inputs, source archives, toolchains, firmware, and CI actions are pinned.

## Build and test

Docker is the supported host prerequisite. Build the pinned environment:

```sh
docker build -t zeroos-build .
```

Run repository checks:

```sh
docker run --rm -v "$PWD:/work" zeroos-build cargo xtask check
docker run --rm -v "$PWD:/work" zeroos-build cargo xtask test
```

Build, boot, or test the host's native architecture:

```sh
docker run --rm -v "$PWD:/work" zeroos-build \
  cargo xtask build --arch aarch64
docker run --rm -it -v "$PWD:/work" zeroos-build \
  cargo xtask run --arch aarch64
docker run --rm -v "$PWD:/work" zeroos-build \
  cargo xtask test --arch aarch64
```

Replace `aarch64` with `x86_64` on an x86_64 host. Architecture acceptance is
native-only by design; a pass on one target is never evidence for the other.
Generated sources and images live under `target/` and are not committed.

## Repository layout

| Path | Purpose |
| --- | --- |
| `init/` | PID 1, supervision, recovery console, and core API |
| `selector/` | UEFI boot selection |
| `storage/` | Disk layout, boot state, and signed slot formats |
| `updater/` | Verified update transport and inactive-slot installation |
| `data/` | Encrypted data provisioning and recovery operations |
| `signer/` | Release-signing interface |
| `kernel/` | Explicit x86_64 and aarch64 kernel configuration |
| `xtask/` | Stable build, image, QEMU, policy, and acceptance interface |
| `policy/` | Pinned inputs and dependency admission records |
| `docs/adr/` | Accepted architecture decisions |
| `docs/ai/` | Change, review, testing, threat-model, and evidence protocols |

## Project policy

Milestones are achieved only when every acceptance criterion passes at an
exact commit on every required architecture. Partial local runs, screenshots,
or prose are not completion evidence. See [ROADMAP.md](ROADMAP.md) for the live
ledger and [AGENTS.md](AGENTS.md) for the engineering contract.

## License

zeroOS is open source under the [MIT License](LICENSE).
