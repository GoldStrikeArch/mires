# jsc-link

`jsc-link` is the linking, optimization, and JavaScript emission binary for `mires`.

`rustc -Zcodegen-backend=librustc_codegen_mires.dylib` compiles MIR into JIR object files and then invokes `jsc-link --manifest <path>`. Reading objects and rlibs, dead-code elimination, deduplication, optimization, JS lowering, chunk splitting, source maps, `.d.ts` output, and reports all live here.

## Why it is a separate crate

Keeping `jsc-link` out of the `rustc` dylib makes the linker and optimizer an ordinary Cargo crate:

- `cargo test -p jsc-link` runs unit and property tests quickly with no `rustc_private` dependency, no nightly dylib relink, and no `TyCtxt`.
- The sysroot cache key hashes the backend dylib (which determines what object files contain), leaving cached sysroots valid across edits to `jsc-link`. `scripts/sysroot-cache.test.sh` verifies this.
- The backend dylib does not depend on `jsc-link`; the shared manifest struct lives in `rcgjs-object`, which both crates depend on.

## The manifest

Before exiting `CodegenBackend::link`, the backend writes a postcard-serialized [`rcgjs_object::manifest::Manifest`] to a temporary path, executes `jsc-link --manifest <path>`, and deletes the file. Use `jsc-link --print-manifest <path>` to inspect a saved manifest.

| Field | Meaning |
| --- | --- |
| `version` | `MANIFEST_VERSION` (currently `4`). Mismatched backend/linker pairs fail with a version error. |
| `opts` | `Opts` parsed once from `-Cllvm-args` in the backend so invalid flags report against the `rustc` invocation. |
| `crate_name` | Name of the crate being linked, used in diagnostics. |
| `crate_type` | `bin`, `cdylib`, or `dylib` (empty in `Prune` mode). |
| `mode` | `Program { output }` or `Prune { first, rest }`. |
| `objects` | The current crate's object files (CGU objects plus the allocator module). |
| `rlibs` | Linked rlibs in `rustc` link order. Their items are demoted to internal linkage on read so dependency exports do not become program roots. |
| `sources` | `(name, text)` pairs for `sourcesContent` when `js-source-map=on`. |

### Link modes

- **`Program`** (`--crate-type bin`, `cdylib`, or `dylib`): reads objects and rlibs, runs the whole-program pipeline, and writes the final JS program to `output` (or stdout). Rlibs bypass `jsc-link` and use `rustc`'s standard archive writer.
- **`Prune`** (single-crate `--emit=obj` used by `scripts/compile.sh`): runs the graph walk over the newly emitted object files, writes the linked program into `first`, and empties the remaining object files.

## Pipeline phases

`link/lane.rs` drives compilation over all reachable JIR bodies in three phases:

1. **Phase A (SSA & analysis):** Decodes reachable JIR-S bodies (including cleanup-only callees referenced by unwind edges), builds JIR-R `Func`s via `lower::build_func`, and computes call graphs, effect summaries, byte-view escape sets, heap eligibility, arena bounds, and ASCII facts.
2. **Phase B (interprocedural planning):** Runs inlining, devirtualization, invoke pruning, whole-type native container lowering (`plan/native_types.rs` for `Box` drop elision and `BTreeMap` → JS `Map`), universal `NativeString` format-kernel rewriting, flat direct-call and zero-offset slice ABI planning (`plan/abi_flat.rs`, priced by `plan/physical_cost.rs`), enum niche specialization (`plan/enum_repr.rs`), `ByteStr` ABI planning (`plan/bstr_abi.rs`), and SoA `Vec` planning (`plan/soa_vec.rs`).
3. **Phase C (per-function opt & emission):** Runs `opt::optimize` on each `Func`, lowers it to a JS AST (`lower::lower_func`), runs JS loop and post-lowering AST passes, applies local minification, and prints the final declarations.

Because Phase C collects declarations before mutating the item table, any ABI-plan decline in Phase C triggers a single retry from Phase A with an empty ABI plan so callers and callees stay consistent (`scripts/abi-retry-test.sh` tests this with `JSC_JIR_ABI_FAULT`).

