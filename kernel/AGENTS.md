# Linux configuration scope

Root `AGENTS.md` applies. Use Linux userspace and both architecture skills.

Roadmap scope: M1 boot enablement, M4/M5 hardware and graphics mechanisms, M10 installation, and M11 certification. Read live status and accepted kernel requirements from `ROADMAP.md`; do not infer the current target from this file.

- Linux provides mechanisms; do not add zeroOS product policy or create a custom-kernel roadmap here.
- `x86_64.config` and `aarch64.config` are the explicit target boundaries. Review capability differences rather than mechanically mirroring settings.
- Preserve UEFI, static initramfs, console, storage/encryption, and current boot acceptance unless the roadmap changes.
- Kernel source/version/checksum and LLVM/LLD build remain pinned; config changes require both target builds or a documented target-specific reason.
