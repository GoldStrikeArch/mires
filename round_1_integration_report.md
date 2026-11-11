# Round 1 integration: promoted wins and remaining work

Date: 2026-11-11. Scope: local `main` promotion of selected round-1 work; no push and no production admission.

## 1. Decision and reading order

This promotion deliberately does **not** merge `integration` wholesale. It starts from `main` at `a386c41d122f1f9e086119495b1d01285c18bc3f`, selects the verified improvements that do not depend on the audited no-throw cleanup-edge pass, and verifies two existing narrow correctness repairs needed by the combination: pointer-element moves and alias-aware final scalar replacement. The source `integration` branch remains at `2f36c3fec03eac4f11a399a6eda3ebf746650d0c`. Failed and unverified optimization branches are preserved rather than silently accepted.

The resulting compiler is a **development baseline with substantial restored improvements and explicitly open correctness cases**. The new alias-store comparison is important: “pre-existing on integration” was not treated as permission to regress the previously working main-branch behavior. It is neither a byte-exact restoration of October dev13 nor the correctness-repaired R1 defined by the root plan. Cleanup remains enabled in release profiles. The default has not been weakened to obtain smaller output.

Read this report for the current promotion decision, [round_1_report.md](round_1_report.md) for the original round's historical handoff, [tasks/todo.md](tasks/todo.md) for the historical investigation ledger and its new promotion addendum, and [V2-OPTIMIZATION-RESTORATION-PLAN.md](V2-OPTIMIZATION-RESTORATION-PLAN.md) for the full restoration roadmap. Later sections of an append-only ledger supersede earlier work-in-progress notes; an earlier “under verification” line is not the current verdict.

The machine-readable receipt is [docs/round1-promotion-verification.json](docs/round1-promotion-verification.json). It separates successful checks from open regression inventories, identifies the exact source and binaries, and states where the checks ran. Reported failures are not converted into passing goldens.

## 2. What was promoted

### What

The selected original commits are:

| Original commit | Restored work | Reason to include |
|---|---|---|
| `cadcfb0` | JIR reference-pointee/fusion correction | Stops expression spelling from selecting the wrong pointee interpretation in the repaired cases. |
| `0b04bdd` | Release-profile fixture for the same correction | Keeps the actual optimized configuration exercised rather than depending on disabled fusion. |
| `9920693` | Remove no-op landing pads after rustc drop elaboration | Removes cleanup scaffolding that has no drop work, without asserting that arbitrary arithmetic or host operations cannot throw. |
| `e3869c3` | Cross-crate cleanup-MIR transport and cleanup-aware duplicate resolution | Recovers cleanup metadata through pipelined `.rmeta` dependencies and preserves the cleanup-bearing instance when resolving duplicates. |
| `26f9ed4` | Exact overflow arithmetic and elimination of per-pair IIFEs | Repairs 32-bit product correctness and removes unnecessary closure construction. |
| `45edc42` | Alias-family/one-half overflow follow-up | Preserves pair carriers through aliases and keeps the extra tests from the independently checked follow-up. |
| `eec2d38` | Bulk byte-buffer host crossings | Replaces per-byte bridge work with general typed operations; includes the workload binding adaptation that uses them. |
| `9c85898` | Byte-boundary negative/mutation controls | Retains the stricter checks rather than only the successful example. |
| `7ebb785` | Combined arithmetic/byte-buffer extern goldens | Makes the previously checked combined spelling explicit. |
| `57375cf` | Native builder byte length and per-piece string sanitation | Avoids repeated length scans when the builder representation is selected and preserves the specified Unicode boundary behavior. |

The compilation-cost work, default string-runtime improvements, ZST aggregate/drop repair and earlier harness corrections were already on `main`; they are preserved, not counted as new cherry-picks.

Two delivered correctness patches received fresh integration verification here. The narrow pointer-element-move patch was not independently qualified in the original handoff. This promotion reviewed and tested that existing patch separately. It changes how pointer-valued elements are moved by `copy`/`copy_nonoverlapping` and carries the paired baseline reference-to-raw correction. Its old fixture number 122 was changed to **202** to avoid the documented pending-worktree collisions. The original worktree is unchanged.

The scalar-replacement alias-write patch (original item **1g**) is also promoted after fresh tests. A newly rebuilt pre-promotion `main` prints the correct `254` on the preserved `lost_write` probe, while the first selected candidate printed `7`. This was therefore treated as a real integration regression, not dismissed as a “base bug.” The delivered conservative fix keeps an aliased component unsplit when a copied local also writes a field. It adds fixtures 130 and 131 and retains positive controls showing that read-only aliases and eligible independent pointer records can still be split. The wider-inlining branch is **not** included with it.

