# Milestone evidence

## Roadmap scope
All milestones M0–M11 and later regressions. The target milestone, its live status, commands, gates, and evidence requirements always come directly from `ROADMAP.md`.

## Purpose and application
Use for acceptance checkboxes, milestone status, dated evidence, tags, hashes, release claims, regressions, or completion review. Reference specification and testing skills.

## Inspect first
Read all milestone tracking rules and the complete target milestone in `ROADMAP.md`, prior evidence/change log, actual CI/workflows/commands, source commit, artifacts, and unresolved gates.

## Decisions and invariants
Allowed status is exactly `Not started`, `In progress`, `Blocked`, `Achieved`. Achievement requires every checkbox, exact/equivalent commands at cited commit, both targets where listed, hashes, date, evidence link/path, and every milestone-specific gate currently recorded in `ROADMAP.md`.

## Forbidden
No unrun pass claim, single-target inference, partial checkbox, prose/screenshot/manual-only achievement, erased evidence, vague unresolved failure, weakened test, specification edit to fit code, or premature achievement tag.

## Workflow
1. Fill `docs/ai/templates/milestone-evidence.md` per acceptance item.
2. Verify clean exact commit/environment and run every command.
3. Collect target logs/hashes and human/independent gates.
4. Compare every checkbox literally; record failures/skips.
5. Append status/evidence/change-log only if rules are satisfied.

## Review checklist
Check commit identity, clean reproducibility, pinned image/toolchain, commands, all targets, artifacts/hashes, fault/security evidence, links, date, append-only history, and tag gate.

## Tests, architecture, security, evidence
Run the roadmap commands, including both native architectures when listed. Security ceremonies/reviews are evidence only when real, dated, scoped, and linked without exposing secrets.

## ADR / stop conditions
ADR for changing acceptance/evidence equivalence. Stop and report any failed/skipped command, missing target/hash/commit/gate, dirty evidence build, contradiction, or request to fabricate/rewrite history.
