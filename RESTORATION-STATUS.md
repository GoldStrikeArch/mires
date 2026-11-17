# Restoration status: current main versus the lost v2

**Assessment date:** 2026-11-17.
**Compiler implementation inspected:** `933d5d0ad82010879c96c29d2cd56e8bba9fd6c5`.
**Main at the start of this assessment:** `ba72c43fc13366e46ff59a071a4e0585c41e1347`; the difference from the implementation commit is the recorded qualification receipt. Subsequent documentation commits do not change compiler behavior.
**Verdict:** the architecture is restored, important optimizer work has been reconstructed or replaced, and OSS bundle sizes are close to the last recorded October sizes. The larger remaining deficit is semantic coverage and integrated product/distribution qualification, not a missing compiler architecture. This is a development checkpoint, not a production release or an exact dev13 reconstruction.

The original late source and binaries remain missing. There is no defensible single percentage for “v2 restored.” Different dimensions have different answers: bundle size is close; some optimizer mechanisms are new or stronger than the old documented implementation; several ordinary Rust operations that belonged to the late restoration goals still fail; and current browser/worker/prebuilt evidence is narrower than the retained October evidence.

## 1. Scope and evidence rules

This assessment reads the actual main source, Git history, Round 3 verifier records, the exact retained artifacts, and the original October qualification receipts. It separates three kinds of evidence:

| Evidence class | Meaning in this report |
|---|---|
| Fresh execution | Executed during this assessment against the installed main compiler or exact retained JavaScript artifacts. |
| Retained execution | A named earlier verifier/coordinator run whose source identity, scope, and limitations are retained. It is not relabeled a new execution. |
| Historical waypoint | A pre-loss result or capability record. Without its exact old inputs and compiler, it is not a controlled before/after benchmark. |

The machine-readable companion is [the main-state receipt](docs/main-restoration-status-2026-11-17.json). The retained independent Round 3 reviews are preserved under [research/v2-restoration/round3](research/v2-restoration/round3/README.md). The complete project narrative is [current_state.md](current_state.md); its original November 12 sections are historical, with a new current-checkpoint notice.

This task commits the pending documentation, the status assessment, and relevant verification records. It does not merge unfinished correctness branches, change the compiler's semantic contract, rewrite incorrect expected outputs, or push to a remote. New native controls and small capability probes are evidence, not new accepted compiler support.

## 2. What actually landed on main

The progression is no longer “early reconstruction plus a promising side branch.” Main includes the next-generation optimization work, the qualification repairs, the previous main improvements, and the selected Round 3 combination.

| Checkpoint | Role |
|---|---|
| `16aafc88` | Earlier main with the float-format route, load-safe BigInt work, and development-test/build improvements. |
| `129be854` | Next-gen optimization branch. Large measured wins, but independently demonstrated cleanup and admission defects. |
| `dfdcf128` / `44161c2` | Qualification implementation and receipt. Repairs the demonstrated name-collision, transitive-call, and numerical failures; integrates earlier main. |
| `895ed1c` / `0a65083` | First combined Round 3 implementation and receipt, including proposed runtime fast paths. |
| `933d5d0` / `ba72c43` | Accepted Round 3: retain the compiler changes and i64 peepholes; remove the runtime-shim changes after measurements did not justify them. |

The installed main linker hashes to `5d7ef8123479aee8d7066f821966b7ab6760ec42d981af2054c18cca47217189`, matching the independent Round 3 rebuild and receipt. The runtime hashes to `0ea2a34ec10f68bbe0d1573dc3ccdde66700f2b9604e25cee73603b841d1497c`, identical to the qualification baseline. Main's backend hash differs from the worktree receipt; backend artifacts embed build-location-dependent information. The source identity and observed linker equality must not be expanded into an unverified claim that all binaries are reproducible across paths.

### 2.1 Retained next-gen and qualification work