### Why

Whole-branch merging would silently enable a cleanup transformation whose independent proof check occurs after mutation, while other pending branches contain explicit verification failures. Selective promotion preserves useful progress without making those unresolved decisions the default. The narrow pointer repair is one integration prerequisite: before it, the selected candidate reproduced the two failing reference-move expectations in fixture 201; after it, that fixture passes.

The pointer patch is **not** the failed full storage-identity redesign. It does not establish a complete pointer model, repair aggregate-place identity, or resolve every address-lifetime and reinterpretation problem.

### Where

The alias-store repair is in `rcgjs-object/src/sroa.rs`. The source work is in `rustc-cleanup/`, `backend/src/coroutine_source/`, `backend/src/{driver,intrinsics,rvalue,ptr}.rs`, `jsc-link/src/lower/`, `jsc-link/src/link.rs`, `mires-js/src/{host,web}.rs`, `rcgjs-object/`, and `runtime/shim.js`. The public fixtures and drivers remain under `examples/` and `scripts/`. Each cherry-pick records its original commit with `-x`; the verification receipt maps original commits to their new local commits.

## 3. The intentional holdback: cleanup-edge removal

### What

Original commit `60eadac` (stage 3b) is **not** promoted. The associated broader no-throw extensions (4′a) are also not promoted. The smaller integration-only packages in the original round report therefore do not describe this selected main candidate.

The selected compiler still removes no-op pads in the producer and performs its existing optimizations. It does not import the new `jsc-link/src/plan/cleanup.rs` edge-removal implementation or the new trust-host-contract control from that commit. Host-exception cleanup remains enabled, and cross-crate cleanup transport is promoted.

### Why

The audit found two distinct prerequisites, neither fixed simply by having many green tests:

1. `cleanup::run` on the integration branch consumes summaries before `summary_check::enforce_with` independently revalidates them. Demoting a summary afterward does not restore removed control-flow edges. The witness for some protected Rust-call categories also accepts the category rather than rederiving its no-throw premise.
2. A value's declared Rust type is not yet an enforced representation invariant at every host boundary. Safe typed bindings can admit unexpected JavaScript values. Independently, the safe-Rust flattened-array bug can manufacture invalid values without a dishonest host. Operations assumed not to throw can therefore throw ordinary JavaScript exceptions after their cleanup edges have been removed.

This holdback does not claim every old edge removal was wrong, nor that no-throw optimization should be abandoned. It keeps the known acceptance gap out of this promotion's default. A size increase relative to the larger `integration` patch set is an intentional trade, not an unexplained performance regression.

### Where and next acceptance test

Future repair spans `jsc-link/src/{analysis/effects.rs,analysis/summary_check.rs,link/lane.rs,opt/witness.rs,plan/cleanup.rs}` on the unpromoted branch and the relevant host-boundary code. Validate the summaries used by each removal **before** committing the rewrite, check the original protected operation and its obligations independently, and make disagreement revert/refuse rather than merely increment a counter. Run planted-false-summary and false-protected-operation controls with an ordinary release linker and no diagnostic environment variable. A validated-summary count alone is not the semantic acceptance condition.

Resolve typed boundary and invalid-view behavior separately. Do not declare a type annotation a proof while safe entry points can violate it. A trusted/unsafe binding contract can be an explicit alternative; it must not silently replace the checked safe contract.

## 4. Fresh verification of this promotion

| Fresh check on selected staging source | Result |
|---|---|
| Format / Clippy warnings denied / Rust tests | Pass; **1055 passed, 0 failed, 7 ignored** |
| Generic and pointer runtime tests | **75/75 pass** |
| Release-balanced behavior/structural fixture suite | **146/146 pass**; default-mode byte budgets not applied |
| Pointer fixture native controls | 82 output lines agree at native O0/O3 |
| Pointer additional compiler settings | Default, JIR opt-off, release-balanced and compact-package MIR-inline pass |
| Alias-store native fixtures | Fixtures 130/131 agree at native O0/O3 |
| Alias-store generated extent | 22 programs / 132 functions × two settings; all 44 executions match fresh native output |
| No-op landing-pad controls | Optimization on/off × MIR inlining on/off pass |
| Active host cleanup | 40 smoke cases plus ZST controls pass; separate active-poll matrix **60/60**; baseline refuses as required |
| Cleanup transport and admission | Pass, including negative producer/checksum/version/metadata cases |
| Extern suite | **5/5**, including 118 byte-boundary and 17 globals assertions |
| Standalone module | Values, cleanup default, maps and manifest pass |
| Cleanup holdback control | 4/4 valid/mistyped host arithmetic cases preserve the destructor |
| October-derived known gaps | **5/6**; aggregate-place identity remains open |
| Round-1 adversarial inventory | **1/5**; alias-store repaired, four other failures remain |
| OSS standard kernel comparisons | **58/58 each** on compact and idiomatic packages |
| Expanded OSS seed API comparisons | **360/477 each**; failures identical to old main |
| Corpus | **10/10 build**, Node pure/reference checks pass; no browser matrix |
| Optimized cleanup-on Pro | Main/worker build and offline API/import/invalid-configuration checks pass |


