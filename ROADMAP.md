# zeroOS 1.0 Specification and Milestone Ledger

This document is the authoritative product, architecture, and delivery specification for zeroOS 1.0. It records accepted architecture decisions and the evidence required to claim implementation progress.

## Current state

Snapshot date: 2026-08-06

- Architecture decisions in this document: **Accepted**.
- Repository implementation: **M2 resident core runtime in progress**.
- Implementation milestones: M0–M1 **Achieved**; M2 **In progress**; M3–M11 **Not started**.
- A milestone may become **Achieved** only after every acceptance item passes and its dated evidence is recorded here.

| Milestone | Status | Evidence |
| --- | --- | --- |
| M0 — Repository Foundation | Achieved | [Native x86_64 and aarch64 CI run](https://github.com/KZagaja/zeroOS/actions/runs/30998375727) |
| M1 — Dual-Architecture UEFI Bootstrap | Achieved | [Native x86_64 and aarch64 CI run](https://github.com/KZagaja/zeroOS/actions/runs/31007390950) |
| M2 — Resident Core Runtime | In progress | Local implementation and verification pending native CI evidence |
| M3 — Storage, Updates, and Recovery | Not started | — |
| M4 — Hardware and Rust Policy Services | Not started | — |
| M5 — Raw Wayland Compositor | Not started | — |
| M6 — zeroOS UI Toolkit | Not started | — |
| M7 — Desktop and Session Experience | Not started | — |
| M8 — Application Platform | Not started | — |
| M9 — Essential First-Party Applications | Not started | — |
| M10 — Installer and Consumer Reliability | Not started | — |
| M11 — Hardware Certification and 1.0 | Not started | — |

## Product vision

zeroOS 1.0 is a coherent consumer laptop and desktop operating system for `x86_64` and `aarch64`. Linux supplies the mature kernel, drivers, and hardware enablement. The userspace, policy layer, graphical shell, UI toolkit, and first-party applications owned by zeroOS are written in Rust.

The product is one integrated system rather than a general-purpose Linux distribution assembled from interchangeable desktop components. Compatibility technologies are deliberate boundaries around third-party software and mature protocol engines; they do not define the native zeroOS platform.

## Hard constraints

These constraints are release requirements, not preferences:

- Build the Linux kernel and C dependencies with LLVM, Clang, and LLD wherever the upstream project and target support them. Any exception must be documented with its cause and removal condition.
- Link zeroOS-owned native userspace statically against musl. glibc is not part of the base image.
- The base image contains no GRUB, GNU userspace, systemd, or distribution package manager.
- Boot through UEFI on both architectures.
- Ship immutable, signed system images with atomic updates, rollback, and an independently bootable recovery path.
- Implement the Wayland server and compositor in zeroOS-owned Rust code from the wire protocol upward. Smithay and forked compositors are out of scope as foundations.
- Build the zeroOS-owned UI toolkit on `wgpu`.
- Retain Flatpak and Xwayland as isolated application-compatibility boundaries.
- First-party applications use zeroOS system APIs and the zeroOS UI toolkit; compatibility runtimes are for third-party applications.

## Selected architecture

```text
UEFI firmware
  └─ signed zeroOS EFI boot artifact (BOOTX64.EFI / BOOTAA64.EFI)
      ├─ recovery image
      └─ selected immutable system slot (A or B)
          └─ Linux kernel + embedded initramfs
              └─ zeroOS PID 1 (static musl Rust)
                  ├─ supervision, logging, shutdown, recovery console
                  ├─ versioned native system-service API
                  │   ├─ update and storage policy
                  │   ├─ permissions, identity, and session policy
                  │   ├─ device and power policy
                  │   ├─ networking policy ── mature WPA engine
                  │   ├─ audio policy ─────── codec/PipeWire compatibility
                  │   └─ Bluetooth policy ── mature protocol engine
                  └─ zeroOS raw Wayland compositor
                      ├─ DRM/KMS, Mesa/vendor drivers, firmware
                      ├─ input, outputs, clipboard, and window policy
                      ├─ Xwayland ─────────── legacy X11 applications
                      └─ zeroOS UI toolkit (`wgpu`)
                          ├─ desktop shell and session experience
                          ├─ first-party applications
                          └─ portals ───────── Flatpak applications
```

The dependency direction is downward: shell and applications consume the toolkit; the toolkit consumes compositor and system APIs; policy services own access to external engines. Applications do not reach around these boundaries to kernel devices or compatibility engines.

### Component ownership and compatibility boundaries

zeroOS owns the product behavior and public policy APIs for:

- PID 1, service supervision, logging, shutdown, and recovery;
- storage layout, signing, updates, rollback, and factory reset;
- permissions, identity, sessions, devices, power, networking, audio, and Bluetooth policy;
- the Wayland wire implementation, compositor, window policy, and input/output routing;
- the UI toolkit, desktop shell, installer, and first-party applications.

For 1.0, mature WPA, media codec, PipeWire compatibility, Bluetooth protocol, Flatpak, Xwayland, Mesa, firmware, and vendor-driver engines may remain external components. Each runs beneath or behind a narrow zeroOS-owned Rust policy API where applicable. Retaining an engine does not delegate policy ownership: zeroOS controls configuration, lifecycle, permissions, observable errors, and the user experience.

### Repository and source convention

- Use one Cargo workspace for all zeroOS-owned code, including `xtask`, system services, architecture configuration, image tooling, integration tests, and later UI crates.
- Keep target-specific differences in explicit architecture configuration rather than duplicated workspaces.
- Keep generated images, build output, and third-party source trees out of Git.
- Fetch upstream source archives from declared URLs with pinned versions and checksums. Cache them outside Git and fail the build on a checksum mismatch.
- Add subsystem documents only when implementation detail can no longer be maintained clearly here.

### Planned build interface

These commands are the stable operator interface; `xtask` hides lower-level build and emulator details:

```sh
cargo xtask build --arch <x86_64|aarch64>
cargo xtask run --arch <x86_64|aarch64>
cargo xtask test [--arch <arch>]
cargo xtask check
```

`cargo xtask check` performs architecture-independent formatting, lint, policy, license, dependency, and reproducibility checks. `cargo xtask test` runs the automated acceptance suite applicable to the current milestone set; with `--arch`, it includes target-specific tests.

## Dependency admission policy

A dependency enters the base system only when its review records all of the following:

1. A concrete current requirement that is not reasonably satisfied by Rust's standard library, an existing admitted dependency, or a native kernel/platform facility.
2. Evidence of active maintenance and a process for vulnerability response.
3. A license compatible with zeroOS distribution, including source and notice obligations.
4. Build and runtime support for both `x86_64` and `aarch64`, or a documented target-specific reason and fallback.
5. Reproducible acquisition through a version and checksum pin.
6. One explicit classification:
   - **Retain**: a deliberate long-term compatibility or hardware boundary.
   - **Replace**: a temporary engine behind a zeroOS API, with replacement trigger and exit criteria.

Every admitted dependency must identify its owner, update path, attack surface, base-image presence, and classification. Transitive dependencies are reviewed as dependencies, not treated as invisible implementation details. Convenience alone is insufficient grounds for admission.

## Cross-cutting release rules

- Security boundaries, artifact formats, service APIs, and on-disk formats are versioned before consumers depend on them.
- Secure Boot key ownership, production signing infrastructure, key rotation/revocation, and release hosting must be specified and exercised before M3 can be achieved.
- Automated functional evidence is required. Human usability research supplements it but never substitutes for it.
- Accessibility, localization, deterministic behavior, and recovery are acceptance concerns throughout the project, not final-stage polish.

## Risk register

| Risk | Consequence | Required mitigation / exit evidence | First gated milestone |
| --- | --- | --- | --- |
| Raw Wayland scope | Protocol or lifecycle defects compromise stability and schedule | Freeze the required 1.0 protocol set; use protocol conformance, fuzzing, malformed-client isolation, and interoperability tests | M5 |
| ARM hardware variation | An image that boots in QEMU fails on real firmware, storage, GPU, or power implementations | Select an `aarch64` reference device, keep board configuration explicit, and run the certification suite on hardware | M4 |
| Graphics compatibility | GPU regressions, unsupported buffer paths, or unusable fallback behavior | Qualify Mesa/vendor paths, test GPU import and software fallback, and retain crash/isolation evidence | M5 |
| Suspend and resume | Data loss, battery drain, or devices failing after resume | Automated repeated suspend cycles with storage flush, network/audio/input recovery, and power assertions on both reference devices | M11 |
| Bluetooth interoperability | Pairing and audio fail across common devices | Retain a mature protocol engine behind Rust policy; test representative HID and audio profiles, reconnect, and failure recovery | M4 |
| Application compatibility | Users cannot run essential third-party software | Maintain Flatpak/native Wayland/Xwayland matrices, portal enforcement tests, and a defined supported application set | M8 |
| Accessibility | The desktop is unusable without pointer or visual interaction | Define semantics in the toolkit; automate keyboard paths and accessibility-tree assertions; test assistive integration | M6 |
| Reproducibility | Releases cannot be independently rebuilt or audited | Pin every input, isolate builds, compare artifacts from clean builders, and record hashes | M0 |
| Update recovery | Interrupted or malicious updates brick devices or lose user data | Signed A/B switching, power-loss injection, corrupted-image tests, automatic rollback, and independent recovery media | M3 |

## Milestone tracking rules

Allowed status values are exactly `Not started`, `In progress`, `Blocked`, and `Achieved`.

- **Not started** means no implementation evidence has been accepted.
- **In progress** means implementation or verification has begun but at least one acceptance item is incomplete.
- **Blocked** means progress cannot continue until a recorded dependency or decision is resolved.
- **Achieved** requires every acceptance checkbox, the listed command or equivalent linked CI run passing at the milestone commit, artifact hashes where artifacts are produced, a completion date, and an evidence link or repository path.
- If an achieved milestone regresses, change it to **In progress**. Preserve its former evidence and add the regression date and reference rather than rewriting history.
- Status edits and acceptance checkmarks must be accompanied by a change-log entry.
- Evidence must be reproducible from a commit identifier. A prose assertion, screenshot alone, or human test alone cannot achieve a milestone.

## M0 — Repository Foundation

**Status:** Achieved  
**Intent:** Establish the smallest reproducible monorepo and policy contract on which every later milestone depends.

**Deliverables**

- Git repository and one Cargo workspace for zeroOS-owned code.
- Project and third-party licensing files.
- Pinned Rust toolchain and declared LLVM/Clang/LLD and musl versions.
- Minimal `xtask` implementing the planned build interface.
- Enforced dependency-admission records and checksum-pinned upstream source manifest.
- Reproducible build-container contract for both architectures.

**Automated acceptance criteria**

- [x] A clean checkout passes formatting, linting, license, dependency-policy, and manifest validation.
- [x] The pinned toolchain and build container are sufficient to run `xtask` without undeclared host dependencies.
- [x] Two clean builds resolve identical declared inputs and produce matching outputs or a documented, tested normalization for unavoidable metadata.
- [x] CI exercises both target architectures.

**Acceptance command**

```sh
cargo xtask check
```

**Completion date:** 2026-08-05  
**Dated evidence:** Native `x86_64` and `aarch64` jobs passed for foundation commit `1fd59d0a20a7095c5e5b9d7bfd402d3ccf78f92c` in [run 30998375727](https://github.com/KZagaja/zeroOS/actions/runs/30998375727).  
**Artifact hashes:** release `xtask` SHA-256: `x86_64-unknown-linux-gnu` `fa9f7409599e9e3d0d451f7d915c71a6c93b4be41b20796610ecfd8b165d8cd9`; `aarch64-unknown-linux-gnu` `21ab809ec66929ebae5d9294eadcba7757a3b9fb8e44b6626713058885e62734`. Build base: `rust:1.97.1-trixie@sha256:f1400ab14caacbb8a2c4a9730718a737499d930e9e59cc3d6890ae428b4edf0b`.

## M1 — Dual-Architecture UEFI Bootstrap

**Status:** Achieved
**Intent:** Prove the complete boot chain on both supported architectures before expanding userspace.

**Deliverables**

- Checksum-pinned Linux 6.18 LTS built with LLVM/Clang/LLD where supported.
- Static-musl Rust PID 1 embedded in the initramfs.
- GPT disk images with FAT EFI System Partitions and fallback paths `EFI/BOOT/BOOTX64.EFI` and `EFI/BOOT/BOOTAA64.EFI`.
- QEMU launch definitions for the applicable x86_64 and aarch64 machines.

**Automated acceptance criteria**

- [x] Both images are built from a clean checkout through `xtask`.
- [x] Each QEMU guest boots via UEFI and prints exactly `zeroOS init: READY` to captured output.
- [x] PID 1 requests and completes a clean poweroff on both guests.
- [x] Image inspection verifies GPT, FAT, correct fallback EFI filename, kernel, and initramfs contents.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Completion date:** 2026-08-05

**Dated evidence:** Native `x86_64` and `aarch64` jobs passed for implementation commit `45d2adde058d264d7c4ee95f345a2a3d0812b8fd` in [run 31007390950](https://github.com/KZagaja/zeroOS/actions/runs/31007390950).

**Artifact hashes:** `x86_64`: kernel `1ed54eeb38a82fdddc73fdd144bf925c33ac1cda293c483c81d3b3c2fa0f0a44`, init `e0eae0846e69c9d50e0bc1f35cdeb5bf538599156123913c1bf6ac3a9f801ee3`, disk image `a44e1bcec00e10c5a399c4f13b4fa56984516886c2b63a2765458c17613c673e`, build image `sha256:e9cf630b4fc34f5381909d5c263183774c53a4d1c68469ee7c8e6efa5aa3b2dd`; `aarch64`: kernel `75ece3851a91468295da55c0337973caf1a0fe96343a23f4e55677f0250504dc`, init `f7d6bb548968acb420f1909f667eec83d125f8a60b19753d1542350329ddc093`, disk image `2cee338fb5cca9f9157624cb145ba0acb1a1d8a7512a6751142b543882eb6c4e`, build image `sha256:1f8feef5f10bad0124ef5e16e39d9c218e090ec2100ab74434dd5c2e3bda41ed`.

**Change log:** 2026-08-05 — implementation started; acceptance remains pending native CI evidence on both architectures.

**Change log:** 2026-08-05 — achieved after native `x86_64` and `aarch64` build, inspection, UEFI boot, readiness, and clean-poweroff acceptance passed in run 31007390950.

## M2 — Resident Core Runtime

**Status:** In progress
**Intent:** Turn the bootstrap PID 1 into a dependable, testable resident runtime.

**Deliverables**

- PID 1 process reaping, signal handling, dependency-ordered service supervision, restart policy, and failure isolation.
- Structured logging, deterministic shutdown, and recovery console.
- A versioned native system-service API boundary with compatibility rules.

**Automated acceptance criteria**

- [ ] Orphan and zombie process tests prove complete reaping.
- [ ] Signal, service ordering, restart-limit, and dependency-failure tests pass.
- [ ] One crashing service cannot terminate PID 1 or unrelated services.
- [ ] Logs survive service failure and are available in the recovery console.
- [ ] Shutdown flushes state, stops services in dependency order, and powers off.
- [ ] API version negotiation rejects incompatible clients without destabilizing the runtime.

**Acceptance command**

```sh
cargo xtask test
```

**Dated evidence:** —  
**Artifact hashes:** —

### M2 core API and recovery contract

PID 1 listens on the root-only Unix socket `/run/zeroos/core-v1.sock` (mode `0600`). A connection carries one newline-terminated request of at most 4096 bytes and then closes. Requests are `ZEROOS/1 STATUS`, `LOGS`, `START <service>`, `STOP <service>`, `RESTART <service>`, or `SHUTDOWN`; PID 1 fixture roles additionally use `ZEROOS/1 FIXTURE READY <service> <pid>` and `ZEROOS/1 FIXTURE LOG <service> <pid> <level> <event> <message>`. Responses begin with `OK ZEROOS/1` or `ERR ZEROOS/1 <code>`. Unknown major versions are rejected before dispatch and cannot change runtime state. New v1 commands and trailing `key=value` response fields are compatible additions; removing a command or changing its meaning requires a new major version and socket path.

The serial `zeroos recovery>` console is a privileged administrative boundary equivalent to root socket access. It accepts only `help`, `status`, `logs`, `start`, `stop`, `restart`, `api-version`, `selftest`, and `shutdown`; it is not a POSIX shell and provides no arbitrary execution, pipes, or redirection. Per-service authorization is deferred to M4.

Services start after their declared dependencies and declaration order breaks ties. Consumers stop before dependencies. Three restart attempts are allowed in a rolling ten-second window; a fourth failure is permanent, stops affected dependents, and leaves unrelated services running. Ten healthy seconds or an administrative restart clears that service's budget.

Shutdown rejects new mutations, sends `SIGTERM` in reverse dependency order, applies one global two-second grace period, sends `SIGKILL` to survivors, reaps all children, calls `sync()`, records completion, and powers off.

**Change log:** 2026-08-06 — M2 implementation started; acceptance remains pending native CI evidence on both architectures.

## M3 — Storage, Updates, and Recovery

**Status:** Not started  
**Intent:** Make system changes transactional and recoverable before user data depends on the platform.

**Deliverables**

- Immutable A/B system partitions and a separate writable user-data partition.
- Signed system artifacts, verified boot selection, atomic slot switching, health confirmation, and automatic rollback.
- Independently bootable recovery environment and factory-reset flow.
- Specified Secure Boot keys, production signing, key rotation/revocation, and release hosting.

**Automated acceptance criteria**

- [ ] Valid signed updates install only to the inactive slot and switch atomically.
- [ ] Invalid signatures and corrupted artifacts are rejected before activation.
- [ ] Power-loss injection at every update phase preserves the previous bootable system.
- [ ] Failed health confirmation automatically returns to the previous slot.
- [ ] Recovery boots when both normal boot attempts fail and can repair or reset without silently deleting user data.
- [ ] Factory reset requires explicit confirmation and produces the specified clean state.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M4 — Hardware and Rust Policy Services

**Status:** Not started  
**Intent:** Own system policy in Rust while retaining mature interoperability engines behind controlled APIs.

**Deliverables**

- Rust services for device discovery, networking, audio, Bluetooth, time, power, identity, and session policy.
- Isolated adapters for retained WPA, codec/PipeWire compatibility, and Bluetooth protocol engines.
- One named `x86_64` and one named `aarch64` physical reference device selected before certification work begins.

**Automated acceptance criteria**

- [ ] Each service exposes a versioned zeroOS API and denies unauthorized operations.
- [ ] Engine processes can crash and restart without bypassing policy or taking down unrelated services.
- [ ] Network connect/reconnect, audio playback/capture, Bluetooth pair/reconnect, time sync, session isolation, and power-event tests pass.
- [ ] Device add/remove events are handled without stale permissions or leaked resources.
- [ ] Reference-device records include exact model, firmware, peripherals, and supported configuration.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Reference devices:** x86_64 —; aarch64 —  
**Artifact hashes:** —

## M5 — Raw Wayland Compositor

**Status:** Not started  
**Intent:** Establish the native display boundary without inheriting another compositor framework or policy model.

**Deliverables**

- zeroOS-owned Wayland wire encoding/decoding, registry, object lifecycle, error handling, and required protocol implementations.
- Shared-memory buffers, `xdg-shell`, seats and input routing, DRM/KMS output management, GPU buffer import, clipboard, and drag-and-drop.
- Xwayland lifecycle and surface integration.
- Multi-window, multi-display, and mixed-DPI behavior.

**Automated acceptance criteria**

- [ ] Wire and object-lifecycle conformance tests pass for the frozen 1.0 protocol set.
- [ ] Fuzzed and malformed clients are disconnected without compositor or peer-client failure.
- [ ] Shared-memory and GPU-imported clients render and release buffers correctly.
- [ ] Keyboard, pointer, focus, clipboard, and drag-and-drop isolation tests pass.
- [ ] Multi-window, hot-plugged multi-display, mixed-DPI, and Xwayland scenarios match deterministic expected results.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M6 — zeroOS UI Toolkit

**Status:** Not started  
**Intent:** Supply one accessible, deterministic native UI foundation for the shell and first-party applications.

**Deliverables**

- `wgpu` renderer and scene composition.
- Layout, controls, styling, text shaping/rendering, animation, focus, and keyboard navigation.
- Accessibility semantics, localization, and display scaling.
- Deterministic screenshot-test harness.

**Automated acceptance criteria**

- [ ] Reference scenes match approved screenshots within a recorded deterministic tolerance on both architectures.
- [ ] Layout, clipping, text, animation timing, localization, and 1x/fractional scaling tests pass.
- [ ] Every interactive control is reachable and operable by keyboard with visible focus.
- [ ] Accessibility-tree snapshots contain correct roles, names, states, relationships, and update events.
- [ ] A sample first-party application uses no GUI toolkit outside the zeroOS workspace.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M7 — Desktop and Session Experience

**Status:** Not started  
**Intent:** Deliver a complete daily session that behaves as one product.

**Deliverables**

- Login, onboarding, lock screen, launcher, application switcher, and window management.
- Notifications, workspaces, shortcuts, status controls, permission prompts, and Settings surfaces.

**Automated acceptance criteria**

- [ ] Login, first-run onboarding, lock/unlock, logout, and multi-user session-isolation journeys pass.
- [ ] Launching, switching, moving, resizing, minimizing, closing, and workspace navigation pass for native and compatible applications.
- [ ] Notification and permission flows preserve origin, focus, denial, and revocation semantics.
- [ ] Every supported journey is completable by keyboard alone.
- [ ] Every shell surface has complete, asserted accessibility-tree coverage.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M8 — Application Platform

**Status:** Not started  
**Intent:** Run useful third-party software without surrendering system policy.

**Deliverables**

- Sandboxed application install, update, launch, termination, and removal lifecycle.
- Permission model and portals for files, notifications, clipboard, and other mediated resources.
- File associations and application identity.
- Flatpak installation plus native Wayland and Xwayland compatibility paths.

**Automated acceptance criteria**

- [ ] Applications cannot access protected files, devices, sockets, clipboard data, or services without declared permission or portal approval.
- [ ] Permission denial, revocation, and application removal leave no continuing access.
- [ ] Portal identity and file-association routing resist application spoofing.
- [ ] The supported Flatpak, native Wayland, and Xwayland application matrix installs, launches, updates, and uninstalls successfully.
- [ ] A hostile test application cannot bypass declared system permissions through any compatibility boundary.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M9 — Essential First-Party Applications

**Status:** Not started  
**Intent:** Cover essential setup, maintenance, and local work without making a browser implementation a 1.0 prerequisite.

**Deliverables**

- Onboarding, Settings, Files, Terminal, Store/Updates, and text editor applications built exclusively with the zeroOS toolkit.
- An existing browser distributed through the sandboxed compatibility platform.

**Automated acceptance criteria**

- [ ] Each first-party application passes its core keyboard-driven functional journeys and accessibility-tree assertions.
- [ ] Files and editor data-loss recovery tests pass for crashes, low space, and interrupted writes.
- [ ] Terminal process, clipboard, and permission isolation tests pass.
- [ ] Store/Updates verifies origin and signatures and accurately reports install, update, rollback, and failure states.
- [ ] The selected browser installs, launches, updates, renders the compatibility test set, and remains sandbox-confined.
- [ ] Dependency inspection confirms first-party applications use no external GUI toolkit.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M10 — Installer and Consumer Reliability

**Status:** Not started  
**Intent:** Make installation and failure recovery safe enough for a consumer device.

**Deliverables**

- Bootable installation media and recovery media for both architectures.
- Safe partitioning, disk encryption, user creation, and first boot.
- Offline recovery, update rollback, crash handling, and data-preserving reinstall.

**Automated acceptance criteria**

- [ ] Clean-disk, supported dual-use layout, encrypted install, and reinstall scenarios pass in automated disposable disks.
- [ ] Every destructive operation identifies the exact target, explains the consequence, and requires explicit confirmation.
- [ ] Cancellation and power-loss injection at every installation phase leave either the prior system or recovery media bootable.
- [ ] Encryption keys are not logged or retained in installation artifacts.
- [ ] Offline recovery repairs boot state, rolls back updates, and performs a data-preserving reinstall.
- [ ] Deliberately crashing system and application processes produce bounded recovery without reboot loops or user-data corruption.

**Acceptance commands**

```sh
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Artifact hashes:** —

## M11 — Hardware Certification and 1.0

**Status:** Not started  
**Intent:** Convert the integrated system into a reproducible, signed release qualified on owned hardware.

**Deliverables**

- Automated certification rigs for the selected `x86_64` and `aarch64` reference devices.
- Signed, reproducible release, installer, and recovery artifacts for both architectures.
- Published supported-hardware, application-compatibility, accessibility, update, and recovery evidence.

**Automated acceptance criteria**

- [ ] Both reference devices pass boot, install, update, rollback, recovery, and factory-reset tests.
- [ ] Wi-Fi, Bluetooth audio, speakers, microphone, suspend/resume, lid behavior, battery reporting, external displays, keyboard, pointer, touch where present, and GPU stability tests pass.
- [ ] Repeated suspend/resume, update interruption, storage-pressure, service-crash, and long-running graphics tests meet recorded reliability thresholds.
- [ ] Keyboard-only operation and accessibility-tree coverage pass across every required shell and first-party application journey.
- [ ] The declared Flatpak, native Wayland, and Xwayland compatibility matrix passes.
- [ ] Independent clean builders produce byte-identical release artifacts, or only explicitly normalized and justified differences.
- [ ] Every published artifact has a verified signature and recorded cryptographic hash.
- [ ] No unresolved release-blocking defect, security exception, or unmet earlier milestone acceptance item remains.

**Acceptance commands**

```sh
cargo xtask check
cargo xtask test --arch x86_64
cargo xtask test --arch aarch64
```

**Dated evidence:** —  
**Release artifact hashes:** —

Human usability research should validate comprehension, comfort, and workflow quality before release. It supplements, and cannot replace, the automated functional acceptance above.

## Change log

Append entries; do not rewrite or remove historical decisions or milestone evidence.

| Date | Change | Rationale / evidence |
| --- | --- | --- |
| 2026-08-05 | Created the zeroOS 1.0 specification; accepted the architecture and initialized M0–M11 as `Not started`. | Initial authoritative roadmap; repository implementation is empty. |
| 2026-08-05 | Began M0 repository foundation implementation. | Workspace, policy, reproducible build contract, and native CI implementation started; acceptance remains pending. |
| 2026-08-05 | Passed local M0 acceptance on native arm64. | Pinned-container `cargo xtask check` and both architecture-context test commands passed; M0 remains `In progress` until both native GitHub jobs pass. |
| 2026-08-05 | Marked M0 Repository Foundation `Achieved`. | Both native jobs passed foundation commit `1fd59d0a20a7095c5e5b9d7bfd402d3ccf78f92c` in [run 30998375727](https://github.com/KZagaja/zeroOS/actions/runs/30998375727); recorded both release hashes and the build-image digest. |