The relevant changes include optimized MIR with unwind edges, conservative admission at non-unwinding host boundaries, removal of proven unnecessary exceptional edges, an independent pre-prune semantic checker and rollback, post-representation inlining, local multi-variant enum scalarization, slice-iterator lowering, i64/u64 pair lowering, constant hoisting, lexical runtime extraction, and string/heap runtime improvements.

The qualification wave removed privileged substring classification of user functions as panic helpers, distinguished trusted runtime provenance, followed transitive call/drop effects in the admission check, and repaired integer-to-f32 double rounding. Those fixes are on main, not merely in a saved experiment.

There is an important limitation: the independent cleanup-pruning checker is still conditional in ordinary release execution. Its existence and its successful checked tests do not establish unconditional enforcement. Section 7 describes this and the remaining receipt problems.

### 2.2 Selected Round 3 work

**R3-A: private call interfaces.** The physical ABI planner can flatten selected private mutable enum/scalar and shared-scalar arguments, reducing boxed slots and copy-back machinery where alias, escape, and exceptional behavior allow it. The focused opaque-helper enum experiment demonstrates a real new opportunity. It does not establish that every mutable enum helper is scalarized in complete programs.

**R3-B: loops and bounds.** Slice/string cursor recognition, provenance-preserving bounds reasoning, and dead-code simplification improve selected loop forms. The important condition is preserved source/bound provenance, not just a matching arithmetic expression.

**R3-C: inlining and cells.** Trivial wrappers and sole-caller sites are handled more effectively, some compiler-owned cells are unboxed, and tracked caller locations can be substituted in supported linker-inlining cases. This is not a complete repair of rustc MIR-inlined caller locations or function-pointer call shims.

**R3-D, selected part only.** The verified integer-pair peepholes remain. The additional runtime string/pointer/copy fast paths were removed. Retained attribution found roughly 150–370 additional gzip bytes per affected bundle without a convincing measured benefit, including slower measured workload arms. Keeping those changes merely because they were called fast paths would have been the wrong decision.

The accepted combination was rechecked after removal because runtime text participates in cost decisions. Seven core programs in one unwind configuration and one in abort changed optimizer decisions; the verifier checked the affected behavior rather than assuming a runtime-text change could not affect generated programs.

## 3. How close are the OSS packages to October?

### 3.1 The numerical gap is now small

The pre-loss October dev12 waypoint recorded compact at 147,950 minified / 44,626 gzip bytes and idiomatic at 265,049 / 74,359. These values are preserved in the project history, including the original section 15 of [current_state.md](current_state.md). They are not new measurements of a recovered dev12 binary.

The current Round 3 artifacts were freshly rehashed and recompressed during this assessment:

| Package / metric | October dev12 waypoint | Current Round 3 | Current minus October |
|---|---:|---:|---:|
| Compact minified | 147,950 B | **155,681 B** | +7,731 B; +5.23% |
| Compact gzip | 44,626 B | **47,856 B** | +3,230 B; +7.24% |
| Idiomatic minified, current default | 265,049 B | **267,044 B** | +1,995 B; +0.75% |
| Idiomatic gzip, current default | 74,359 B | **78,583 B** | +4,224 B; +5.68% |

Measured from today's output, reaching those gzip waypoints would require another approximately **6.75% compact** and **5.38% idiomatic** reduction. That arithmetic is useful for prioritization, but it is not a promise that one historical pass will supply the gap.

The old/current inputs are not fully matched: source snapshots, packaging, compiler policies, and compression implementations are not all proven identical. The old gzip figures were checked with a different tool path from the current Node zlib measurement. Therefore these rows show proximity of output size, not a controlled compiler win/loss or an implementation-completion percentage.

### 3.2 Progress from the reconstructed baseline is large

| Package | Reconstructed R0 min / gzip / Brotli | Current min / gzip / Brotli |
|---|---|---|
| Compact | 515,481 / 110,636 / 80,491 | **155,681 / 47,856 / 36,762** |
| Idiomatic | 822,496 / 186,452 / 135,211 | **267,044 / 78,583 / 63,111** |