The cleanup holdback is additionally exercised by a focused typed-host-result control: a Number in a declared `u64` position must still run the initialized destructor when arithmetic throws; a valid BigInt must return the expected result and drop once. This control does not replace future boundary validation.

The original round's integration totals are retained in the historical report; they are not relabeled as tests of this selected candidate. The selected tree intentionally omits 3b and therefore has a different set of unit tests and different code-size economics.

The release fixture run explicitly requests `js-minify=on` as well as `release-balanced`. The existing test driver's historical default-mode byte budgets are consequently not applied in that run. It is a behavior/structural check, not a statement that all historical size budgets pass. Package sizes are measured separately. Expected compile refusals and expected abort fixtures must still fail in the specified way; a skip is not a passing test.

The two alias-store fixtures were also checked against the pinned native compiler at optimization levels 0 and 3. A bounded generated extent run selects two shape groups from each of eleven alias-formation families: 22 programs containing 132 functions, tested in release-balanced and in the compact package's `-Oz`/MIR-inline configuration. This is not a claim that the entire earlier 660-function investigation was rerun. The original pre-fix incorrect `7` result and the freshly correct old-main `254` control are preserved.

The new pointer test's 82 output lines were independently checked using the pinned native nightly at Rust optimization levels 0 and 3. Its eight runtime tests fail with the pre-fix runtime and pass with the candidate. Additional compiler configurations test the default lane, unoptimized JIR, and the compact package's `-Oz`/MIR-inlined configuration. Native equality on these fixtures is bounded evidence, not a proof for every unsafe Rust program or every pointer operation.

These checks do not constitute another independent Fable verifier run. They are integration review and fresh execution of the previously delivered work, with scope and unresolved cases recorded explicitly.

### Main-checkout replay after promotion

After the fast-forward to `cfae69e841e2c1286910734dd78c06e57c3267af`, both compiler binaries were rebuilt in the actual main checkout. The standalone values/cleanup/maps/manifest check passed there. The six October-derived probes again report 5/6, and the five round-1 probes again report 1/5 with the alias-store case repaired. These open failures remain explicit.

The compact OSS package was also freshly rebuilt in main and passes 58/58 kernel comparisons. Its minified ESM SHA-256 is `a0ceb34e7f2f429e605a5a9f5124f6e844fc72633c1e9210d0d1663d453a93ed`, byte-identical to the tested staging artifact. All 431 compiler/source inventory entries match the staging snapshot. The full Rust/core/runtime suites and the other package targets were tested in that source-identical staging worktree, not claimed as rerun a second time in main. This follow-up commit updates evidence only; no compiler source changed after the replay.

## 5. Package results and the cost of the conservative selection

| Target / metric | R0 historical | Full integration, not promoted wholesale | Selected candidate, fresh |
|---|---:|---:|---:|
| compact / bytes | 515,481 | 209,000 | **241,745** |
| compact / gzip | 110,636 | 55,869 | **61,301** |
| compact / brotli | 80,491 | 43,533 | **47,412** |
| idiomatic / bytes | 822,496 | 381,395 | **443,971** |
| idiomatic / gzip | 186,452 | 97,710 | **111,722** |
| idiomatic / brotli | 135,211 | 77,454 | **86,772** |

The ten separately packaged corpus entries sum to 632,834 bytes bytes, 172,818 gzip bytes, 148,384 brotli bytes. Each entry is compressed separately; this does not imply all ten are fetched together.


The R0 and full-integration values are historical, identified waypoints. The selected-candidate columns are freshly generated artifacts, not a recompression of old `dist` files. The exact bytes, hashes, Node/zlib/Brotli versions, output paths and build outcomes are in the receipt. Separately fetched resources must be compressed separately. Readable raw output is diagnostic and is not added to the deployment total.

The earlier lost dev12 waypoints were 147,950 minified bytes for compact OSS and 265,049 for idiomatic OSS. Those are prior audited historical observations, not freshly remeasured matched baselines. They show that substantial restoration work remains; different source/cleanup/packaging identities prevent treating those byte counts as automatic acceptance gates.

