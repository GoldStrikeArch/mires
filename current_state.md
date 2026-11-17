# Current state: mires, the lost v2 compiler, and its reconstruction

## Current checkpoint — 2026-11-17

**Latest inspected compiler implementation:** `933d5d0ad82010879c96c29d2cd56e8bba9fd6c5`, already on main with receipt commit `ba72c43`. Subsequent documentation commits do not change its implementation. Read [RESTORATION-STATUS.md](RESTORATION-STATUS.md) for the current comparison with the lost compiler and [the fresh status receipt](docs/main-restoration-status-2026-11-17.json) for exact scope.

**We are close to the old OSS size waypoint, not finished restoring semantic coverage.** Compact is 155,681 minified / 47,856 gzip bytes; default idiomatic is 267,044 / 78,583. Relative to the recorded October dev12 waypoints these are +7.24% and +5.68% gzip. The comparison is historical and not fully input/compressor matched. It does not mean the compiler is 93% or 95% complete.

Main now includes next-gen qualification repairs and selected Round 3 private-ABI, inlining, cell, loop/bounds and integer-pair work. The proposed Round 3 runtime fast paths were removed; the runtime remains identical to the qualification baseline. Older statements below saying that all cleanup-edge pruning is held back or that only the first promotion is on main are historical.

Fresh checks during this assessment: **174/174 explicit JIR+optimizer fixtures in each of unwind and abort modes**, including six intended compile refusals per mode; **88/88 runtime tests**; **14/14 exact artifact remeasurements**; and **58/58 package-kernel comparisons for each of compact and default idiomatic**. These are not all-profile or browser qualification. The known-gap inventories remain **5/6, 2/5, 0/4**.

New native-controlled capability probes for boxed slices, shared `Rc::make_mut`, and a constant closure coerced to a function pointer fail in both optimizer modes. They are not byte-exact historical fixtures or newly attributed Round 3 regressions. They demonstrate why package-size recovery must not substitute for restoring ordinary Rust behavior. Sources and outcomes are preserved in [the Round 3 evidence archive](research/v2-restoration/round3/README.md).

The remaining hardening gaps are explicit: exceptional-edge witness checks are optional in ordinary release links; the timing driver records but does not enforce A/A validity (the current MiniSearch arm is false); and sysroot identities omit `CARGO_INCREMENTAL`, which the retained review found can change code generation. None was silently fixed by this documentation task.

The latest original candidate was October 7 dev13, whose receipts explicitly denied production/timing authority. Its generic browser, worker and clean-prebuilt evidence was broader than the new compiler's latest qualification; its Topcoat-specific islands/keyed UI are not standalone restoration requirements. Current runtime improvements are meaningful diagnostics, but no matched dev13 runtime comparison exists.

**The sections below preserve the November 12 snapshot and its evidence as written. Their uses of “current,” branch names, file counts, dates and pass totals describe that snapshot unless this checkpoint or the newer status report supersedes them.** The original text was committed separately before this update so its provenance is retained.

---

## Historical snapshot — 2026-11-12

**As of:** 2026-11-12.  
**Repository:** `mires`, standalone Rust-to-JavaScript compiler.  
**Repository revision inspected for this document:** `a868dcba9dca98cc924f362fe07b7be757a686fc` on local `main`.  
**Latest tested compiler implementation:** the combined Round 2 source at `1b582b84793842bd64698256d2811531f638e99b`; the later preservation/documentation commits do not change that implementation.  
**Overall verdict:** a working, substantially reconstructed development compiler with meaningful improvements and explicitly failing safe-Rust cases. **Not an exact October dev13 restoration, not the plan's qualified R1, and not a production-qualified general Rust compiler.**

This is the consolidated project narrative and continuation guide. It explains why v2 existed, what was lost, what was recovered, which improvements have been reimplemented, which attractive experiments failed, what the latest measurements actually describe, and how to continue without depending on deleted worktrees.

Writing this document involved reading the source, reports, manifests, and Git history, checking the latest compiler-input hashes and installed binary identities, and checking recovery locations. It **did not rerun compiler suites or benchmarks, merge a pending implementation, or make a release decision**. Test results below are attributed to their recorded execution, not presented as fresh executions performed while writing this file.

---

## Contents

