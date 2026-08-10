# wgpu UI toolkit

## Roadmap scope
M6 owns the toolkit; M7–M11 consume and certify it. Read live status, accepted toolkit decisions, and acceptance criteria from `ROADMAP.md`.

## Purpose and application
Use for toolkit implementation and later consumers. If implementation is absent, avoid scaffolding without a concrete live roadmap acceptance target. Reference Wayland, Rust, concurrency, security, architecture, and testing skills.

## Inspect first
Read M6, then actual renderer/layout/control/text/accessibility/localization code, wgpu dependency admission, compositor API, screenshot harness, and sample app.

## Decisions and invariants
The zeroOS-owned toolkit is built on `wgpu`; shell/first-party apps use it exclusively. Accessibility, keyboard operation, localization, scaling, deterministic rendering, and recovery are acceptance requirements.

## Forbidden
No external GUI toolkit, inaccessible control, pointer-only path, missing visible focus/semantics, unbounded GPU/user asset allocation, nondeterministic unrecorded screenshot tolerance, or architecture-specific UI divergence.

## Workflow
1. Define semantic/control/layout state independent of rendering.
2. Bound assets/text/layout/GPU resources and device-loss recovery.
3. Implement keyboard/focus/accessibility/localization with the control.
4. Keep renderer behind admitted wgpu boundary.
5. Add deterministic layout/tree/screenshot/failure tests.

## Review checklist
Check roles/names/states/relations/events, focus order, scaling/clipping/text, locale/RTL, reduced motion, device loss, software fallback, memory pressure, and first-party dependency direction.

## Tests, architecture, security, evidence
Run unit/tree/keyboard tests, deterministic screenshots at 1x/fractional scale and locales, GPU/device-loss/fallback tests on both targets/reference hardware. Record tolerance and artifacts.

## ADR / stop conditions
ADR for renderer architecture, wgpu admission/version, text/accessibility backend, screenshot tolerance, or public toolkit API. Stop when the live roadmap does not authorize the requested scope or required accessibility/rendering decisions are absent.