Under `js-opt=report`, passes run on clones and the ABI plan is discarded before rewriting, so `<out>.lane.json` records `passes.would_apply` beside `passes.applied` while the emitted JavaScript matches `js-opt=off` (`scripts/jir-report-test.sh`).

### Lowering stages (`lower/`)

| Module | Role |
| --- | --- |
| `lower/ssa.rs` | Braun-style SSA construction from JIR-S `Body` to JIR-R `Func`, separating SSA values from addressable cells while preserving unwind edges. |
| `lower/fuse.rs` | Out-of-SSA edge-copy placement and single-use expression fusion. |
| `lower/structure.rs` | Adapts a `Func` CFG for `structurizer`, splitting critical edges that carry copies and handling dispatch fallbacks. |
| `lower/plan.rs` | Data structures passed from `fuse`/`structure` into JS emission (`Plan` and `Structured`). |
| `lower/js/` | Emits `jsast` from `Func` + `Plan` + `Structured` using `TypeTable` and `Interns`: functions, places, rvalues, pointers, values, constants, calls, intrinsics, unsizing, host bindings, SoA columns, `i64` pairs, loop opts, and post-lowering AST opts. |
| `lower/bstr.rs` | `ByteStr` (`&str`/`&[u8]` UTF-8 byte) lowering and ASCII LUT/switch emitters. |
| `lower/semantic_ops/` | Native lowering for `fmt_kernel`, `BTreeMap` (`map.rs`), numeric formatting (`num.rs`), and `Box`/structural drops (`drop.rs`). |

`lower_item` returns `Result<LoweredItem, Decline>` without panicking. Any reference emitted by a lowered body must exist in the prepared item graph; an unprepared reference is a hard link error.

## Finding the binary

The backend looks for `JSC_LINK_BIN` first and otherwise expects `jsc-link` next to `librustc_codegen_mires.dylib` (the layout produced by `cargo build --release -p rustc_codegen_mires -p jsc-link` and `scripts/build.sh`). `scripts/compile.sh` and `scripts/compile_core.sh` export `JSC_LINK_BIN` explicitly.

## Diagnostics and reports

- `diag::Sink` writes `error:` / `warning:` messages and indented `= note:` lines directly to stderr, inherited by `rustc`. A non-zero exit causes `rustc` to report that linking failed after the linker's diagnostics have already printed.
- `js-opt-report=<path>` writes the version-6 JSON size and optimization report, including per-item `link_dependencies`, `jir_census`, and embedded `jir_lane` summaries (unsplit programs only).

## Module layout

| Module | Contents |
| --- | --- |
| `link.rs` | Manifest entry point (`run`), object/rlib reading, `dedup`, reachability, ordering, printing, chunking, stubs, zombie reporting, shim imports, source maps, and `.d.ts` output. |
| `link/lane.rs` | Whole-program JIR driver, retry handling, and `<out>.lane.json`. |
| `link/statics.rs`, `link/hoist_consts.rs` | Static cycle initialization and constant hoisting. |
| `analysis/` | Callgraph, effects, byte views, heap eligibility, monotone arena bounds, ASCII tracking, allocation sites, and reference analysis. |
| `plan/` | `native_types`, `abi_flat`, `physical_cost`, `cost`, `enum_repr`, `bstr_abi`, `soa_vec`, `lowering_reuse`, `versions`. |
| `opt/` | `pipeline`, `dce`, `partial_escape`, `inline`, `devirtualize`, `prune_invoke`, `range`, `slice_iter`, `slice_forward`, `host_args`, `projection`, `witness`. |
| `lower/` | JIR-S → JIR-R → JS AST lowering and AST post-passes. |
| `link_dedup.rs` | Merges identical printed function bodies across monomorphizations. |
| `report.rs` | `js-opt-report` JSON writer and deduplicated workspace miss warnings. |
| `diag.rs` | Stderr diagnostic sink. |

Shared types (`object`, `jir`, `jsast`, `minify`, `item`, `opts`, `sourcemap`, `dts`, `manifest`) live in [`rcgjs-object`](../rcgjs-object/README.md).

[`rcgjs_object::manifest::Manifest`]: ../rcgjs-object/src/manifest.rs
