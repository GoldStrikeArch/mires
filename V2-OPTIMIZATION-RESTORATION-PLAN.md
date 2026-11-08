# V2 optimization restoration plan

**Project:** `mires`, standalone Rust-to-JavaScript compiler  
**Plan date:** November 8, 2026  
**Status:** implementation plan; the work packages below are not execution receipts  
**Objective:** reimplement the useful late-v2 capabilities and demonstrate their combined benefits without weakening the supported Rust/host contract.

> Restore the capabilities and evidence, not the historical patch sequence. Preserve the recovered architecture. Establish correctness and a measurable cleanup-on baseline before accepting optimization wins. Measure compiler build cost, shipped bytes, JavaScript execution, and retained memory separately.

This plan follows the agreement to make **compiler build cost** and **one complete public, non-fingerprinting holdout** explicit requirements alongside bundle size and generated-program performance. It does not authorize publishing private Pro material, committing existing work, changing the release contract silently, or treating the lost October sources as recovered.

## Contents

1. [Outcome and fixed decisions](#1-outcome-and-fixed-decisions)
2. [Evidence and starting position](#2-evidence-and-starting-position)
3. [Baselines and completion milestones](#3-baselines-and-completion-milestones)
4. [Architecture and implementation boundaries](#4-architecture-and-implementation-boundaries)
5. [Work-package sequence and dependencies](#5-work-package-sequence-and-dependencies)
6. [Foundation and correctness work](#6-foundation-and-correctness-work)
7. [Cleanup restoration](#7-cleanup-restoration)
8. [Strings and formatting](#8-strings-and-formatting)
9. [Bounded output improvements](#9-bounded-output-improvements)
10. [Composition, comparisons, and qualification](#10-composition-comparisons-and-qualification)
11. [Experimental method and acceptance rules](#11-experimental-method-and-acceptance-rules)
12. [Evidence layout and receipt contract](#12-evidence-layout-and-receipt-contract)
13. [Execution discipline and first implementation batch](#13-execution-discipline-and-first-implementation-batch)
14. [Finish lines and deferred research](#14-finish-lines-and-deferred-research)
15. [Existing commands and historical targets](#15-existing-commands-and-historical-targets)

## 1. Outcome and fixed decisions

### What

Produce a standalone v2 compiler whose cleanup-on release configurations compile representative real Rust programs, preserve their declared behavior, and recover the useful late-stage gains in cleanup, string processing, formatting, constants, calls, and runtime reachability. Retain a reproducible experiment system that explains which transformation caused each improvement.

The result is not a byte-identical reconstruction of October dev13. It is a new, identified implementation based on recovered source, independently tested against executable contracts and the available historical evidence.

### Why

The active compiler already has the expensive foundation: a rustc producer, semantic object transport, an independent linker, SSA analyses, whole-program representation planning, and artifact closure. Replacing that foundation would repeat recovered work. Copying the newest-looking historical file for each path would instead combine incompatible intermediate and sometimes rejected experiments.

Several missing improvements are proof-composition problems, not missing architectural layers. Other important work is straightforward correctness or avoidable implementation cost. Those categories need separate tasks and separate success criteria.

### Where

The active workspace is the repository root. Immutable recovery inputs remain under [recovery/v2](recovery/v2/). This document is the canonical restoration roadmap; current verdicts belong in [VERDICT.md](VERDICT.md), and execution results belong in identified receipts. Do not use an old status document as an implicit release decision.

### Decisions to preserve

| Decision | Consequence |
| --- | --- |
| Keep the producer / semantic-object / independent-linker design. | No v3 rewrite is on the critical path. |
| Cleanup is on in release profiles. | Cleanup-off output is a separately labeled diagnostic or explicit opt-out, never an equivalent performance baseline. |
| Missing required semantic cleanup must fail closed. | No reachable fallback may silently discard destruction or exceptional edges. |
| The compiler is framework-independent. | Topcoat islands, hydration, reactive owners, keyed DOM, and view macros are not release gates here. Generic host, worker, async, and cancellation obligations remain. |
| Preserve the pinned rustc configuration during restoration. | Toolchain migration is a separate change with its own qualification. |
| Union strings are an experimental, opt-in reconstruction. | Reintroducing the option is not permission to make it the default. |
| Exact primitive semantics precede economic wins. | No ignored destructor, changed error, reordered host effect, or incorrect string boundary can be counted as optimization. |
| Measure the complete delivered dependency graph. | Runtime modules, workers, wrappers, shared chunks, and adapters are not free. |
| Public and private evidence remain separate. | Private Pro sources/artifacts must not enter public archives, source maps, CI logs, or commits accidentally. |
| Correctness repairs and optimization changes are distinguishable. | Establish a correctness-repaired baseline before attributing savings to later passes. |

There is no requirement that every small improvement reduce two complete application bundles by a fixed percentage. There is a requirement that an optimization's actual application and benefit be demonstrated, that important regressions be excluded, and that the combined candidate generalize beyond fingerprinting.

## 2. Evidence and starting position

### What

Use the latest standalone restoration records as the starting evidence. Preserve the October records as historical targets and design constraints, not as results for this checkout.

### Why

The current implementation is a coherent September 7 producer-integration snapshot with subsequent standalone repairs. It is neither the early imported backend named by Git HEAD alone nor the complete October candidate. Much of the active work remains uncommitted, so a Git revision by itself does not identify the current compiler.

### Where: evidence index

| ID | Local evidence | Use and limitation |
| --- | --- | --- |
| E01 | [Standalone restoration record](docs/V2-RESTORATION.md) | Implementation lineage and repairs. Its statement excluding other-language restoration describes that earlier change, not the latest repository state. |
| E02 | [Assembly verification](docs/v2-assembly-verification.json) | Scoped fresh assembly results, exact options, known failures, and interrupted optimized Pro attempt. |
| E03 | [Assembly source inventory](docs/v2-source-identity.json) | Recorded compiler source digest and individual file hashes; revalidate before creating R0. |
| E04 | [Current verdict](VERDICT.md) | Useful assembly verdict. Its final statement that other-language ports were not restored is superseded by E05/E06. Preserve the history and update the current summary in P00. |
| E05 | [Comparator guide](docs/COMPARATORS.md) | Restored families, profiles, build instructions, ownership qualifications, and repairs. |
| E06 | [Comparator restoration status](docs/comparator-restoration-status.json) | Fourteen builds and offline checks; explicitly not browser or timing qualification. |
| E07 | [Late-stage recovery ledger](recovery/v2/LATE-STAGE-WINS.md) | Historical mechanisms, numbers, and missing implementation work. Claims must be re-established on current sources. |
| E08 | [Missing October source ledger](recovery/v2/MISSING-OCTOBER-SOURCE.md) | Recovery frontier. Filling the September 23 missing list would not automatically recreate October. |
| E09 | [Later regression fixtures](examples/known-gaps/) and [strict runner](scripts/check-known-gaps.py) | Conversation-derived fixtures with native expectations; not byte-exact archived October source. |
| E10 | [Known-gap execution receipt](build/known-gaps/receipt.json) | Two of six pass in the recorded cleanup-on balanced configuration. Requires retained local evidence. |
| E11 | [Optimized Pro interruption](build/assembly/pro-optimized-interruption.json) and [live profile](build/assembly/pro-link-sample.txt) | Compilation-cost lead only. This is not a completed link duration or application runtime measurement. |
| E12 | [Compiler contract](CONTRACT.md), [JIR design](JIR.md), and [optimizer design](OPTIMIZER.md) | Existing design context; reconcile inherited framework language and historical limitations with the standalone contract. |

**Review scope for this plan:** the evidence and relevant source locations were inspected. Creating this plan does not rerun the suites, verify a new release candidate, or execute any work package below.

### Recorded baseline facts

| Area | Recorded result | Scope that must remain visible |
| --- | --- | --- |
| Rust tests | 1,021 passed, zero failed, seven ignored | Assembly candidate, not proof that every later mode works. |
| Recovered fixture suite | 138/138 | Explicit JIR + optimization, cleanup off; not release-profile qualification. |
| Generic runtime | 40/40 | Runtime tests, distinct from compiled application tests. |
| Active host-call cleanup | 40/40 | Cleanup on, optimization on/off, scoped active-call/drop cases. |
| Later regression targets | 2/6 | Four open cases under cleanup-on `release-balanced`. |
| Reduced browser corpus | Ten builds | Node checks ran for four workloads; no complete three-browser qualification. |
| Compact Rust OSS | Optimized, cleanup-on package builds; 58 kernel comparisons pass | Historical size gates fail; full browser behavior has not been requalified. |
| Idiomatic Rust Pro | Cleanup-on optimizer-off main and worker build | Optimized attempt was interrupted; package/import checks do not prove full Pro behavior. |
| Other-language implementations | Fourteen builds; 62 offline check commands pass | Seven families, OSS and Pro. Default artifact aliases are not independent implementations. |

Recorded assembly source digest: `eed71e26087504afedae82418024875b11a0b19fff20b2707053817adfc27505`.

Git base at plan creation: `4ab4bd6b32751c861a3720eafebe77ce1b8975ba`. This is the original import commit, **not the identity of all current uncommitted source**.

The open cases are `195_zst_array_drops`, `audit_zst_closure_drop`, `audit_zst_vec_drop`, and `audit_aggregate_place`. The other two later probes pass only in the configurations actually recorded. Do not fix an already passing case based solely on an older October narrative; expand its matrix instead.

## 3. Baselines and completion milestones

### What

Create two immutable baseline records and one final restoration candidate. Distinguish implementation progress, optimization restoration, and production qualification.

### Why

A correctness fix can increase code size. Comparing later optimization against a smaller but incorrect program rewards the wrong behavior. Likewise, combining an optimizer-off Pro control with an optimized OSS candidate hides configuration differences.

### Where

Proposed baseline registry: `docs/optimization-restoration/baselines.json`. Proposed public records: `experiments/v2-restoration/`. Proposed private records: `experiments-pro/v2-restoration/`. These paths are **deliverables to create**, not tools that already exist.

| Identity | Definition | What it may establish |
| --- | --- | --- |
| R0 | Exact current reconstructed source, dependencies, binaries, workload sources, options, and known failures. | Preservation and before-repair diagnosis. An incomplete build remains incomplete. |
| R1 | Correctness-repaired, operational cleanup-on baseline: admitted known-gap cases resolved, optimized Pro build completes under the frozen resource budget, public holdout implemented and frozen, required behavioral controls pass. | Main reference for optimization economics. This is a tested scope, not a theorem of correctness for all Rust. |
| R2 | Integrated restoration candidate after selected work packages and interaction testing. | Demonstrated restoration of generic benefits with one identity and explicit remaining limits. |

For each optimization, compare the candidate to both its immediate parent and R1 on identical workload inputs. Preserve R0 separately. When a new correctness fix is needed after R1, produce a paired correctness-only control or a versioned successor such as R1.1; retain the original R1 and explain why a baseline changed. Do not silently overwrite its data.

Milestone M0 is a sealed R0 and executable experiment controls. M1 is R1. M2 is restored cleanup behavior with economic evidence. M3 is validated string/formatter and bounded-output improvements. M4 is the combined R2 comparison. Production release admission is a later, separate gate.

## 4. Architecture and implementation boundaries

### What

Use the existing division of responsibility. Add only the facts and mechanisms needed by an evidenced work package.

### Why

Semantic identity, cleanup obligations, and source origins can be lost between phases. Fixing their transport is preferable to recognizing workload names in generated JavaScript. Conversely, a printing-only change need not require a new semantic representation framework.

### Where

| Area | Existing implementation | Responsibility during restoration |
| --- | --- | --- |
| Rust producer | [backend/src](backend/src/) | Exact instantiated operation/type identity, source MIR, constants, cleanup obligations, and explicit declines. |
| Cleanup bridge | [rustc-cleanup](rustc-cleanup/) | Pinned audited rustc adaptation; preserve source/commit checks. |
| Shared contract | [rcgjs-object/src](rcgjs-object/src/) | Versioned object/IR schema, profiles, semantic operations, origins, runtime and artifact identities. |
| Link analyses | [jsc-link/src/analysis](jsc-link/src/analysis/) | Effects, references, calls, legality, ownership/value flow, and summary verification. |
| Optimization | [jsc-link/src/opt](jsc-link/src/opt/) | Verified body transformations and their witnesses. |
| Planning/economics | [jsc-link/src/plan](jsc-link/src/plan/) | Representation/ABI selection, rollback, and complete-program cost comparison. |
| Lowering/emission | [jsc-link/src/lower](jsc-link/src/lower/) | Preserve effects and exceptional boundaries while emitting legal JavaScript. |
| Runtime | [runtime/shim.js](runtime/shim.js) | Value storage, UTF-8, exceptions, lifetimes, and selected helper implementations. |
| Consumer build | [mires-build/src](mires-build/src/), [scripts](scripts/) | Source/binary/sysroot invalidation, deterministic packaging, and consumer ergonomics. |
| General host bindings | [mires-js](mires-js/) | Host interoperation and generic async/cancellation seams; not framework-specific ownership. |

`jsc-link/src/plan/cleanup.rs`, `jsc-link/src/link/trailing.rs`, and a `js-str=union` mode are proposed reconstruction locations/features, not currently existing implementations. A task may choose a better module boundary after design review, but must not describe a proposed file as recovered source.

## 5. Work-package sequence and dependencies

All tasks start **not started under this plan**, even where they extend working code. A prior test result is evidence for the starting point, not completion of the expanded task.

| ID | Work package | Prerequisites | Priority |
| --- | --- | --- | --- |
| P00 | Seal R0 and reconcile current documentation | None | First |
| P01 | Measurement, attribution, and executable pass controls | P00 | First |
| P02 | Correct zero-sized storage and destruction | P00; P01 for expanded matrix | Critical |
| P03 | Place/reference identity and remaining semantic inventory | P00; P01 for expanded matrix | Critical |
| P04 | Bound repeated optimizer/linker work | P01 | Critical, parallel to P02/P03 |
| P05 | Public document-processing worker holdout | P00; P01 for evidence format | Early, parallel |
| P06 | Seal correctness-repaired operational baseline R1 | P01–P05 | Milestone M1 |
| P07 | Prove and remove unnecessary cleanup edges | P06 | Main optimization track |
| P08 | Optimize bodies that retain cleanup | P06; integrate after P07 | Main optimization track |
| P09 | Preserve cleanup/proofs through protected and cross-crate inlining | P06; coordinate P07/P08 | Main optimization track |
| P10 | Repair string scaling under the default policy | P06; P05 input corpus | Main optimization track |
| P11 | Reconstruct optional union strings and bounded literal pinning | P03, P10; relevant P18 schema work | Experimental until admitted |
| P12 | Restore formatter recognition and carrier commitments | P06; coordinate P08/P09/P10 | Main optimization track |
| P13 | Pool eligible immutable referenced constants | P03, P06 | Bounded parallel work |
| P14 | Eliminate eligible dead trailing arguments | P06 | Bounded parallel work |
| P15 | Improve numerical printing and redundant terminal emission | P06 | Bounded parallel work |
| P16 | Recover worker/runtime and pointer-view savings | P03, P06 | Bounded parallel work |
| P17 | Verify interactions and proof transport, including typed storage | Relevant passes landed | Integration |
| P18 | Complete schema, build-cache, and delivery integrity | Start after P00; ongoing | Cross-cutting |
| P19 | Requalify comparison contracts and choose production artifacts | P00/P01; begin early | Comparison track |
| P20 | Run combined semantic/economic matrix and freeze R2 | P07–P19 decisions recorded | Milestone M4 |
| P21 | Close restoration, preserve evidence, separate release work | P20 | Finish |

The practical critical path is **P00 → P01 → {P02, P03, P04, P05} → P06 → cleanup/string/formatter restoration → P17/P20 → P21**. P18 and P19 run alongside it. Do not block compiler fixes on all seven comparator browser suites; unqualified comparators can be excluded from ranking without blocking same-Rust experiments.

Research may be prototyped before R1, but economic promotion happens against R1. Missing infrastructure is not a reason to report an unmeasured prototype as a restored optimization.

## 6. Foundation and correctness work

### P00 — Seal R0 and reconcile current documentation

**What.** Capture the exact active reconstruction and all inputs needed to interpret it. Preserve the current uncommitted work without automatically committing, staging, or resetting it. Reconcile the current documentation so later comparator restoration is visible without rewriting historical receipts.

**Why.** The Git import commit alone does not describe the active compiler. Build outputs, private sources, and recovery material are ignored in different ways. A backup of tracked files alone would repeat the earlier evidence loss.

**Where.** Start with [docs/v2-source-identity.json](docs/v2-source-identity.json), [docs/v2-assembly-verification.json](docs/v2-assembly-verification.json), [docs/comparator-restoration-status.json](docs/comparator-restoration-status.json), [Cargo.lock](Cargo.lock), [package-lock.json](package-lock.json), [rust-toolchain.toml](rust-toolchain.toml), and [scripts/sysroot-cache.sh](scripts/sysroot-cache.sh). New baseline records belong in the proposed registry from section 3.

**Deliverables.** Hash compiler sources, manifests, lockfiles, patches, selected procedural macros, actual backend/linker binaries, sysroot product contents, runtime sources, packaging/harness sources, workload sources, input corpora, and generated artifact closures. Separate an implementation-content identity from Git provenance and documentation metadata. Store the normalized effective options as well as the command line. Preserve public and private archives separately; include a manifest and restoration instructions. Record whether a verified backup exists off-device; do not label a same-disk copy off-device.

**Acceptance and controls.** A verifier rejects a changed runtime, linker, sysroot member, macro, workload source, harness, or selected artifact. A Git-only metadata change can be recorded as a new provenance identity with explicit content equivalence, not silently accepted by the old checker. Restore a small public consumer from the archive and verify its input inventory. Inventory ignored private Pro inputs without publishing their contents.

**Stop condition.** Missing or unbound inputs make R0 incomplete. Preserve that state; do not synthesize an original receipt or claim an exact October candidate.

### P01 — Measurement, attribution, and executable pass controls

**What.** Build the minimum experiment plumbing for semantic, compile-cost, emitted-code, and runtime comparisons. Give every reconstructed transformation a reliable on/off or observation mechanism and stage-specific decision counts.

**Why.** A passing program may have bypassed the intended optimization. The current `js-opt-passes` mask enumerates selected legacy/backend options; it must not be assumed to disable every independent-linker transformation. Current package tools also use more than one compression implementation, so their numbers need a single declared comparison protocol.

**Where.** Extend [rcgjs-object/src/opts.rs](rcgjs-object/src/opts.rs), [rcgjs-object/src/optimizer.rs](rcgjs-object/src/optimizer.rs), [jsc-link/src/opt/pipeline.rs](jsc-link/src/opt/pipeline.rs), [jsc-link/src/link/lane.rs](jsc-link/src/link/lane.rs), [scripts/test.sh](scripts/test.sh), and [experiments/tools](experiments/tools/). Reuse [scripts/artifact-manifest.mjs](scripts/artifact-manifest.mjs) and [scripts/minimal-shim.mjs](scripts/minimal-shim.mjs).

**Deliverables.** Record candidates recognized, proved legal, priced, selected, emitted, declined, and rolled back, with stable reasons and instantiated item identities. Distinguish semantic declines from economics and verification failures. Add strict timeout/child-process cleanup, input-stability checks, baseline identity checks, nonempty-case assertions, and retained failed-attempt directories. Report semantic, size, structural, and runtime checks separately rather than skipping later checks after the first budget failure.

**Acceptance and controls.** An unknown pass name fails immediately. A positive fixture has nonzero intended selection; disabling that pass removes its application. Report-only mode does not rewrite program output. A deliberately broken transformation is rejected by behavior/witness checks. A mismatched artifact and an empty test selection fail. Run the control under minified ESM and a cross-crate configuration, not only a handwritten runtime call.

**Stop condition.** A test that passes with zero relevant applications cannot certify that transformation. Do not loosen profitability merely to make its semantic guard test select; use a separate deterministic pass-level test and a separate economic fixture.

### P02 — Correct zero-sized storage and destruction

**What.** Repair zero-sized arrays and nested owning values across local storage, returns, closures, vectors, and other admitted aggregate contexts. Preserve logical element count, initializedness, move state, and destruction even where physical data bytes are absent.

**Why.** Three current later-regression cases crash during ordinary destruction. This is correctness work, not a size-budget issue. A fix for one local array shape is insufficient when the same type can be captured or placed in a container.

**Where.** Start from [examples/known-gaps](examples/known-gaps/), [backend/src/rvalue.rs](backend/src/rvalue.rs), [backend/src/place.rs](backend/src/place.rs), [backend/src/ptr.rs](backend/src/ptr.rs), [jsc-link/src/lower/js](jsc-link/src/lower/js/), [jsc-link/src/lower/semantic_ops/drop.rs](jsc-link/src/lower/semantic_ops/drop.rs), and [runtime/shim.js](runtime/shim.js). Diagnose the producer, shared representation, and drop glue before selecting the repair layer.

**Deliverables.** A representation/drop design for zero-sized aggregate values; regression tests for `[Z; N]`, nested arrays, closure capture, `Vec<[Z; N]>`, zero-length arrays, moves, replacement, partial initialization, and normal/exceptional destruction. Add a large logically sized case so a fix does not unnecessarily allocate one JavaScript object per element. Preserve `len`, indexing, and bounds behavior without reading nonexistent buffers.

**Acceptance and controls.** The three failing destruction cases and the expanded valid native-oracle cases pass with optimization on/off, MIR inlining on/off, and all admitted release profiles. Drop traces check exact count/order and cleanup after a host throw. A mutation that skips or duplicates an element drop is detected. Zero-sized address observations are checked against the supported contract; do not require distinct physical addresses for zero-sized elements.

**Stop condition.** Do not erase drops, change expected traces, allocate a shared mutable element accidentally, or convert accepted safe programs into runtime crashes. An interim explicit diagnostic may prevent a miscompile, but it is not completion of this restoration task.

### P03 — Place/reference identity and the remaining semantic inventory

**What.** Establish coherent admitted storage identity across aggregate replacement, field/element projections, references, raw-pointer comparisons, and exposed-address operations. Inventory other late repairs and test before assuming they are missing.

**Why.** `audit_aggregate_place` differs from the recorded native expectation, while two other pointer/reference probes already pass. Recreating an older broken pointer patch or special-casing equality could regress those passes. The lost work also included Box-from-slice, `Rc`/`Arc::make_mut`, constant closure-to-function-pointer conversion, and cleanup-aware dyn upcasts.

**Where.** Use [examples/known-gaps](examples/known-gaps/), [backend/src/ptr.rs](backend/src/ptr.rs), [backend/src/unsize.rs](backend/src/unsize.rs), [backend/src/vtable.rs](backend/src/vtable.rs), [backend/src/constant](backend/src/constant/), [rcgjs-object/src/ptr.rs](rcgjs-object/src/ptr.rs), [jsc-link/src/lower/js/ptr.rs](jsc-link/src/lower/js/ptr.rs), and [runtime/shim.js](runtime/shim.js).

**Deliverables.** A small storage/identity invariant document and tests spanning assignment, same-place borrows, scalar and aggregate fields, slices, closures, casts, and legal interior mutation. Add targeted probes for the other late repairs, classifying each as already working, failing, unsupported with diagnostic, or not yet exercised. Record the exact domain of integer-address comparisons and cross-crate representation conversions.

**Acceptance and controls.** Reproduce and explain the aggregate discrepancy; fix the admitted case without changing its expected observation. Preserve the two passing probes and consistent thin-pointer observations. Test both MIR settings and independent-crate boundaries. Native oracles compare meaningful observable relations, not host-specific numerical addresses, allocator choices, target-width assumptions, or undefined behavior. Include reused allocations and zero-offset projections as separate cases.

**Stop condition.** Do not replace a coherent model with opcode-specific tricks, globally relax the pointer contract, or apply an unreviewed array-representation rewrite. A proposed restriction must be explicit and diagnosed; it cannot be used to claim restored support for a required case that was merely excluded.

### P04 — Bound repeated optimizer and linker work

**What.** Instrument and reduce repeated analysis, lowering, name reservation, printing, and compression during candidate-cost evaluation. Separate behavior-preserving implementation acceleration from a change in the optimizer's economic policy.

**Why.** The cleanup-on optimized Pro link was interrupted after 9 minutes 38 seconds. Its retained sample points to repeated lowering inside physical-cost measurement. `lower/fuse.rs` also reserves all interned strings for each function containing `Invoke`. Repeating large-table work across functions and trials is a plausible source of excessive cost, but its total contribution still needs measurement.

**Where.** Inspect [jsc-link/src/plan/physical_cost.rs](jsc-link/src/plan/physical_cost.rs), [jsc-link/src/plan/cost.rs](jsc-link/src/plan/cost.rs), [jsc-link/src/link/lane.rs](jsc-link/src/link/lane.rs), [jsc-link/src/lower/fuse.rs](jsc-link/src/lower/fuse.rs), and [rcgjs-object/src/jir/interns.rs](rcgjs-object/src/jir/interns.rs). E11 is the profiling lead.

**Deliverables.** Per-phase time, CPU and peak-memory observations; counts of functions lowered, interned names considered, trials priced, bytes compressed, and cache hits/misses. Compare the existing cache against safe shared immutable lookup structures and dependency-aware reuse of unchanged lowered functions. Preserve collision avoidance for identifiers/free references introduced by emitted code. First prefer an equivalent implementation that produces the same decisions and bytes. Evaluate budget or search-policy changes in a separate arm.

**Acceptance and controls.** Cached and uncached modes agree on a bounded exhaustive suite, emitted outputs, decisions, and dependency closure. Mutating a body, representation label, callee effect, profile, naming input, or helper dependency invalidates affected entries. The optimized Pro program completes under a predeclared resource envelope with cleanup enabled. Smaller benchmarks show reduced repeated-work counts and no unexplained memory growth. Record cold and warm cache states separately.

**Stop condition.** Do not call an interrupted run a completed duration, infer a numerical speedup from it, drop exact-cost checks silently, or replace repeated work with an unbounded persistent cache. A heuristic search limit must report skipped candidates and be qualified as a policy change.

### P05 — Public document-processing worker holdout

**What.** Implement one complete, public, non-fingerprinting application workload with Rust-owned document parsing, search indexing/querying, formatting, and a worker protocol. Provide a deterministic JavaScript reference and a native Rust control for the applicable behavior.

**Why.** Recovered fingerprint ports are important integration targets but cannot establish generality by themselves. The ten-workload corpus is diagnostic and reduced. A frozen end-to-end program can expose interactions among strings, allocation, formatting, calls, package closure, and task lifetime.

**Where.** Proposed home: `experiments/document-workbench/`, with `CONTRACT.md`, authored Rust sources, an independent reference, public synthetic fixtures, a browser/Node harness, and a worker adapter. Reuse ideas from [experiments/browser-workloads](experiments/browser-workloads/) but do not advertise its Markdown/search subsets as full upstream implementations.

**Deliverables.** Specify request/response schemas, stable result ordering, document IDs, supported text syntax, valid and malformed inputs, checksumable output, incremental update/delete operations, and explicit cancellation/error behavior. The Rust implementation owns the application algorithms; JavaScript may provide actual host I/O and worker messaging, not a hidden application implementation. Use deterministic generated or clearly licensed text, including Unicode, BOMs, empty input, large inputs, and repeated/distinct documents. Freeze sources and the tuning corpus before optimizer work. Keep a second, previously unused input set for the final check.

**Acceptance and controls.** End-to-end reference agreement includes values, worker messages, errors, cancellation, and repeat-run lifetime behavior. A mutation that omits an update, swallows an error, or returns a stale result fails. Measure full main/worker deployment, cold startup, warm operations, and retained memory. Scaling claims distinguish source-algorithm work from compiler-added encoding/copying. Cancellation on hard worker termination is not assumed to execute Rust destructors.

**Stop condition.** Do not rewrite the workload to favor a pass, change its contract after measuring, or call a visible frozen corpus a blind holdout. New application versions get new identities. Ports to seven other languages are not a prerequisite.

### P06 — Seal the operational baseline R1

**What.** Freeze the correctness-repaired baseline, the performance protocol, the public holdout, and the representative cleanup-on application builds.

**Why.** Optimization attribution needs a stable, executable comparator. R0's missing optimized Pro result and known incorrect programs cannot serve as the complete economic baseline.

**Where.** Use the P00 registry, P01 harness, P02/P03 tests, P04 measurements, and P05 application. Include both idiomatic and compact OSS Rust ports, the idiomatic Pro main/worker, the ten-workload corpus, and declared representative subsets of the additional Rust variants.

**Deliverables.** Build and test the baseline under all three release profiles, with MIR inlining configurations recorded. Run the full restored fixture semantics in cleanup-on mode as well as historical controls. Split historical size gates from behavior so a byte-budget failure does not hide an unrun semantic/structural check. Capture the effective supported scope, work left open, compile-resource budgets, fixed inputs, and package-size vectors.

**Acceptance and controls.** Required known-gap cases are correct; no new admitted semantic mismatch is accepted. Optimized cleanup-on Pro completes. The holdout's contract and initial reference behavior are stable. Failed-build receipts are rejected, and benchmark artifacts match the exact passing semantic artifacts. A second clean input capture reproduces the identities or explains declared nondeterminism.

**Stop condition.** Do not seal R1 while a required normal-destruction case still crashes, while the representative optimized Pro link is incomplete, or while the public holdout exists only as a microkernel. R1 is a finite tested baseline, not a promise of every Rust library working.

## 7. Cleanup restoration

### P07 — Prove and remove unnecessary cleanup edges

**What.** Classify exceptional edges using verified effects and cleanup obligations. Simplify protected operations only when the transformation preserves every admitted exceptional outcome.

**Why.** The recovered producer conservatively retains substantial cleanup machinery. Some edges have no observable cleanup work or lead from operations that cannot throw under the declared contract. Removing those edges may also unlock ordinary lowering and optimization. Unknown effects, however, cannot be treated as non-throwing to obtain that benefit.

**Where.** Existing inputs are [jsc-link/src/analysis/effects.rs](jsc-link/src/analysis/effects.rs), [jsc-link/src/analysis/host_contract.rs](jsc-link/src/analysis/host_contract.rs), [jsc-link/src/link/lane.rs](jsc-link/src/link/lane.rs), [jsc-link/src/lower/js/tests/invoke.rs](jsc-link/src/lower/js/tests/invoke.rs), and [backend/src/coroutine_source.rs](backend/src/coroutine_source.rs). A focused `jsc-link/src/plan/cleanup.rs` is a proposed addition.

**Deliverables.** A conservative effect/cleanup classifier, explicit proof records for each removed edge, and a simplifier that handles empty cleanup chains without swallowing or changing the original exception. Unknown direct/indirect calls, host getters, callbacks, coercions, and unsummarized foreign bodies retain protection. Recursive summaries require a sound fixed point. Distinguish a registered abort from an ordinary JavaScript exception and a destructor exception.

**Acceptance and controls.** Eligible no-throw and empty-obligation examples lose the intended edges; required edges remain. Test returned values, object identity of thrown tokens, effect order, partial initialization, moved arguments, nested cleanup, and destructor behavior. Include mutations that wrongly label a throwing host call as pure or omit one live destructor. Full-package reports show removed handlers and their reached-helper consequences, not only counts of `try` keywords.

**Rollback/stop.** Verification failure restores the original body and records a reason. A summary conflict or imported-identity mismatch is not resolved by accepting the optimistic result. No cleanup-off build may be used as proof that this transformation preserves behavior.

### P08 — Optimize bodies that still require cleanup

**What.** Allow safe SSA simplification, dead-code removal, projections, fusion, and representation reasoning within cleanup-bearing bodies. Carry initializedness and exception boundaries through transformations instead of permanently treating the entire function as opaque.

**Why.** Handler elimination alone cannot recover the historical gains. Current [jsc-link/src/lower/fuse.rs](jsc-link/src/lower/fuse.rs) restricts normal fusion when `Invoke` exists. The remaining helper/dependency costs need direct attention, while that restriction protects evaluation order and exception scope.

**Where.** Work in [jsc-link/src/opt/pipeline.rs](jsc-link/src/opt/pipeline.rs), [jsc-link/src/opt/dce.rs](jsc-link/src/opt/dce.rs), [jsc-link/src/opt/partial_escape.rs](jsc-link/src/opt/partial_escape.rs), [jsc-link/src/opt/witness.rs](jsc-link/src/opt/witness.rs), [jsc-link/src/lower/fuse.rs](jsc-link/src/lower/fuse.rs), and [jsc-link/src/lower/structure.rs](jsc-link/src/lower/structure.rs). Extend shared IR only when the required fact is absent.

**Deliverables.** Define protected regions and operation-motion rules. Keep argument evaluation and host property reads on the correct side of each boundary. Preserve cleanup-only uses, drop flags, saved exceptions, and normal/exceptional dominance. Reuse verifier/witness infrastructure, with transactional rollback. When selecting an optimized alternate body, verify that its instantiated calls and cleanup obligations match; lack of proof means decline, not assumption.

**Acceptance and controls.** One eligible retained-handler fixture becomes smaller or cheaper with the handler still present. It must not pass only because P07 deleted every handler. Add a custom throwing argument, aliased write, partially initialized aggregate, and destructor-reading-state controls. Measure helper reachability, actual executed work, bytes, and compile cost on a full fingerprint package and the public holdout where the mechanism applies.

**Rollback/stop.** Removing blanket restrictions without replacement invariants is unacceptable. A smaller body with reordered host effects or skipped exceptional uses fails even when ordinary outputs match.

### P09 — Protected inlining and proof transport across crates

**What.** Make inlining preserve exact callee identity, protected-call semantics, cleanup ownership, carrier facts, source origins, and caller/callee namespace mappings. Restore cross-crate wins without bypassing incompatible object boundaries.

**Why.** A transformation can work in an isolated crate and disappear after MIR inlining or a crate partition change. Conversely, importing a body without its drop obligations is unsafe. The existing inliner already imports some provider metadata and has deliberate restrictions around carrier boundaries; this task extends measured limitations rather than replacing those guards wholesale.

**Where.** [jsc-link/src/opt/inline.rs](jsc-link/src/opt/inline.rs), [jsc-link/src/opt/inline](jsc-link/src/opt/inline/), [jsc-link/src/analysis/callgraph.rs](jsc-link/src/analysis/callgraph.rs), [backend/src/source_mir/portable.rs](backend/src/source_mir/portable.rs), [backend/src/coroutine_source/transport.rs](backend/src/coroutine_source/transport.rs), and [rcgjs-object/src/jir](rcgjs-object/src/jir/).

**Deliverables.** Remap values, blocks, types, origins, instantiated callees, exception destinations, and cleanup chains together. Re-root callee cleanup into caller cleanup with explicit initialized-state handling. Keep recursion, unsupported coroutine shapes, hidden caller-location parameters, and unproved carrier conversion cases conservative. Compare equivalent programs in one crate, several private crates, and a prebuilt dependency, with MIR inlining both on and off.

**Acceptance and controls.** Safe cases select; an inlined callee with a live destructor at a host throw produces the exact required trace. A deliberately lost cleanup edge or wrong metadata remap fails verification or execution. Schema mismatch is rejected. Reports distinguish recognition loss, legality failure, economic decline, and rollback. Package benefit must survive the full dependency closure and compile-time budget.

**Rollback/stop.** Never infer the callee instance solely from a rendered name or source span. Do not force native carriers across an incompatible ABI. Cross-crate support may be admitted incrementally, with explicit reasons for remaining declines.

## 8. Strings and formatting

### P10 — Repair string scaling under the default policy

**What.** Reproduce current conversion/scan amplification, then improve default-policy string length tracking and conversion reuse without introducing unbounded retention or incorrect sharing.

**Why.** The current runtime retains the four-entry `str_bytes` memo and `str_len` scanning/encoding paths. Historical measurements found near-quadratic encoding on a Markdown-shaped workload, but those numbers belong to older artifacts. The new baseline must establish where repeated work occurs now.

**Where.** [runtime/shim.js](runtime/shim.js), [jsc-link/src/lower/semantic_ops/string.rs](jsc-link/src/lower/semantic_ops/string.rs), [jsc-link/src/plan/representation.rs](jsc-link/src/plan/representation.rs), [jsc-link/src/analysis/valueflow.rs](jsc-link/src/analysis/valueflow.rs), [scripts/runtime-utf8.test.mjs](scripts/runtime-utf8.test.mjs), the reduced Markdown/search workloads, and P05's public application.

**Deliverables.** Count encoded code units/bytes, length scans, decoding, copies, and materializations separately. Introduce exact builder byte-length metadata where legal; maintain it through construction, append, clone, clear, truncate, reserve, and conversions. Investigate operation- or owner-lifetime conversion reuse rather than merely increasing a global ring. A copied owned mutable String must not alias memoized immutable storage. Cache invalidation must account for supported mutation and materialization paths.

**Acceptance and controls.** Freeze geometric input families covering ASCII, multi-byte text, astral characters, BOMs, repeated strings, many distinct strings, slicing, and incremental appends. Compare output first, then operation counts. Claim linear conversion overhead only for workloads whose measured algorithm and reuse make that claim appropriate; search index growth can have independent non-linear costs. Test byte/character boundaries, capacity, mutation through admitted aliases, invalid input policy, and repeated-session memory retention. Confirm end-to-end gains with instrumentation disabled.

**Rollback/stop.** Do not turn JavaScript string length into Rust byte length, expose a mutable memo buffer, pin arbitrary input strings forever, or reinterpret warm-cache success as fixed asymptotic behavior. A bound must be tested beyond its cache capacity.

### P11 — Optional union strings and bounded immutable-literal pinning

**What.** Reconstruct an opt-in representation that can retain host text or a UTF-8 backing/view where profitable, with explicit conversion and ownership rules. Evaluate bounded pinning of admitted immutable literals separately.

**Why.** Default host strings may repeatedly encode/slice where retained UTF-8 is advantageous. Yet representations that improve a kernel can break pointer identity, mutation, ABI compatibility, or whole-program economics. The historical union mode remained opt-in and must return under that discipline.

**Where.** Proposed option handling in [rcgjs-object/src/opts.rs](rcgjs-object/src/opts.rs); shared carrier/type facts in [rcgjs-object/src/jir](rcgjs-object/src/jir/); selection in [jsc-link/src/plan/representation.rs](jsc-link/src/plan/representation.rs); lowering in [jsc-link/src/lower/semantic_ops/string.rs](jsc-link/src/lower/semantic_ops/string.rs); runtime, producer, and sysroot transport changes coordinated with P18.

**Deliverables.** Specify immutable/mutable storage, owner identity, UTF-8 ranges, lengths/capacities, snapshots, retained views, and host export boundaries. Enumerate unsupported observations and fallback conditions. Do not infer unique ownership from a full-buffer view. Literal pinning has an explicit eligibility rule and memory budget, excludes arbitrary dynamic input, and does not intern observably distinct mutable allocations. All conversions participate in cost and effect analysis.

**Acceptance and controls.** A positive compiled workload automatically selects the representation and benefits after all adapters/helpers are counted. Aliased writes, capacity observations, incompatible objects, host callbacks, and mutation/detachment races either remain correct or produce a declared compile-time refusal. Test read-only snapshots and distinct owned Strings with equal content. Cache/profile identity changes with the representation policy. Full-runtime and extracted-runtime versions agree.

**Rollback/stop.** This work is an experimental capability until the supported subset and economic result are established. A valid negative result may retain default strings and leave union mode experimental; it does not permit marking the historical union capability fully restored. Never change the default in the same patch that first introduces the option.

### P12 — Formatter recognition and carrier commitments

**What.** Repair the construction/use proofs and representation commitments that prevent eligible formatting from eliminating generic formatting dependencies. Validate and extend the existing primitive formatter rather than implementing a second parallel pipeline.

**Why.** Historical package savings were substantial, but they resulted from a specific proof/commitment repair. Current code already contains `fmt_kernel` and a specialized floating formatter. There is no evidence that adding another transitive dependency-credit mechanism is the missing fix. Full inlined applications can still differ from reduced inlined fixtures.

**Where.** [jsc-link/src/lower/semantic_ops/fmt.rs](jsc-link/src/lower/semantic_ops/fmt.rs), [jsc-link/src/lower/semantic_ops/fmt_kernel.rs](jsc-link/src/lower/semantic_ops/fmt_kernel.rs), [jsc-link/src/plan/representation.rs](jsc-link/src/plan/representation.rs), [jsc-link/src/analysis/valueflow.rs](jsc-link/src/analysis/valueflow.rs), producer operation identities, and `native_f64_to_string` in [runtime/shim.js](runtime/shim.js).

**Deliverables.** Diagnose each stage for eligible primitive `Display`/String constructions: recognized, legal, carrier selected, emitted, dependencies removed. Preserve exact monomorphized standard-operation identity. Exercise aggregate arguments, String fields, moves, drops, and cross-crate/MIR-inlined forms. Unsupported formatting beside eligible formatting must not either corrupt the unsupported case or unnecessarily prevent every independent eligible case.

**Acceptance and controls.** Match native formatting for the declared primitive subset, including negative zero, infinities, NaN, subnormals, exponent transitions, decimal ties, integer widths, and Unicode scalar values. Custom `Display`, custom writes, precision/alignment or other unsupported options retain their correct implementation. Observe argument evaluation and write order. Assert removal of the relevant reached generic dependency cluster on positive cases and retention on controls. Measure the idiomatic package and at least one public complete workload where formatting matters.

**Rollback/stop.** Plain JavaScript `String(value)` is not an automatic replacement for every Rust formatter. A proof that works only with MIR inlining disabled is a scoped result, not full restoration; report the remaining field/access or transport blocker precisely.

## 9. Bounded output improvements

### P13 — Pool eligible immutable referenced constants

**What.** Deduplicate sufficiently repeated, eligible immutable backing data at link time, with explicit use/reference and deployment analysis.

**Why.** Repeated large constants can dominate output even after local minification. The historical compact-OSS improvement is a useful lead, but identical bytes alone do not establish that sharing is legal or profitable.

**Where.** [backend/src/constant.rs](backend/src/constant.rs), [backend/src/constant](backend/src/constant/), [backend/src/constread.rs](backend/src/constread.rs), [jsc-link/src/link.rs](jsc-link/src/link.rs), [jsc-link/src/link_dedup.rs](jsc-link/src/link_dedup.rs), [rcgjs-object/src/item.rs](rcgjs-object/src/item.rs), and shared type/ownership facts.

**Deliverables.** An eligibility rule covering immutable shared-reference use, absence of admitted interior/host mutation, initialization effects, identity observations, and drop behavior. Distinguish shared immutable backing from copying a value into independent mutable storage. Integrate pooled constants with actual references, exported roots, runtime closure, chunks, and source maps. Price added indirection and retained shared data, not only literal-text savings.

**Acceptance and controls.** Positive repeated-table/string constants select at documented thresholds. Mutable/interior-mutable, address-sensitive, independently copied, and host-exposed cases remain correct or are declined. Include function references stored in pooled constants so call escape analysis sees them. Single-entry and worker/multi-entry packages have correct transfer and initialization behavior. A pool-use count of zero cannot certify the pass. Compare final raw/gzip/Brotli graph sizes and retained memory.

**Rollback/stop.** Do not merge mutable objects just because serialized fields match, share data across JavaScript realms implicitly, or count compression of separate helpers as a package saving. No package-specific constant-name matching.

### P14 — Eliminate eligible dead trailing arguments

**What.** Remove unused trailing formal parameters and safely adjust exclusively known internal call sites while preserving argument evaluation and hidden ABI obligations.

**Why.** The historical pass provided modest generic wins but exposed a pooled-function-table escape bug. The useful mechanism is bounded; the legality conditions are more important than maximizing the number of removed arguments.

**Where.** [jsc-link/src/analysis/refs.rs](jsc-link/src/analysis/refs.rs), [jsc-link/src/analysis/param_use.rs](jsc-link/src/analysis/param_use.rs), [jsc-link/src/analysis/callgraph.rs](jsc-link/src/analysis/callgraph.rs), [jsc-link/src/link.rs](jsc-link/src/link.rs), and [rcgjs-object/src/jsast.rs](rcgjs-object/src/jsast.rs). `jsc-link/src/link/trailing.rs` is a proposed focused module.

**Deliverables.** Complete escape/reference discovery through constants, pooled tables, aliases, exports, callbacks, dynamic calls, and cross-crate references. Preserve hidden caller-location/environment parameters and any ownership/drop work attached to arguments. If an argument expression is removed from a call, its required effects, exceptions, evaluation order, and temporary lifetime still occur. Treat host-observable arity and foreign-call conventions as barriers unless explicitly adapted.

**Acceptance and controls.** Internal eligible calls shrink; exported or escaped callbacks do not change observable arity. A removed argument that calls a getter, throws, mutates state, or moves a drop-bearing value retains its behavior. A pooled function-table fixture exercises the same guard that failed historically. Verify pass-off restores the relevant structure and selection counts are nonzero. Measure whole packages and interaction with P13.

**Rollback/stop.** Do not infer non-escape from direct-call counts alone. Never delete argument effects to make the size gate pass. If the complete reference graph is unavailable, decline the affected function.

### P15 — Numerical spelling and redundant terminal emission

**What.** Shorten emitted numeric literals while preserving parsed value and context, and remove genuinely redundant terminal operations only after a verified non-returning operation.

**Why.** These are bounded printer/lowering opportunities. They may primarily improve pre-Terser output and compilation/readability rather than final compressed bytes. Panic locations and diagnostic messages are not automatically unobservable.

**Where.** [rcgjs-object/src/jsast.rs](rcgjs-object/src/jsast.rs), [rcgjs-object/src/minify.rs](rcgjs-object/src/minify.rs), [jsc-link/src/lower/js/emit.rs](jsc-link/src/lower/js/emit.rs), relevant call/control-flow lowering, and [rcgjs-object/src/sourcemap.rs](rcgjs-object/src/sourcemap.rs).

**Deliverables.** A deterministic shortest-admitted-literal policy with correct sign, negative zero, precision, exponent syntax, token adjacency, and precedence. Test in actual expression contexts, not only a parser in isolation. Define non-returning facts separately from string labels such as “unreachable.” Evaluate location/message interning only when runtime-visible contents and identity requirements are preserved.

**Acceptance and controls.** Property/golden tests check parsed values and arithmetic observations, including negative zero and boundary magnitudes. Source maps still identify authored code. An operation that can return retains the following terminal behavior; a truly divergent operation can lose dead following text. Retain source, pre-minified, packaged, and compressed measurements separately. A zero post-Terser benefit is a valid result, not grounds to claim an unseen runtime win.

**Rollback/stop.** Do not replace caller locations with zero, alter panic text, or remove a reachable abort merely because it appears boilerplate. Any diagnostic-policy reduction is a separate user-visible product decision.

### P16 — Worker/runtime closure and redundant pointer views

**What.** Restore generic runtime pruning and legal reuse/exclusion of unnecessary pointer/view helpers, especially around opaque host handles and worker entries.

**Why.** A small worker can import a large helper cluster through one unnecessary cast. The historical worker improvement does not establish a universal runtime floor, and a worker that imports the main module must be priced with that dependency.

**Where.** [backend/src/ptr.rs](backend/src/ptr.rs), [jsc-link/src/lower/js/rvalue.rs](jsc-link/src/lower/js/rvalue.rs), [jsc-link/src/lower/js/ptr.rs](jsc-link/src/lower/js/ptr.rs), [jsc-link/src/analysis/effects.rs](jsc-link/src/analysis/effects.rs), [runtime/shim.js](runtime/shim.js), [scripts/minimal-shim.mjs](scripts/minimal-shim.mjs), and [scripts/artifact-manifest.mjs](scripts/artifact-manifest.mjs).

**Deliverables.** Prove when a host handle must remain opaque instead of entering a numeric scalar-byte view. Reuse repeated casts only when their relevant base, provenance, allocation, and observations remain valid. Keep invalidation across mutation, reallocation, disposal, transfer/detachment, callbacks, and other admitted effects. Compare standalone workers and shared-main workers as different delivery graphs.

**Acceptance and controls.** A positive worker drops the unnecessary helper dependency with equal behavior. Boxed host values, numeric arrays, aliases, zero-offset projections, exceptions, and unknown calls are controls. Verify both the full runtime and the extracted closure. Report total transfer once per fetched resource, per-realm evaluation separately, and worker startup separately from steady state.

**Rollback/stop.** Do not infer ownership from buffer extent, drop checks because a benchmark has no aliases, or treat imported runtime bytes as free. An operation moved across a callback requires an effect/alias proof, not repeated textual spelling.

## 10. Composition, comparisons, and qualification

### P17 — Interaction testing and proof transport

**What.** Test the combined pipeline across cleanup, inlining, crate partitioning, representation choice, constant pooling, and call trimming. Revisit typed-storage opportunities only with complete-workload evidence.

**Why.** Historical improvements often disappeared after another pass changed the shape, or exposed bugs only in combination. A successful reduced typed-vector example did not imply profitable storage selection in a full application. No new architecture is required merely to investigate those failures.

**Where.** [jsc-link/src/analysis/valueflow.rs](jsc-link/src/analysis/valueflow.rs), [jsc-link/src/plan/representation.rs](jsc-link/src/plan/representation.rs), [jsc-link/src/plan/abi_flat.rs](jsc-link/src/plan/abi_flat.rs), [jsc-link/src/opt/inline.rs](jsc-link/src/opt/inline.rs), shared IR/verifiers, [examples/cross-crate-tests](examples/cross-crate-tests/), and the P01 experiment runner.

**Deliverables.** A selected interaction matrix plus leave-one-out builds for major restored passes. Preserve facts through transformations: exact operation identity, ownership/escape, cleanup, initializedness, standard-container structure, ABI labels, and source origins. For typed storage, select a naturally bounded Rust-owned numeric workload, then record recognition, legality, selection, adapters, and whole-package benefit. Include negative bounds/alias/capacity cases rather than forcing selection.

**Acceptance and controls.** The six later regressions pass in all admitted release configurations. Cleanup and parameter trimming compose with pooled callback tables; formatter and strings compose with inlining and crate splits. Each major feature has a positive emitted-selection assertion and negative/rollback tests. A fixture that changes crate layout alone may change economics, but must not change behavior. Full-workload zero selection is recorded honestly.

**Rollback/stop.** A pass is not considered restored merely because it works in isolation. Keep optional typed storage experimental when the whole-program case is unproved or unprofitable. Do not “fix” a composition failure by turning off cleanup globally or changing workload source.

### P18 — Schema, build-cache, and delivery integrity

**What.** Keep all new representations, effects, cleanup facts, pass controls, and runtime changes bound to compatible objects, cache keys, and delivered artifacts. Validate clean and warm consumer paths.

**Why.** A correct compiler can still ship stale output. The linker watch has been repaired, but source/runtime/macro/sysroot changes and cross-crate cache reuse need tests. Syntax compatibility and availability of runtime facilities are distinct requirements.

**Where.** [rcgjs-object/src/object.rs](rcgjs-object/src/object.rs), [rcgjs-object/src/item.rs](rcgjs-object/src/item.rs), [rcgjs-object/src/artifact.rs](rcgjs-object/src/artifact.rs), [backend/src/identity.rs](backend/src/identity.rs), [mires-build/src/rerun.rs](mires-build/src/rerun.rs), [mires-build/src/config.rs](mires-build/src/config.rs), [scripts/sysroot-cache.sh](scripts/sysroot-cache.sh), [scripts/mires.mjs](scripts/mires.mjs), and packaging tools.

**Deliverables.** Version incompatible transport/schema changes explicitly. Bind effective flags and runtime ABI to producer, object, sysroot, and consumer identities. Watch the actual selected binaries rather than merely similarly named outputs. Add isolated warm-cache tests for linker-only, backend-only, runtime-only, macro-only, stdlib-patch, and option changes. Publish main/worker/runtime/declaration/map manifests with required imports verified. Define required runtime features such as BigInt separately from the ES2018 syntax ceiling.

**Acceptance and controls.** Mixed incompatible objects fail with a useful rebuild diagnostic. A selected input change rebuilds the affected consumer; irrelevant changes do not force unnecessary recompilation. An intentional missing runtime export, stale sysroot, tampered graph, or missing chunk is detected before successful publication. Failed links leave no apparent final program. A clean stable-Cargo consumer runs without Topcoat and without private experiment files.

**Rollback/stop.** Do not change only one copy of a profile expansion, or use a source hash as a substitute for checking actual selected build products. Full source-map validity is not the same as useful executable debugger stepping; P20 tests both at their declared scope.

### P19 — Comparator equivalence and production configuration

**What.** Requalify the application contracts and choose explicit artifact variants for cross-language context. Keep same-Rust improvement evidence independent from cross-language ranking.

**Why.** Fourteen successful builds and offline tests do not prove complete product parity. The Pro ports can implement different behavioral subsets. Elm OSS has a substantial JavaScript host. Several `dist/` roots are development/readable aliases. Ignoring those distinctions would recreate misleading size tables.

**Where.** [docs/COMPARATORS.md](docs/COMPARATORS.md), [experiments/tools/run-comparator-builds.py](experiments/tools/run-comparator-builds.py), [experiments/tools/check-comparators.py](experiments/tools/check-comparators.py), [experiments/tools/measure-comparators.mjs](experiments/tools/measure-comparators.mjs), [experiments/fingerprintjs-compare](experiments/fingerprintjs-compare/), and private Pro harnesses.

**Deliverables.** Contract/version identifiers for each compared product, explicit ownership of algorithms and host adapters, exact upstream/reference identities, and an eligibility table by tested feature. Use Scala.js full-link, Dart O2, Kotlin Gradle production, and the declared Elm production packaging for production-oriented rows; retain other variants as separately labeled diagnostics. Test malformed input, callbacks, async ordering, cleanup/cancellation, worker lifecycle, retries, cache, debug paths, and other compared Pro branches. Do not invoke real analytics/identification endpoints; use deterministic local fixtures and controlled browser contexts.

**Acceptance and controls.** Only contract-matched artifacts enter a ranking. An omitted request, swallowed error, missing worker message, or wrong result field fails the relevant qualification. A copied declaration checksum is not a behavioral/type-inference proof. Reuse unchanged frozen comparator artifacts after hash verification; rebuild when sources, toolchains, packaging, or contracts change. Runtime and compression versions are recorded per row.

**Rollback/stop.** Ineligible or incomplete ports remain visible as such, not optimized baselines. Comparator gaps do not block same-Rust compiler restoration. Update vulnerable packaging dependencies as a separate maintenance change and regenerate affected baselines; never silently mix old and new minifiers into a claimed compiler win.

### P20 — Combined semantic and economic qualification

**What.** Qualify one integrated R2 candidate using the frozen protocol, complete delivery graphs, and declared configuration matrix. Re-run economics on the final bytes rather than combining wins from different intermediate commits.

**Why.** Individual passes can cancel each other's savings, cause new dependencies, or regress a different profile. Current evidence has no complete three-browser runtime qualification. The final claim must name one compiler, one configuration per row, and the exact verified program.

**Where.** P01's runner and evidence schema, [scripts/check.sh](scripts/check.sh), [scripts/test.sh](scripts/test.sh), [scripts/cleanup-smoke.sh](scripts/cleanup-smoke.sh), [scripts/cleanup-admission-test.sh](scripts/cleanup-admission-test.sh), [scripts/standalone-check.mjs](scripts/standalone-check.mjs), the six later regressions, actual Rust package builders, P05, and browser harnesses.

**Deliverables.** Run semantic gates first on exact candidate artifacts, then cold/warm compile tests, shipped-byte measurements, isolated JavaScript timing, and memory/lifecycle tests. Cover current available Chrome, Firefox, and Safari with recorded versions; unsupported environments remain explicitly unqualified. Include all three release profiles and the default string policy. Optional union results stay separate. Exercise clean archive, stable consumer, ESM/script scope, workers, shared runtime, and selected authored breakpoints/stepping.

**Acceptance and controls.** No supported semantic mismatch, silent ordinary fallback, identity mismatch, or new required-test failure. Each claimed optimization has selection and economic evidence; all promoted passes coexist. Public holdout benefits support generality. Protected workload regressions are decided using the prospectively frozen margins, confidence procedure, and memory/build budgets. Deliberate slowdowns near the decision margin and A/A trials validate measurement sensitivity. Inconclusive data stays inconclusive.

**Rollback/stop.** No best-of-N run picking, post-result threshold changes, warm-data substitution for cold claims, or mixed candidate receipts. A build that times out is not assigned a completed time. A failed browser cell cannot be hidden by a geomean or a faster engine.

### P21 — Close restoration and preserve a usable development baseline

**What.** Publish a precise restoration verdict, reproducible commands, current source/evidence inventories, and an explicit inventory of deferred work. Separate a successful restoration milestone from a production release decision.

**Why.** The previous loss made original source and evidence recovery expensive. The new system must be restorable and must not let a broad “complete” statement outrun its receipts.

**Where.** Update [README.md](README.md), [VERDICT.md](VERDICT.md), [docs/V2-RESTORATION.md](docs/V2-RESTORATION.md), the proposed baseline/status registry, and public/private backup manifests. Preserve original recovery archives unmodified.

**Deliverables.** A feature-by-feature ledger labeled recovered, reconstructed, improved, experimentally retained, explicitly deferred, or unsupported. Link every completion to tests and the exact candidate. Archive public compiler/workload sources with lockfiles and essential passing/failing receipts; archive private Pro material separately. Document restore verification and arrange an authorized off-device copy rather than only another path on the same disk. No automatic public push or release signing is implied.

**Acceptance and controls.** Another clean checkout can rebuild the public compiler and holdout, verify source hashes, run the documented regression suite, and reproduce the declared output contract. Private files are unnecessary for ordinary compiler use. A restored archive is checked by content, not merely created successfully. Historical records remain accessible and clearly labeled.

**Stop condition.** A remaining required semantic failure blocks the correctness/restoration signoff. Missing optional gains or an inconclusive runtime result must appear in the verdict; they cannot be converted into a successful October-recovery claim.

## 11. Experimental method and acceptance rules

### What

Use one method for deciding whether a change is correct, selected, useful, and safe to combine. Keep compiler execution cost, generated-program execution cost, transfer size, and memory separate.

### Why

The historical work contains useful positive results, expected-negative controls, unprofitable legal transformations, and later superseding repairs. A single green/red label loses those distinctions. It also invites size claims built from non-equivalent semantics or timings gathered on a contaminated development machine.

### Where

P01 creates the public protocol and runner under the proposed `experiments/v2-restoration/`. Existing comparator tools remain the starting point for language-port artifacts. Private Pro data uses the separate location in section 12.

### 11.1 Required evidence for each change

A work package has four independent dimensions:

| Dimension | Required observation |
| --- | --- |
| Semantics | Correct output/effect/drop/error behavior, plus admitted diagnostics and negative controls. |
| Selection | Nonzero intended transformation in a positive case; justified decline in a hostile case; pass-off/report controls. |
| Economics | The declared metric improves on a matched before/after case, or the feature is retained only experimentally with a recorded negative result. |
| Integrity | Sources, binaries, sysroots, harness, inputs, artifact graph, and effective options match the receipt. |

A correctness repair does not need an economic improvement. A behavior-preserving compiler implementation speedup may intentionally produce identical JavaScript. A size optimization is not a runtime speedup unless runtime was measured. A legal transformation declined by the cost model is not a compiler correctness failure.

For major promoted optimization families, demonstrate at least one complete-package benefit and a mechanism-relevant public generality check. For a tiny printer change, an exact emission improvement with no final-size gain can justify a narrower claim. Explain maintenance/complexity costs rather than applying a universal “must save 2%” rule.

### 11.2 Baseline and protocol freezing

P01 must write a versioned limits/protocol record **before economic candidate results are collected**. P06 cannot complete while its required fields are unset. At minimum record:

- Named reference machine, operating system, power settings, compiler/build profile, worker/job counts, dependency cache state, and measured stage boundaries.
- Finite wall-time and peak-memory budgets for each required compiler build, including the optimized Pro link and the public holdout.
- Runtime input sizes, warmups, samples, independent sessions, seed/order randomization, cold/warm definitions, and invalidation conditions.
- Protected workload/profile metrics, acceptable regression margins, minimum practical improvements, and the statistical decision rule.
- Memory lifecycle procedure, repetitions, idle/settlement boundaries, and what counts as retained growth.
- Exact compressor implementation, versions, levels, headers/options, source-map policy, and deployment graph accounting.

Do not invent a link-time target from the interrupted nine-minute run. Use R0 profiling and machine capacity to propose a resource envelope, freeze it, and then test P04. Changing the envelope later requires a new protocol and an explicit explanation; previous failures remain failures under the old protocol.

A starting policy to evaluate during calibration is a **5% protected runtime-regression margin** on the chosen metrics. This is a proposed policy, not an inherited measured fact or an automatic gate for every microbenchmark. The adopted value must be justified by A/A noise and sensitivity tests and recorded before ranking candidates. Size regressions are exact measurements: zero is the default for a size-only claim on protected graphs; any tolerated tradeoff must be profile-specific and explicit.

### 11.3 Compiler build cost

Measure backend construction, sysroot production, application producer compilation, independent linking, and packaging separately. Distinguish a cold dependency download from compiler work; record both when reporting end-to-end setup, but do not call network variability a compiler regression.

For P04 use deterministic work counters as well as time. Capture trial counts, function lowerings, name-table work, graph reachability passes, compression calls/bytes, cache occupancy, and retained memory. Compare a verified uncached mode on tractable inputs. A fixed cache improves reuse, but must not reuse results across changed effects, aliases, object identities, profiles, or helpers.

Timed-out or manually stopped runs are censored/incomplete observations. They can support “did not complete within this envelope,” not a precise runtime or a numeric speedup. Stop owned child processes, retain diagnostics, and leave no stale result that a subsequent check can mistake for success.

### 11.4 Generated-program time and memory

Run no compiler builds, dependency installations, or unrelated stress work during protected timing. Use repeated independent sessions and randomized paired candidate/control order. Treat cold import/initialization, worker startup, warm computation, asynchronous end-to-end operations, and teardown as different metrics.

Predeclare a confidence method appropriate to the sampling unit. Do not treat many calls inside one warmed process as independent machine-level observations. For a protected regression margin, classify an interval overlapping the decision boundary as inconclusive. Keep raw samples and exclusions. Use a prospective sequential method only if it was specified before observing results; never add runs selectively until a pass appears.

Calibrate A/A pairs and injected slowdowns around the actual margin, not just a trivial 2x slowdown. A sensitivity test validates the harness; it is not a workload performance result. Use p50/p95 only where enough observations exist to estimate them meaningfully. Geomeans may summarize eligible cells, but never hide a failing protected cell.

For string/cache and lifecycle work, measure repeated create/use/dispose cycles with sufficiently many distinct inputs. Separate temporary allocation from retained heap. Record the engine's GC/measurement procedure and its limitations; process RSS alone is not a precise JavaScript live-object measure. Instrumented work counts establish mechanisms; uninstrumented builds establish performance.

### 11.5 Package sizes and dependencies

Measure at least raw emitted program, selected runtime, packaged/minified resources, and separately compressed deployment resources. For a set of independently fetched resources `G`, define the transfer total as the sum of each resource's compressed size, with each fetched URL counted once for that declared graph. Do not gzip a concatenation unless that concatenation is the delivered artifact.

Record startup graph, worker graph, and optional/lazy graph separately. Sharing a main resource avoids duplicate transfer in a graph but does not eliminate per-realm parsing/evaluation. Source maps and declarations have their own download/install totals rather than being inconsistently included in some execution bundles.

Use the same pinned compression procedure for paired comparisons. Existing system-gzip and Node-zlib receipts are separate datasets unless remeasured consistently. Runtime/closure savings require a graph change or reduced resource bytes, not a sum of isolated helper compression estimates. A custom minifier experiment is a separate candidate with its own semantic qualification.

### 11.6 Matrix tiers

| Tier | Frequency | Required scope |
| --- | --- | --- |
| Focused patch | Every semantic/optimizer change | Reproducer, positive selection, negative/mutation control, unit/witness checks, exact artifact identity. |
| Integration | Each coherent work-package landing | Restored fixture semantics, known gaps, cleanup matrix, same/cross-crate and MIR-inline cases, representative full package, holdout slice, compiler resource report. |
| Milestone | R1 and integrated R2 | Three release profiles; selected optimizer/report controls; complete declared workload set; default policy; applicable optional-policy rows; clean/warm build tests. |
| Final runtime/browser | R2 economic claim | Recorded Chrome/Firefox/Safari cells, delivered main/worker graphs, frozen timing and memory protocol, holdout inputs, no concurrent builds. |

The interaction matrix is not an uncontrolled Cartesian explosion. Every changed boundary gets targeted pairwise or higher-order cases, and known dangerous combinations are mandatory. Examples include cleanup + inlining + partially initialized values, pooling + escaped function tables + trailing-argument trimming, and union strings + retained views + host callbacks. Optional policy combinations are admitted only after their own tests.

The legacy baseline lane remains a useful diagnostic, not a license to emit baseline text for cleanup-bearing functions. A mode intentionally unsupported by the contract must fail explicitly. Do not make an expected failure look green by excluding its selection from the report.

## 12. Evidence layout and receipt contract

### What

Create a small, append-only evidence system. Reuse the recovered harnesses, but connect every assertion to exact inputs and a declared scope.

### Why

Status must survive source changes, failed experiments, interrupted builds, machine migration, and the loss of disposable caches. Public source should remain easy to clone without bundling private references or gigabytes of historical outputs.

### Where: proposed layout

```text
V2-OPTIMIZATION-RESTORATION-PLAN.md       # This roadmap

docs/optimization-restoration/
  baselines.json                        # Immutable identities and lineage
  status.json                           # Current per-package decisions, receipt links
  decisions/                            # Contract, default-policy and protocol decisions

experiments/v2-restoration/
  README.md
  protocol.json                         # Frozen versioned methodology and limits
  fixtures/                             # Public metamorphic/negative/selection cases
  tools/                                # Build, execute, measure, attribute and verify
  manifests/                            # Input inventories, not unqualified result summaries
  retained/                             # Small public receipts needed for interpretation

experiments/document-workbench/         # P05's complete public workload

experiments-pro/v2-restoration/
  manifests/                            # Private Pro source/reference/fixture identities
  retained/                             # Private Pro observations and required artifacts

build/v2-restoration/<candidate>/<run>/
  inputs.json
  build.json
  semantics.json
  decisions.json
  sizes.json
  compile-cost.json
  runtime.json
  memory.json
  artifacts.json
  verdict.json
  logs/
```

These are proposed deliverables, not paths whose existence is claimed by this plan. Build output is disposable only after essential evidence and source manifests are archived. Public summaries may refer to private evidence by identity and scope without exposing its source or payloads.

### Required receipt fields

| Group | Minimum content |
| --- | --- |
| Identity | Schema version, run ID, plan/protocol version, UTC timestamps, parent/baseline IDs, content digest and Git provenance. |
| Build inputs | Source inventories including generated inputs, dependency locks, rustc identity, backend/linker/macro hashes, sysroot manifest, runtime, effective options, harness and input hashes. |
| Environment | Host/engine versions, resource/job limits, cache state, packaging/compression versions, relevant environment overrides. |
| Execution | Exact command and working directory, start/end, exit/signal, timeout/interruption, case count, attempted and skipped scopes, stdout/stderr references. |
| Decisions | Recognition/legality/selection/emission counts, declines with reasons, rollback, verification results, pass-mask resolution. |
| Outputs | Resource bytes and hashes, complete import/worker/runtime graph, maps/declarations, matched artifact identities for semantic and economic checks. |
| Result | Separate semantic, selection, economic, integrity and qualification states; measured values; confidence or uncertainty; links to retained evidence. |

Suggested result states are `not-run`, `blocked`, `passed-scoped`, `failed-semantic`, `failed-selection`, `failed-economic`, `failed-integrity`, `timed-out`, `interrupted`, `inconclusive`, and `superseded`. An expected-negative control records both the expected outcome and whether the detector worked. Never use `passed` alone for an omitted browser suite.

Work-package states are separate: `planned`, `in-progress`, `implemented-unqualified`, `validated-scoped`, `restored`, `experimental`, and `deferred-with-decision`. A work package can have successful semantic tests and still be economically inconclusive. A superseding receipt references the original; it does not erase it.

### Every implementation review includes

A concise hypothesis; baseline/candidate identities; exact source change; positive application evidence; negative and rollback controls; semantic result; compiler-cost result; complete-package metrics; runtime/memory result when relevant; interaction coverage; and unresolved limits. Newly reconstructed code must be labeled as such instead of assigned a historical archive hash.

## 13. Execution discipline and first implementation batch

### What

Make small, reviewable, independently reversible changes with a strict first batch. Parallelize independent source areas and testing preparation, not concurrent protected benchmarks or competing writes to one build tree.

### Why

The repository has a large uncommitted reconstruction. Combining framework cleanup, rustc migration, new optimizations, source-port repairs, and economic policy changes in one patch would make attribution and rollback unreliable.

### Where

Track active work by the P00–P21 IDs. The existing [tasks](tasks/) directory may point to this roadmap and the current batch, but should not duplicate another conflicting full plan. Worktrees or isolated build/output directories should have independent manifests, locks, and candidate identities.

### First batch: start here

| Order | Concrete change | Completion artifact |
| --- | --- | --- |
| 1 | P00: capture current inputs, reconcile current status, and preserve public/private recovery archives. | R0 manifest and verified restoration check. |
| 2 | P01: add explicit experiment status, pass-application evidence, compile-work counters, and strict identity/negative controls. | Executable harness controls and frozen calibration protocol. |
| 3A | P02/P03: fix the three ZST destruction cases and aggregate-place identity; expand the matrix around the two already passing probes. | Correct native/host traces, selection-independent regression receipts. |
| 3B | P04: quantify repeated costing/name work; implement equivalent reuse with cache invalidation tests. | Matched output/decision controls and a completed optimized Pro link. |
| 3C | P05: implement and freeze the public application and reference. | Contract, source/input hashes, end-to-end behavior baseline. |
| 4 | P06: rerun the defined candidate matrix and seal R1. | Operational baseline ready for economic comparisons. |
| 5 | Begin P07/P08 plus P10; prepare P12 recognition diagnostics. | Small causal cleanup/string/formatter experiments, not one optimizer rewrite. |

P13–P16 may proceed after R1 when isolated and supported by their own tests. Do not prioritize a tiny Murmur graph target ahead of a correctness failure, the Pro compile bottleneck, or measured string amplification.

### Integration rules

Preserve the old failing fixture before changing the implementation. Do not delete a check because its expected result is inconvenient. Split mechanical renames, semantics, economics, and harness repairs when possible. A harness fix gets a test proving the previous harness could miss the defect.

Rebase or merge only after work-package validation, then rerun interaction controls on the merged content identity. Parallel workers may investigate shared areas but should not land conflicting changes to `link/lane.rs`, representation contracts, or the runtime without an explicit integration order. No assumption that two passing branches produce a passing union.

The current root dependency advisories and portability gaps are separate maintenance work. Security-driven upgrades should not be deferred indefinitely, but they create a new packaging/toolchain baseline and require affected comparisons to be rebuilt. Do not mix them invisibly into an optimizer speedup.

A commit/tag is useful after a reviewed milestone, but creating this plan or running its tasks is not permission to commit or push private material. Use a content-addressed archive when Git publication is inappropriate.

## 14. Finish lines and deferred research

### What

Declare what it means to finish optimization restoration and what remains a distinct product/research decision.

### Why

The original program accumulated broad north-star goals alongside a narrower release. The standalone project needs a finite goal without pretending that all safe Rust or all applications have been certified.

### Where

P21 writes the final current verdict and feature ledger; P20 supplies the deciding candidate receipts.

### Optimization-restoration finish

Restoration is ready to close when the following are true for one identified R2 candidate:

| Requirement | Evidence required |
| --- | --- |
| Correct admitted semantics | The six later probes and expanded supported cases pass; cleanup/exception and regression matrices have no known required semantic failures. |
| Operational compiler cost | Representative optimized cleanup-on builds finish within frozen budgets; cache/invalidation controls and phase measurements are retained. |
| Restored main mechanisms | Cleanup optimization, default string-scaling work, formatter proof/commitment work, and each promoted bounded pass have actual-selection and economic evidence. |
| Generality | The public complete holdout passes unchanged and supports a measured relevant improvement; fingerprint-only wins are labeled narrowly. |
| Composition | Major passes work together under declared crate, MIR, cleanup, profile, and deployment combinations. |
| Reproducible delivery | Exact source/toolchain/runtime/artifact identities, clean stable consumer, warm-cache checks, and useful declared map/debugger scope. |
| Honest comparisons | Other-language rows identify contract, ownership, profile, and qualification; no rank includes an ineligible implementation as equivalent. |
| Durable record | Public/private source and evidence archives verified, decisions retained, and backup location/limitations stated. |

Optional union strings or typed storage may remain experimental or be explicitly deferred after a valid experiment. A historical idea can be rejected if its new evidence is negative. That is a **restoration decision**, not proof the old code was reproduced. Claims of “all October optimizations restored” require every such capability to be separately accounted for; this roadmap does not make that blanket promise.

No calendar completion date is inferred from the reports. Completion is evidence-based. The plan is complete as a roadmap when written; implementation milestones remain open until their receipts exist.

### Separate production-release work

Production admission additionally needs a maintained support matrix, clean supported-platform builds, distribution reproducibility, dependency/security handling, public API/diagnostic documentation, a release-specific test and performance decision, and authorized signing where the release format requires it. Neither a green restoration suite nor copied historical release notes substitutes for that decision.

Topcoat-specific keyed K6/K7, hydration, SSR islands, framework procedure routing, and macro freshness requirements are not imported as standalone release gates. Their generic lessons become ordinary module/worker/host-lifetime and consumer-build tests.

### Deferred research, not critical-path implementation

A native-by-default restricted policy, arbitrary Rust-future replacement with JavaScript generators, a new compiler frontend, universal zero-allocation kernels, and broad automatic typed storage are separate proposals. Each needs an explicit supported contract, a faithful current control, negative cases, and complete-workload economics.

For a future generator experiment, cancellation before the first resume, partially initialized captures, resumed cancellation, and late settlement must be modeled explicitly. Do not assume the presence of `finally` alone implements the Rust future's full drop obligations. This research must not displace the known cleanup/string/formatter repairs.

## 15. Existing commands and historical targets

### What

Keep a practical inventory of commands that exist now, distinct from proposed tools. Retain historical numbers only with their original scope.

### Why

An implementation plan should be executable without pretending new flags or files already exist. Historical output sizes help locate opportunities, but cannot be used as equal-contract performance baselines after changing sources, semantics, toolchains, or compressors.

### Where: existing entry points

Run from the repository root. These commands may build artifacts, require installed dependencies, or intentionally fail for an open case. They are not a claim that all rows below were rerun while writing this plan.

```sh
# Current compiler and standard checks.
bash scripts/build.sh --locked
bash scripts/check.sh

# Strict later regressions: currently expected to report four open cases.
python3 scripts/check-known-gaps.py

# Cleanup and fail-closed admission.
bash scripts/cleanup-smoke.sh
bash scripts/cleanup-admission-test.sh

# Generic standalone deployment and stable host consumer.
node scripts/standalone-check.mjs
cargo +stable run --manifest-path examples/standalone-consumer/Cargo.toml

# Public reference and reduced diagnostic workload corpus.
npm run build:oss-reference
npm run build:corpus
npm run test:corpus

# Restored non-Rust comparison implementations; not needed on every Rust patch.
npm run build:comparators
npm run check:comparators
node experiments/tools/measure-comparators.mjs
```

The current `scripts/check.sh` is not the full cleanup-on release matrix, and `npm run test:corpus` is Node/pure scope rather than a browser matrix. The existing known-gap runner hard-codes a cleanup-on balanced configuration; P01 must parameterize new matrix work instead of assuming environment overrides change that script.

For a current optimized OSS compact build:

```sh
node experiments/fingerprintjs-rust-compact/tools/build.mjs \
  --profile release-balanced --optimizer on --lane jir --cleanup on \
  --scheduler boundary \
  --output experiments/fingerprintjs-rust-compact/dist/mires-v2

node experiments/tools/check-oss-kernels.mjs \
  experiments/fingerprintjs-rust-compact/dist/mires-v2/fp.esm.min.js
```

For the existing private Pro build path:

```sh
# Established optimizer-off control; cleanup remains on.
node experiments-pro/fp-pro-rust/tools/build.mjs \
  --profile release-balanced --optimizer off --lane jir --cleanup on \
  --output experiments-pro/fp-pro-rust/dist/mires-v2-opt-off

# The optimized row is P04's target, not a currently completed result.
# Run it through the new bounded runner once P01's protocol exists.
```

Do not pass a proposed union/pass flag to these commands before its implementation and validation land. Do not describe package checks invoked with a size-failure allowance as successful size qualification.

### Historical targets: investigation leads only

| Historical observation | What to recover | What not to claim |
| --- | --- | --- |
| Formatter example: idiomatic gzip 65,525 → 48,847 bytes | Exact primitive recognition, carrier commitment, and dependency removal | A required percentage on today's different package, or proof the architecture was unnecessary. |
| Constant-pool example: compact gzip 48,029 → 44,030 bytes | Eligible referenced-constant sharing with identity/mutation controls | Permission to pool all equal literals or a current matched-size target. |
| Worker experiment: roughly 726–736 gzip bytes saved | Avoidable helper/cast reachability and deployment closure | A universal worker-runtime floor or an import-free comparison against an importing worker. |
| Trailing-argument examples: roughly 210/143 gzip bytes on idiomatic/compact | Safe call/formal trimming plus complete escape detection | Permission to omit side effects or ignore pooled function tables. |
| String experiments: reduced conversion amplification | Lifetime-aware byte/length preservation and bounded reuse | A general source-algorithm complexity guarantee or a reason for an unbounded global cache. |
| October main sizes: idiomatic OSS 265,049 raw / 74,359 gzip; compact 147,950 / 44,626 | A later capability reference worth explaining after scope alignment | Exact output equality from the reconstructed compiler or a baseline without matching toolchain/source/contract. |
| Murmur closure: 6,987 bytes against a 6,344-byte target, three loop allocation sites | Remaining generic inlining/representation opportunities | A historically completed zero-allocation or size goal. |

The mechanism references above are retained in E07 and the earlier audit conversation. The October size rows come from the earlier dev12 merge/audit record, not a fresh measurement performed for this plan. Reconcile each with an original local receipt before using it quantitatively. The comparator and assembly size reports currently use different compression paths; remeasure rather than splice those tables together.

**Immediate next action after approval of this file:** execute P00 and P01, then work on P02/P03 and P04 in parallel while implementing P05. Do not start by rewriting the architecture, tuning for a historical fingerprint byte count, or disabling cleanup to make the benchmark green.