The current default idiomatic artifact compresses better than the separately measured Rust `-Oz` artifact, which is 267,497 minified / 79,136 gzip / 63,586 Brotli bytes. A smaller raw program or a stronger size flag does not automatically minimize transfer bytes.

The 14-row receipt includes duplicate compact configurations and alternative idiomatic configurations. Its aggregate gzip reduction of 5,741 bytes versus the qualification baseline is useful accounting, not the bytes saved by one deployed application.

### 3.3 Corpus results

| Reduced workload | Current minified | Current gzip |
|---|---:|---:|
| marked | 25,970 | 9,825 |
| minisearch | 46,760 | 14,314 |
| fflate | 12,049 | 4,977 |
| uplot | 1,877 | 1,057 |
| idb | 23,618 | 8,692 |
| idb_lines | 25,947 | 9,518 |
| ky | 26,715 | 9,600 |
| explorer | 80,626 | 25,339 |
| workbox | 25,510 | 8,656 |
| workbox_direct | 5,104 | 2,097 |

The sum is 274,176 minified / 94,075 gzip bytes for ten separately packaged programs. Ky is the remaining +58 gzip-byte regression versus qualification, attributed by the retained verifier to over-inlining a Result branch. The other listed packages are smaller.

These are reduced diagnostic implementations, not full API-equivalent replacements for all the upstream libraries named by the directories. Nor is their sum comparable without qualification to an earlier full deployment or a corpus built with different input sources.

## 4. Where the restored compiler is ahead, close, and behind

| Dimension | Assessment | Meaning |
|---|---|---|
| Core architecture | Restored | Real rustc frontend, semantic objects, independent linker, analyses, lowering, runtime extraction, maps and manifests exist as working source. |
| OSS package density | Close to the recorded late waypoint | Roughly 5–7% gzip above the two October OSS numbers, subject to input/configuration caveats. |
| Selected optimizer mechanisms | Reconstructed or extended | Optimized-MIR unwind consumption, checked pruning machinery, integer pairs, local enum PEA and private mutable ABI work are substantial capabilities; they are not a byte-exact reimplementation of the lost code. |
| Runtime performance versus the reconstructed compiler | Major diagnostic improvement | The large compute/string wins are supported by current and retained evidence, but no matched old-dev13 runtime comparison exists. |
| General semantic coverage | Still behind the intended late capability set | Aggregate copies, views, constants, shared ownership, mutations and caller-location combinations remain incomplete. |
| Optional union strings | Not restored as the old feature | Current byte-view/memoization improvements are not the old `js-str=union` implementation, admission rules and literal pinning. |
| Broad constant pooling and trailing arguments | Incomplete | Current scalar-table/Location/BigInt/factory hoists and ABI simplification are useful, but not equivalent to every old pooling or escaping-function-table repair. |
| Current Pro/worker qualification | Not established | Source/reference recovery and earlier builds survive; latest next-gen/Round 3 receipt does not provide a new full Pro/worker behavioral and economic matrix. |
| Browser and clean consumer qualification | Behind the old scoped evidence | October had broader identified Chrome/Safari/Firefox and clean-prebuilt observations, although not a completed production release. |
| Topcoat islands/keyed UI | Deliberately outside standalone restoration | Do not restore framework coupling just to make an old framework-specific checklist green. Preserve equivalent generic callback, worker, lifecycle and packaging obligations. |

The practical conclusion is not “almost nothing is left” or “we still need to rewrite everything.” The expensive architecture is back. The remaining work is concentrated in representation invariants, safety/admission enforcement, selected missing optimizations, and reproducible qualification.

## 5. What the lost compiler actually achieved

The latest retained original candidate is **dev13, October 7**, revision `b2e8718dbfe54422bf6a5cb856b9d315f10a08bd`, identity `dd9c6d8a77b0bd7f903ff4d9f327eb84023ee5302ae205131cfe9508059eb680`. The OSS size waypoints above refer to dev12, not a newly invented dev13 size table.

The four original receipts reopened for this comparison are:

