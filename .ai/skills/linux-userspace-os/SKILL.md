# Linux userspace OS

## Roadmap scope
M1–M11 and later maintenance. Read the currently applicable Linux/userspace ownership decisions and milestone status from `ROADMAP.md`.

## Purpose and application
Use for Linux mechanisms, system services, devices, filesystems, networking primitives, DRM/KMS, input, process control, and external engine adapters.

## Inspect first
Read `ROADMAP.md` selected architecture/hard constraints, root/scoped instructions, kernel configs, `init/`, and affected adapter/service APIs.

## Decisions and invariants
Linux owns mature kernel mechanisms; zeroOS owns product behavior and narrow versioned Rust policy APIs. Native userspace is static musl; base image excludes GRUB, GNU userspace, systemd, distribution package manager, and glibc.

## Forbidden
No custom-kernel detour, policy delegated to CLI output or compatibility engine, ordinary shell orchestration, public engine API leakage, or duplicate kernel facility.

## Workflow
1. Classify mechanism versus product policy.
2. Identify syscall/kernel API and privilege boundary.
3. Define a narrow typed zeroOS API, lifecycle, authorization, errors, timeout, and recovery.
4. Isolate/restart retained engines and bound all input/output.
5. Test engine/kernel rejection and removal/restart.

## Review checklist
Check namespaces/sessions, descriptor ownership/CLOEXEC, process identity, kernel event loss/reorder, engine escape, capability minimization, logs/secrets, and recovery independence.

## Tests, architecture, security, evidence
Test host state machines/faults plus both target builds/QEMU; physical-device evidence when required. Record kernel/tool version, exact command, target, and retained engine behavior.

## ADR / stop conditions
ADR for new compatibility engine, kernel/userspace ownership change, base-image component, syscall ABI/privilege model, or kernel fork. Stop if roadmap ownership is ambiguous or policy would depend on shell parsing.