The public report does not publish Pro source, byte tables, private package traces or reference contents. Its build/check status is reported here; the detailed Pro observations are retained in the ignored private evidence and backup directories.

Compilation elapsed times collected during this integration session are diagnostic only. Some validation commands can compete for the integration lock or host resources. No new Rust-versus-JavaScript speedup, p95 claim, or quiet-machine performance qualification is inferred from those durations. The round's earlier 632→146-second Pro link result remains its own identified S2 observation, not a new measurement here.

**Additional compatibility finding:** both Rust OSS ports pass 360/477 of the expanded numeric-seed comparisons; their 117 failures match the pre-promotion main artifacts exactly. Their checked Rust `u32` import rejects negative, fractional, non-finite or out-of-range seeds that the JavaScript reference normalizes. This is an existing port/API compatibility gap, not a new compiler regression and not a passing full-parity result. Repair the public adapter according to the reference contract; do not weaken Rust representation checks to make this benchmark green. The ordinary 58-case kernel suite remains passing.

The other-language comparator implementations and their toolchains were not rebuilt during this promotion. Their last offline checks do not certify new Rust behavior, browser parity, or an end-to-end product-equivalent comparison. Frozen artifacts may be reused only after their identities and input contracts are checked.

## 6. Correctness inventory: passing suites are not the finish line

### What

The six October-derived probes in `examples/known-gaps/` and the five smaller round-1 probes in `examples/round1-open/` are explicit, separate inventories. Their expected outputs remain native expectations. Neither runner treats a known wrong answer as success.

| Inventory / case | Final result | Classification |
|---|---|---|
| October / `195_zst_array_drops` | Pass | Pass in the selected release configuration |
| October / `201_reference_field_pointers` | Pass | Pass in the selected release configuration |
| October / `audit_aggregate_place` | **OPEN** | Original aggregate-place model gap remains |
| October / `audit_named_reference` | Pass | Pass in the selected release configuration |
| October / `audit_zst_closure_drop` | Pass | Pass in the selected release configuration |
| October / `audit_zst_vec_drop` | Pass | Pass in the selected release configuration |
| Round 1 / `address_fields` | **OPEN** | Also fails on freshly rebuilt pre-promotion main |
| Round 1 / `copy_alias` | **OPEN** | Also fails on freshly rebuilt pre-promotion main |
| Round 1 / `flattened_safe` | **OPEN** | Also fails on freshly rebuilt pre-promotion main |
| Round 1 / `lost_write` | Pass | Fixed promotion regression; old main already produced the correct result |
| Round 1 / `niche_constant` | **OPEN** | Also fails on freshly rebuilt pre-promotion main |


### Why

The historical report's phrase “base bug” needs a named base. An issue can predate a pending pooling/inlining patch while still being exposed by earlier changes in this round. Maintain four classifications: present in R0; introduced/exposed since R0; present before the current patch but not yet bisected to R0; and unclassified. “Not caused by this patch” is useful attribution, not a production acceptance waiver.

The integration branch's `201_reference_field_pointers` failure was a concrete example: its source had passed the initial reconstruction, then two reference-move observations failed on integration. This promotion requires the repaired observations to pass. The unrelated aggregate-place test remains open and is not renamed as supported behavior.

### Where

Run `python3 scripts/check-known-gaps.py` and `python3 scripts/check-round1-open.py` after building the selected compiler. Their nonzero exit codes are intentional signals of unfinished correctness work. To exercise the package configuration, run the round-1 runner with `JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on`. Preserve separate receipts before changing a mode; the current small diagnostic drivers reuse their local output directories.

The five new public reproductions were copied byte-for-byte from the earlier independent audit, not transcribed from prose. `examples/round1-open/PROVENANCE.json` records their original source hashes and observations. This preserves critical knowledge outside a temporary directory without exposing private product code.

## 7. Highest-priority remaining work

