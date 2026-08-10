# Security review

Inventory assets, actors, entry points, privilege transitions, secrets, persistence, and recovery authority. For each untrusted input verify authentication/authorization ordering, strict bounds and parsing, checked arithmetic, stable errors without secret leakage, resource exhaustion controls, audit ownership, fail-closed behavior, and isolation after malformed input.

For M3 verify exact signed bytes, signer trust/rotation, downgrade rejection, inactive-slot targeting, durability, rollback, production-key exclusion, recovery independence, and explicit destructive confirmation. Use `templates/threat-model.md`; unresolved high-impact assumptions block implementation.
