# System API review

Confirm explicit version negotiation precedes mutation; request size, fields, identifiers, descriptors, object ownership, authorization, timeouts, cancellation, and per-client resource use are bounded. Define stable machine-readable errors and compatibility rules. Keep engine/kernel details behind zeroOS policy types. Test malformed and incompatible clients, duplicate/reordered requests, disconnects, service restart, authorization revocation, and cross-version behavior on both targets where relevant.
