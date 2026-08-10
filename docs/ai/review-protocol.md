# Review protocol

Review in this order:

1. requirement and source-of-truth consistency;
2. scope and subsystem ownership;
3. trust boundaries, authorization, secrets, parsing, and resource limits;
4. error/recovery behavior and partial failure;
5. unsafe/FFI contracts and ownership;
6. concurrency, signal/fork behavior, lock order, and ARM64 ordering;
7. on-disk/API compatibility and crash consistency;
8. dependency admission and reproducibility;
9. tests for failure, interruption, both targets, and evidence quality.

Report findings with file/line, violated invariant, concrete failure mode, and smallest correction. Do not accept a milestone/evidence edit until every acceptance item is reproducible from the cited commit.
