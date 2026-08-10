# AI engineering system

`AGENTS.md` is the canonical operating contract; `ROADMAP.md` is the authoritative product, architecture, and milestone ledger. This directory supplies review procedures and templates. `.ai/skills/` supplies task-specific workflows and links back here instead of copying policy. Scoped `AGENTS.md` files only strengthen the root contract.

Skills and scoped instructions declare which milestones own their subsystem, but never cache milestone status or the currently targeted milestone. Always read those live from `ROADMAP.md`; achieved milestone rules continue to govern maintenance and regression work.

Current repository map:

| Path | Ownership / trust boundary |
| --- | --- |
| `init/` | PID 1, supervision, root IPC, signals, child processes, shutdown |
| `storage/` | GPT constants, boot journal, untrusted signed-container manifest parsing, interruption-safe copies |
| `updater/` | HTTPS engine adapter, RSA-PSS verification, inactive-slot writes |
| `data/` | privileged LUKS/ext4 engine adapter and secret memory/FD handling |
| `selector/` | architecture-specific UEFI firmware ABI, block I/O, A/B/recovery selection |
| `xtask/` | build, pinned sources, images, QEMU acceptance, repository policy |
| `kernel/` | explicit x86_64/aarch64 Linux configuration |

No networking service, compositor, UI toolkit, application platform, or integration-test tree exists yet. Their skills are readiness contracts, not evidence that those milestones started.

Start with `change-protocol.md`; finish with `review-protocol.md` and `testing-and-evidence.md`.