| Priority / original item | What is left | Why it blocks further work | Where to start | Completion evidence |
|---|---|---|---|---|
| P0 / 1c | Correct by-value copies and storage-sensitive fusion, including interior-mutability through shared views | Widening inlining and pooling amplify source/operand corruption | `jsc-link/src/lower/{fuse,ssa}.rs`; pending `agent-aa0340f7…` | Preserve static/local/slice sources; include `Cell`, discriminant stores, `-Oz`, MIR inlining, negative controls and the generated extent corpus. |
| Promoted / 1g follow-up | Preserve the conservative alias-store repair and review any future flow-sensitive refinement separately | Wider inlining must revalidate this invariant; a new split must not lose an observable write | `rcgjs-object/src/sroa.rs`, fixtures 130/131 | The repaired counterexample, native extent tests and read-only positive controls remain passing when inlining is widened. |
| P0 / 1d and 1f | Correct multi-row flattening, boxed arrays, vector-of-array storage and destruction | Valid Rust manufactures undefined or wrongly typed values, undermining later proofs | `runtime/shim.js`, `backend/src/ptr.rs`, JIR pointer/slice lowering; pending flattening work | Read/write across row boundaries, zero lengths/ZSTs, heap and stack forms, normal/unwind drops, alias coherence. Runtime-helper cost and reachability measured separately. |
| P0 / 1e | Stop recognizing pointer records by user field names | Plain structs can produce null, misaligned or inconsistent synthetic addresses | Runtime address/record handling | Distinct runtime tagging or explicit typed representation; adversarial `buf/off/sc/es/pr/length` fields; no field-name whitelist patch. |
| P0 / 1i | Decode niche-optimized constants correctly | `Some` silently becomes `None` in constant tables | Constant decoding, layout/niche and static emission | Native constants/promotions/statics/locals, nested enums and pointer niches; preserve initialization order. |
| P0 / 3b-gate | Make independent cleanup validation actually gate mutation | Required before promoting 3b or expanding no-throw rules | Analysis, witness and cleanup planner on the retained integration branch | Deliberately wrong summaries/protected-op classifications retain drops or are refused in release mode. |
| P0 / typed boundaries | Enforce or explicitly trust the representation at every host entry | A safe-looking wrapper currently bypasses the invariant the optimizer assumes | `mires-js`, binding/export lowering, callback argument and result paths | Mismatched numbers/BigInts, symbols, objects/getters, chars and callback/export inputs; declared error/effect/drop behavior. |
| P1 / 1h | Preserve `#[track_caller]` through chains/function pointers | Current JIR can lose `$loc` and throw instead of implementing the call | ABI/call lowering, inlining and function-pointer wrappers | Chained/direct/indirect calls under the real release and MIR-inline modes; observable locations retained. |
| P1 / aggregate places | Define consistent value versus place identity | Remains the sixth October-derived known gap; a full earlier redesign introduced regressions | Producer places/pointers, JIR storage facts, runtime overwrite/address behavior | Stored-value moves, restoring writes, nested field pointers, future values, bounds/alignment and conversion aliases; model review before another large patch. |
| P1 / granule-min | Validate release only at true destruction, not argument conversions | Premature release can give two live allocations one address | Pending `agent-ac96e6c4…`, conversion/drop metadata and allocator helpers | Native lifetime controls, refused-bump retry, address stability, long drop loops and representation-decision diffs. |

Do not wait for an all-encompassing memory-model redesign to fix each bounded, understood miscompile. Equally, do not claim the full model is repaired because one minimum patch unblocks a package. Every patch must preserve the invariant it relies on across nearby forms, not just the reported spelling.

## 8. Optimizations still to restore, in dependency order

### 8.1 Cleanup-bearing bodies and protected/cross-crate inlining

**What:** after the cleanup gate and representation assumptions are repaired, promote proven edge removal and optimize functions that genuinely retain handlers. This includes effect-fenced fusion, scalar replacement around exceptional paths, accurate initialized-value obligations and safe protected inlining.

**Why:** eliminating empty pads is a large first win, but retaining a handler should not permanently disable all useful optimization. The October work had progressed farther in this composition. Conversely, removing a handler because an unchecked input “should have been a number” is not that recovery.

**Where:** `jsc-link/src/lower/fuse.rs`, `jsc-link/src/opt/inline.rs`, `opt/inline/remap.rs`, `opt/partial_escape.rs`, the cleanup planner, and producer identity/cleanup metadata. Carry exact monomorphized identity and exceptional obligations rather than dropping conservative guards without a replacement proof.

**Acceptance:** effects and errors retain order and identity; partial initialization and moved locals drop exactly as the declared contract requires; cross-crate and MIR-inlined equivalents agree; cleanup stays on. Show recognized/legal/selected/emitted counts and actual package/runtime changes.

### 8.2 Formatter and String representation commitments

**What:** recover the proven Move/Copy-spine commitment and broaden precisely supported String/format shapes. The builder's byte-length field and per-argument sanitation are now present; real-workload selection is still limited.

**Why:** the existing formatter machinery is not missing wholesale. The ledger reports that native format-site commitment fails when only a call result is marked while its proved spine is not. Fixing that alone reportedly leaves the idiomatic package unchanged because its String regions are pinned by additional unsupported operations and boundaries. Both barriers must be measured independently.

