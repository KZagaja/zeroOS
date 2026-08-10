# Cross-architecture review

Build a capability table for x86_64 and aarch64. Check UEFI fallback names, firmware discovery, ACPI/device tree/PSCI, timers, cache/DMA, alignment/page-size, atomics/order, device enumeration, kernel configs, compiler targets, and QEMU-versus-hardware gaps. Target behavior belongs in `kernel/`, `selector/`, or an accepted architecture module; generic code consumes typed capabilities.

Evidence must name both commands/results or explicitly state why a component is host-only and architecture-independent.