- `experiments-2026-09-05/rc-step5-dev13/receipt.json`
- `experiments-2026-09-05/rc-step5-dev13-firefox/receipt.json`
- `experiments-2026-09-05/p08-keyed-step5-dev13/receipt.json`
- `experiments-2026-09-05/p14-prebuilt-step5-dev13/receipt.json`

They record scoped Node and browser lifecycle work, a Firefox supplement to Chrome/Safari runs, packaging under three release profiles, authored-map delivery checks, and a clean-archive prebuilt run reporting 203/203 fixtures. Stable consumer/profile-switch/multicrate workflows had meaningful coverage. Current standalone work should recover those generic delivery properties rather than only the inner compiler's unit-test count.

They also explicitly record limitations. `productionQualified` is false. Where present, `timingAuthority` is false. The protected runtime protocol was not run, executable debugger work was incomplete, prebuilt Firefox/procedure cells were not completed, an authorized public signing identity was absent, and a nightly-compatibility arm was not run. Some realm/cleanup browser arms used cleanup off; the receipt distinguishes them from cleanup-on islands and keyed pages.

Thus the old target was a strong research/development compiler with valuable demonstrated behavior, not a flawless finished product. The current project should not reproduce its defects or adopt an old “only qualification remains” claim as a fact.

The original late source archive described by the prebuilt receipt was not recovered. The source-recovery frontier remains documented in `recovery/v2/MISSING-OCTOBER-SOURCE.md`. Its 125 missing September 23 file identities are not a complete inventory of October's changes.

## 6. Verification and remaining correctness

### 6.1 Fresh checks in this assessment

The exact outcomes, modes and log hashes are recorded in [the status receipt](docs/main-restoration-status-2026-11-17.json). Artifact checks cover the precise retained Round 3 outputs; they are not relabeled as a new full application rebuild. The runtime checks execute current main's runtime. The compiler inventory runs use the installed current main binaries and explicit JIR settings.

| Fresh check | Result | Scope |
|---|---:|---|
| Explicit JIR + optimizer core fixtures, unwind | **174/174** | 168 executing programs, including nine expected runtime aborts, plus six intended compile refusals; no skips. |
| Same core configuration, abort | **174/174** | Same execution/refusal accounting; no skips. |
| Runtime subset | **88/88** | General runtime, native-value, host-future and shim tests. |
| Retained artifact hashes and recompression | **14/14** | Both compact configurations, both idiomatic configurations, ten corpus outputs. |
| Standard package kernels | **58/58 each** | Exact compact and default idiomatic artifacts; no browser collection. |
| October-derived / Round 1 / Round 2 inventories | **5/6, 2/5, 0/4** | Strict expectations preserved; failures remain open. |
| Additional historical-capability probes | **0/6** | Three new sources, optimizer off/on; native O0 and O3 agree for all three. |

The successful core gate must be named precisely: `JS_EXTRA_ARGS='js-lane=jir js-opt=on'`, witness checking enabled, non-incremental environment, in the two panic modes. It is not a claim that every release-profile/MIR-inlining combination passes. An intended diagnostic is a successful refusal test, not a successfully executed Rust program.

### 6.2 Retained independent Round 3 verification

Two independent reviews were repeated after the runtime fast paths were removed. Their PASS verdicts mean no newly incorrect behavior was found relative to the identified qualification base within their test scopes. They do not mean every Rust program tested was native-correct.

The correctness recheck includes 441 targeted runs over nine settings, 96 host-throw runs, and 200 generated programs over three settings. It explicitly labels cases identical to an already incorrect base separately from native-identical cases. The gate reviewer reproduced all 14 artifact rows and distinguished pass, refusal, skip, inherited failure and environment failure.

The retained workspace result is 1,203 Rust tests passed in the ordinary environment. Broad wildcard Node tests include inherited missing-jsdom failures; an abort-environment workspace run has a non-hermetic inherited failure. Those are not interchangeable with this assessment's passing 88-test runtime subset. The explicit JIR gate passes 174 cases per mode, whereas the default baseline lane skips cleanup-bearing programs and stronger MIR-inline/`-Oz` configurations still fail known rows.

### 6.3 Remaining semantic families

