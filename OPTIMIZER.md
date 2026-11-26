# Optimizer architecture

Optimization runs in `jsc-link` over the SSA representation JIR-R (`jsc-link/src/opt/`), whole-program planners (`jsc-link/src/plan/`), and JS AST post-passes (`jsc-link/src/lower/js/`), followed by link-time duplicate merging (`jsc-link/src/link_dedup.rs`).

## Semantic guarantees

When enabled (`js-opt=on`), the optimizer may change internal value layouts (such as native `String` / `Vec` carriers, structure-of-arrays columns, flattened call ABIs, and `Option`/`Result` niches) while preserving Rust-visible length, mutation, move, aliasing, panic, unwind, and drop behavior.

- **Exact capacity exception:** native `String` values created by `format!` or `to_string` may report an exact-fit `capacity()` equal to `len()` where standard amortized growth would reserve extra headroom; `capacity()` is never below `len()`.
- **Fail-closed proofs:** every pass runs `Func::verify` after rewriting. With `RCGJS_WITNESS_CHECK=1` (and in debug builds), independent witness checkers replay constant folding, branch folding, range/bounds proofs, and invoke pruning. A failed verification or witness check discards the transform instead of risking miscompilation.
- **Pinned boundaries:** `JsValue`, `JsString`, `JsArray`, `repr(C)` layouts, foreign host handles, exports, indirect calls, and raw-pointer provenance edges stay on canonical representations unless proven safe across the boundary.
- **Generic rules:** passes match Rust types and standard-library diagnostic items (`core`/`alloc`), never workload names, source file paths, or application annotations. All compiler code is safe Rust (`unsafe_code = "deny"`).

## Profiles and build options

Profiles are passed via `-Cllvm-args=js-profile=<name>` (or `--profile` in `scripts/mires.mjs`):

| Profile | Optimizer | Minification | String repr | Purpose |
| --- | --- | --- | --- | --- |
| `dev` | `off` | off | UTF-16 | Fast compiles, unoptimized readable JS |
| `report` | `report` | off | UTF-16 | Measures what `on` would rewrite while emitting `off` output |
| `release-size` | `on` | `on` (locals + items) | `bytes` (`ByteStr`) | Smallest minified and compressed bundle size, ES2018 syntax ceiling |
| `release-balanced` | `on` | `on` (locals + items) | `bytes` (`ByteStr`) | Balanced size and speed, ES2018 syntax ceiling |
| `release-speed` | `on` | `locals` (item names kept) | `bytes` (`ByteStr`) | Maximum runtime throughput, aggressive inlining, ES2018 syntax ceiling |

Individual `-Cllvm-args` options parsed by the backend and forwarded to `jsc-link` in the manifest:

| Option | Effect |
| --- | --- |
| `js-opt=off\|report\|on` | Disable passes (`off`, default without a profile), simulate passes on clones and report stats (`report`), or emit optimized code (`on`). |
| `js-opt-passes=all\|none\|LIST` | Developer pass mask over `sroa`, `ast-dce`, `future-poller`, `native-representations`, `link-dedup`. |
| `js-str=utf16\|bytes` | String representation (`bytes` enables `ByteStr` UTF-8 byte-slice lowering; default on release profiles). |
| `js-ecma-target=2018\|latest` | Syntax ceiling for the JS printer (`2018` on release profiles). |
| `js-minify=off\|locals\|on` | Rename local variables only (`locals`) or both locals and internal item names (`on`). |
| `js-jir-flat-abi=on\|off` | Toggle flat direct-call ABI planning (default `on`; useful for bisecting). |
| `js-opt-report=PATH` | Write the version-6 whole-program JSON size and optimization report. |
| `js-opt-warnings=on\|off` | Emit warnings for rejected optimization opportunities in workspace source (default `on`). |
| `js-workspace-root=PATH` | Restrict optimization warnings to files inside this directory. |
| `js-lane-report=PATH` | Write the per-item JIR lane availability report. |

`js-opt`, `js-opt-passes`, `js-ecma-target`, and `js-minify` are emit-affecting and key the sysroot cache (`scripts/sysroot-cache.sh`), so the application crate, dependencies, and sysroot must match. Link-only flags such as `js-jir-flat-abi`, `js-str`, and `js-opt-report` reuse an existing sysroot.

`mires-build` exposes these through `BuildConfig` (`profile`, `optimizer`, `optimizer_passes`, `optimization_warnings`, `optimization_report`, `workspace_root`, `rust_opt_level`).

For debugging and bisecting, `RCGJS_JIR_PASSES` takes a comma-separated subset of `constprop,inline,devirt,pea,host-args,slice-iter,range,dce` at link time, and `RCGJS_WITNESS_CHECK=1` enables full witness verification in release builds.

## Pipeline overview

`jsc-link/src/link/lane.rs` runs whole-program optimization in three phases:

1. **Phase A (SSA & whole-program analysis):** Builds JIR-R `Func`s (`lower/ssa.rs`) for reachable items, computes call graphs, effect/throw summaries (`analysis/effects.rs`), byte-view escape analysis (`analysis/byte_views.rs`), heap eligibility (`analysis/heap.rs`), monotone arena proofs (`analysis/arena.rs`), and known-ASCII tracking (`analysis/ascii.rs`).
2. **Phase B (interprocedural optimization & planning):**
   - **Pre-repr group:** Inlining (`opt/inline.rs`), devirtualization (`opt/devirtualize.rs`), invoke/landing-pad pruning (`opt/prune_invoke.rs`), and dead-code cleanup (`opt/dce.rs`).
   - **Phase B1 (`plan/native_types.rs`):** Whole-type native container lowering — `Box<T, Global>` drop-glue elision and order-unobserved `BTreeMap<K, V, Global>` lowered to JS `Map`, followed by post-repr inlining and universal `NativeString` format-kernel lowering (`lower/semantic_ops/fmt_kernel.rs`).
   - **Phase B2 (`plan/abi_flat.rs`, `plan/physical_cost.rs`, `plan/enum_repr.rs`, `plan/bstr_abi.rs`, `plan/soa_vec.rs`):**
     - **Flat direct-call ABI & zero-offset slices (`plan/abi_flat.rs`):** Flattens private tuple, array, small-struct, and slice parameters into scalar JS arguments (`buf, off, len` or `buf, len` when zero-offset is proven across all callers) and returns multi-lane product results via lane 0 plus `$rN` spill bindings. Priced per ABI-connected group against exact compacted/compressed size and allocation sites (`plan/physical_cost.rs`).
     - **Enum specialization (`plan/enum_repr.rs`):** Null-niche `Option`/`Result`, integer-tag enums, and Smi-safe `u32` struct fields.
     - **Structure-of-Arrays `Vec` (`plan/soa_vec.rs`, `lower/js/soa.rs`):** Splits non-escaping `Vec<FlatStruct, Global>` into parallel `TypedArray` columns (for example, `Float64Array` / `Int32Array` per field) so tight geometry and tree loops avoid object allocations.
3. **Phase C (per-function optimization, JS lowering, & post-passes):**
   - **Per-function JIR-R pipeline (`opt/pipeline.rs`):** Alternates `dce` (constant folding, discriminant jump-threading, trivial block-param cleanup) with partial escape analysis (`opt/partial_escape.rs`, scalarizing non-escaping aggregates and unboxing local cells), followed by `opt/host_args.rs`, `opt/slice_iter.rs` (checked index cursors for raw-pointer slice loops), `opt/slice_forward.rs`, `opt/range.rs` (monotone-arena and induction-variable bounds-check elimination, bitmask bounds, exact allocation-length propagation), and trailing `dce`.
   - **JS lowering & AST post-passes (`lower/js/`):** Lowers JIR-R + structured control flow (`structurizer`) to JS AST, including universal `NativeString` (`{ s, cap }`) and `NativeVec` (`TypedArray` / packed JS `Array`) operations, `ByteStr` UTF-8 ops (`lower/bstr.rs`), monomorphic 4-arg `i64` pair arithmetic (`lower/js/i64_pair.rs`), monomorphic Future pollers, loop hoisting and induction optimizations (`lower/js/loop_opt.rs`), and JS AST cleanup/dominated guard elision (`lower/js/post_opt.rs`).
   - **Link-time dedup (`link_dedup.rs`):** Merges reachable functions that print identical JavaScript under different monomorphized names and updates all references (skipped when source maps are requested).

If a Phase C function declines after Phase B reshaped its ABI, the driver retries cleanly with an empty ABI plan so callers and callees never get out of sync.

## Reports and diagnostics

- **`js-opt-report=<path>`** writes a deterministic version-6 JSON report listing per-crate and per-item byte counts, reachability parents, `link_dedup` merges, selected passes, and per-item JIR operation censuses (unsupported for chunked builds).
- **`<out>.lane.json`** records per-phase pass counts (`would_apply` vs `applied`), inlining/devirtualization/range/witness totals, flat-ABI decisions, and `native_types` commits. Under `js-opt=report`, `would_apply` reflects simulated passes on clones while `applied` stays zero and the emitted JS matches `js-opt=off`.
- Workspace-scoped optimization warnings are deduplicated to at most one warning per source span; `core`, `alloc`, and dependency misses remain in the JSON report only.

## Verification

```sh
bash scripts/check.sh
RCGJS_WITNESS_CHECK=1 JS_EXTRA_ARGS='js-profile=release-speed' bash scripts/test.sh --require-core
bash scripts/slice-lane-test.sh
bash scripts/native-box-test.sh
bash scripts/prune-invoke-test.sh
bash scripts/bstr-probes.sh
bash scripts/jir-report-test.sh
```
