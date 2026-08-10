# Testing and evidence

Choose the smallest layer that proves the invariant: unit/property tests for parsers and state machines; fault/crash/interruption tests for supervision/storage; fuzzing for hostile byte protocols; Miri for host-executable unsafe; Loom/TSan for concurrency; QEMU and native CI for both targets; physical hardware when the roadmap requires it.

Evidence records commit, clean command, environment/toolchain/build-image identity, target, result, artifact hashes, and retained logs. A local pass, screenshot, prose claim, single architecture, or partial test never achieves a milestone. Failed or skipped commands remain explicit. Use `templates/test-plan.md` and `templates/milestone-evidence.md`.