The persistent families include value copy versus aliasing; array rows and flattened/view references; static pointer projection and metadata; niche/constant/function representation; shared ownership and destruction; callable identity/caller location; and typed host effects. Fixing one manifestation is not a general proof of the corresponding memory model.

The Round 3 reviewers retain P1–P10 as base-identical defects, including skipped panic-handler behavior in one mode, wrong MIR-inlined caller locations, tracked function-pointer failure, lost ASCII mutations, remaining drops skipped after a throwing destructor in a specific configuration, a generic closure refusal, empty-array mutable-iterator failure, tuple clone failure, `Rc::make_mut` failure, and vector comparison failures under particular inlining settings.

Those findings should not be silently turned into Round 3 regressions. They also should not disappear from the restoration goal merely because both sides of a differential test are wrong. Reconstitute the late compiler's positive capability cases and require native/contract expectations, not only parent-version equality.

The three additional probes created for this assessment target boxed-slice construction, shared `Rc::make_mut`, and constant closure-to-function-pointer calls. They are newly written capability probes, not recovered byte-exact October fixtures. Their native O0/O3 expectations and current compiler outcomes are preserved alongside the report.

| New capability probe | Native O0/O3 result | Current JIR off and on |
|---|---|---|
| Copy a slice into `Box<[u32]>`, sum, mutate and drop | `3 / 6 / 2 / 9`, normal return | `3 / 0 / 2 / 9`, then invalid heap deallocation abort. |
| Clone `Rc<u32>`, mutate through `Rc::make_mut`, then mutate the unique owner | `9 / 7 / 1 / 10`, normal return | Byte-view size mismatch (12-byte owner versus 4-byte value) before output. |
| Constant noncapturing closure coerced to `fn(u32) -> u32` | `42` | `0`, normal return. |

These are concrete examples of functional restoration lagging bundle-size recovery. This assessment did not bisect their introduction, and no claim is made that Round 3 caused them. The late historical record included successful tests in these capability families, but the missing old compiler prevents executing these exact new programs against it.


## 7. The remaining safety and qualification gaps

### 7.1 Independent pruning checks are not mandatory in release

`jsc-link/src/opt/witness.rs` enables checks in debug builds or when `RCGJS_WITNESS_CHECK` is present. `prune_invoke::run_checked` skips the independent semantic checker otherwise. The previous re-audit demonstrated that injected pruning faults roll back only with checking enabled. Current source retains that conditional.

The mechanism is useful and the original counterexamples were repaired. Nevertheless, counting a recorded witness does not prove it was independently validated in a release link. Make the semantic check mandatory for exceptional-edge removal or explicitly choose a different assurance contract; do not describe the current default as unconditional checked rollback.

### 7.2 Timing success does not enforce timing validity

The current receipt records MiniSearch `aaWithinGate=false`, with A/A about 1.169 after three retries, while the outer timing/passed fields remain true. The driver computes the diagnostic flag but does not veto the run on failure; missing cells can be skipped.

The latest coordinator notes also identify bimodal MiniSearch module instances. Selecting only the faster mode after observing a run would not establish an unbiased application-speed result. Freeze a protocol that includes the relevant distribution, initialization/JIT regime, and all rejected attempts. Until then, mark that timing arm inconclusive. Large diagnostic improvements elsewhere remain useful, but the current receipt is not an authoritative universal runtime qualification.

### 7.3 The sysroot key omits an input that changes emitted code

`scripts/sysroot-cache.sh` binds the backend, patches, toolchain, target, Rust optimization, emit settings and unwind additions, but does not include `CARGO_INCREMENTAL`. The retained Round 3 review found that this setting changes rustc behavior and emitted output while sharing a cache key.

This is a build-integrity issue, not merely cosmetic reproducibility. Record the normalized incremental configuration, keep incompatible products apart, and add a cold/warm comparison that deliberately switches it. The present assessment uses the non-incremental environment of the deciding receipts and does not claim to have fixed the cache.

### 7.4 Baselines and gate receipts need stronger binding