**Where:** `jsc-link/src/plan/representation.rs`, `analysis/legality.rs`, `lower/semantic_ops/{fmt,fmt_kernel,string}.rs`, and runtime converters. Preserve the already landed per-piece Unicode normalization before enabling previously dead formatting paths.

**Acceptance:** a positive selection assertion, custom-`Display` and unsupported-format controls, adjacent surrogate-half host inputs, native numeric formatting edge cases, and removal of the actual expensive dependency cluster in a full package. Do not add workload-name recognition. Historical 65,525→48,847 gzip is a scoped waypoint, not an automatic new gate.

### 8.3 Structural string views and optional union strings

**What:** keep byte-derived strings as well-scoped views where legal, avoid repeated encode/decode and length scans, and evaluate optional union/literal-pinning work without changing the default contract silently.

**Why:** the default runtime memo repairs important observed scaling, but memoization is not a full ownership/alias model. Equal text must not erase distinct live-storage identity where the contract admits that observation. Mutable string views still need real write-through semantics or an explicit refusal; a fresh detached copy only contains corruption.

**Where:** runtime string helpers, String carriers, representation proofs, byte-view conversions and host-boundary lowering.

**Acceptance:** deterministic operation counts on growing input families; memory-retention tests; UTF-8 byte indexing and char boundaries; capacity/alias/drop observations; async/poll/callback lifetimes; full-workload benefit. Union remains an explicit experimental choice until separately qualified.

### 8.4 Constant pooling (4′c)

**What:** redesign or complete the eligibility/isolation proof before accepting the delivered patch. The multi-chunk ownership fix bundled inside it should be separated into a correctness-only change.

**Why:** two verification rounds found host exposure through primitive references, array aliases, transparent wrappers and logging/marker routes. The copy bug also changes whether sharing is safe. Deep freezing is not automatically an equivalent fix: it can alter writes, exceptions and observable identity.

**Where:** the pending pooling worktree, constant graphs, host reachability, `Freeze`/interior-mutability facts and link ownership/order. Prefer an explicit internal context to an opaque thread-local selection if that makes dependencies auditable.

**Acceptance:** candidate selection is exercised; every host route and interior-mutability control is rejected or preserved correctly; by-value copies remain independent; cycles and multi-entry initialization behave correctly; schema incompatibility is refused; fault injection is not exposed accidentally in release builds. Measure package closure, not isolated literal savings.

### 8.5 Wider inlining (4′b), trailing arguments and small emission wins

**What:** verify the delivered inliner on top of the newly promoted 1g fix (and relevant remaining storage fixes), transport inline hints carefully, then implement eligible trailing-argument and repeated-terminal emission reductions.

**Why:** 634 inlined sites is coverage, not a correctness or speed result by itself. The delivered branch still reports a loop-growth case with more allocations. Removed trailing arguments must retain their evaluation effects; escaping/indirect/exported functions have additional ABI and arity constraints.

**Where:** `opt/inline.rs`, its remapper/declaration cost oracle, parameter-use analysis, link-time emission and the JS printer. Keep each pass individually disableable with a positively exercised test.

**Acceptance:** no lost writes, no representation-boundary explosion, no new loop allocation without an approved measured tradeoff, true end-to-end package gains and unchanged errors/effects. Small numeric-spelling and location-hoisting ideas whose measured final-package gain was zero/negative stay deprioritized; their negative results are useful evidence.

## 9. Runtime, address lifetime, and host-contract tail

The following are not “small” merely because they are outside the headline packages:

- Synthetic-address exhaustion and reuse: drop conversions, native Box glue, finalization-dependent stack/view addresses, refused allocation retries, ranges for blocks larger than one granule, and retaining address identity across ownership conversions need separate tests. A free-list change that releases a live argument is a correctness regression.
- `String::as_ptr` identity for equal-text live strings and consistency with `as_bytes().as_ptr`; mutable-string views; and address stability through Vec representation round trips remain unresolved.
- Destructor behavior when a destructor throws, both on the normal and already-unwinding path, and coroutine poisoning must be stated once in the contract and tested. Native panic behavior is not a complete specification of JavaScript host throws.
- Reinterpreting views over different element storage must either implement the declared semantics or fail closed at a well-defined boundary. Do not allow a malformed representation to propagate and then justify removing cleanup by its Rust type.
- The reported `host_try`/registered-abort interaction needs its own confirming test and explicit policy; do not promote an unverified claim as either a feature or an accepted limitation.
- Global runtime state can make separately imported modules share maps/helpers. Module-scoped runtime work may help packaging and isolation, but is an ABI/lifetime change to test, not a textual minification exercise.

