# Engineering principles

- Linux supplies mechanisms; zeroOS Rust userspace owns product policy.
- Preserve the accepted `ROADMAP.md` architecture; resolve live milestone status from the ledger for every task and never duplicate it in reusable instructions.
- Validate and bound every external byte, size, identifier, descriptor, event, and state transition.
- Fail closed at privilege, signature, authorization, and format boundaries.
- Make ownership, cleanup, synchronization, durability, rollback, and recovery explicit.
- Design and test for x86_64 and aarch64; x86 success is not portability evidence.
- Prefer deletion/reuse/stdlib/native facilities and the smallest complete diff. No speculative layers.
- Reproducible commands and artifacts are evidence; prose is not.