The qualification driver reads old-main inputs from a mutable main checkout and an ignored historical benchmark location. After a promotion, the same script can compare against a different compiler without changing the label. Bind each arm to immutable sources, binaries, sysroots, settings, runtime, packaging inputs and artifacts.

The top-level receipt stores an initial active sysroot, not every arm's complete input identity. Test-gate counts and log hashes should accompany the benchmark receipt. Source/binary stability should be checked after the run as well as before. The new Round 3 fixtures 314–317 also need explicit coverage in the general unwind driver rather than relying only on their dedicated default-setting ABI script.

These are finite hardening tasks. They should be completed before calling the candidate a fully qualified restoration or publishing release-performance claims.

## 8. Runtime: what can and cannot be concluded

The current evidence shows substantial improvements over the reconstructed compiler. It does not provide a matched runtime comparison with a recovered dev13 executable, because that executable is missing and the old deciding timing protocol was not completed.

For the latest accepted Round 3 versus qualification, retained paired runs suggest fflate approximately 5% faster and marked approximately 2% faster, with the latter small relative to controls. Murmur improves approximately 20% on Node in the coordinator's repeated measurements; JSC is not comparably improved in that run and has bimodality. CRC/string-builder changes are within roughly 1%. These observations are narrower than the headline ratios against much older emitted programs.

The focused private-enum matrix is promising: the opaque helper falls from roughly 483 to 19 microseconds in V8 and 631 to 11 in JSC in the retained receipt. However, the combined `bench_kernels.rs` opaque-enum case remains near 499/623 microseconds in the same receipt. Program composition and selection matter. Report both, and test whether the private-ABI win generalizes to a real opaque helper in a complete workload.

The current default idiomatic package commits two native representations; compact commits zero. These counts do not mean the optimizer is doing nothing: inlining, scalar replacement, iterator and ABI changes deliver substantial benefits independently. They do mean native library/representation composition is not universally solved. A targeted unchanged-source native String/Vec experiment remains useful; it should not become an unmeasured replacement architecture.

## 9. What is still missing from the useful October work

### What — structural strings and safe materialization

Current string memoization, cached lengths, substring reuse and builder work restore important benefits. The old union-string option, its alias/ownership admission boundaries and literal pinning are not present as the same supported feature. The next test should establish which remaining String/as_str/as_bytes round trips dominate complete workloads, then compare representations without hiding retention, capacity, UTF-8 indexing or mutation costs.

### Why — source shape and inlining still affect recognition

The formatter/native String route has regained real package value. The default idiomatic artifact is smaller than its `-Oz` variant, and its representation selection differs. This is evidence for improving construction/use facts and cross-inlining transport, not for turning off correctness checks to force a carrier. Repeated native f64 output checks and custom Display/format-option declines must remain part of the acceptance conditions.

### Where — target the current machinery

Relevant implementations are `jsc-link/src/plan/representation.rs`, `lower/semantic_ops/fmt_kernel.rs`, `opt/inline.rs`, `analysis/valueflow.rs`, and the string/runtime boundaries. The previous architecture audits proposed useful experiments, but they do not supersede the now-landed compiler or prove a clean-slate rewrite necessary.

### What — pooling, trailing arguments and worker closure

Current constant hoists cover useful scalar tables, locations, BigInt constants and factories. They do not automatically implement the old repeated immutable referenced-table pooling with all host exposure, alias and escaping-function controls. Likewise, ABI simplification and DCE are not the lost dedicated whole-program trailing-argument pass. Restore the economic effect where it remains measurable, rather than reproduce a filename for its own sake.

Worker runtime closure and duplicate pointer/cast work have useful current foundations. The latest Round 3 receipt is not a Pro main/worker qualification. Rebuild that workload with matched source/contract identities and count transitive imports, not an isolated worker stub. Do not compare later Rust Pro behavior with a smaller port that omits the same product branches.

## 10. Recommended sequence from this checkpoint