A documented limitation is visible technical debt, not proof of correct behavior. Each supported safe operation must eventually work or receive an explicit, coherent diagnostic boundary. The fact a bug predates a pending optimization permits a scoped stage decision, not a claim of faithful production Rust.

## 10. Compiler economics and measurement policy

**What:** preserve the equivalent-output compile-cost wins and investigate the remaining expensive trial compression separately from semantic optimization.

**Why:** S2 reduced physical lowerings from 93,917→2,941 for compact and 536,262→7,987 for Pro. The retained Pro run then spent most link time in Brotli-11. A cheaper ranking model may be useful, but it changes which plans are selected and must not be described as an output-equivalent cache improvement.

**Where:** `jsc-link/src/plan/{cost,physical_cost,lowering_reuse}.rs`, lane diagnostics and `experiments/v2-restoration/`.

**Acceptance:** cached/uncached or reused/fresh lowerings agree on controlled tests; every semantic dependency is in the reuse key; plans and artifacts are deterministic; final gzip/Brotli and runtime costs are measured on exact shipped bytes. Treat an unrelated runtime-text edit that moves an allocation into a hot loop as a decision-stability investigation. Compression discontinuities are real, but compressed bytes alone must not override the declared runtime/ownership constraints.

For compiler-change attribution, compare identical Rust sources, flags, runtime and packaging. For application comparisons, report algorithm and boundary differences. The reduced search workload is not the same algorithm as MiniSearch, and the stored-gzip CRC implementations differ. The large Markdown improvement is valuable, but those ratios are not pure compiler tax.

Reuse the existing paired harness with A/A controls only under a prospectively recorded measurement protocol. A/A exposes noise; it does not turn a loaded development machine into a quiet reference host. Preserve failed, interrupted, contaminated and inconclusive runs. Do not select only the favorable repetition or compare per-sample time with per-call time.

## 11. Integration and build-integrity work still needed

**What:** finish identity coverage, stale-object refusal, clean consumers and multi-entry ownership without restoring framework dependencies.

**Why:** cross-crate cleanup transport is now real, but stale pre-fix objects and pipelining timing must be classified. A matching source commit is not sufficient when runtime inputs, sysroot contents, bindings, object schemas or packaging toolchains differ.

**Where:** object headers and schemas, producer identity, `mires-build`, `scripts/sysroot-cache.sh`, artifact manifests, module ordering, and clean-prebuilt workflows.

**Acceptance:** a stale incompatible object is rejected clearly rather than silently selected or causing a compiler ICE; changing only the linker invalidates generated outputs; changing any ABI-relevant profile input selects a compatible sysroot; a clean consumer needs no developer-only cache; modules do not import and redeclare the same item; statics including cycles have a declared initialization behavior. Keep production and diagnostic metadata versions synchronized where serialization actually changes.

The old Topcoat DOM/keyed suites are not standalone compiler release requirements. Retire or explicitly archive framework-only tests; retain generic callback/host-loop/template coverage where the corresponding generic support remains. A loud skip must not count as a pass. Removing those suites must not remove generic async, cleanup, worker and map obligations.

## 12. Proposed next batch

1. **Correctness prerequisite batch:** verify/refine 1c in actual package modes and preserve the newly promoted 1g invariant; verify the flattening/heap-array work together where their representations meet; fix the field-name address and constant-niche cases. Preserve all current failures before applying fixes.
2. **Cleanup acceptance batch:** enforce the independent proof gate, specify checked versus trusted host entry points, and test invalid-view containment. Only then reconsider original 3b and its extensions.
3. **Separate correctness-only link repairs:** extract multi-chunk ownership/static initialization fixes from pooling, and validate `#[track_caller]` chains. Do not make a correctness fix depend on enabling a new optimization.
4. **One integrated checkpoint:** build all four Rust targets from one identified source tree and compiler configuration; run the public behavioral gates and private Pro checks; publish a bounded evidence receipt. Do not add unverified size candidates into this measurement.
5. **Next performance batch:** formatter/spine/String legality, protected-body optimization and structural strings, then proven pooling/inlining/bounded emission changes. Use the latest runtime profile rather than assuming historical attribution is unchanged.

The public document-processing worker holdout from P05 remains not implemented. The round's narrower compiler-track decision may defer it, but that is not completion of the original R1 requirement. Freeze its contract and corpus before using it to tune passes. It initially needs a trustworthy reference and one Rust implementation, not seven additional language ports.

A finite restoration finish line is one reproducible standalone configuration with the admitted correctness cases resolved, optimized Pro compilation practical, substantial same-source output/runtime improvements measured, the public holdout present, and interaction/build-closure checks passing. Production release signing and broad compatibility qualification are separate decisions. Recreating Topcoat's framework UI is not on this critical path.

