# Current verdict — standalone mires v2 reconstruction

Date: November 8, 2026. **Working development compiler assembled; not production-qualified and not the complete October dev13 implementation.**

The active workspace is now the recovered producer / semantic-object / independent-linker v2 architecture. It compiles real Rust `core` and `alloc`, produces runnable JavaScript and source maps, and is usable through both the standalone command and a stable Cargo build script. No Topcoat framework crate is required. The coherent implementation base is September 7; later release cleanup defaults, a fail-closed cleanup guard, linker invalidation, and recovered manifest packaging corrections are layered on it. See [the restoration record](docs/V2-RESTORATION.md).

## Fresh verification

| Check | Result |
| --- | --- |
| Formatting / Clippy with warnings denied | Passed |
| Rust workspace tests | 1021 passed, 0 failed, 7 ignored |
| Recovered JIR+optimizer fixture suite | 138/138; explicit non-release cleanup-off mode |
| Generic runtime suite | 40/40 |
| Active host-call cleanup | 40/40; cleanup on with optimizer on/off |
| Missing semantic cleanup body | Reachable case rejected; unreachable dependency harmless; failed link leaves no misleading final output |
| Sysroot profile/cache normalization | Passed, including release cleanup default and explicit opt-out |
| Standalone ESM example | Values, minimal runtime manifest and authored source map passed |
| Stable Cargo consumer | Built through `mires-build`; generated JavaScript executed successfully |
| Entire `scripts/check.sh` | Passed |
| Browser-workload corpus | 10/10 builds, stable input hashes, ES2018 syntax checks and zero ordinary fallback |
| Node-runnable corpus | Markdown, search, gzip and canvas-recording cases passed; search stress through 240 documents |
| Negative evidence control | A deliberately failed build receipt is rejected by the runtime harness |
| OSS reference | Rebuilt from original TypeScript |
| Rust compact OSS package | Cleanup-on optimized build and API/import/package checks pass; 58/58 offline reference kernel comparisons |
| Rust idiomatic Pro main + worker | Cleanup-on optimizer-off build and API/import/package checks pass |

The optimized Pro link was stopped after approximately 9 minutes 38 seconds without completion. A live sample is retained in `build/assembly/pro-link-sample.txt`: it showed repeated physical-cost lowering and catch-name reservation work. This is compile-time evidence, not a measured runtime regression or a claim about a completed optimized build. The optimizer-off control did **not** disable cleanup.

The package checkers were invoked with their explicit size-failure allowance to separate API/import checks from economic gates. Their full reports retain **6 compact-OSS and 6 Pro historical size failures**; those gates are not waived or called passed. New compiler/profile/toolchain identities differ from the old measurements.

## Open later-regression cases

The separate `examples/known-gaps` corpus uses fresh native expectations from conversation-reconstructed October tests. Its strict runner reports **2/6 passing** under cleanup-on `release-balanced`. The remaining failures are:

- `195_zst_array_drops`
- `audit_aggregate_place`
- `audit_zst_closure_drop`
- `audit_zst_vec_drop`

Three fail during zero-sized-array destruction; aggregate-place identity differs. `201_reference_field_pointers` and the named-reference audit probe pass on this reconstruction in the tested configuration. Do not infer their behavior in untested modes. None of the wrong answers was written into a golden as acceptable behavior.

```sh
python3 scripts/check-known-gaps.py
```

That diagnostic intentionally exits nonzero while any case remains open. The recovered September suite passing does not supersede these failures.

## Identity and evidence

Compiler source digest: `eed71e26087504afedae82418024875b11a0b19fff20b2707053817adfc27505`. Exact current source inventory, binary/artifact hashes, scope and commands are in [v2-source-identity.json](docs/v2-source-identity.json) and [v2-assembly-verification.json](docs/v2-assembly-verification.json). Raw logs and package receipts are in `build/assembly/`; the first corpus attempt's detected source mutation is retained separately from the clean rerun. All 8,544 recovered files and 3,755 recovery content objects were reverified unchanged.

The root checkout remains uncommitted. The pre-assembly source is preserved outside the repository under `/Users/mikhail.pertsev/mires-recovery-backups/assembly-20261108T025023`. Backups are local to this Mac, not off-device.

## Still not claimed

No complete October optimization restoration, three-browser parity matrix, release timing, signed prebuilt, clean Linux CI run, or production admission. The original Pro source checkout remains incomplete. Legacy template markers / small compatibility ABI crates remain internally; the Topcoat framework, UI macros, server and island implementation are not active workspace dependencies. Other-language ports were not restored.
