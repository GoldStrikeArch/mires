# jsc-link

The consumer half of `rustc_codegen_js`: object files in, a finished JavaScript program out.

`rustc -Zcodegen-backend=librustc_codegen_mires.dylib` no longer links anything itself. It codegens
MIR into object files and then execs this binary, handing it a manifest. Everything from reading an
object to writing the program lives here — the graph walk, deduplication, chunking, the source map,
the `.d.ts`, the optimization report, and the optimizer passes.

## Why it is a separate program

Not for build speed: a sysroot rebuild is about ten seconds either way. The point is that the
linker and the optimizer are now a **normal cargo crate**. `cargo test -p jsc-link` runs their unit
tests in well under a second with no nightly, no `rustc_private`, no dylib, and no `TyCtxt` — and
Phases 2–5, which move the whole lowering into the link step, are built on top of that.

Two consequences fall out of the split:

- **The sysroot cache key does not see it.** The key digests the backend dylib, because that is
  what decides what an object file *contains*. `jsc-link` only reads objects, so editing the entire
  optimizer leaves every cached sysroot valid. `scripts/sysroot-cache.test.sh` pins this.
- **The dylib does not depend on it.** The one shared type, the manifest, lives in `rcgjs-object`,
  which both sides already depend on. A `jsc-link` edit therefore does not even relink the dylib.

## The manifest

The dylib writes a postcard-serialized [`rcgjs_object::manifest::Manifest`] into the temp directory
(`OutputFilenames::temp_path_for_diagnostic("jsc-link-manifest")`), runs
`jsc-link --manifest <path>`, and deletes it. Postcard rather than JSON because both sides are
always built from one tree and both already speak it; `jsc-link --print-manifest <path>` dumps one
when a build has to be inspected.

| field | meaning |
| --- | --- |
| `version` | `MANIFEST_VERSION`. Postcard is positional, so a mismatched pair is rejected rather than misread. |
| `opts` | `Opts`, exactly as the backend parsed `-Cllvm-args`. Parsed once, in the compiler, so a bad option is reported against the command line that carried it. |
| `crate_name` | The crate being linked, for diagnostics. |
| `crate_type` | The crate type that made this a program (`bin`, `cdylib`, `dylib`). Empty in prune mode. |
| `mode` | `Program { output }` or `Prune { first, rest }`. See below. |
| `objects` | This crate's own objects: the codegen unit modules, then the allocator module. |
| `rlibs` | Every linked rlib, in `each_linked_rlib` order. Their items are demoted to internal linkage as they are read, which is what keeps a dependency's exports from rooting the program. |
| `sources` | `(name, text)` for every source file this compilation still holds, keyed by the name rustc prints in a diagnostic. Only filled under `js-source-map=on`; the map embeds it as `sourcesContent`, and it is the one input to the link step that lives in the session rather than in an object file. |

### The two modes

- **`Program`** — `--crate-type bin`/`cdylib`/`dylib`. The objects and the rlibs are read, the
  graph walk runs, and the program is written to `output` (or printed, when rustc was asked for
  stdout). An rlib never gets here: the dylib hands it to rustc's own `link_binary`.
- **`Prune`** — the single-crate `--emit=obj` path `scripts/compile.sh` uses, which never reaches
  `CodegenBackend::link` at all. The same graph walk runs over the objects that were just written;
  the program replaces `first`, and every other candidate is overwritten with an empty module,
  because rustc only ever copies one of them onto the `-o` path.

## The lanes

`js-lane` decides which lowering each item is printed from, and it is a **link step** decision: every object carries both the producer's baseline text and the item's JIR-S body, so nothing about it reaches the sysroot cache key. The baseline text is a selected carrier and unsupported-case fallback, not a second link-time optimizer or program graph.

| `js-lane` | what is written |
| --- | --- |
| `baseline` (default) | the program, from the producer's baseline text. `resolve` runs once. |
| `jir` | the program, from the JIR lowering wherever it succeeded, plus `<out>.lane.json`. |
| `shadow` | both: the baseline program at the output path, the JIR program at `<out>.jir.js` (with its own `.map` and `.d.ts`), and `<out>.lane.json` describing the second. `resolve` runs twice over one set of objects, so the two files differ in nothing but the lane. |

`js-lane-classes` narrows which `ItemClass` flags the lane may own (the promotion order), and
`js-lane-only` narrows it to named items (bisection). `js-lane-report=<path>` writes the per item
availability report, which now carries the lane each item took. See `link/lane.rs`.

### The lane driver

Before lowering, JIR lanes prepare the potential item dependencies recorded in verified
semantic bodies, including cleanup-only calls. Newly prepared items also bring their baseline
support dependencies. This preparation does not change baseline references or retain unused
candidates: final emitted references still decide the delivered graph. Bodies outside this
closure remain undecoded.