| Priority | Deliverable | Completion condition |
|---|---|---|
| 1. Safety/build evidence | Mandatory exceptional-edge validation; honest timing status; incremental-aware sysroot identity; immutable baseline arms. | Negative controls fail for the intended reason, ordinary release execution cannot bypass required checks, and a repeat run is bound to the same inputs. |
| 2. Restore old positive capabilities | A compact contract matrix for boxed slices, Rc/Arc mutation, constants/function pointers, rows/views, copying, drops and caller locations. | Supported cases match native/target-contract expectations; unsupported cases are explicit refusals, not silent wrong answers. |
| 3. Close the remaining OSS size gap | Attribute current retained package cost; evaluate safe aggregate pooling, formatting/String commitment, boundary round-trip elimination and dead private arguments. | Benefits measured on exact full packages with selection and adversarial controls; no source-specific hash-kernel substitutions. |
| 4. Restore product-level confidence | Current OSS/Pro lifecycle and worker cases, a public non-fingerprinting holdout, three-engine coverage, maps and clean external consumer/profile-switch tests. | Same-candidate retained receipts, complete dependency closure and declared omissions. |
| 5. Decide a standalone release | Freeze the supported contract and toolchain; complete intended distribution and performance qualification. | Explicit admission decision; not inferred from fixture totals or old Topcoat release language. |

Correctness work can be bounded by invariants and declared supported cases. It need not wait for every theoretical pointer reinterpretation to be implemented, but it cannot redefine safe, accepted operations as correct merely because they match an incorrect parent compiler.

The original 30% size/15% p50/10% p95 north-star is a separate ambition from regaining the lost development state. Neither milestone should silently replace the other. Restoring a Topcoat UI feature is also not a standalone compiler requirement; generic multi-entry, callback, lifecycle and worker behavior is.

## 11. Committed state and continuation

The pending `tasks/todo.md`, `tasks/lessons.md`, `tasks/dev-speed.md`, and original `current_state.md` were first committed without changing their content, preserving the exact history and decisions available at the start of this assessment. This report and the updated current-status entry points are a separate documentation/evidence change.

The preserved review records contain source IDs, named baselines, retained limits and inherited defects. Their PASS labels mean scoped non-regression, not production admission. Raw logs, native executables and generated compiler artifacts remain in the separate audit backup; private Pro references and binary caches are not added to Git.

No unfinished branch was merged in this assessment. No compiler implementation was changed. No push was performed. A clean final Git state means the relevant current documentation is committed; it is not a certificate that every open compiler behavior passes.

## 12. Evidence index

| Source | What it supports |
|---|---|
| [Current machine-readable status](docs/main-restoration-status-2026-11-17.json) | Fresh command results, exact measured artifacts, source/binary identity, caveats. |
| [Round 3 preserved reviews](research/v2-restoration/round3/README.md) | Independent correctness and gate rechecks, original scope and limitations. |
| [Qualification/size receipt](experiments/qualification/next-gen-qualify-receipt.json) | Retained Round 3 package sizes, selection counts and diagnostic timing samples. |
| [Task ledger](tasks/todo.md) | Implementation sequence, discarded shim changes, verification and coordinator timing attribution. |
| [Historical/current narrative](current_state.md) | Recovery provenance, original baseline tables and older decisions. |
| [Restoration plan](V2-OPTIMIZATION-RESTORATION-PLAN.md) | R0/R1/R2 definitions and the intended correctness, performance, holdout and qualification obligations. |
| [Compiler contract](CONTRACT.md) | Admitted semantics, unwind/abort policy and declared limitations. |
| [Comparator guide](docs/COMPARATORS.md) | Restored other-language builds and their qualified scope. |
| Local original October receipts | Historical browser/prebuilt capability, explicit non-production/non-timing qualification. Hashes and selected fields are preserved in the status evidence. |

**Bottom line:** measured OSS density is now near the lost late-v2 waypoint, and the standalone compiler has recovered substantial optimization capability. The remaining restoration is primarily about coherent semantics and broader verified delivery, with a smaller but still worthwhile package-size gap. Do not equate that with exact source restoration or completed production qualification.
