# Restore other-language comparators

- [x] Preserve compiler identity and affected configuration; restore 14 archived ports.
- [x] Adapt build paths and install isolated or pinned compiler dependencies.
- [x] Build both OSS and Pro ports; record failures as failures.
- [x] Check emitted artifacts, offline functional comparisons, and package imports.
- [x] Document commands, versions, scope, and publication boundaries.

Compiler sources must remain unchanged. Historical receipts are not current validation.

## Review

14/14 fresh builds with unchanged port source inventories; 62/62 check commands; 8,560 offline OSS comparisons across variants/aliases; 83 package assertions; seven Pro invalid-input checksuites. The artifact-identity mutation is rejected. All 246 compiler files compared with the initial snapshot remain unchanged. Browser parity and runtime benchmarking are not claimed.
