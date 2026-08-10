# Wayland compositor

## Roadmap scope
M5 owns the compositor; M7–M11 integrate and certify it. Read live status, protocol decisions, and acceptance criteria from `ROADMAP.md`.

## Purpose and application
Use for Wayland wire/compositor code and later integrations. If implementation is absent, do not scaffold it without a concrete live roadmap acceptance target. Reference Rust, unsafe, concurrency, system API, security, testing, and architecture skills.

## Inspect first
Read M5 and frozen protocol decisions, then actual wire/object/buffer/input/output code, generated protocol boundaries if admitted, tests/fuzz targets, DRM/KMS adapters, and Xwayland admission.

## Decisions and invariants
zeroOS implements the Wayland wire server upward; Smithay or another compositor framework is not the foundation. Every client is hostile; each object has client owner/interface/version/lifecycle/unique ID/destruction/validation/error behavior.

## Forbidden
No unchecked wire field/array/string/FD/ID/size, panic/cross-client corruption, leaked buffer/FD/object, stale focus/clipboard, ambiguous buffer release, unbounded objects/messages/pools/surfaces/callbacks/clipboard/DnD, or framework foundation.

## Workflow
1. Freeze request/version/object transition and resource limits.
2. Decode into bounded validated types before dispatch.
3. Tie RAII objects/FDs/buffers to one client and explicit lifecycle.
4. Isolate malformed client teardown from peers.
5. Add conformance, malformed, fuzz, lifecycle, and isolation tests.

## Review checklist
Check ID reuse/destruction, FD count/CLOEXEC, shm overflow/seals/size, GPU import, buffer busy/release, reentrancy, focus/input/clipboard/DnD isolation, Xwayland lifecycle, outputs/scaling, and client quotas.

## Tests, architecture, security, evidence
Wire/property/fuzz malformed clients, peer isolation, buffer release, multi-window/display/mixed DPI, DRM/GPU fallback, both targets, and physical graphics where required. Retain minimized corpus and logs.

## ADR / stop conditions
ADR for protocol set/version, renderer/DRM ownership, Xwayland boundary, buffer model, or any compositor foundation dependency. Stop when the live roadmap does not authorize the requested scope or protocol/resource/security decisions are absent.