`link/lane.rs` decides, item by item, which spelling is printed. It offers the lowerer every reachable item whose JIR-S body the object carried and whose `ItemClass` the promoted set covers, and it keeps the producer's baseline text for everything else: a `Decline` from `lower_item`, a class the run has not promoted, or a name in `examples/jir-allowlist.txt` (compiled in with `include_str!`, and empty today). A fallback is counted. A reachable body with cleanup blocks must be emitted through JIR: using baseline text would discard its cleanup paths, so a decline or lane exclusion fails the link. This check follows final references and precedes text deduplication; unused prepared bodies do not fail the link.

DCE can fold ordinary branches and remove unrelated dead operations in functions with exceptional control flow. It preserves protected calls and their private normal successors. Partial escape analysis and inlining leave exceptional functions unchanged. Flattening a writable reference argument requires a callee proven not to throw, because copy-back is implemented only on normal returns.

Two rules the driver enforces on what comes back are worth knowing before reading it:

- **References.** A lowering may mention *fewer* linkable names than the baseline (it proved
  something dead), and *more* only when the extra name is an item the walk already reached. A
  reference to something the link step did not prepare is a hard error naming the item. The
  comparison is on the linkable subset — the item table's names plus the namer-shaped `$h` stubs —
  because the lowering spells its temporaries `$vN` where the baseline spelled `_N`, and that is a
  spelling difference rather than a program difference. The lowered item's own `refs` are written
  back, so `stub_missing`, the chunk walk and any later `reach` see what the printed body mentions.
- **Printing belongs to the driver.** `lower_item` returns an AST; `link/lane.rs` builds the
  `JsItem`, runs `minify::shorten_locals` on it and prints it, which is the same pair of calls the
  baseline path makes. That is why `js-minify` is tested on the lane rather than on the baseline.

Under `js-opt=on` the driver is three phases rather than one loop, because a whole-program pass has
to see every item's IR at once. **A** builds each offered function's `Func` (`lower::build_func`);
**B** plans flat direct-call parameter ABIs and eligible field-only product-result ABIs over all
of them, then rewrites callees and callers; **C** runs
`opt::optimize` over each `Func` and prints it (`lower::lower_func`). Phase C produces declarations
without touching the item table and the whole of it is applied afterwards, which is what makes the
retry affordable: an item the ABI plan changed that then declines in C would leave its callers
calling a shape that no longer exists, so the run restarts from A with an empty plan — one retry is
a fixed point, since an empty plan touches nothing. The `js-opt=off` path takes the same three
phases with every pass off, so there is one driver and not two.

`js-opt=on` on the jir lane runs the passes in `opt/`: `sroa` is
`opt/partial_escape.rs` and `ast-dce` is the SSA builder's `fold_runtime_checks` plus `opt/dce.rs`.
Each pass is verified after itself, and a verify failure is a `Decline` — that item falls back and
the link still produces a program. See `OPTIMIZER.md` for the full mapping. On the baseline lane
`js-opt=on` runs no link-time rewriting at all since W5h deleted the frozen v1 text optimizer:
it means the producer-baked passes plus `link_dedup`, and its output is byte-identical to
`js-opt=off` plus dedup.

`js-opt=report` on the jir lane measures that same work without keeping any of it. Every pass runs
over a **clone** of the function and the rewritten clone is dropped, so a clone that fails to
verify is a "would not have applied" rather than a decline — a decline would change what `report`
prints, and `report` owes the program the bytes `js-opt=off` prints. The ABI plan is made and
reported for the same reason and then emptied before phase B, so no callee is reshaped and no
spill register joins the program. The sidecar therefore carries both numbers:
`passes.would_apply` beside `passes.applied`, and `abi.applied`. Schema 7 also separates
representation planning, exact validation, and final emitted reachability, and reports literal
allocation sites from the final JavaScript AST. Its report-only `physical` block derives the
selected `ValueShape`, `CallAbi`, `StoragePlan`, and `DropPlan` from the current ABI and carrier
plans without changing output.
`scripts/jir-report-test.sh` is the byte-identity check, and `scripts/abi-retry-test.sh` drives the
phase-C retry above through `$JSC_JIR_ABI_FAULT`.

Product-result lane 0 uses the ordinary return value; remaining lanes use collision-safe `$rN`
spill bindings that the direct caller consumes immediately. Fixture 91 protects this ABI. The
projection pass may substitute original call arguments only for an immediately adjacent
same-block result copy; it never crosses a mutation or intervening operation. Fixture 92 protects
that temporal boundary.

## The lowering pipeline

`lower/mod.rs` is the contract: `lower_item` is total — every item either produces a `LoweredItem`
or a `Decline`, and it never panics and never emits a diagnostic. Behind it the lowering is four
stages over one item, and every stage is an ordinary unit-testable function.