## 13. Pending branches and merge discipline

| Original worktree / branch | Work | Current decision |
|---|---|---|
| `integration` / `2f36c3f` | Full round-1 composition | Preserved; selected commits only, 3b excluded |
| `agent-aae8e384d9b94c2e6` | 4′a no-throw extensions | Failed twice; not promoted; gate and boundary prerequisites |
| `agent-a12dd166f55ef46f8` | 4′c pooling / chunk fix | Failed host isolation; not promoted; extract correctness-only chunk work |
| `agent-ab531332a87a0b330` | 4′b wider inlining | Not promoted; rebase onto repaired alias semantics and verify |
| `agent-aa0340f7b4c30f8c0` | 1c copy/storage fusion | Not promoted; shared interior-mutability case still needs repair |
| `agent-aeaee6e0ef235816f` | 1d flattening | Delivered; not independently qualified here; representation/cost questions remain |
| `agent-ac96e6c4714b6a992` | Granule-min release | Not promoted; verify destruction-only lifetime and allocator retry |
| `agent-ae104b5a611c870ef` | Minimal pointer moves | Reviewed patch incorporated and tested; fixture renamed 202; original worktree unchanged |
| `agent-ae6fe0b2bcf6f29f7` | Conservative alias-store scalar replacement | Delivered patch reviewed and tested; incorporated without wider inlining |
| `worktree-agent-a1ab8f94827df1889` | Full pointer/storage model | Failed redesign remains parked |
| `worktree-agent-acd9d634a1811beab` | Exact-size address allocator | Failed redesign remains parked |


Do not merge those worktrees wholesale onto the promoted main. They were developed on a different integration base containing 3b and, in several cases, mutually colliding fixtures/schema edits. Rebase or reapply a reviewed patch onto the new main, resolve each dependency explicitly, rerun the exact cases, and preserve the original branch as provenance. Do not drop the existing stash merely because a report suggests it is redundant; it was left untouched here.

Assign globally unique descriptive fixture names or reserved ranges before restarting parallel work. The pointer-move fixture uses 202 in this promotion to avoid the known 121–126 collisions. Keep implementation and verification separate when the original delegated workflow is resumed; do not count an interrupted verifier as a pass.

## 14. Preservation and no-push boundary

Local preservation directory:

```text
/Users/mikhail.pertsev/mires-recovery-backups/round1-main-20261111T114756Z
```

It contains a verified `all-refs-before.bundle`, original main report, every recorded worktree base/status/tracked patch, bounded untracked-fixture archives, curated decisive verifier evidence, fresh validation/build logs, source/binary/artifact identities, native controls, and the pre-fix failure records. The broad scratch archive is explicitly size-bounded; it is **not** a complete backup of every generated file. A separate curated archive retains the important reported reproducers independently of that broad archive's budget.

All 23 original worktrees were checked unchanged during preparation. Their pending patches, branch tips, integration tip and stash are retained; only the selected main promotion is authorized. Final Git/bundle checks are retained with the completed report. These are **same-device copies**, not protection against losing the Mac. No remote backup or push is claimed.


The original `round_1_report.md` is committed without rewriting its historical contents. This new report is the current supplement. Existing source worktrees, the integration branch and stash entries remain available. A local commit is not a remote backup; no network push is performed in this task.

The Pro tree, recovery inputs, generated binaries/build directories and raw private logs remain ignored and are not part of these commits. Public receipts contain only public artifact measurements and private build/check status. Private package details remain in the local private evidence.

## 15. Reproduce and interpret the checks

From the repository root, after dependencies are installed:

```sh
bash scripts/build.sh --locked
bash scripts/check.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh --require-core
SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
SKIP_BUILD=1 python3 scripts/cleanup-transport-test.py
SKIP_BUILD=1 bash scripts/cleanup-admission-test.sh
SKIP_BUILD=1 bash scripts/extern-test.sh
node scripts/standalone-check.mjs

# Strict OPEN inventories: these are expected to exit nonzero while defects remain.
python3 scripts/check-known-gaps.py
python3 scripts/check-round1-open.py
JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
```

The exact package commands and paths used for this promotion are in the verification receipt. Build/measurement commands which record failed outcomes must have their receipt checked; a process exit meaning “record written” is not a passing semantic verdict.

**Bottom line:** useful, tested round-1 work is now integrated locally without accepting the known cleanup-proof gap or the failed pooling/inlining candidates. The remaining work is concrete and preserved. This is progress toward the October capability level, not a claim that all of that state has been recovered or qualified.
