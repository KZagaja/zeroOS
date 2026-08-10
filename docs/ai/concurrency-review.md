# Concurrency review

List shared state, owner, mutation sites, locks/channels/atomics/signals, and shutdown interactions. Document lock hierarchy and poisoning policy. Reject I/O, callbacks, child waits, blocking channels, sleep, notification, or `.await` under synchronous guards.

For each nontrivial atomic record publisher, observer, synchronized state, happens-before edge, and why ordering holds on ARM64. Review PID reuse, cancellation, duplicate/reordered events, timeouts, crash/restart, and repeated signals. Require a deterministic model or stress/fault test; use Loom/TSan when the reduced model/environment supports it.
