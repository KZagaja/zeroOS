# zeroOS specification

## Roadmap scope
All milestones M0–M11 and later maintenance. Resolve live status and the current target from `ROADMAP.md`; this skill never stores them.

## Purpose and application
Use before any zeroOS change, roadmap interpretation, milestone/status edit, or contradiction review.

## Inspect first
Read `ROADMAP.md` completely, then root/nearest `AGENTS.md`, affected code/tests, and `docs/ai/change-protocol.md`.

## Decisions and invariants
`ROADMAP.md` is authoritative; Linux supplies kernel mechanisms while zeroOS owns Rust userspace policy. Preserve the live milestone state, accepted architecture, dependency direction, status vocabulary, and all historical evidence.

## Forbidden
No competing roadmap, silent reinterpretation, invented accepted decision, rewritten evidence, partial checkbox, or prose-only completion.

## Workflow
1. Name milestone and exact acceptance item.
2. Quote/locate the governing decision and inspect implementation evidence.
3. List contradiction, exact files, invariants, and missing decision/evidence.
4. Make only a compliant code correction; otherwise draft an ADR.
5. Validate with the exact roadmap commands and review the diff.

## Review checklist
Check hierarchy, status, change-log append, acceptance completeness, commit reproducibility, artifact hashes, and no historical deletion.

## Tests, architecture, security, evidence
Run `cargo xtask check` and applicable x86_64/aarch64 acceptance. Treat trust, privilege, signing, recovery, and destructive actions as blockers to inference. Evidence names commit, image/toolchain, exact commands/results, target, logs, and hashes.

## ADR / stop conditions
Use `docs/ai/templates/architecture-decision.md` for any roadmap/format/API/trust/dependency-direction change. Stop and report exact contradictory files, absent acceptance decision, failed command, missing target evidence, or request to fabricate/rewrite evidence.