| module | contents |
| --- | --- |
| `lower/ssa.rs` | Braun-style on-the-fly SSA construction: the JIR-S body becomes a JIR-R `Func`. Decides which MIR locals stay values and which become cells (address-taken primitives, aggregates written through projections, spread args, state read across a dispatch region), preserves normal and exceptional edges, and verifies the result. |
| `lower/fuse.rs` | Out-of-SSA and expression fusion. Which values are spelled inline (single use, same block, purity ladder — v1's `queue.rs` rule over SSA), what everything else is named, and how block arguments become assignments on their edges. |
| `lower/structure.rs` | The `Func` as a graph the `structurizer` crate can read: switch cases sign-biased as v1's `cfg_adapter.rs` did, critical edges split where an edge carries copies, the `force_dispatch` retry. |
| `lower/plan.rs` | The frozen interface between those three and the JavaScript side: the `Plan` (inline set, names, edge copies) and the `Structured` region tree. |
| `lower/js/` | JavaScript out of a `Func` + `Plan` + regions, over `&TypeTable + &Interns` rather than a `TyCtxt`: `function.rs` (declaration, prelude, boxed params, spread rebuild, vtable and static bodies), `place.rs`, `rvalue.rs`, `ptr.rs`, `value.rs` (clone/mask/tag/field/enum spellings), `constant.rs` (`ConstTree` → expression), `emit.rs` (region tree → statements), `call.rs`, `intrinsics.rs`, `unsize.rs`, `host.rs` and `template.rs` (the `js_extern` walk, `js!` blocks, the `rcgjs.host.*` markers and the `view!` template markers). |

What the lowerer owes beyond the declaration is whatever printing reads and the AST does not
carry: the local-name table the source map's `names` field is built from, the item's `Loc`, its
`.d.ts` signature, the zombies the body carried, and `refs`.

## Finding the binary

`JSC_LINK_BIN` if it is set, otherwise the sibling of the dylib — which is where
`cargo build --release -p rustc_codegen_mires -p jsc-link` puts both of them. The dylib's own path is
the one rustc was given on the command line (`sess.opts.unstable_opts.codegen_backend`), because a
dylib loaded into rustc cannot ask for it any other way: `current_exe` is rustc's.

`scripts/build.sh` builds both and prints both paths; `scripts/compile.sh` and
`scripts/compile_core.sh` export `JSC_LINK_BIN` so a stale or relocated build tree fails with a
name rather than inside rustc.

## Diagnostics

For an unsplit program, `js-opt-report=<path>` includes a linked operation census.
Each final reachable item records its emitted `link_dependencies` and captured
`jir_census` body, call identities, instantiated types, layouts, and source locations.
Captured calls can have been inlined or removed and are not emitted graph edges.
`nativeSignature` reports registry signature validation, including specific declines;
it does not certify ownership, glue resolution, or a representation commit. The embedded
`jir_lane` preserves the lane sidecar's planned, validated, and final-emitted decisions.

The repository's `experiments-pro/compiler-v2/tools/summarize-jir-census.py` accepts
`--linked-report <path> --output <new-directory>` to retain JSON/CSV inventory and
exclusive/shared entry item bytes. Those bytes precede packaging and compression.
The matching artifact manifests remain necessary for runtime and deployment closure.

`diag::Sink` replaces the compiler's `DiagCtxt`. Messages are written to stderr as they are
produced — `error: …` / `warning: …`, with one indented `= note:` per note — and the error count
becomes the exit status. The dylib inherits this process's stderr, so a user sees exactly what
`jsc-link` printed; a non-zero exit becomes one further rustc error saying the link failed, because
the real messages are already on screen.

## Layout

| module | contents |
| --- | --- |
| `link.rs` | `run` (the manifest entry point), reading objects and archives, `emit` (one `resolve` per lane), `resolve`, `dedup`, `reach`, `order`, `print_units`, `write_chunks`, `stub_missing`, `report_zombies`, `shim_import`, the source map and `.d.ts` writing. |
| `link/lane.rs` | The lane driver: which items `js-lane=jir` lowers, the fallback count, `<out>.lane.json`. |
| `lower/` | JIR-S to JavaScript. `lower_item` is the contract; a declined item keeps its baseline text. |
| `opt/` | The JIR-R passes `js-opt=on` runs on the jir lane: `pipeline.rs` (the order and the verify-or-decline rule), `partial_escape.rs` (`sroa`), `dce.rs` (`ast-dce`). |
| `link_dedup.rs` | Link-time merging of identical bodies printed under several names. |
| `report.rs` | The deterministic `js-opt-report` JSON and the workspace miss warnings. |
| `diag.rs` | The diagnostics sink. |

Everything these modules build on — the object format, the JIR-S schema, `jsast`, `minify`,
`item`, `opts`, `late_ir`, `sourcemap`, `dts` — is in `rcgjs-object`.

[`rcgjs_object::manifest::Manifest`]: ../rcgjs-object/src/manifest.rs