1. [The current position in one page](#1-the-current-position-in-one-page)
2. [Why the original v1-to-v2 work existed](#2-why-the-original-v1-to-v2-work-existed)
3. [What the original v2 program tried and learned](#3-what-the-original-v2-program-tried-and-learned)
4. [The loss and what survived](#4-the-loss-and-what-survived)
5. [The standalone goal and its boundaries](#5-the-standalone-goal-and-its-boundaries)
6. [Chronology and source lineage](#6-chronology-and-source-lineage)
7. [Architecture and responsibility map](#7-architecture-and-responsibility-map)
8. [Current contracts, profiles, and supported use](#8-current-contracts-profiles-and-supported-use)
9. [What has worked in the reconstruction](#9-what-has-worked-in-the-reconstruction)
10. [What failed or was deliberately withheld](#10-what-failed-or-was-deliberately-withheld)
11. [Round 2: landed work and unfinished candidates](#11-round-2-landed-work-and-unfinished-candidates)
12. [The current correctness inventory](#12-the-current-correctness-inventory)
13. [Fingerprint references and Rust experiments](#13-fingerprint-references-and-rust-experiments)
14. [Other-language implementations](#14-other-language-implementations)
15. [Bundle-size history and how to read it](#15-bundle-size-history-and-how-to-read-it)
16. [Compilation cost and application runtime](#16-compilation-cost-and-application-runtime)
17. [The remaining optimization work](#17-the-remaining-optimization-work)
18. [Verification: what passes and what it proves](#18-verification-what-passes-and-what-it-proves)
19. [Evidence preservation after worktree cleanup](#19-evidence-preservation-after-worktree-cleanup)
20. [Roadmap reconciliation and the next sequence](#20-roadmap-reconciliation-and-the-next-sequence)
21. [Finish lines and non-goals](#21-finish-lines-and-non-goals)
22. [Commands for continuing work](#22-commands-for-continuing-work)
23. [Decisions, rejected shortcuts, and working rules](#23-decisions-rejected-shortcuts-and-working-rules)
24. [Stale statements and how to resolve them](#24-stale-statements-and-how-to-resolve-them)
25. [Glossary and evidence index](#25-glossary-and-evidence-index)
26. [How to maintain this document](#26-how-to-maintain-this-document)

## 1. The current position in one page

### What we have

The active repository contains an operational v2 compiler, not just an archive of the previous effort. Its architecture separates the rustc-facing producer, a semantic object/IR contract, and a rustc-independent linker. Real `core` and `alloc` programs compile to JavaScript. There are general JavaScript bindings, a stable Cargo build-script bridge, runtime extraction, source maps, artifact manifests, compiler fixtures, and actual application-sized Rust ports.

The coherent implementation base was recovered from a September 7 source archive. It has since been adapted to standalone `mires`, repaired, measured, and extended. The missing late compiler was **not** secretly found in full: current code is a reconstruction with traceable recovered foundations and new patches.

Release profiles keep host-exception cleanup enabled. Significant improvements already on `main` include repeated-lowering reuse in cost trials, zero-sized aggregate destruction, string conversion work, producer-side empty landing-pad removal, cross-crate cleanup transport, exact integer overflow arithmetic, bulk byte-buffer host operations, String-builder length bookkeeping, pointer-element moves, alias-aware scalar replacement, copy/storage repair, and chunk/static initialization repair.

### What we do not have

The complete October dev13 source and final binaries were not recovered. The most aggressive Round 1 integration branch was not promoted wholesale. Its general cleanup-edge-removal pass, broader no-throw rules, wider inlining, and constant pooling are not accepted merely because they produced smaller packages.

Several safe Rust programs still silently produce wrong answers. The latest strict inventories are **5/6** on the October-derived cases, **2/5** on the Round 1 cases, and **0/4** on the newly preserved Round 2 cases. These are small targeted inventories, not a count of every remaining bug.

The public non-fingerprinting holdout is still deferred. Full current-candidate browser equivalence, quiet-machine runtime qualification, broad consumer/distribution qualification, and the original north-star targets are not complete.

### The latest positive evidence

The November 12 consolidation rebuilt the installed compiler and checked the combined accepted source: **1,077 Rust tests passed, seven were ignored, 75 runtime tests passed, 160 release fixtures passed, 13 chunk cases passed, and 60 active-poll cleanup cases passed**. Cleanup transport/admission, extern, and standalone checks also passed. Five copy-repair fixtures passed under the actual compact-package Rust optimization and MIR-inlining settings. The default-mode historical byte budgets were not applied to the release fixture run.

These successes do not erase the open inventories. Both facts belong in the verdict.

### What is safe to rely on after cleanup

The work in all 43 registered source worktrees was preserved, including nine with uncommitted source. Public reconstruction patches and research are committed. Private raw evidence and exact Git bundles are separately retained under `recovery/` and in an external same-device backup.

At this document's inspection, `.claude/worktrees/` was absent. Continued work should use the preservation index and restore commands, not assume an old worktree path still exists. The installed compiler binaries were present and matched the latest consolidation receipt; all 252 compiler-input records also matched. The local working tree was clean before creating this file. Recorded workflow commits were not pushed; the local remote-tracking comparison showed main ahead by 108 commits, which is not a live remote-server check.

**Primary current authority:** [combined-main verification receipt](docs/round2-consolidation-verification.json) and [cleanup/preservation handoff](WORKTREE-CLEANUP-HANDOFF.md). Historical reports remain evidence for their own identified stages.

## 2. Why the original v1-to-v2 work existed

### 2.1 Original product context

The compiler began inside Topcoat. It served a concrete framework/application design point: Rust client islands, typed JavaScript interaction, asynchronous browser work, and code sharing with a Rust server. The experimental FingerprintJS and Fingerprint Pro ports then stressed a much broader use case: substantial libraries and browser programs, not only a small framework-controlled component.

The goal was not simply to translate Rust syntax into JavaScript syntax. rustc already supplied type checking, monomorphization, layouts, drop elaboration, and coroutine transformations. The problem was how to preserve enough of that meaning while generating JavaScript that was compact, fast, debuggable, and faithful to the admitted Rust behavior.

### 2.2 The v1 diagnosis

The August architecture review identified a text-first boundary: individual Rust/MIR items became JavaScript AST/text and runtime-shaped objects before the linker had finished deciding representations and cross-function interfaces. Later passes then had to infer facts that the Rust producer had previously known.

Typical costs included pointer/slot wrappers, aggregate objects, iterator and enum carriers, standard-library helper clusters, explicit Rust-shaped coroutine machinery, and missed cross-function simplification. A locally short replacement could still retain a large dependency cluster. Conversely, removing one formatting operation could evict a large amount of otherwise reachable machinery.

The conclusion was to retain the rustc integration and useful backend components, while making typed semantic information survive into whole-program linking. It was **not** evidence that rustc was the wrong frontend or that another hand-written source parser was needed.

This also was not a theorem that v1 could never be improved. It was an engineering argument that recovering ownership, effects, representations, and call contracts after printing JavaScript was becoming an increasingly expensive approach.

**Source:** [original compiler architecture review](recovery/v2/snapshots/2026-09-05/documents/COMPILER-ARCHITECTURE-REVIEW.md), especially its executive conclusion and canonical-IR recommendations. This is an ignored local historical recovery input, not an active release verdict.

### 2.3 Original v2 goals

The original north-star program sought materially better bundle density and runtime without weakening behavior, source ownership, or decline/fallback rules. Its ambitious targets included at least 30% lower gzip and Brotli on substantial programs, at least 15% protected p50 and 10% protected p95 runtime improvement, and no protected runtime cell worse than 1.05 times v1.

The intended technical path included physical representations and private ABIs, product/result lanes, slices and range proofs, removal of unreachable panic/location clusters, cross-call scalar replacement, iterator/ADT virtualization, typed numeric storage, cleanup-aware representations, native continuation research, and exact runtime/dependency closure.

Those were goals and staged experiments, not facts automatically true of every later candidate. Some reduced mechanisms passed; several broad economic and release gates did not.

**Source:** [original north-star plan](recovery/v2/snapshots/2026-09-05/documents/RUSTC-CODEGEN-JS-V2-NORTH-STAR-PLAN.md), including its minimum-improvement and definition-of-done sections.

### 2.4 The framework work was a separate dimension

The keyed-view effort addressed row identity, live item/index state, row owners, disposal, movement, hydration, and browser behavior. Compiler optimization could make a list implementation smaller, but could not make an algorithm that rebuilt every row behave like a fine-grained keyed algorithm without changing the framework execution model.

This distinction remains useful: an application algorithm, a host bridge, a compiler transformation, and a runtime policy can each affect a benchmark. Their benefits must be attributed separately.

Topcoat-specific keyed DOM and hydration requirements no longer define the standalone compiler's release. Generic ownership, callbacks, exception cleanup, workers, source maps, and lifecycle interoperation still do.

**Source:** [original keyed-view plan](recovery/v2/snapshots/2026-09-05/documents/RUSTC-CODEGEN-JS-V2-KEYED-VIEW-PLAN.md).

## 3. What the original v2 program tried and learned

### 3.1 The original success was broader than one kernel

The original implementation established a usable semantic linker architecture and a substantial experimental program. It exercised ordinary Rust containers and formatting, real application-sized ports, cross-crate calls, host effects, async transitions, workers, packaging, prebuilt consumers, and source maps. These are the assets the reconstruction is trying to retain.

Some representative historical wins and limitations are recorded below. They are **historical observations**, not measurements of current `mires`.

| Historical area | What worked | What must not be inferred |
|---|---|---|
| Formatting | Construction/use recognition and carrier fixes removed substantial formatting dependencies; one recorded idiomatic package fell from 65,525 to 48,847 gzip bytes. | Every formatting shape worked, or the same number belongs to today's cleanup-on candidate. |
| Worker output | Host-handle/view lowering and runtime reachability repairs reduced worker overhead. | A universal worker-runtime floor, or equivalence to a tiny worker that imports its main module. |
| Strings | Later work addressed conversion scaling, builder byte lengths, byte views, and opt-in union/literal policies. | All pointer/capacity/mutation observations were automatically supported, or union strings were the default. |
| Constant pooling | Repeated immutable referenced tables could save meaningful bytes when sharing was legal. | By-value mutable copies or host-accessible data may always be pooled. |
| Trailing arguments | Eligible private direct calls could omit unused trailing values with escape and effect controls. | Public function arity, escaped function tables, or argument side effects can be ignored. |
| Scalar/ABI/iterator work | Narrow product, range, slice, and iterator mechanisms reduced carriers and some helper clusters. | A synthetic allocation-free kernel proves whole-application density or runtime admission. |
| Cleanup and lifecycle | Substantial producer, transport, drop, cancellation, and owner-related cases were investigated and repaired. | Host exceptions are identical to every native panic behavior, or all cleanup-on economic goals were met. |
| Distribution and tooling | Candidate identities, clean-archive consumers, manifests, and maps became more rigorous. | Production signing, debugger behavior, timing, and every browser/profile combination were all qualified. |

**Source:** [late-stage recovery ledger](recovery/v2/LATE-STAGE-WINS.md). Preserve the original experiment scope beside each number.

### 3.2 Important negative findings

Typed storage often worked in reduced bounded kernels but selected nothing in complete workloads. MIR inlining and crate partitioning could destroy the facts that a semantic optimization needed. Native byte operations could be attractive in isolation but lose their benefit after validation, conversion, fallback, and dependency costs were included.

A smaller helper was not necessarily a smaller package. A legal representation was not necessarily selected. A selected representation was not necessarily profitable. A profitable local transformation could still conflict with another pass or change a hot-loop allocation decision.

The experiments therefore supported a progression that remains central today:

```text
recognized → proven legal → selected → emitted → behavior checked → full-program benefit measured
```

Skipping a stage led to overstated conclusions. In particular, a zero-selection test could pass its output checks without testing the optimization it purported to validate.

### 3.3 The earlier reviews and competing proposals

The earlier `f_1.md`, `f_2.md`, and the synthesized `f_3.md` discussion challenged the project from different angles. The useful conclusions were to investigate emitted-code costs directly, separate compiler time from generated-program runtime, test faithful versus restricted-native alternatives explicitly, and fix repeated string conversion rather than focus exclusively on bundle graphs.

Some proposed shortcuts were not accepted as established facts. An aggressive minifier passing a handful of hash inputs did not prove semantic equivalence for getters, exceptions, exports, and all host effects. A handwritten JavaScript prototype did not prove that the compiler could legally emit it. SSA bindings did not by themselves prove that captured storage was immutable. A generator's `.return()` did not by itself solve destruction of captured values before first poll.

Those review conclusions survive as work-package requirements and archived design discussions. They do not imply that every suggestion was implemented or experimentally settled. The root restoration plan is the executable roadmap; the original review text is not assumed to be fully present in this repository.

### 3.4 The final old compiler was not a flawless target

Earlier discussions identified dev12; recovery later found October 7 dev13 evidence. Those later receipts identify revision `b2e8718dbfe54422bf6a5cb856b9d315f10a08bd` and candidate identity `dd9c6d8a77b0bd7f903ff4d9f327eb84023ee5302ae205131cfe9508059eb680`.

They record further successful tests, including a clean-archive fixture run, but retain production and timing qualification limitations. Some dev12 problems had been addressed by dev13, so older failures cannot simply be assumed still present. Conversely, receipts for dev13 do not substitute for its missing source.

The restoration target is therefore **the useful demonstrated capabilities and economic benefits, with explicit semantics**, not reproducing every old limitation or claiming the lost candidate was production-complete.

**Sources:** [recovery status](recovery/v2/manifests/FINAL-STATUS.json), [missing October source](recovery/v2/MISSING-OCTOBER-SOURCE.md), and the historical/conversation-derived status distinctions in the restoration plan.

## 4. The loss and what survived

### 4.1 What was lost

The original local project and its latest source were no longer available. The original Mac/recovery directory was confirmed unavailable by the user. The new standalone repository initially contained an early compiler import, not the final v2 middle end and linker.

The first new-repository commit was `4ab4bd6b32751c861a3720eafebe77ce1b8975ba`, importing the older Topcoat spike from `f325bc93`. The initial active workspace lacked the later `jsc-link`, `rcgjs-object`, and `rustc-cleanup` architecture. Treating that initial tree as an almost-complete October compiler would have been a serious misclassification.

### 4.2 What the copied experiments contained

The copied `experiments-2026-09-05/` directory was much more than benchmark output. Despite its name, it contained material through October, including source archives, individually retained files, manifests, reports, reference packages, and implementations.

| Recovery input | What was established |
|---|---|
| September 5 archive | Original archive SHA-256 and all 5,389 inventory entries checked before selective extraction. |
| September 7 producer-integration archive | A coherent compiler source snapshot; 2,378 extracted inventory files matched. One npm configuration file was deliberately excluded. |
| September 23 inventory | 1,558 exact files recovered; 125 missing; 19 comparison files excluded from that recovery scope. This remained a partial tree. |
| October dev13 receipts | Four receipts and all 156 listed files preserved and hash-checked. Full October implementation/binaries absent. |
| Source variants | Multiple retained original versions with content identity and provenance, not an automatically merged latest compiler. |
| Initial recovery verification | 8,544 restored files and 3,755 content-addressed objects checked with no mismatches in the recorded scope. |

The count of missing September 23 files is not a complete October change list. Finding those 125 files would not, by itself, recreate dev13.

### 4.3 Recovery, porting, and reconstruction are different

**Recovered:** original bytes matched an archive or source inventory.  
**Ported:** recovered implementation was adapted to standalone layout/names/interfaces.  
**Reconstructed:** a missing later capability was written again using its contract, reports, and regression evidence.  
**Proposed:** an idea or experiment has not yet established an implementation result.

These labels must remain distinct in code provenance and reports. The archive contains rejected experiments and incompatible intermediate schemas. Picking the newest-looking file for every path would not create a coherent compiler.

### 4.4 What is no longer blocked on recovery

We have original source for the architectural foundation and much of the optimizer; the OSS reference source; the exact historical Pro reference module and declarations; the Rust ports; and the seven other-language comparison families. We do not need to recreate these from memory.

The remaining work is chiefly **semantic repair, integration, and reimplementation of selected late mechanisms**, not searching forever for the old computer or replacing the frontend.

**Sources:** [recovery README](recovery/v2/README.md), [recovery manifest](recovery/v2/manifests/FINAL-STATUS.json), and [missing-source ledger](recovery/v2/MISSING-OCTOBER-SOURCE.md). The README describes the recovery stage; its statements that the active compiler was unchanged or a rebuild had not completed are historical, not today's state.

## 5. The standalone goal and its boundaries

### 5.1 Current goal

Build a standalone Rust-to-JavaScript compiler that preserves its declared Rust/host behavior, keeps cleanup on in release configurations, compiles representative real programs at acceptable compiler cost, and restores the useful late-v2 improvements in output size and application runtime.

The current strategy is to retain the semantic architecture, repair the representation and proof gaps, and measure transformations on actual delivered programs. No v3 rewrite is on the critical path.

### 5.2 What standalone means

The compiler must work without Topcoat's server, view macros, islands application, reactive owners, procedure routes, or framework-specific release process. It may retain small compatibility crates and legacy lowering while those are disentangled deliberately.

The active workspace still has `view-abi`, `view-async`, and legacy template marker support. That is not the same as requiring the Topcoat framework. The standalone example and general Rust library workflow do not need a Topcoat application.

The old keyed tests are useful historical evidence but should not force restoration of a large UI dependency solely to verify a generic callback feature. Where legacy lowering remains supported, use standalone/pre-expanded contract fixtures or clearly retire the unsupported path.

### 5.3 Four independent success dimensions

| Dimension | Question |
|---|---|
| Correctness | Does an admitted program produce the required values, effects, errors, and destruction/lifecycle behavior? |
| Compiler economics | How much time and memory are required to build, link, and package it? |
| Generated-program economics | How much does the delivered graph transfer, initialize, allocate, and execute? |
| Product/reproducibility quality | Can another consumer rebuild, debug, identify, and use the compiler without stale dependencies or missing artifacts? |

A speedup in compilation is not a JavaScript runtime speedup. A package-size win is not a proof of semantics. An archive hash is not a pass of its tests. A functional test is not a full application-ownership or browser-equivalence certificate.

### 5.4 Scope changes do not silently satisfy old gates

The broad root plan included a public document-processing worker holdout and a correctness-repaired R1. Later execution was narrowed to a compiler-first track, deferring the holdout and some qualification work. That was an execution-scope decision, not completion of those requirements.

Similarly, the newer frozen v1 size targets are a concrete optimization objective. They do not redefine the original October restoration goal or supply a missing v1 runtime baseline.

**Sources:** [root restoration plan](V2-OPTIMIZATION-RESTORATION-PLAN.md), [Round 2 report](round_2_report.md), and the latest addenda in [task ledger](tasks/todo.md).

## 6. Chronology and source lineage

Dates below are the dates recorded in source/recovery receipts and reports. They are not inferred from an old directory name.

| Period/checkpoint | Event | Current significance |
|---|---|---|
| August 2026 | Architecture review proposed semantic whole-program linking while retaining rustc. | Design rationale, not a new task to repeat. |
| September 2026 | v2 architecture, physical representations, cleanup, formatting, and application experiments developed. | Coherent recovered implementation base plus later incomplete snapshots. |
| October 6–7, 2026 | dev12/dev13 integration and qualification evidence. | Historical capability/size waypoints; complete source not recovered. |
| Initial `mires`, `4ab4bd6` | Early spike imported into the standalone repository. | Not the later v2 implementation. |
| November 8 recovery/assembly, `93c0785` | Recovered v2 architecture installed and adapted; compiler, ports, and tools operational. | Beginning of the current implementation lineage. |
| November 8 comparator restoration | Seven non-Rust families restored for OSS and Pro, built and checked offline. | Real comparison sources available again; browser/product parity not qualified. |
| Round 1, November 8–11 | Compilation-cost, string, ZST, cleanup, arithmetic, host-buffer, and representation work. | Some accepted on main, some only on the historical integration branch. |
| Round 1 promotion, `b1bcca4` | Selective integration; pointer-move and alias-store repairs; general cleanup-edge pass withheld. | Conservative working checkpoint, not the minimum-size integration branch. |
| Round 2, through `1b582b8` | A1 copy/storage and A6/A6b link repairs landed; A2/A4/A5 remained unfinished. | More correctness, slightly larger retained packages. |
| Consolidation, `a868dcb` | Combined-main checks, source/evidence preservation, portable tests, cleanup handoff. | Latest accepted state; no unfinished compiler branch promoted. |
| This document, November 12 | Histories reconciled; source/binary hashes checked against the latest receipt. | Documentation-only synthesis. |

The historical `integration` branch at `2f36c3f` remains a research source. It contains excluded cleanup optimization and must not be merged wholesale just because its reported packages are smaller.

The preservation commits on main are `7ac7530` (archive), `eba291e` (portable tests/tools), and `a868dcb` (combined-main record and handoff). They preserve source states without enabling unaccepted compiler transformations.

## 7. Architecture and responsibility map

### 7.1 Pipeline

```text
Rust client/library source and dependencies
    ↓
pinned rustc frontend, monomorphization, layouts and MIR
    ↓
backend + rustc-cleanup
    ↓
versioned semantic objects, identities, origins and cleanup facts
    ↓
jsc-link: JIR lowering, analyses, transformations, representation/ABI planning
    ↓
JavaScript emission and exact reached runtime/dependency closure
    ↓
packaged entries/workers, source maps, declarations/contracts, manifests and receipts
```

JIR-S is the source-facing semantic representation. JIR-R is the derived representation used for analyses and transformations. Their purpose is to retain typed operations and relevant behavior until whole-program decisions have been made, rather than make final JavaScript text the only link-time information.

The linker also has finished-JavaScript simplification machinery. That machinery must respect the distinction between JavaScript object aliasing and Rust value copies; the alias-store repair is a concrete example of why that distinction cannot be inferred from an assignment's superficial shape.

### 7.2 Active source map

| Area | Location | Responsibility and current cautions |
|---|---|---|
| Rust producer | [backend](backend/) | Produce exact instances, types, constants, calls, source facts, and cleanup-aware objects. Callable/static-pointer correctness remains important here. |
| Pinned cleanup bridge | [rustc-cleanup](rustc-cleanup/) | Adapt audited rustc drop/coroutine operations and retain required cleanup MIR. A pinned bridge is not arbitrary-nightly compatibility. |
| Semantic object contract | [rcgjs-object](rcgjs-object/) | Object/IR schemas, identities, options, representation metadata, artifacts, shared AST utilities and interpretation. |
| Independent linker | [jsc-link](jsc-link/) | Reachability, duplicate selection, module/chunk ownership, optimization, physical plans, and final output. |
| Analyses | [jsc-link/src/analysis](jsc-link/src/analysis/) | Effects, summaries, calls, references, legality, parameter uses and value flow. Unknown facts must remain conservative. |
| Transformations | [jsc-link/src/opt](jsc-link/src/opt/) | Inlining, DCE, devirtualization, ranges, escape/materialization, and related passes with checks/witnesses. |
| Planning/cost | [jsc-link/src/plan](jsc-link/src/plan/) | Representation and ABI alternatives, measured costs, reuse, rollback, and reporting. |
| Lowering/emission | [jsc-link/src/lower](jsc-link/src/lower/) | Preserve storage and effects while reconstructing JavaScript. Fusion cannot turn a copied value into source storage. |
| Static initialization | [jsc-link/src/link/statics.rs](jsc-link/src/link/statics.rs) | Binding order, strongly connected components, two-phase initialization, and unsafe split refusals. |
| Final AST scalar replacement | [rcgjs-object/src/sroa.rs](rcgjs-object/src/sroa.rs) | Simplify object/field use without splitting an aliased mutable object into independent values. |
| Control flow | [structurizer](structurizer/) | Reconstruct structured control flow from graphs. |
| Runtime | [runtime/shim.js](runtime/shim.js) | Storage, pointers, strings, exceptions, ownership helpers and host operations. Changes also affect cost-model inputs. |
| Stable build integration | [mires-build](mires-build/) | Invoke the pinned compiler from consumers and invalidate on relevant source/binary changes. |
| General host API | [mires-js](mires-js/) | JavaScript values, bindings, collections, callbacks, byte buffers and future interoperation. |
| Macros | [js-extern-macro](js-extern-macro/), [js-macro](js-macro/) | Typed external descriptions and explicit JavaScript expressions. |
| Driver and verification | [scripts](scripts/), [examples](examples/) | Reproducible compile/package commands, native expectations, compiler/runtime tests, strict failure inventories. |

### 7.3 The recurring failure boundary

Many remaining bugs arise when one layer assumes a fact that another layer did not preserve: a reference's storage identity, a copied aggregate's independence, a static pointer's projection or metadata, a function's hidden location argument, or a host value's runtime representation.

The remedy is to transport and validate those facts at the boundary, not to add application-name checks. This is also why a transformation that passes alone can fail after MIR inlining, private ABI selection, or another representation rewrite.

An eligible optimization should have an explicit account of what it reads, writes, allocates, can throw, exposes to the host, and requires on an exceptional edge. Diagnostics should explain why a rewrite was declined, not only how many sites were selected.

**Design context:** [JIR.md](JIR.md), [OPTIMIZER.md](OPTIMIZER.md), [CONTRACT.md](CONTRACT.md). Some inherited statements need reconciliation with the current limitations listed later in this document.

## 8. Current contracts, profiles, and supported use

### 8.1 Intended supported workflow

The operational entry model is a `#![no_std]` Rust client/library using real `core` and `alloc`, explicit exports, a panic handler, and general host bindings. It is not a claim that any arbitrary desktop `std` application can be compiled to the browser.

The standalone driver emits an ES module, the selected runtime module, authored source maps, a linker-owned artifact manifest, and a build receipt. The stable-Cargo example demonstrates that a consumer's `build.rs` can remain on stable while invoking the pinned backend for its client code.

The backend is pinned to `nightly-2026-07-27` through [rust-toolchain.toml](rust-toolchain.toml). Changing rustc, standard-library patches, object schema, or macro ABI is an explicit compatibility change, not a routine incidental update during optimization restoration.

### 8.2 Release profiles

`release-size`, `release-balanced`, and `release-speed` select the semantic JIR path and optimization with host-exception cleanup enabled. The established syntax ceiling is ES2018. The runtime may require newer host capabilities, so that ceiling is not a compatibility promise for an engine implementing only ES2018 facilities.

The user accepted investigating ES2018/ES2020 as a target baseline. No substantial general ES2020 optimization has thereby been implemented or qualified. A prior `latest`-syntax experiment saved only a few dozen minified bytes on the tested package; meaningful improvements require actual lowering/runtime changes and measurements.

`js-cleanup=off` is an explicit semantic opt-out for recoverable host exceptions. It remains useful as an attribution/control arm; it is not an equivalent replacement for cleanup-on release output.

### 8.3 Cleanup is more than printing try/catch

The required behavior includes the correct destruction of initialized values, preservation of moved/uninitialized state, exact thrown-value identity, effect order, and behavior across ordinary calls, resumed polls, and cancellation/lifecycle seams within the declared contract.

A reachable function that owes cleanup but lacks an admissible semantic body must not silently fall back to baseline text that loses those obligations. The active implementation reconstructs this fail-closed requirement. A missing body in an unreachable dependency should not poison an otherwise valid program.

Host exceptions, registered aborts, native Rust panic strategies, foreign boundaries, and engine resource failure are not interchangeable. Some destructor-throws-during-cleanup and coroutine-poisoning cases remain recorded limitations requiring a coherent contract and tests. Passing the active-poll fixture set is not a universal theorem about all destruction.

### 8.4 Typed host boundaries are not yet uniformly checked

The approved Round 2 direction is to check representations at typed host boundaries before enabling the withheld no-throw optimizer. A trusted binding remains an explicit opt-in contract, not the silent default.

A JavaScript Number supplied where the Rust representation expects BigInt, a forged `char`, an object with an observable `valueOf`, or an invalid pointer/view can cause supposedly ordinary operations to throw or change behavior. The optimized program cannot safely rely on a type annotation unless the corresponding representation invariant is established.

This work covers binding results, external results, callback arguments, export parameters, and representation-changing seams as applicable. It must also account for malformed values created by compiler bugs, such as flattened array reads: checking the foreign entry alone cannot fix incorrect internal lowering.

### 8.5 Refusal is not the same as silent wrong output

For an explicitly unsupported construct, a clear compile-time refusal or a declared registered abort may be a bounded engineering outcome. Returning an incorrect value from admitted safe code is not.

A baseline crash does not license a patch to return wrong values. The cyclic-static projection case demonstrates why complete-program behavior must be compared, even when the underlying decoder defect already existed elsewhere.

## 9. What has worked in the reconstruction

### 9.1 A coherent compiler was assembled

The first major success was restoring the recovered architecture into the active workspace rather than building another partial optimizer on the early import. Names and layout were adapted to `mires`, the old framework dependency was removed from the generic workflow, and source-backed compiler/package tests ran.

The initial assembled candidate recorded 1,021 Rust test passes, 138 fixture passes in its explicitly cleanup-off historical mode, 40 runtime tests, and scoped cleanup-on checks. Several later adversarial tests still failed. Those counts are preserved as assembly evidence and are not substituted for the larger current counts.

The initial optimized Pro attempt was interrupted. It was not assigned a made-up completion time. Later compile-cost work obtained completed optimized builds; the interruption is no longer the current operational blocker.

**Sources:** [assembly verification](docs/v2-assembly-verification.json), [standalone restoration record](docs/V2-RESTORATION.md).

### 9.2 Repeated compiler work was removed without changing output

Profiling showed large repeated costs in candidate measurement, including repeated item lowering, copying/reserving global string names, runtime-shim parsing, and large cost-cache keys. The first accepted changes retained the same emitted packages while avoiding redundant operations.

For the recorded S2 comparison, compact physical-item lowerings fell from 93,917 to 2,941, and Pro physical-item lowerings from 536,262 to 7,987. Runtime-shim parsing fell to one parse in each recorded link. The main commits include `9a940b3` and `a82e341`.

The remaining bottleneck shifted toward Brotli-11 evaluation of alternatives. Reusing equivalent work is distinct from changing the scoring policy. A cheaper trial compressor can be explored, but must be labeled a decision-policy change if it selects different output.

**Where:** [physical cost](jsc-link/src/plan/physical_cost.rs), [cost measurement](jsc-link/src/plan/cost.rs), [lowering reuse](jsc-link/src/plan/lowering_reuse.rs), and [profiling](jsc-link/src/profile.rs).

### 9.3 Zero-sized aggregates regained required destruction

Zero-sized values can have observable destructors despite having no ordinary data bytes. The initial reconstruction failed normal destruction of arrays stored locally, in a closure, or inside a vector. The landed work separates storage elision from drop obligations.

The latest six-case October-derived inventory passes the local ZST, closure ZST, vector ZST, named-reference, and reference-field cases. Aggregate-place identity remains open. This is meaningful recovered correctness, not merely an optimization.

**Where:** commit `dda8ae3`, [known-gap fixtures](examples/known-gaps/), [active cleanup smoke](scripts/cleanup-smoke.sh), and the latest consolidation receipt.

### 9.4 String conversion complexity improved substantially

The original reconstructed runtime repeatedly converted a string to UTF-8 and copied the bytes into a plain array. A four-entry memo was insufficient for parser workloads: repeated full-document encoding could turn a modest input into quadratic cumulative conversion work.

The landed runtime work adds short-ASCII paths, more economical native encoding/copying, operation-scoped bounded memoization, and cheaper byte-length handling. Historical profiling and workload experiments show a large reduction in the Markdown-shaped workload's cost. This is a concrete runtime improvement, even though the exact old timing ratios are not current-main admission evidence.

The memo is an implementation strategy with limits, not a full ownership/string-view model. Equal host strings can share cached byte views; mutable views are separated for containment, but supported mutable-string write-through and stable pointer identity are not universally solved. The cache budget is not a bound on every retained byte in every workload, particularly its separately handled large strings.

**Where:** [runtime/shim.js](runtime/shim.js), `str_bytes`, `str_len`, `_utf8`, and the string runtime tests. Commit `7bdd932` records the original landing.

### 9.5 Empty producer cleanup paths were removed

Drop elaboration can leave landing pads with no useful cleanup work. The producer-side no-op landing-pad repair removes that redundant structure while retaining executable cleanup. It produced a large measured size improvement and is on main.

This must be distinguished from the **general linker no-throw edge-removal pass**, which is not on main. The presence of the producer repair does not mean the disputed pass was accepted, and withholding the disputed pass does not undo the producer improvement.

**Where:** commit `d09a23d` on main, [rustc-cleanup](rustc-cleanup/), [no-op landing-pad test](scripts/noop-landing-pads-test.sh).

### 9.6 Cross-crate cleanup transport became real

An important discovered failure was Cargo's use of `.rmeta` while the cleanup information was in an adjacent `.rlib`. Missing transport meant downstream instantiations could use optimized bodies without required cleanup, and duplicate resolution could choose a less suitable instance.

The repair finds the sibling archive or fails clearly, carries the cleanup information through cross-crate use, and resolves duplicate candidates with cleanup obligations in view. Tests include metadata-only dependency paths, changed/warm builds, producer identity, corruption/version controls, and actual runtime drops.

This is a soundness repair, not merely a smaller-output pass. Some valid cleanup-bearing bodies become larger because previously lost precondition/cleanup work is restored.

**Where:** commit `8b5fd48`, [producer transport](backend/src/coroutine_source/transport.rs), [linker](jsc-link/src/link.rs), [transport test](scripts/cleanup-transport-test.py).

### 9.7 Arithmetic and byte-boundary work improved useful primitives

The overflow-arithmetic work avoids unnecessary closure-per-pair construction in applicable shapes and fixes exact 32-bit multiplication behavior that floating-point multiplication could get wrong. The alias follow-up preserves the pair representation through copies/moves and can carry a single observed half where legal. Some specialized carriers still select in reduced tests rather than complete workloads, so the mechanism and the application win are reported separately.

Bulk byte-buffer host operations replace per-byte crossings with a single checked operation where the interface permits it. This materially changed the workload's cost, but includes a host API/workload adaptation and should not be described as purely compiler optimization of unchanged source.

**Where:** commits `b8ce40f`, `b9973a0`, `d8a4e87`; [intrinsic lowering](jsc-link/src/lower/js/intrinsics.rs), [rvalue lowering](jsc-link/src/lower/js/rvalue.rs), [host web API](mires-js/src/web.rs), and extern fixture `03_bytes_bulk`.

### 9.8 Builder byte lengths and Unicode boundaries were repaired

A native String builder that recomputed the UTF-8 length of all accumulated text on each append remained quadratic. The builder now tracks byte length. However, concatenating two isolated surrogate halves before sanitation can produce a different result from sanitizing each incoming piece first.

That subtlety caused failed verification rounds. The landed version sanitizes each host string piece at the relevant point, preserving baseline representation behavior rather than accidentally pairing halves across appends or formatting arguments.

This mechanism is implemented and tested. It is **not yet a large whole-package win**, because the current planner still declines important real builder/format sites. Restoring selection/proof composition is separate remaining work.

**Where:** commit `d342e6f`, [String lowering](jsc-link/src/lower/semantic_ops/string.rs), [formatter kernel](jsc-link/src/lower/semantic_ops/fmt_kernel.rs), [builder runtime tests](scripts/runtime-string-builder.test.mjs).

### 9.9 Pointer-element moves and alias stores were repaired before promotion

For an array of references or raw pointers, moving an element must move the pointer value rather than overwrite the object it points to. The minimal pointer-move repair preserves that distinction while retaining write-through behavior for aggregates actually held by value.

Separately, finished-JavaScript scalar replacement had treated `x = y` as independent field copies even when it meant two names for the same mutable object. The conservative fix declines the split for components whose aliases perform field stores.

The old main passed a small lost-write case; the initial selected promotion candidate did not. That was treated as a composition regression requiring repair, not waived as an integration-branch base bug. The repaired candidate preserved all 12 measured public artifacts byte-for-byte in the recorded comparison.

**Where:** commits `26b41f6` and `045860d`, [runtime-moves tests](scripts/runtime-moves.test.mjs), [scalar replacement](rcgjs-object/src/sroa.rs), fixtures 130/131/202.

### 9.10 Copy/storage and chunk initialization landed in Round 2

A1 (`1362e12`) keeps a single-use value declared when the use requires storage, including shared interior-mutable views, and ensures appropriate no-clone locals do not write into a source value. This fixes `copy_alias` in both ordinary release and package-mode tests.

A6 (`cf80163`) prevents a chunk from declaring an item it only imports, and initializes supported cyclic statics in two phases. A6b (`7979702`) refuses unsafe import cycles involving module-level binding reads and conservatively rejects unsupported prototype-key cases.

These are supported by fresh combined-main tests. They still have adjacent limitations, especially static projections inside a cycle, which are detailed below.

## 10. What failed or was deliberately withheld

### 10.1 The smallest Round 1 branch was not the accepted candidate

The full integration branch reached 209,000 minified / 55,869 gzip bytes on compact OSS and 381,395 / 97,710 on idiomatic OSS. Those are real measured artifacts, and their standard kernels passed.

They also included general cleanup-edge removal whose proof enforcement and representation assumptions were not sufficient. Selective promotion retained the other wins and omitted that pass. The promoted packages were therefore larger: 241,745 / 61,301 and 443,971 / 111,722.

That difference is an intentional acceptance decision, not accidental loss of an optimization. The smaller implementation is preserved so it can be repaired and compared properly.

### 10.2 General no-throw cleanup removal lacked an effective independent gate

The withheld pass could remove exceptional edges using summaries before independent rederivation validated them. Later disagreement demoted facts without restoring the removed edge. Some witness categories accepted Rust/foreign operations too broadly, and release behavior could depend on an optional witness flag.

Planting false facts demonstrated why a counter or a post-hoc warning is not enough. Acceptance needs validation before mutation or reliable rollback, validation of the protected operation itself, and mandatory enforcement for release candidates. Shared tables used by both derivations can also limit independence.

This is a prerequisite repair, not a proposal to discard cleanup optimization. The expected savings justify making the proof boundary dependable.

**Where:** preserved `integration` and expanded-no-throw snapshots; [Round 1 audit](research/v2-restoration/rounds-1-2/reports/round1-audit-20261111T030722Z.md); ledger item `3b-gate`/Round 2 B2.

### 10.3 No-throw extensions trusted representations not actually guaranteed

The broader candidate classified more arithmetic/intrinsics as harmless. A mis-typed host result, an observable coercion, or a malformed view could then throw after the relevant cleanup edge had been removed. `compare_bytes` was one concrete rejected audit classification; removing it did not solve the underlying representation-boundary problem.

Early discussion considered documenting unsafe-only reinterpretation cases. Later counterexamples showed that the problem was reachable through safe typed bindings and through compiler-generated malformed array views. The accepted direction changed to checked host boundaries and repaired internal representation facts before promotion.

Trusted host declarations may remain an explicit restricted contract. They must not silently change destruction behavior in the normal release configuration.

### 10.4 Wider inlining exposed aliasing and economic weaknesses

The existing inliner had a bounded budget and minimum charges that effectively capped sites in ways that missed cheap/shrinking opportunities. A wider candidate improved many outputs and exposed the lost-write scalar-replacement defect.

The alias-store defect was repaired on main, but that does not certify the whole wider inliner. Re-probe caps, twin-declaration credit, caller-local accounting, representation-boundary declines, and loop-allocation effects still need qualification against the current source. A small emitted-size gain that adds hot-loop allocations is not automatically an acceptable balanced-profile choice.

The old wider-inlining patch is preserved as a candidate, not landed behavior.

### 10.5 Constant pooling failed host isolation

Pooling repeated immutable referenced tables gave attractive compact-package savings. Verification found that apparently internal data could reach host mutation through multiple routes: nested references, map values, C-ABI calls, erased values, transparent wrappers, array views, shared references containing interior mutability, and helper paths.

The copy-aliasing bug also made a local by-value write corrupt shared table data. Repairing that bug is necessary but does not prove pooling isolation.

The approved redesign is a sharing rule that cannot leak writable pooled storage by construction within its admitted scope. Repeatedly patching an incomplete host-reachability walk without a clear invariant is not the selected plan. Deep freezing is not automatically a semantics-preserving substitute: it can change what a host operation observes or throws.

### 10.6 Full pointer and allocator redesigns were parked

The broad storage-identity model failed several verification rounds. It mixed necessary small repairs with a cross-cutting model change. The project split out narrowly testable pointer moves rather than continuing to expand the merge blocker.

The exact-size/buddy allocator also failed verification and added package cost. The retained minimum address-release candidate is distinct: it aims to release synthetic address bookkeeping only when ownership is truly consumed. Its first version freed addresses at call-argument conversions even though the value remained alive in the callee, allowing live addresses to alias. A later drop-only variant was delivered but remains unqualified.

Do not merge the full redesign or drop-only candidate merely because an allocation stress test runs longer. Address identity, moves, conversions, and retained aliases must remain coherent.

### 10.7 Other ideas did not establish broad wins

| Attempt or proposal | Lesson/current treatment |
|---|---|
| More aggressive external minification | Candidate savings, not complete semantic validation; getters, exports, exceptions, and module contracts need controls. |
| Numeric spelling changes | Historical raw-output savings were not necessarily retained after Terser; later trials found no meaningful final-package saving for some changes. |
| Location interning/hoisting | Can worsen compression and cannot discard runtime-observable caller locations. |
| Native async/generator substitution | Requires explicit poll, capture, cancellation, pre-first-poll drop, pinning and wake behavior; not implemented as a blanket replacement. |
| A native-by-default policy | A separate policy experiment, not permission to remove capacity/address/alias observations from ordinary Rust. |
| Typed storage in reduced kernels | Useful mechanism evidence; complete workload selection/economics remain separate. |
| More modern JavaScript syntax | An acceptable target investigation, not a substitute for semantic representation improvements. |
| Benchmark-specific source rewrites | May be separately labeled application experiments; not generic compiler restoration wins. |

## 11. Round 2: landed work and unfinished candidates

### 11.1 What the round actually accomplished

Round 2 prioritized correctness because the large optimization levers depended on unresolved semantics. It did not claim a fresh broad size/runtime win. Three implementation changes landed: A6, A6b, and A1. Several independent verifications failed despite ordinary gates and package kernels passing.

That failure record is evidence that the tests needed more neighboring shapes. It is not a statistical estimate that a particular fraction of arbitrary compiler changes is wrong: these were selected adversarial attempts.

The report originally said the three landed changes had never been gated together and installed binaries were stale. Those statements were true at the pause, but are superseded by the November 12 consolidation. The current combined checks are complete in the scope recorded in Section 18.

### 11.2 A1's size cost and the unaccepted refinement

The copy/storage fix adds 527 minified / 298 gzip bytes to compact and 596 / 378 to idiomatic in retained A1-branch builds. The verifier's `tight5` experiment restores both packages to their previous exact hashes, but is not a generally accepted implementation and conflicts with an edited unit expectation.

A1b should refine a rule by value/storage meaning, not just reproduce a favorable package hash. A1c's real package-mode gate now has a portable script, but only its focused five-fixture subset was run during consolidation. Full-suite scheduling and the remaining mutation control are still open.

### 11.3 A2: flattened and heap arrays

**Status:** second round delivered, verification incomplete; restore ID `agent-afbe5ab2cda06b284`, original head `96b89e3`.

The scope includes `as_flattened`, `as_flattened_mut`, arrays in `Box`/`Vec`, row widths, byte rows, whole-array stores, zero-sized rows, and destruction. The first version inferred block identity from sizes/freshness/JavaScript shape and silently corrupted unrelated one-byte heap values or fresh byte vectors under certain views.

The second version claims type-directed row registration, removal of unnecessary hot-path lookups, and repairs of the reported controls. Its reported full-runtime cost is approximately +4,599 minified / +1,213 gzip bytes on compact and +6,144 / +1,640 on idiomatic, against its round-start baseline. These are implementer measurements, not a new main result.

Verification must decide whether row/view identity is coherent and whether unsupported observations are diagnosed. Runtime pay-per-use work should recover costs where program facts permit it; correctness is not optional while waiting for that optimization.

### 11.4 A4: niche constants and adjacent pointer/function constants

**Status:** third round delivered after two failures, final verification incomplete; restore ID `agent-a0a61a0aaa5ae144a`, original head `e2245df`.

The initial issue was decoding `Some` as `None` in constant data. Pointer provenance is not interchangeable with raw integer bytes, and niche layout must respect actual variant layout rather than an incorrect reconstructed numbering rule.

Later attempts expanded into static slices, dynamic metadata, and closure-to-function-pointer constants. Failures included single-present-variant enums, newly executable cyclic statics, one-element slice identity/write-through, and wrappers that invoked a closure with the wrong callable representation.

The final candidate reports local repairs/refusals and narrower interpreter handling. It remains pending. A complete gate run or byte-identical package alone is not a completed verifier verdict, and the 16-commit branch should be reviewed as a coherent set of contracts rather than judged by commit count.

### 11.5 A5: caller locations and callable identity

**Status:** second round delivered, verification incomplete; restore ID `agent-af471d6991ec28a6b`, original head `a420032`.

A direct-call instance with an extra location requirement cannot simply be used as a function-pointer target. A shim must also have an identity distinct from the function it wraps; otherwise duplicate selection can depend on hash/order.

The initial patch worked broadly but regressed a retained keyed callback shape. The second version repairs that path and adds generic host-callback coverage. The standalone compiler should verify the generic ABI without requiring restoration of the full Topcoat UI.

The related tracked-body-plus-virtual-call issue (A5b) remains separate and may require explicit location-forwarding representation/schema work. A4 and A5 must be tested together on constant function pointers and wrappers after rebasing.

### 11.6 A6/A6b: useful repair with a dangerous adjacent case

Supported cyclic statics are declared with empty storage and then initialized, so their references can be established without JavaScript temporal-dead-zone failures. Chunk ownership no longer imports and redeclares the same item. Unsafe import cycles involving binding reads are refused; function-only import cycles are treated separately.

The retained A6b tables contain 7,393 splits: 5,565 accepted and correct before/after, 1,333 newly refused former load failures, and 495 refused before/after. That is strong scoped evidence. It is not proof that every pointer embedded in every initialized static is correctly lowered.

A static first-field pointer is still decoded incorrectly. In a cyclic program, the old compiler failed at load; the new initialization machinery lets it return wrong values. The decoder defect is inherited, but the complete-program transition is newly visible. A7 repair or a precise refusal is urgent, not a minor post-size-gate cleanup.

**Sources for this section:** [Round 2 report](round_2_report.md), [Round 2 audit](research/v2-restoration/rounds-1-2/reports/round2-audit-20261112T005723Z.md), [snapshot index](research/v2-restoration/rounds-1-2/worktree-snapshots.json), and the latest consolidation receipt.

## 12. The current correctness inventory

### 12.1 Three inventories, three scopes

These inventories are intentionally strict: their `.expected` files contain the required outputs, not accepted miscompilations. A nonzero runner exit is the correct indication that the compiler is not yet fixed.

| Inventory | Latest recorded result | What it covers |
|---|---|---|
| [October-derived known gaps](examples/known-gaps/) | **5/6** | Reconstructed later regressions: ZST arrays in three storage contexts, reference fields, named references, aggregate-place identity. |
| [Round 1 open cases](examples/round1-open/) | **2/5** in release and package modes | Pointer-field recognition, copy aliasing, flattened arrays, lost alias write, niche constants. Copy aliasing and lost write now pass. |
| [Round 2 open cases](examples/round2-open/) | **0/4** in each mode, eight failing executions total | Array-rest write, large-enum copy, static projection in cycles, and a static field named `__proto__`. |

These counts do not sum to a complete language-conformance score. Some overlap in underlying cause; the larger ledger contains more cases and unsupported/refused shapes.

### 12.2 High-priority semantic families

| Family | Concrete failure/observation | State and implementation area |
|---|---|---|
| Aggregate-place identity | Reassigning a field can change the synthetic identity of storage that should remain the same place. | Open; design review, producer/runtime place model. |
| Pointer recognition by field names | A user struct with `pr`, `off`, `buf`, `sc`, `es`, or related names can be mistaken for an internal pointer record. | A3 open; explicit representation identity rather than shape guessing. |
| Flattened arrays | Reads past the first row become `undefined`, zeros, or exceptions; some writes do not reach original rows. | A2 pending; type-directed storage/view model. |
| Heap arrays | Boxed arrays and vector rows can be read incorrectly or abort. | A2 pending; allocation, retyping, indexing, drop, and view composition. |
| Niche constants | `Some` becomes `None`, or single-present-variant layout is decoded incorrectly. | A4 pending; producer constants and shared niche logic. |
| Static projections | Offset-zero reference to a field/element may be emitted as the whole static rather than the projected location. | A7 open; static target/type/projection identity. |
| Large enums | Above a variant threshold, shallow cloning shares nested payloads, so a copy's write changes its source. | A8 open; both baseline and JIR value-copy lowering. |
| Array rest patterns | A borrowed tail is treated as a copy and mutations are lost. | A8 open; slice/view lowering. |
| Prototype-sensitive field name | A literal `__proto__:` field is not emitted as an ordinary own data property. | A7/printer open; cycle refusal does not fix noncyclic objects. |
| Caller location and function pointers | Direct-call hidden arguments and pointer-call shims disagree or collide in item identity. | A5 pending; A5b virtual-call forwarding separate. |
| Host representation checks | Safe typed bindings can admit values that violate the representation assumed by optimizations. | B1 open; checked boundary policy approved. |
| Cleanup-proof enforcement | A false no-throw fact can remove cleanup without a mandatory successful independent check. | Withheld pass; B2 required before promotion. |

### 12.3 Representative wrong answers

The compact examples below identify the kinds of behavior, not an exhaustive reproducer body. Complete sources and expectations are in the linked inventory.

**Array tail mutation:** a safe mutable rest-pattern write should produce `99 / 99 / 3`; the current compiler produces `3 / 3 / 3` in the preserved case. A borrowed view must reach the original storage.

**Large enum copy:** writing the payload of a copied enum must not change the source payload. The preserved six-line native expectation is `12 / 992012 / 12 / 34 / 12 / 12`; the audited compiler produced `992 / 992992 / 17 / 129 / 17 / 992`.

**Static first-field projections in cycles:** the native program prints `1112 / true / 60506 / true / 80708 / true / 90910 / true`. The repaired initializer currently lets the misdecoded pointers run and produces zeros and false comparisons. The previous linker failed at load instead. This is precisely why "the decoder was pre-existing" is not a sufficient complete-program acceptance statement.

**Pointer-shaped user data:** the `address_fields` case demonstrates that normal field names can alter address observations. Metadata must be identified by an explicit compiler/runtime contract rather than collisions with user properties.

### 12.4 Additional recorded cases

The following remain relevant but are not all rerun by the latest small inventories: static dynamic-trait pointers losing vtables; slices/strings into statics losing lengths; closure `fn` constants; function-item references; tuple/closure clone shims writing before destination initialization; boxed-self dynamic calls; tracked virtual calls with cleanup; some boxed slices, row concatenation, array hashing, rotation/swap/sort shapes; panic-handler differences for an out-of-bounds vector operation; and baseline-only `-Oz`/MIR-inline failures.

Runtime address lifetime also remains incomplete. Recorded concerns include equal String contents sharing an address, conversions changing a vector's address, native-carrier drops leaking synthetic address bookkeeping, and large allocations overlapping fixed-size synthetic granules. The admitted meaning of addresses must be explicit even though JavaScript does not expose the same physical address space as native Rust.

Some unsupported reinterpretation cases can involve unsafe Rust. That classification must be made case by case. A failure reachable through normal safe Rust, normal representations, or safe library operations cannot be dismissed by citing an unrelated unsafe limitation.

### 12.5 How to classify bugs without distorting progress

Use a named baseline and separate the root cause from complete-program behavior:

- **Inherited and still wrong:** both previous main and candidate return the same incorrect result.
- **Introduced regression:** previous main returns the required result; candidate does not.
- **Newly exposed by added support:** previous main refuses/crashes; candidate now reaches a known defective path. A silent wrong answer is still not acceptable successful support.
- **Explicit unsupported/refused shape:** the contract names the boundary and the compiler reliably diagnoses it.
- **Unclassified:** no valid baseline run exists yet. Missing sysroots, timeouts, and failed setup are not behavioral controls.

A discovered bug increases the known-defect list without necessarily making the compiler worse. A patch that hides a defect behind a different failure also does not necessarily make it better. Track the actual observation and the support boundary.

**Detailed evidence:** [Round 2 report, correctness sections](round_2_report.md), [task ledger](tasks/todo.md), portable inventory provenance files, and archived verifier sources under [historical evidence](research/v2-restoration/rounds-1-2/historical-evidence/).

## 13. Fingerprint references and Rust experiments

### 13.1 FingerprintJS OSS

The original OSS source was recovered, including the archived package metadata identifying **5.2.0**, license/build inputs, and 101 TypeScript source/test files. Its reference build has been reproduced. The reference is not a newly invented JavaScript approximation of the Rust port.

Active source locations:

```text
experiments/fingerprintjs/
experiments/fingerprintjs-rust/
experiments/fingerprintjs-rust-compact/
experiments/fingerprintjs-rust-hybrid/
experiments/fingerprintjs-compare/
```

The idiomatic and compact Rust ports are central same-source compiler benchmarks. The hybrid is a separately labeled implementation strategy, not interchangeable with language-owned Rust. Historical source-percentage estimates must not be interpreted as exact post-minification/post-compression byte attribution.

The standard offline kernel checks exercise hashing, component serialization, and selected deterministic inputs. They are useful and reproducible, but do not execute all browser probes, every export behavior, or complete application lifecycle.

### 13.2 A remaining Rust-port API mismatch

During the Round 1 promotion, expanded numeric-seed tests passed **360/477** for each Rust OSS port, with the same 117 failures as before that promotion. Some non-finite, fractional, negative, or oversized inputs were rejected by the typed Rust interface where the JavaScript reference normalizes them.

This is an adapter/API-contract issue, not a reason to weaken the compiler's internal type representation checks. Decide whether the public export accepts the JavaScript reference's broader inputs and implement the conversion at that boundary, preserving the internal Rust contract.

The seven other-language OSS families were repaired to pass their expanded seed checks. That does not mean their complete browser behavior has stronger qualification than Rust, nor that a matching kernel validates the whole product.

### 13.3 Fingerprint Pro

The exact historical **4.1.4 self-contained reference module** and declaration file were recovered. The main module is 218,567 bytes with SHA-256:

```text
40c43c3afde41f9dbc3b15ac9f11dd4d3593fe4522f37ae22b04de6ea95144f9
```

The original TypeScript source checkout was only partially recovered: 36 original files, embedded original metadata, and 44 extracted transpiled module factories were preserved. Those factories came from an observed-comparison harness with substituted dependencies; they are not a complete production source distribution or replacement oracle.

Active local material is under:

```text
experiments-pro/fp-pro/
experiments-pro/fp-pro-rust/
experiments-pro/fp-pro-rust-compact/
experiments-pro/fp-pro-rust-hybrid/
experiments-pro/shared/
```

The optimized cleanup-on Rust main/worker build later completed, superseding the first assembly's interrupted attempt. Offline checks established imports, export surfaces, worker graph properties, declaration checksums, and selected invalid-configuration behavior. They did not establish full request, cache, collection, worker-drain, cancellation, retry, or browser parity.

The exact reference bundle is therefore a valuable historical oracle, but the incomplete source and service-dependent behavior impose real test boundaries. This material remains private and ignored by Git. Do not publish it, embed it in public source maps, or copy raw private logs into committed evidence.

### 13.4 Reduced browser workloads

The ten restored diagnostic workloads are:

```text
minisearch     marked         fflate        idb           idb_lines
ky            uplot          explorer      workbox       workbox_direct
```

Their names describe the workloads they model; they are not declarations of complete API parity with the upstream libraries. They provide useful coverage of strings, search, byte processing, storage, host calls, workers, and composition.

The Markdown implementation is a line-oriented subset; reference adapters and selected inputs make particular comparisons meaningful. The search implementation uses different scoring/indexing choices from MiniSearch. The gzip workload uses stored blocks and a different CRC implementation; a reference check also normalizes specified header differences and has known input boundaries. Consequently, a Rust/JS runtime ratio here includes algorithm, library, and host-adapter costs, not only compiler overhead.

### 13.5 The missing public holdout

A complete, public, non-fingerprinting document-processing worker application remains in the roadmap. It should combine parsing, UTF-8, searching, formatting, and a worker under a fixed behavioral contract, with sources/inputs frozen before relevant optimization tuning.

The reduced corpus is not automatically that holdout. A JavaScript application that merely orchestrates a few compiled kernels does not establish a language-owned end-to-end result. The holdout need not have seven language ports on day one: its first purpose is a generality control for the Rust compiler.

**Sources:** [experiments guide](experiments/README.md), [workload scope/method](experiments/browser-workloads/README.md), [promotion verification](docs/round1-promotion-verification.json), and the recovery/comparator records.

## 14. Other-language implementations

### 14.1 Restored families

All seven non-Rust families were restored for both OSS and Pro: 14 implementations. The source came from 1,155 archived port files plus shared comparison support. Build portability and several narrow semantic adapter problems were repaired.

| Family | Toolchain recorded at restoration | OSS production-comparison artifact |
|---|---|---|
| ReScript | 12.3.0 | `dist/fp.esm.min.js` |
| Melange | 7.0.1-53 with OCaml 5.3.0 | `dist/fp.esm.min.js` |
| js_of_ocaml | 6.4.1 with OCaml 5.3.0 | `dist/fp.esm.min.js` |
| Scala.js | Scala 3.8.4; Scala.js 1.22.0 | `dist/full/fp.esm.min.js` |
| Dart2JS | Dart SDK 3.13.7 | `dist/o2/fp.esm.min.js` |
| Kotlin/JS | Toolchain 0.12.0; Gradle 9.4.1/Kotlin plugin 2.4.10 | `dist/production/fp.esm.min.js` |
| Elm | 0.19.2 | `dist/production/fp.esm.min.js` |

These are recorded historical toolchain identities, not claims about the latest upstream versions today. Build receipts bind the actual used tools and artifacts.

Do not compare a root `dist/` development alias against another compiler's production output. Scala fast/full, Dart O1/O2, Kotlin readable/proxy/production, and Elm standard/compiler-aware minification are intentionally different rows.

### 14.2 What passed

The recorded restoration run passed all 14 builds and 62 check commands: 928 standard OSS comparisons, 7,632 numeric-seed comparisons, 83 package assertions, and seven Pro invalid-start-configuration suites. Sixteen OSS artifact variants include aliases; they are **seven implementations, not sixteen independent ports**.

The checker rejects mismatched artifact identities, and build receipts record unchanged input sources. Declarations are copied contracts; a checksum match does not prove inferred types or every implementation behavior agrees.

### 14.3 Repairs that mattered

Scala.js and Kotlin numeric seed conversions saturated values where the reference required wrapping. Dart's non-finite seed conversion threw. Elm's bridge decoded fractional seeds incorrectly. These were corrected at the appropriate implementation/adapter boundary without relaxing expected results.

Build repairs addressed root dependency lookup, local OCaml switches, Kotlin output discovery, and explicit Dune job counts. The local OCaml switch is isolated under `.toolchains/opam`; a pinned package export is retained.

### 14.4 What these comparisons cannot yet prove

No restored-comparator three-browser differential and quiet-machine performance qualification was performed. Several original Pro ports were built against a narrower comparison surface than later Rust Pro work. Full source ownership and deep product branches still need matched contracts. The Elm OSS version has a substantial JavaScript host and must not be labeled all-Elm.

Separate three questions: Rust semantic conformance; same-Rust compiler before/after improvements; and cross-language application cost. Other languages are useful application-level comparators, not correctness oracles for Rust's ownership/drop semantics.

Pinned packaging dependencies had seven reported npm audit findings in the restoration record, including three high-severity findings. That is a dated maintenance observation, not a fresh security scan. Upgrading those dependencies needs its own rebuild/recheck rather than silently changing a benchmark baseline.

**Sources:** [comparator guide](docs/COMPARATORS.md), [restoration status](docs/comparator-restoration-status.json), [recorded toolchains](docs/comparator-toolchains.json).

## 15. Bundle-size history and how to read it

### 15.1 Distinguish the baselines

| Name | Meaning | Proper use |
|---|---|---|
| Historical v1 | Recovered checkpoint's baseline lane with frozen v1 optimizer, cleanup off. | Frozen size objective; not a current rebuild or runtime baseline. |
| October dev12/dev13 | Late original project before the loss; related but different source/feature/qualification states. | Capability/history waypoint; not byte-exact current candidate identity. |
| Assembly | First executable standalone reconstruction. | Recovery baseline with known failures and an interrupted initial optimized Pro attempt. |
| R0 | Recorded reconstructed compiler and workload measurement on November 8. | Before-repair economic diagnosis; not correctness-qualified R1. |
| Full Round 1 integration | Candidate containing the withheld cleanup-edge optimization. | Research comparison, not accepted main. |
| Round 1 promotion | Conservative selected source plus necessary pointer/alias repairs. | Accepted development checkpoint with fresh package builds. |
| A1 Round 2 artifact | Verifier-built round-start source plus copy/storage repair. | Measured A1 cost; expected combined-main size, not a fresh full main package rebuild. |
| Latest consolidated main | Combined accepted implementation and updated checks/preservation. | Current source/behavior authority; no new package-size/timing qualification was performed in consolidation. |

### 15.2 Compact OSS

All rows refer to `fp.esm.min.js`; minified/gzip/Brotli bytes are separate measurements of the same selected package form.

| Checkpoint | Minified | Gzip | Brotli | Qualification |
|---|---:|---:|---:|---|
| Frozen historical v1 | 160,615 | 44,283 | 33,890 | Historical, cleanup off; recovered phase-3 table. |
| R0 reconstruction | 515,481 | 110,636 | 80,491 | Recorded same-session baseline, cleanup on. |
| Full integration `2f36c3f` | 209,000 | 55,869 | 43,533 | Measured candidate; not promoted wholesale. |
| Round 1 promoted selection | 241,745 | 61,301 | 47,412 | Fresh selected-source build; main replay byte-identical. |
| Round 2 A1 retained artifact | 242,272 | 61,599 | 47,650 | Independently remeasured; expected combined-main row. |

### 15.3 Idiomatic OSS

| Checkpoint | Minified | Gzip | Brotli | Qualification |
|---|---:|---:|---:|---|
| Frozen historical v1 | 259,527 | 64,908 | 53,064 | Historical, cleanup off; recovered phase-3 table. |
| R0 reconstruction | 822,496 | 186,452 | 135,211 | Recorded baseline, cleanup on. |
| Full integration `2f36c3f` | 381,395 | 97,710 | 77,454 | Measured candidate; not promoted wholesale. |
| Round 1 promoted selection | 443,971 | 111,722 | 86,772 | Fresh selected-source package. |
| Round 2 A1 retained artifact | 444,567 | 112,100 | 87,101 | Independently remeasured; expected combined-main row. |

On the retained A1 rows, gzip remains approximately **44.3% below R0 for compact and 39.9% below R0 for idiomatic**. Reaching the frozen v1 gzip targets from those rows still requires approximately **28.1% and 42.1% further reduction**, respectively. These percentages are arithmetic on identified historical rows, not a fresh benchmark.

The original October dev12 figures previously checked in the project history were 147,950 minified / 44,626 gzip for compact and 265,049 / 74,359 for idiomatic. They are useful reminders of the later program's progress, but are not inserted as controlled same-input results here: their exact source/configuration/packaging equivalence to current `mires` is not established, and their compression provenance differs from some newer comparisons. They must not be relabeled the frozen v1 row.

### 15.4 Compression and whole-program accounting

The frozen v1 row in recovered `phase-3.md` specifies **Node zlib level 9 and Brotli quality 11**. An earlier approved-plan copy incorrectly described it as system gzip. The authoritative checkpoint and the root Round 2 report resolve that discrepancy.

Report package bytes for the exact artifact whose behavior was tested. Count runtime modules, workers, wrappers, adapters, and transitively fetched resources. Compress separately fetched files separately. Do not sum individual compressed function sizes into an invented bundle total, and do not add non-fetched readable diagnostics to deployment cost.

A corpus sum is an aggregate of separately packaged workloads, not a claim that a user fetches all ten together. Corpus numbers in different documents also have differing compressed totals; use their exact receipt/compressor identity rather than normalizing them by guesswork.

The old integration's removed-`try` count is structural evidence, not an independent soundness proof. The marginal savings of pooling, inlining, cleanup, and runtime pruning overlap and are not additive.

**Sources:** [results log](experiments/v2-restoration/RESULTS.md), [historical phase-3 table](recovery/v2/snapshots/2026-09-07/compiler/checkpoints/phase-3.md), [promotion report](round_1_integration_report.md), [Round 2 report](round_2_report.md), and the archived audits. The recovered checkpoint is a local ignored input; the current public results log transcribes the selected target row with its caveats.

## 16. Compilation cost and application runtime

### 16.1 Compile cost: a demonstrated success

| Recorded comparison | Before S2 | After S2 | Interpretation |
|---|---:|---:|---|
| Compact link total | 70.9 s | 32.5 s | Reduced repeated compiler work. |
| Pro link total | 632.3 s | 146.1 s | Completed optimized link with substantial work reduction. |
| Pro package-build wall time | 654.1 s | 205.5 s | Not identical to link time; the later run also rebuilt a sysroot. |
| Compact physical-item lowerings | 93,917 | 2,941 | Deterministic work counter, stronger causal evidence than elapsed time alone. |
| Pro physical-item lowerings | 536,262 | 7,987 | Equivalent-output reuse, not a weakened semantic contract. |

The S2 recorded package outputs were byte-identical to their comparison baseline. These historical durations are not promised current build times. They were measured on the recorded machine/configuration, and peak memory did not improve in parallel with elapsed time.

The post-S2 Pro link spent roughly 104 seconds in Brotli-11 trials. The next compilation-economics question is how to avoid repeated exact work or use a separately qualified ranking policy without making final acceptance less accurate. A different trial compressor is not an output-equivalent cache repair.

### 16.2 Runtime: distinguish mechanism proof from admission

The string work's operation counts and profiling identified repeated UTF-8 work as the dominant cause of the Markdown-shaped slowdown. Historical per-call observations improved from roughly 80–88 ms to a few milliseconds on that workload. Paired comparisons recorded a large gap reduction versus the selected JavaScript reference.

Those observations support the mechanism and motivate structural string improvements. They do not establish the current main's complete runtime profile, particularly because some old ratios came from the full integration branch and some from the string-only checkpoint.

The search and gzip ratios include algorithmic differences. The CRC lookup-table versus bitwise implementation is application work, not purely compiler overhead. A same-build A/A control helps reveal noise but does not remove host load, engine caches, shared runtime state, or semantic mismatches.

### 16.3 Current runtime claim

The strongest accurate current statement is: **the repository contains successful scoped runtime repairs and a functioning measurement harness, but no authoritative same-candidate runtime qualification for the latest combined main**.

The latest receipt says `runtimeTimingQualified: false`. The original v1 was not measured on this workload trio; no v1 runtime speedup can be derived from current Rust/JavaScript ratios.

For the next run, bind the exact source, binaries, input corpus, artifacts, host adapters, engines, flags, and sampling protocol. Collect cold/import, first call, steady-state, allocations/GC and retained-memory observations separately where useful. Calibrate close-decision sensitivity before interpreting a 5% threshold. Preserve failed, contaminated, and inconclusive runs.

**Sources:** [results log](experiments/v2-restoration/RESULTS.md), [workload methodology](experiments/browser-workloads/README.md), [promotion report, compiler economics](round_1_integration_report.md), and [latest qualification flags](docs/round2-consolidation-verification.json).

## 17. The remaining optimization work

### 17.1 Cleanup-bearing bodies, not just edge deletion

Once typed boundary and proof prerequisites are satisfied, restore general cleanup-edge removal with mandatory validation. Then optimize bodies that still genuinely need handlers: inlining, scalar/escape work, effect-preserving fusion, and removal of helper dependencies should not be disabled merely because an exception edge exists.

The transformation must retain initialized-value obligations and the caller/callee cleanup composition across crate/object boundaries. Removing a guard that currently blocks optimization is not a substitute for expressing the missing proof.

**Where:** producer cleanup transport, `jsc-link/src/link/lane.rs`, effects/summary analysis, protected-call lowering, and the preserved cleanup pass. The withheld `plan/cleanup.rs` must not be described as an active-main implementation until restored and admitted.

### 17.2 Formatter and String commitment

The current tree already has a formatter kernel and semantic String operations. The task is not to implement native formatting from nothing.

A recorded planner defect proves a Move/Copy spine but commits only selected call-result carriers, causing representation verification to reject the result. Scratch spine marking can make reduced format cases commit. It does not, by itself, change the full idiomatic package because unsupported String operations, field access, unknown `format_inner`, and baseline-equality boundaries still prevent legal selection.

Restore exact operation/construction identity, support the required operations where lawful, carry the proven spine into commitment, and validate the resulting body. Preserve custom `Display`, unsupported options, UTF-8 and exceptional behavior. The success criterion is actual removal of the reached formatting dependency cluster in a complete package, not a selected-site count alone.

**Where:** `jsc-link/src/plan/representation.rs`, `jsc-link/src/lower/semantic_ops/fmt.rs`, `fmt_kernel.rs`, `string.rs`, and ledger P12 findings.

### 17.3 Structural string views and optional union strings

The current memoization repair should be complemented by representations that retain UTF-8 storage/views across eligible operations rather than repeatedly converting. Borrowed views, owned strings, mutable views, host strings, and retained aliases need explicit distinctions.

Union strings and bounded literal pinning remain experimental reconstruction work, not a current option that can simply be enabled or a decision to switch defaults. Required tests include byte slicing, non-ASCII boundaries, length/capacity observations, mutation, source ownership, pointer identity, escape and conversion/drop behavior. Memory retention must be measured alongside conversion counts.

### 17.4 Wider inlining and proof transport

The retained wider-inlining patch offers useful work on signed budgets, shrinking sites, private/twin declaration credit, and re-probe limits. Reapply it on the actual current base, not on the old integration tree that includes withheld cleanup changes.

Require effects and caller-location facts to survive remapping, test A4/A5 function-pointer interactions, and inspect changes in physical representations and loop allocations. Transporting `#[inline]` hints is a separate potential improvement with schema and semantics requirements.

### 17.5 Isolated constant pooling

Complete type/ownership/static fixes first. Pool only eligible immutable referenced data, using a rule that excludes host-accessible writable aliases by construction in its supported scope. Keep by-value copying independent.

Test nested references, transparent wrappers, array/slice conversions, trait objects, maps, C-ABI/erased routes, callbacks, interior mutability, chunk ownership, and reachability. Keep pooling-off counterfactuals and assert actual sharing when testing the positive path.

### 17.6 Smaller emission and runtime-closure work

Dead trailing arguments need exact-once evaluation and effect preservation, direct-call/escape analysis, and controls for function tables, public APIs, callbacks, and externally observed arity. Shared abort helpers must preserve actual nonreturning/error behavior. Module-scoped runtime experiments need multi-entry/realm behavior, not only shorter property names.

Numeric tags and literal printing must preserve layout and round-tripping; some shorter raw text disappears as a win after Terser. Runtime helper pruning should reflect reached behavior and valid specialized variants. Worker totals must include imports and per-realm initialization rather than comparing an independent worker with a tiny loader stub.

### 17.7 Cost-model stability and reuse

The whole-program compressed cost model is valuable, but compression discontinuities can flip a representation or flat-ABI decision after an unrelated shim edit. Recorded examples put allocations back into loops. This is not proof that exact compression should be discarded; it means profile constraints, hot-loop effects, and plan stability need explicit treatment.

Keep cached-versus-fresh lowering controls and all semantic dependencies in cache keys. When changing scoring, emit decision differences and compare the final delivered graph. Do not conflate a more efficient implementation of the same oracle with a new oracle that changes chosen plans.

### 17.8 Experimental ideas remain separate

A restricted-native policy, native continuations/generators, exact typed storage, and more aggressive runtime changes can still be investigated. Compare current faithful output, an improved faithful implementation, and a clearly restricted policy rather than assume a contract relaxation is required for every saving.

These investigations are not an excuse to leave ordinary value-copy or cleanup bugs unfixed, and their prototypes are not restoration milestones until behavior, selection, and whole-program economics are established.

## 18. Verification: what passes and what it proves

### 18.1 Latest combined-main execution

The November 12 preservation/consolidation task rebuilt main's compiler before testing the accepted combined Round 2 source. The compiler implementation was not changed by that task.

| Check | Recorded result | Scope/limitation |
|---|---|---|
| Formatting, Clippy, Rust workspace tests | 1,077 passed, zero failed, seven ignored | Includes unit/property coverage in the recorded setup; not universal source-language conformance. |
| Generic/runtime JavaScript tests | 75/75 | Runtime-level tests, distinct from complete compiled programs. |
| Release-balanced fixture suite | 160/160 | Cleanup on; historical default-mode byte budgets not enforced. |
| Chunk suite | 13/13 | Included by `check.sh`; supported/refused module shapes. |
| Active-poll cleanup | 60/60 | Three optimizer modes, required baseline refusals, maps and behavior in scope. |
| Cross-crate cleanup transport | Passed | Metadata/archive, identity/version/corruption, warm and runtime controls in the script's scope. |
| Cleanup admission | Passed | Reachable unsupported cleanup cannot be silently discarded. |
| Extern suite | Passed | Includes scoped host/global/byte-interface checks; not every host representation boundary. |
| Standalone consumer output | Passed | Values, cleanup default, manifest publication and authored source map. |
| A1 package-mode fixtures | 5/5 | Actual `-Oz` + MIR-inline settings; not the full suite in that extra mode. |
| October-derived inventory | 5/6 | Aggregate-place remains open. |
| Round 1 inventory | 2/5 in both modes | Address fields, flattened arrays and niche constants remain open. |
| Round 2 inventory | 0/4 per mode | Four wrong-answer programs remain wrong; no expectation was relaxed. |
| Preservation tools | Positive restore/extract checks and negative controls passed | Correct source-tree identity, content hashes, overwrite/traversal rejection. |

**Source:** [docs/round2-consolidation-verification.json](docs/round2-consolidation-verification.json).

### 18.2 Identity confirmed while writing this document

The 252 recorded compiler inputs still match the latest receipt. Its compiler-source digest is:

```text
0a9572f84933e8e972393e45df9843393efb275e1b08b142b9c94d28e598a70b
```

The installed artifacts were present and matched:

```text
jsc-link:
4e0d781e9725dc0bdfe29a6da821445387d0a34bea5966e80b9609604e9c0cec

librustc_codegen_mires.dylib:
026c1823a19701a7355c57a82113985a00c8bd2be127da28b32b89c7367e7b20
```

This is a content-identity check, not a rerun of the tests. If build products are removed or rebuilt, record the new binary identities instead of assuming these hashes will remain true across different paths/environments.

### 18.3 What remains unqualified

The full additional package-mode fixture suite was not run during consolidation. The relevant focused subset passed and a portable full-suite command now exists. Its CI scheduling and some mutation controls remain work.

No new all-target package build, three-engine browser matrix, protected runtime timing, clean external distribution, or release signing was performed in consolidation. Earlier stages have package results; those results retain their original source identity rather than being reassigned to latest main.

The comparator restoration and earlier corpus browser runs are also their own scoped evidence, not current main's browser qualification.

### 18.4 Why a green gate set is not enough

The historical gates did not cover important configurations and adjacent observations. Some once reported success with no executed corpus, a missing `.lane.json`, a skipped framework suite, or a failed upstream build receipt. Many of these were repaired, but the lessons remain.

A useful gate must make its selected path observable, require nonempty input, distinguish setup failure from execution outcome, bind artifacts to inputs, and test an independent required result. Mutation tests should show that a meaningful broken rule fails. A surviving mutant is not automatically proof of a real bug, but an untested behavior-changing path is not established by unrelated passing tests.

Do not invent success by increasing byte budgets, changing native expectations to current output, labeling deterministic failures intermittent, or treating a missing sysroot as a valid baseline refusal.

## 19. Evidence preservation after worktree cleanup

### 19.1 Public committed material

The authoritative recovery entry point is:

```text
research/v2-restoration/rounds-1-2/
```

It contains the source-tree index, patches from bases reachable from main, preserved reports, original counterexamples/generators/mutation tools, and provenance/hash records. There are 2,082 original historical source/report files plus indexes and patches. Redundant full compiler copies were not unnecessarily duplicated in public research.

The files have deliberately not all been rewritten into portable tools. An old script's absolute path is part of its provenance. Use the new restore/extraction commands to recover evidence, then adapt a copied driver deliberately when execution is needed.

### 19.2 Private retained material

The local private archive is:

```text
recovery/worktree-preservation-20261112T013511Z/
```

Its verified evidence database has **812,662 file records** and **146,007 unique contents**, representing approximately 28.3 GB before deduplication/compression. A supplement preserves **16,735 `.bin` records** rather than assume every binary-looking input is disposable. The project-session archive contains **224 files**. Raw evidence may include Pro material and unsanitized local logs, so it is intentionally ignored by Git.

A separate copy is retained at:

```text
/Users/mikhail.pertsev/mires-recovery-backups/worktree-preservation-20261112T013511Z/
```

The final source archive was checked against 4,302 files. Git bundles preserve refs/history and the stash. The final backup-copy verification covered approximately 2.78 GB. These are same-device copies; no verified off-device backup or push was performed by the recorded workflow.

### 19.3 What was excluded

Bulk native executables, object/rlib/rmeta products, dependency/compiler/sysroot caches, and installed language toolchains are not required contents of the evidence archive. Exclusions are recorded. Source, native expectations, emitted JavaScript, reports, and restoration inputs are retained in the declared scope.

This is a source/evidence recovery, not a disk image or a hermetic mirror of every external dependency. A future rebuild may need pinned dependencies to be installed again.

### 19.4 Source states and pending implementation IDs

| Candidate | Restore ID | Interpretation |
|---|---|---|
| A4 niche constants, third round | `agent-a0a61a0aaa5ae144a` | Delivered; unfinished verification. |
| A2 flattening/heap arrays, second round | `agent-afbe5ab2cda06b284` | Delivered; unfinished verification. |
| A5 caller location, second round | `agent-af471d6991ec28a6b` | Delivered; unfinished verification. |
| Wider inlining | `agent-ab531332a87a0b330` | Older delivered candidate; rebase/requalify. |
| Constant pooling | `agent-a12dd166f55ef46f8` | Known isolation failures. |
| No-throw extensions | `agent-aae8e384d9b94c2e6` | Known proof/boundary defects. |
| Minimal drop-address release | `agent-ac96e6c4714b6a992` | Unverified; separate from allocator redesign. |
| Full address allocator | `agent-acd9d634a1811beab` | Parked after failed verification. |
| Earlier copy-alias fix | `agent-aa0340f7b4c30f8c0` | Superseded by A1. |
| Earlier pointer-move repair | `agent-ae104b5a611c870ef` | Adapted and verified on main. |
| Earlier alias-store repair | `agent-ae6fe0b2bcf6f29f7` | Adapted and verified on main. |
| Earlier flattening repair | `agent-aeaee6e0ef235816f` | Superseded by A2; not independently accepted. |
| Old integration | `integration` | Includes withheld cleanup optimization; not a merge target. |

The index covers all 43 states, not just these prominent candidates. Nine uncommitted states became preservation-only snapshot commits under `archive/rounds-1-2/`. Their original worktrees were not altered by preservation. Patches were separately applied to recorded bases and reproduced the expected Git trees.

### 19.5 Current physical checkout state

At inspection for this document, `.claude/worktrees/` was absent. The public research and both named archive locations were present; `target/`, `node_modules/`, and `build/` were also present. The earlier handoff correctly says no deletions were performed by preservation; it does not assert the user never cleaned up afterward.

Do not make live instructions depend on `.claude/worktrees/...` or `/private/tmp/...`. Those paths in old reports identify origin, not the only remaining copy. Do not delete `.git`, the public research, private recovery archive, experiment/reference inputs, or the external backup as part of rebuilding caches.

**Sources:** [WORKTREE-CLEANUP-HANDOFF.md](WORKTREE-CLEANUP-HANDOFF.md), [research archive README](research/v2-restoration/rounds-1-2/README.md), [snapshot index](research/v2-restoration/rounds-1-2/worktree-snapshots.json).

## 20. Roadmap reconciliation and the next sequence

### 20.1 What the original P00–P21 plan means today

This table is a reconciliation, not a claim that old checklist boxes have been automatically completed. Some implementation proceeded before the planned R1 gate under the approved narrower compiler track.

| Work package | Present assessment |
|---|---|
| P00 — Seal R0 and reconcile documentation | R0 measured and source identities retained; this synthesis reconciles major stale statements. A single timeless baseline should not be inferred from an old import HEAD. |
| P01 — Measurement and pass controls | Harnesses, artifact checks, counters and controls exist. Protected timing, some non-vacuity/mutation controls, and full policy calibration remain. |
| P02 — ZST storage/destruction | Three important regression contexts fixed and still pass. Adjacent collection/lifecycle coverage and explicit residuals remain. |
| P03 — Place/reference and semantic inventory | Narrow reference/move/copy/alias repairs landed; broad identity/array/static/address defects remain. |
| P04 — Compiler cost | Major repeated-work reductions landed with equivalent output; compression cost and memory are further work. |
| P05 — Public holdout | Deferred; not implemented/frozen as the complete intended application. |
| P06 — Seal R1 | Not reached: correctness and holdout requirements remain. |
| P07 — Safe cleanup-edge removal | Prototype preserved; withheld pending representation/boundary/proof prerequisites. |
| P08 — Optimize retained-cleanup bodies | Still an important output-quality task, not solved by producer pad removal. |
| P09 — Protected/cross-crate proof composition | Cleanup transport repaired; broader protected inlining/representation composition incomplete. |
| P10 — Default-policy string scaling | Runtime improvement and builder bookkeeping landed; structural/full-workload selection work remains. |
| P11 — Union strings/literal pinning | Historical design/evidence retained; not restored as an accepted current feature. |
| P12 — Formatter commitment | Existing kernels, known proof/legality leads; whole-package historical gain not restored. |
| P13 — Immutable pooling | Candidate failed isolation; construction-safe rule and current-base validation needed. |
| P14 — Trailing arguments | Analysis foundations exist; complete late pass/escape/effect qualification still needed. |
| P15 — Numeric printing/terminal emission | Some historical ideas tested; not every proposed spelling produces final savings. Shared-abort work remains. |
| P16 — Worker/runtime/pointer views | Bulk-byte and narrow pointer improvements landed; reachability, lifetime, registry and worker work remains. |
| P17 — Interaction qualification | Several scoped composition runs completed; full current interaction matrix still incomplete. |
| P18 — Build/schema/delivery integrity | Manifest-bound packaging, linker invalidation and source identities restored; future schema changes and external consumers require qualification. |
| P19 — Comparator equivalence | Fourteen builds/offline checks restored; complete application/browser contracts and ownership matching remain. |
| P20 — Combined R2 economic/semantic matrix | Not reached. Latest combined main is a tested development state, not final restoration admission. |
| P21 — Close restoration and preserve it | Preservation now materially stronger; restoration itself remains unfinished. |

**Source:** [V2-OPTIMIZATION-RESTORATION-PLAN.md](V2-OPTIMIZATION-RESTORATION-PLAN.md). That document's starting-position statements describe the assembly date; this table provides the later state without rewriting history.

### 20.2 The next practical batch

**First, continue from a defined source state.** Current main is built and combined-tested; the old "run the first combined gate before anything" action is no longer outstanding. Re-run relevant gates after any new change, and restore pending work into isolated repositories/branches using its recorded base and patch. Never combine all preserved diffs.

**Second, close the urgent semantic boundaries.** Prioritize static projections and the newly executing cycle wrong answers, large-enum copies, array-rest writes, and completion of A2/A4/A5 verification. Test overlapping features together. A file conflict is a scheduling issue, not a semantic reason to indefinitely defer address-field or projection correctness.

**Third, make the test controls durable.** Run/schedule the full package-mode suite, add the remaining A1 mutation pin, harden refusal parsing, and commit A6c's multi-file/hoisted-constant/nested-prototype controls. Permanent tests should capture the decisive discovered shape, not only a nearby easier one.

**Fourth, implement cleanup prerequisites in parallel where independent.** Checked host boundaries and mandatory proof validation can be developed separately from array/caller-location work. Do not enable the no-throw optimization until the combined assumptions actually hold.

**Fifth, freeze a measured checkpoint.** Build compact, idiomatic, corpus and Pro from one identity; record package graphs, operations selected, compile time/memory, and controlled runtime observations. Run the public holdout before claiming the broader restoration gate. Correctness-only controls should separate required code growth from later economic changes.

**Then restore the remaining size/runtime mechanisms.** Formatter/String commitment, protected inlining, isolated pooling, dead trailing arguments, runtime closure and registry costs should be ordered by fresh attribution. A1b's small savings can proceed as bounded work but should not displace a silent-wrong-answer repair.

### 20.3 Avoid an endless bug-collection project

Group counterexamples by violated invariants and give each family explicit acceptance cases. A finite admitted contract and clear refusals are better than a vague claim of all Rust support with unbounded exceptions.

Do not require every unrelated research experiment to finish before a development checkpoint. Equally, do not claim correctness-qualified R1 by declaring every newly discovered safe-Rust case non-gating. The exact scope, support boundary, and known limitations must be stated together.

## 21. Finish lines and non-goals

### 21.1 Restoration milestone

A useful restoration candidate must be a single identified source/binary/configuration set that compiles the declared workloads, passes their behavioral contracts, demonstrates the selected optimization benefits together, and retains reproducible evidence.

It must resolve or explicitly refuse the agreed semantic inventory, keep release cleanup on, preserve effects and ownership across optimization, compile within an established resource budget, and generalize beyond fingerprinting. Its results must be compared against exact earlier same-source baselines, with historical v1/October figures remaining separately labeled.

Recreating the exact October bytes, commit order, or every rejected experiment is not required. Recreating a result that depended on a bug is not success.

### 21.2 R1 versus R2

R0 preserves the reconstructed starting point, including known failures. R1 is the correctness-repaired, operational cleanup-on baseline with the agreed holdout and controls. R2 is the integrated restoration/economics candidate after selected passes and interaction testing.

Current main is a meaningful development checkpoint before those full gates. Passing 160 fixtures does not redefine R1, and an archive snapshot is not R2.

### 21.3 Production qualification remains separate

Production qualification requires a declared support contract, reliable refusals, qualified engine/consumer configurations, exact delivered dependency closure, useful debugging/source maps, lifecycle behavior, reproducible distribution, and whatever release/signing policy the standalone project adopts.

The old Topcoat islands-specific signing and K6/K7 gates are historical context, not automatically this standalone release contract. Generic multi-entry, worker, host, async and artifact concerns remain.

No defensible completion date follows merely from the number of tasks or test passes. A finite scope and explicit gates make progress assessable without promising a date unsupported by the unresolved work.

### 21.4 Not on the critical path

A new frontend, a v3 rewrite, restoring the Topcoat application, recreating every comparison language experiment, making every possible unsafe reinterpretation work, changing toolchains mid-restoration, or chasing a small Murmur byte target ahead of semantic failures are not required next steps.

Native-policy experiments and continuation research remain legitimate future work when separately specified. They must not silently alter the contract of the faithful compiler track.

## 22. Commands for continuing work

These commands refer to the active standalone repository. They are documented workflows, not a claim that they were all run while writing this file. Builds and tests write generated output under ignored directories. Run from the repository root.

### 22.1 Build and smoke-check

```sh
npm ci --ignore-scripts
bash scripts/build.sh --locked
node scripts/mires.mjs build examples/standalone/hello.rs --out build/hello.mjs
node --input-type=module -e \
  'import {sum_squares, greeting} from "./build/hello.mjs"; console.log(greeting(), sum_squares(5))'
```

The recorded expected output is `Hello from Rust 30`. The example uses real containers/iterator operations, not a handwritten JavaScript kernel.

For stable Cargo build-script integration:

```sh
cargo +stable run --manifest-path examples/standalone-consumer/Cargo.toml
```

### 22.2 Main checks and package settings

```sh
bash scripts/check.sh
SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' \
  bash scripts/test.sh --require-core

# No fixture names means the entire suite in the compact-package Rust settings.
SKIP_BUILD=1 bash scripts/package-mode-test.sh

SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
SKIP_BUILD=1 python3 scripts/cleanup-transport-test.py
SKIP_BUILD=1 bash scripts/cleanup-admission-test.sh
SKIP_BUILD=1 bash scripts/extern-test.sh
node scripts/standalone-check.mjs
```

A matching sysroot is required. The package-mode helper builds its matching sysroot. For manual fixture runs, prepare the matching emit options explicitly rather than borrowing another mode's cache. Historical byte budgets are not applicable to every release/minification mode; report their scope separately from behavior.

### 22.3 Strict open inventories

```sh
python3 scripts/check-known-gaps.py
python3 scripts/check-round1-open.py
JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
python3 scripts/check-round2-open.py --mode both
```

These currently return nonzero because required behavior remains incorrect. They are development targets, not a substitute for the positive suite. Do not change expectations just to make them green.

### 22.4 Restore a candidate after its worktree is gone

```sh
python3 scripts/preserved-work.py list-work

python3 scripts/preserved-work.py restore-work agent-a0a61a0aaa5ae144a \
  --destination /absolute/path/to/new-restored-a4
```

The destination must not exist. Restoration uses a locally available base, verifies the patch, applies it to the index, and compares the exact resulting Git tree. It does not silently merge, qualify, or push the candidate. The result is an independent local repository with changes staged for review.

### 22.5 Verify and extract private evidence

```sh
python3 scripts/preserved-work.py verify \
  --archive recovery/worktree-preservation-20261112T013511Z

python3 scripts/preserved-work.py list-evidence \
  --archive recovery/worktree-preservation-20261112T013511Z

python3 scripts/preserved-work.py extract-evidence \
  --archive recovery/worktree-preservation-20261112T013511Z \
  --root scratch-bf1d1602-d3a2-4fa9-88f8-ca711756aab5 \
  --prefix verify-a1/prebug \
  --destination /absolute/path/to/new-restored-counterexamples
```

The archive is local/ignored and must be retained separately from a public clone. Extraction verifies hashes and records symlink targets rather than recreating external links into deleted worktrees.

### 22.6 Workloads and comparisons

```sh
npm run build:oss-reference
npm run build:corpus
npm run test:corpus

npm run setup:comparators:ocaml
npm run build:comparators
npm run check:comparators
node experiments/tools/measure-comparators.mjs
```

For smaller comparison runs, use `--languages` and `--suite` as documented in [COMPARATORS.md](docs/COMPARATORS.md). Pro builds/checks require the private local inputs and should retain private receipts. Exact Rust package commands, flags and output paths are in the promotion report and measurement driver; choose a new output/label rather than overwrite historical evidence.

```sh
# The measurement driver writes an identified new run; inspect its options first.
node experiments/v2-restoration/measure.mjs --help
```

Even a successful measurement-driver exit does not confer runtime authority if the machine/protocol is unsuitable. A read-only review of a receipt is different from rerunning it.

## 23. Decisions, rejected shortcuts, and working rules

### 23.1 Decisions that remain active

Keep the existing semantic producer/linker architecture, the pinned toolchain during restoration, release cleanup on, and fail-closed handling for missing required cleanup. Keep public and private inputs/evidence separate. Preserve immutable historical rows and exact source/artifact identity. Record an explicit support contract and do not change it implicitly to win a benchmark.

Checked typed host boundaries precede restoration of the withheld no-throw pass. Pooling needs a rule with a defensible non-leakage invariant. Flattening is intended to support real behavior, with pay-per-use runtime specialization where sound. Broad pointer redesign remains separate from narrow fixes.

Local commits have been authorized in previous implementation/consolidation tasks; pushing main was explicitly prohibited and has not been performed by those workflows. This documentation task does not independently authorize a push.

### 23.2 Execution lessons

Implementation and independent verification should be distinct activities. A verifier that stops because of credits, process failure, or missing infrastructure has no completed verdict. Tool/model labels are provenance, not evidence of correctness; do not claim an unavailable model version was pinned.

After repeated failures, split the work by semantic responsibility. A send-back should address the actual failed contract, not accumulate unrelated extra features. Keep adjacent bugs explicit; use precise refusal where needed instead of claiming newly reachable wrong behavior is inherited and acceptable.

Verify every worktree's actual starting commit. In this workflow, some agents started from a stale remote-tracking ref, not current local main. A branch label alone does not establish its source base. Restored patches likewise need rebase and full composed validation.

Run the settings that packages actually use. Preserve native O0/O3 controls where they help establish expectations. Use explicit nonempty inputs and independent positive/negative controls. Assign noncolliding fixture ranges across parallel workstreams.

Compiler/sysroot caches can occupy tens of gigabytes per worktree. Prefer shared immutable baselines where safe and isolated write paths. Treat source, raw evidence, build caches, and measured artifacts as different retention classes; do not delete uncommitted work because a directory looks temporary.

### 23.3 Reporting rules

| Do not write | Write instead |
|---|---|
| "All tests pass" when strict inventories fail | Name the passing suite and separately list required failures. |
| "Restored October compiler" | Name the recovered base, reconstructed changes, and missing late source. |
| "The optimization works" from a fallback run | Assert the transformation selected/emitted, then show behavior and economics. |
| "Pre-existing" after a setup failure | Record infrastructure failure and obtain a valid baseline execution. |
| "Faster" from a shared-machine timing anecdote | State the diagnostic measurement and qualification limits. |
| "Full Pro parity" from API/import checks | State the exact branches/host/network/worker behavior actually executed. |
| "Only qualification remains" with admitted wrong answers | Keep the correctness/admission tranche explicit. |
| "Backup complete" from copied filenames | Verify contents, source-tree reconstruction, scope exclusions, and off-device status. |

### 23.4 Three layers of test evidence per optimization

A behavioral test establishes values/effects/lifetimes. A selection test establishes that the intended transformation, not fallback, ran. An economic test establishes the relevant improvement including complete dependency and packaging costs.

An optimization may improve compile time without changing output, or improve a hot loop without moving package size. A correctness repair may legitimately grow output. Acceptance must match the intended benefit rather than force every change through an arbitrary whole-package percentage.

## 24. Stale statements and how to resolve them

The repository deliberately preserves old reports. Some statements are correct at their date and wrong as descriptions of today. Use this table rather than deleting historical evidence.

| Historical statement | Current resolution |
|---|---|
| "The active repository is just the early import." | Superseded by standalone v2 assembly and subsequent implementation. |
| "The compiler cannot yet be rebuilt because serde_json is missing offline." | A recovery-validation attempt, superseded by the successful active compiler assembly. |
| "Other-language ports were not restored." | Describes the earlier recovery/assembly scope. Seven families for OSS/Pro were subsequently restored and built. README retains a contradictory older sentence; the comparator section/receipts are the later authority. |
| "Optimized Pro does not finish." | The initial assembly attempt was interrupted; S2 and later selected-source builds completed. Current controlled runtime admission is still absent. |
| "Main is close to the huge R0 packages." | True before selective Round 1 promotion, not after it. |
| "The 209 KB compact integration package is current main." | False. General cleanup-edge removal was withheld; selected main is larger. |
| "Pointer moves and alias-store fixes are uncommitted pending work." | Older Round 1 candidates were adapted, verified and committed on main; archived originals remain for provenance. |
| "Copy aliasing is still one of the failing five Round 1 cases." | Superseded by A1; the latest inventory passes copy aliasing and lost write. |
| "Round 2 changes were never tested together; main binaries are stale." | Superseded by the November 12 consolidation; current recorded identities still match at this document's inspection. |
| "No package-mode gate exists." | A portable command now exists and five A1 fixtures passed through it; full extra-suite execution/scheduling remains open. |
| "The verification evidence exists only in /private/tmp." | Public source/report preservation and private content-addressed evidence are now verified and retained. |
| "Work must continue in the old .claude/worktrees path." | That directory is absent; use reconstruction patches and restore IDs. |
| "The frozen v1 row used system gzip." | Recovered phase-3 explicitly specifies Node zlib level 9 and Brotli quality 11. The preserved plan contains the older erroneous statement. |
| "The old runtime ratios describe current main." | They are historical diagnostic runs, sometimes on unpromoted integration source. No current-main protected timing result is claimed. |
| "A4/A2/A5 are complete because some verifier logs are green." | Final verdicts were unfinished; preservation did not promote them. |

The task ledger is append-only in spirit and contains old proposals followed by later decisions. Newer accepted decisions supersede old status lines; source and identified receipts decide factual implementation/execution questions.

## 25. Glossary and evidence index

### 25.1 Terms used in this project

| Term | Meaning here |
|---|---|
| Baseline lane | Older Rust-shaped/text-oriented output path; useful as a control but not uniformly a correctness oracle. |
| JIR | Semantic JavaScript-targeting intermediate representations transported/derived before final emission. |
| Carrier | A selected physical representation for a semantic value, such as baseline object, host string or product lanes. |
| Physicalization | Choosing concrete value shapes and call interfaces while preserving observable semantics. |
| SROA | Scalar replacement of aggregates; only valid where splitting fields preserves alias and observation behavior. |
| PEA | Partial escape analysis/materialization work to avoid objects when their observations permit it. |
| Protected operation / Invoke | Operation with explicit exceptional control flow and cleanup obligations. |
| Cleanup | Required destruction/lifecycle work on relevant exits, not merely the existence of a try statement. |
| Witness | Recorded justification/check for a transformation; useful only when sufficiently strong and enforced before acceptance. |
| Runtime closure | The complete set of helper/runtime dependencies reached by a delivered program. |
| Selected candidate | A specific source/configuration/artifact set; not every branch containing an attractive individual win. |
| Negative control / mutant | Deliberately incorrect input or implementation used to show a test rejects the intended failure. |
| R0/R1/R2 | Reconstruction baseline, correctness-repaired operational baseline, integrated restoration candidate. |
| Preserved snapshot | Recoverable source state; it can be failed, unfinished, superseded, or accepted. |
| Qualification | A decision based on stated gates and identified evidence, not a synonym for build success. |

### 25.2 Evidence authority and reading order

| Source | Use |
|---|---|
| [Latest consolidation receipt](docs/round2-consolidation-verification.json) | Current tested compiler identity, test counts, remaining inventories, and qualification flags. |
| [Cleanup handoff](WORKTREE-CLEANUP-HANDOFF.md) | Preservation scope, private/public split, restore instructions and cleanup boundary. |
| [Current verdict](VERDICT.md) | Latest consolidation note, followed by historical selected-promotion results. |
| [Round 2 report](round_2_report.md) | The round's original handoff, candidates, failures, and decisions; read its stale statements with Section 24. |
| [Round 2 audit](research/v2-restoration/rounds-1-2/reports/round2-audit-20261112T005723Z.md) | Focused independent rechecks and adjacent failure analysis. |
| [Round 1 integration report](round_1_integration_report.md) | Selective promotion rationale, exact package results and held-back work. |
| [Round 1 promotion receipt](docs/round1-promotion-verification.json) | Detailed execution and artifact identities for that selected candidate. |
| [Round 1 report](round_1_report.md) | Historical staging of wins, including the smaller non-promoted integration branch. |
| [Round 1 audit](research/v2-restoration/rounds-1-2/reports/round1-audit-20261111T030722Z.md) | Reproductions, caveats and original promotion recommendations. |
| [Restoration plan](V2-OPTIMIZATION-RESTORATION-PLAN.md) | P00–P21, baselines, protocols, acceptance and stop conditions. |
| [Task ledger](tasks/todo.md), [lessons](tasks/lessons.md) | Detailed evolving implementation/verifier history and latest addenda. |
| [Results log](experiments/v2-restoration/RESULTS.md) | Append-only public measurement rows and frozen v1 targets. |
| [Workload README](experiments/browser-workloads/README.md) | Actual comparison semantics and runtime-method limitations. |
| [Comparator guide](docs/COMPARATORS.md), [status](docs/comparator-restoration-status.json) | Fourteen restored builds, exact offline scope and toolchains. |
| [Assembly record](docs/V2-RESTORATION.md), [receipt](docs/v2-assembly-verification.json) | Coherent recovered base and initial standalone repair history. |
| [Recovery ledger](recovery/v2/README.md), [late wins](recovery/v2/LATE-STAGE-WINS.md), [missing source](recovery/v2/MISSING-OCTOBER-SOURCE.md) | Local ignored recovery provenance, exact versus missing original material. |
| [Research archive](research/v2-restoration/rounds-1-2/README.md), [snapshot index](research/v2-restoration/rounds-1-2/worktree-snapshots.json) | Recover unfinished implementations and original verifier sources without old worktrees. |
| [Compiler contract](CONTRACT.md), [IR design](JIR.md), [optimizer design](OPTIMIZER.md) | Implementation design and declared semantics; inherited broad claims must be reconciled with explicit current failures. |

Links under `recovery/` and `experiments-pro/` refer to local ignored material and will not exist in a public clone unless separately supplied. Links under committed `research/`, `docs/`, and the active source tree are the portable public entry points. Historical private absolute paths should be used only as provenance clues or extraction keys.

## 26. How to maintain this document

When a candidate is accepted, update the relevant capability and failure rows, its source/artifact identity, and the deciding receipt. Preserve the old measurement with its label. Do not silently assign old timing, package sizes, or language-parity results to a new compiler.

When a candidate fails, retain the smallest decisive counterexample, baseline execution, expected output, configuration, and rejected rule. Commit the useful regression test when appropriate. A failed broad patch can contain a valid narrow fix; separate them before promotion.

When the contract changes, name the changed observation and admission rule. A trusted host mode, declined view, new representation, or fallback is a semantic decision before it is an optimization option.

When evidence is moved or worktrees are removed, verify content and restore independence, update the index, and retain a durable copy outside the disposable tree. Keep private evidence out of public commits and make an off-device backup; a second directory on the same Mac is not device-loss protection.

**The project is no longer starting over. The architecture, active compiler, reference implementations, measurement tools, substantial repaired behavior, and unfinished research are all present or recoverable. The remaining job is to turn the promising retained transformations into one correct, efficient, reproducible standalone compiler configuration—without repeating the earlier confusion between a smaller experiment, a passing scoped suite, and a qualified compiler.**
