# G2 checklist: the audited gate state

> **This is the wave-5 AUDIT, dated 2026-07-30, and it is kept as written.** It is
> not the current state and must not be read as one — its whole value is being a
> snapshot someone can check a later claim against. The current state is
> [`contract/G2-FINAL.md`](G2-FINAL.md), which carries every row below through to
> its final disposition.
>
> What wave 6 closed from this page, in one line each:
>
> - **Structural finding 1** (no committed machine-readable L2 status) —
>   CLOSED. `contract/fixtures/l2-status.json`.
> - **Structural finding 2 / F3** (L2 cannot fail the suite) — CLOSED.
>   `contract/harness/check-l2-status.mjs`, wired into `harness/run-all.mjs --check`
>   and `parity/check.sh`. Fails on a regression AND on an improvement.
> - **Structural finding 4 + [Stale documents](#stale-documents)** — CLOSED for the
>   six in `contract/`, and two more were found that this audit missed (families 16
>   and 17 both claimed a construct is not lowered that is). Two of the six are not
>   in `contract/` and are carried to G2-FINAL as required corrections.
> - **The unkeyed list** — DECIDED, and the answer is that no reconcile fix moves
>   family 13 to MATCH. See G2-FINAL.
>
> Everything else on this page was still open when this banner was written; G2-FINAL
> is the file that says how each one ended.

Gate G2 is the Stage 4 checkpoint, whose milestone the plan states as "the stock-dashboard app: charts via an npm library through `#[js_extern]`, live data via SSE feeding compiled-Rust signals".

This file exists so the verdict writer starts from an audited list rather than assembling one. Every row below was checked against the tree on 2026-07-30 (wave 5), and every row carries the file that establishes its state. A row saying PASS with no file reference would be worth nothing, so there are none.

**Read this before trusting any status prose elsewhere.** The audit's single most useful finding is that several of the repo's own status documents are behind the measured state by two waves, and one of them contradicts the code. Those are listed in [Stale documents](#stale-documents) and each is a row someone will otherwise re-derive by hand at the gate.

## How to read a row

| state | meaning |
|---|---|
| MATCH | measured equal to the reference. |
| ATTRIBUTED | measured different, with every difference explained by a named rule or a written structural reason. |
| EXCLUDED | deliberately not measured, with the reason written down at the point of exclusion. |
| GAP | neither measured nor excluded. This is the only state that is a gate item. |
| PENDING | the thing it measures does not exist yet in this wave. |

## Gate item 1: every corpus family MATCH or attributed

18 families. **1 MATCH, 12 ATTRIBUTED, 2 EXCLUDED, 3 GAP.**

The L2 verdict comes from `scripts/dom-test.sh`, which compiles `examples/dom-tests/NN.rs` through the backend, traces it, and runs `contract/harness/compare-trace.mjs` against `contract/fixtures/corpus/<family>/expected.reference.trace`. A family is only compared if an `examples/dom-tests/NN.family` file names it. The latest full run is `build/logs/wave4-final-dom.log:83-97`.

| family | state | evidence |
|---|---|---|
| 01-simple-elements | MATCH | `build/logs/wave4-final-dom.log:84`; `build/logs/dom-01_simple_elements.l2.log:5` "traces agree (0 accepted delta(s))" |
| 02-text-interpolation | ATTRIBUTED | `wave4-final-dom.log:85`; rule `anchor-placement`, `examples/dom-tests/deltas.json:19` |
| 03-attribute-expressions | ATTRIBUTED | `wave4-final-dom.log:86`; rule `write-once-vs-always-reactive`, `deltas.json:20` |
| 04-event-expressions | ATTRIBUTED | `wave4-final-dom.log:87`; rule `delegate-events-placement`, `deltas.json:22` |
| 05-conditional | ATTRIBUTED (L1 by design) | `wave4-final-dom.log:88`; `contract/fixtures/corpus/README.md:98` puts 05 in "L1 expected, L2 open" |
| 06-insert-children | ATTRIBUTED | `wave4-final-dom.log:89`; rule `anchor-placement`, `deltas.json:19` |
| 07-components | ATTRIBUTED | `wave4-final-dom.log:90`; `deltas.json:26-27`, including one permanently unwritable case |
| 07b-nested-components | ATTRIBUTED | `wave4-final-dom.log:91`; `deltas.json:26` |
| 08-fragments | **GAP** | no `examples/dom-tests/` fixture, no `.family`, no verdict in any log |
| 09-svg | **GAP** | same |
| 10-custom-elements | **GAP** | same |
| 11-topcoat-if | ATTRIBUTED | `wave4-final-dom.log:92`; rule `different-reference-program`, `deltas.json:21` |
| 12-topcoat-match | ATTRIBUTED | `wave4-final-dom.log:93`; `deltas.json:21` and `:24` (`memo-hoisting-for-a-match`) |
| 13-topcoat-for | ATTRIBUTED | `wave4-final-dom.log:94`; rule `keyed-list-vs-a-rust-loop`, `deltas.json:25` |
| 14-topcoat-local | ATTRIBUTED | `wave4-final-dom.log:95`; `deltas.json:20` |
| 15-topcoat-bind | ATTRIBUTED | `wave4-final-dom.log:96`; `deltas.json:19` and `:22` |
| 16-topcoat-spread | EXCLUDED | `examples/dom-tests/16_topcoat_spread.rs:3-7`: the corpus writes `topcoat::view::Attributes`, which the spike does not build, so the fixture pins emitted shape and not runtime behaviour. "It carries no `.family` for that reason." |
| 17-topcoat-signal | EXCLUDED | `examples/dom-tests/17_signal.rs:9-10`: "The reference module for corpus family 17 is a different program, so a trace comparison against it would not be measuring anything." |

The reference side is green for all 18 independently of the above: every `expected.reference.trace` header reports `"status":"completed","error":null`, and all 84 distinct template strings pass upstream's validator (`contract/README.md:97-99`).

### The three findings behind this item

**F1. Three families have never been run against the emitter.** 08-fragments, 09-svg and 10-custom-elements have no fixture in `examples/dom-tests/` at all. Their only status is prose. Each has a written blocker: a non-element node at the top level of a view body (`corpus/08-fragments/NOTES.md:39-40`), namespace detection with no `isSVG` flag and no `setAttributeNS` (`corpus/09-svg/NOTES.md:48-54`), and `isImportNode` plus the attribute-versus-property decision, for which there is no Topcoat syntax today (`corpus/10-custom-elements/NOTES.md:41-43`). So the gate choice is to write three fixtures or to move three families to EXCLUDED with those blockers as the reason. Either is defensible; leaving them unstated is not.

**F2. "Attributed" is not "match", and 12 of 13 compared families are attributed.** Only family 01 is a true trace match. Every difference has a written cause, which is what the gate item asks for, but a verdict that reads "every corpus family MATCH or attributed" without saying that the ratio is 1:12 would be technically true and misleading. State the ratio.

**F3. L2 is non-blocking, so a regression on family 01 would print and exit 0.** `scripts/dom-test.sh:20-23` says the L2 result is "reported per fixture rather than being allowed to fail the suite", and the exit code at `:263-264` counts only `$failures`, which the L2 block never increments. This is deliberate and the reason given is sound (a difference is a finding about two emitters). But it means the one MATCH in the table is not defended by anything: family 01 could silently become ATTRIBUTED. If G2 wants to claim family 01 is a match, something has to fail when it stops being one.

## Gate item 2: every grammar module covered or expect_fail'd

**FAILS as written. 6 of ~42 enumerated grammar nodes are in neither bucket.**

"Grammar module" here is a node of the un-forked `topcoat-view-grammar` crate, which the spike consumes as a path dependency and mirrors file for file in `view-dom/src/lower/` (`view-dom/src/lower.rs:1-5`: "they mirror the layout of `topcoat-view-grammar` so a variant added upstream lands in the file that matches it"). The unit is a grammar AST node; the spike's side of it is a `WriteDom` impl or a refusal.

The mechanism is `DomWriter::unsupported` (`view-dom/src/dom_writer.rs:114-120`) or `DomWriter::error` (`:122-125`) for the refusal, and a row in one of two `#[test]` tables for the assertion: `unsupported_nodes_report_a_spanned_error` (`dom_writer.rs:1305`, 2 rows) or `unsupported_attributes_report_a_spanned_error` (`dom_writer.rs:1358`, 8 rows). There are 13 `unsupported` call sites and 5 `error` call sites.

Covered nodes, in brief: `view::Node` covers Text, Element, Component, Expr, RuntimeExpr, If, Local, ForLoop, Match, Block and SignalDecaration; `attributes::AttributeNode` covers Attribute, Spread, BindAttribute and EventHandler; the leaf enums cover `AttributeKey::Ident`, both `AttributeValue` arms, `EventHandlerValue::Expr`, both `TemplateOrRuntimeExpr` arms, the `TemplateElse` arms including a missing else, and the `for` key clause. Expect_fail'd, with both halves present: `Node::DocumentType`, `ElementName::Expr`, `AttributeKey::Expr` at all three of its sites, `EventHandlerValue::LitStr`, `AttributeNode::{If, ForLoop, Match, Local}`, `Component.children`, `NamedArgValue::Runtime`, and `leading_cx::LeadingCx`.

### The six gaps

Five of them are the same shape: the refusal exists in code and nothing asserts it, so the branch is untested and a change that silently stopped refusing would not be noticed.

| # | node | refusal | why it is a gap |
|---|---|---|---|
| G-1 | `Node::Continue` | `view-dom/src/lower/node.rs:41-43` | not in the table at `dom_writer.rs:1305` |
| G-2 | `Node::Break` | `view-dom/src/lower/node.rs:44-46` | same |
| G-3 | `AttributeNode::Continue` | `view-dom/src/lower/attributes.rs:53-58` | not in the table at `dom_writer.rs:1358` |
| G-4 | `AttributeNode::Break` | `view-dom/src/lower/attributes.rs:59-64` | same |
| G-5 | `AttributeNode::Block` | `view-dom/src/lower/attributes.rs:50-52` | same |
| G-6 | `ElementName::LitStr` | none: silently accepted at `view-dom/src/lower/element.rs:20` | this one is NOT a missing assertion. The `<"tag-name">` literal-string form is accepted and lowered, and no test or fixture exercises it. It is untested behaviour rather than an untested refusal, which makes it the more dangerous of the six. |

G-1 through G-5 close as five table rows in `view-dom/src/dom_writer.rs:1305` and `:1358`. G-6 needs a decision first: either a test that pins what `<"tag-name">` lowers to, or a refusal plus a row.

**Nothing forces the two halves to stay in sync.** There is no registry file, no index, and no exhaustiveness check that a new `unsupported` call site gains a table row. That absence is why these six exist, and it is the structural finding behind this gate item: closing the six without closing the mechanism means the seventh appears the same way.

Two grammar modules are structurally out of scope and have no written record saying so: `class` (`grammar/src/class.rs`, the `class!{}` macro body) and `props` (`grammar/src/props.rs`, `#[derive(Props)]`). Neither is reachable from `view-dom`, which never sees those macro bodies. They should be listed as OUT rather than left to look like omissions.

### A second expect_fail mechanism, which does not cover the grammar

`scripts/test.sh` has a real `NN.expect_fail` convention (documented `scripts/test.sh:37-39`, implemented `:210-235` and `:360-385`): a file of required substrings beside the fixture, registered by its own existence. It has exactly one instance, `examples/tests/zombie_reachable.expect_fail`. Its siblings are `NN.expect_abort` (5 instances) and `NN.absent` (1). **This mechanism does not exist in `scripts/dom-test.sh`, `emit-test.sh`, `module-test.sh` or `extern-test.sh`**, so a dom fixture cannot be marked "must fail". Worth knowing before someone at the gate proposes closing G-1..G-5 with an `expect_fail` file, which is not available there.

## Gate item 3: budgets enforced

Two independent budget systems, both enforcing, both linked here.

**Size budgets** are `contract/parity/budgets.mjs` over `contract/parity/budgets.json`, run last by `contract/parity/check.sh`. 9 subjects: `island:counter`, `island:nested`, `island:search`, `chunk:shared`, `runtime`, `loader`, `island-rt`, `page:island`, `page:search`. Every baseline is a measured first-green number rather than a chosen one (`budgets.json:5`), gzip is pinned at level 9 rather than a realistic 6 so a change in the number means a change in content (`budgets.json:23`), and the tolerance is proportional and deliberately tight because the builds are deterministic and there is no measurement noise to absorb (`budgets.json:18`). Coverage is itself checked: `unbudgetedRoutes` catches a module appearing in the delivery with no budget at all, and `$notBudgeted` is the written exclusion list.

**Emission budgets** are per-fixture `NN.maxbytes` files under `examples/emit/`, enforced by `scripts/emit-test.sh`.

Gate state: **PASS**, with one qualification. Three of the nine subjects are the three existing islands. The dashboard is the G2 milestone and will be the fourth and largest, and it has no budget yet because it does not exist yet. See gate item 4.

## Gate item 4: the milestone islands

| island | parity fixture | state |
|---|---|---|
| counter | `contract/parity/fixtures/counter/` | present: `fixture.json`, `interact.mjs`, `ssr.html` |
| nested | `contract/parity/fixtures/nested/` | present |
| search | `contract/parity/fixtures/search/` | present; the first island carrying a procedure call |
| **dashboard** | `contract/parity/fixtures/dashboard/` | **LIVE, 62/62.** The G2 milestone island: a chart through five `#[js_extern]` declarations (three of them global-scope) and a movers list driven by SSE. Flipped off pending 2026-07-30. Both budget subjects measured (`island:dashboard` 2630 gzip, `page:dashboard` 21077 gzip). Both negative controls trip. Carries the wave's sharpest finding: [the movers loop is not keyed](#the-unkeyed-list-an-attributed-deviation-from-the-keyed-list-contract). |

The parity run is 39/39 (`VERDICT.md:149-153`) and is deliberately excluded from `contract/harness/run-all.mjs` (`contract/README.md:50`), because run-all regenerates from pinned upstream and the parity harness regenerates from demo-app's own server. Two different drift questions, two different gates.

Known limits carried into G2, from `contract/parity/README.md:206-216`: pairing a template with its hydration key is still open (`:212`), and `fixtures/search/` still declares `islandTemplate: 0` labelled as unverified (`:210`).

## Gate item 5: the standing OUT list

The existing list is `VERDICT.md:180-188`. It is inherited rather than rewritten, with three amendments.

Current entries: components / `#[component(client)]`, `#[procedure]` from compiled code, keyed `for`, nested islands, client-side text conversion for non-primitive hole values, `js-names=mangled` with core+alloc, memo for `match` arms (deliberate), and per-island chunking and lazy hydration.

**Amendment 1: one entry is stale.** `VERDICT.md:182` qualifies the components entry with "family 07 - props equivalence undefined". The props equivalence IS now defined: `contract/fixtures/corpus/07-components/NOTES.md:5` and `corpus/README.md:105-113`. Keep the entry if the emission is still missing, but drop the parenthetical.

**Amendment 2: additions from this audit.** The two out-of-scope grammar modules (`class`, `props`, gate item 2), and the three unrun corpus families if they are resolved as EXCLUDED rather than as fixtures (F1).

**Amendment 3: the deferred-shapes list is separate and should stay separate.** `contract/CONTRACT-DOM.md:730-762` (section 13) marks Suspense/resources/lazy, Portal, seroval serialization, streaming SSR, and assets/request context as out of scope, each with a Topcoat delta explaining why, plus async hydration at `:944-945`. That list is about the dom-expressions runtime contract, not about this compiler's coverage, and merging the two would make the OUT list look longer and less decided than it is.

The one literal "out for G2" in the tree is `contract/fixtures/corpus/07-components/NOTES.md:11`: getter props, which "only pay for themselves with spread and dynamic props, which are out for G2".

## Gate item 6: `#[js_extern]` and the .d.ts emission

Stage 4 names both. Neither is in the older gate wording, so they are added here.

`#[js_extern]` landed in wave 4 and gained the global-scope root in wave 5 (descriptor VERSION 1 to 2, a `g` flag). The encoding is documented at `contract/JS-EXTERN.md`, read out of `js-extern-macro/src/descriptor.rs`, which is the one definition of the format (the macro compiles it to encode, the backend includes the same file with `#[path]` to decode). 17 vectors are recorded in `contract/fixtures/js-extern/vectors.json` and driven by `contract/harness/check-js-extern.mjs`. The suite is `scripts/extern-test.sh`, 4/4 as of wave 5.

**Checked and NOT a gap: `new-default` is measured end to end.** This was raised as a suspected gap during the audit and disproved. `examples/extern-tests/01_chart.rs:53` declares `#[js(new = "default")]`, `01_chart.js.expected:5` shows the emitted `import { default as ext$default$... }`, and `scripts/extern-check.mjs:115` drives the compiled entry point against the `new-default` vector. Recorded here because `via` is the only thing `new-named` and `new-default` differ on, so it is the kind of thing a gate reader will want to re-check, and it has been.

**GAP J-2: the global root is proven next door, not on this page's vectors.** All 17 vectors drive the chart library, which is a module, so none exercises a global. The backend covers eight global shapes in `examples/extern-tests/02_globals.rs` with `scripts/globals-check.mjs` (17 checks, including one that reads the emitted text to assert a global imported nothing, which running the program cannot tell you). So the behaviour is measured; what is missing is a vector for it in `contract/fixtures/js-extern/`. The eight ids are proposed in `contract/JS-EXTERN.md` and `globals-check.mjs` becomes their driver unchanged. Low risk, but it means `JS-EXTERN.md` must not be read as having pinned the global root.

**Open, not a gap: who resolves the import specifier**, and **callbacks**, which `contract/JS-EXTERN.md` calls "the largest hole in this page and it is deliberate": nothing covers handing JavaScript a compiled Rust closure.

**The `.d.ts` emission LANDED in wave 5** (backend item B3, `-Cllvm-args=js-dts=on`, `scripts/dts-test.sh`) and the contract side is armed and passing. `contract/parity/dts.mjs` reads both emission sources (`build/dtstest/` and demo-app's OUT_DIR), records verbatim goldens plus a structural summary, and runs `tsc --noEmit` over the declarations together with a scaffolded consuming `.ts` per module. **6/6, including a real `tsc` pass over the two emitted files.**

The backend banked this as a wave-6 ask: "there is no `typescript` in `contract/vendor` today ... if you can pin `tsc` under `contract/vendor`, `build/dtstest/01_shapes.d.ts` is ready". **Closed in this wave instead**: `typescript@5.9.3` is pinned in `contract/upstream.lock` (zero dependencies, one tarball, one sha512 -- see the lock's notes for why 5 and not 7, and why it is deliberately not in `vendor/package.json`). So the `.d.ts` goldens are NOT structural-only, which is what both the backend's report and the earlier drafts of this checklist assumed.

## The unkeyed list: an attributed deviation from the keyed-list contract

The single most consequential finding of wave 5, recorded here because it is a gate item that will otherwise be read the wrong way round.

The dashboard's movers list re-orders on every tick, which is the textbook case for a keyed `for`. It was written keyed, and running it showed the design is wrong for this list. `view_abi::push_keyed` hands back the node a key contributed last time and DISCARDS the row just built, text included. So the list re-ordered correctly and then rendered its first values for ever: measured, a row read `4310` after a tick that set it to `4460`.

Keying is right for a row whose text does not change, or whose changing parts are reactive holes of the row's own template. Neither holds here, and per-row reactive holes are not reachable, because a `$( )` cannot carry a free function call. So the loop rebuilds five short rows per tick and the prices are correct.

Three things follow, and all three belong in the verdict.

1. **It is an ATTRIBUTED deviation, not a MATCH.** Corpus family 13 (`13-topcoat-for`) is already attributed under the rule `keyed-list-vs-a-rust-loop` (`examples/dom-tests/deltas.json:25`). This is the same divergence reaching the milestone island, and it means the keyed-list half of the dom-expressions contract is met by a rule rather than by behaviour.
2. **The parity fixture asserts the OPPOSITE of the obvious claim,** deliberately. `contract/parity/fixtures/dashboard/interact.mjs` section 4 asserts the rows are REBUILT and the list element is not, because asserting keyed identity would pass for the broken version and fail for the correct one. `fixture.json`'s `expected.$notKeyed` records why, and the check names the reason so a silent return to keying is caught rather than welcomed.
3. **What would have to change for keying to be right here:** `push_keyed` would have to update the retained node from the freshly built row, or the row's changing parts would have to become reactive holes of its own template. Until one of those, keyed identity is not a property this list can have AND be correct. That is a wave-6 decision, and it is the one that decides whether family 13 can ever move from ATTRIBUTED to MATCH.

## What wave 5 closed

Recorded so the verdict writer can tell a closed item from an open one at a glance.

| item | state entering wave 5 | now |
|---|---|---|
| the milestone island | did not exist | LIVE, `contract/parity/fixtures/dashboard/`, 62/62, both controls tripping |
| `#[js_extern]` global scope | a member shape with no module silently read argument zero | an explicit third root, descriptor VERSION 2, `g` flag; documented in `contract/JS-EXTERN.md` |
| `.d.ts` emission | not implemented | emitted, goldened, and type-checked by a pinned `tsc` |
| `.d.ts` type checking | assumed impossible (no TypeScript in the tree) | `typescript@5.9.3` pinned in `contract/upstream.lock`, 6/6 |
| the tree oracle vs a server-rendered list | could not express one | teaches `data-hk`-bearing extras as hole content, `contract/parity/lib/tree.mjs` |
| parity fixtures | 3 | 4 |
| parity harness unit tests | 160 | 176 |

Still open, unchanged by this wave: the three unrun corpus families, the six grammar gaps, the non-blocking L2 gate, the missing committed L2 status, and the six stale documents below.

## Stale documents

Every one of these will otherwise be re-derived by hand at the gate, and two of them contradict the code.

| document | what is stale |
|---|---|
| `contract/fixtures/corpus/README.md:45-64` (status column) and `:115-152` (the NOT-YET-LOWERABLE ledger) | still lists families 11 to 15 as not lowerable, and still lists `if`, `match`, `for`, `let`, block, bind attribute and attribute spread as blocking errors. All of them now lower (`view-dom/src/lower/node.rs:21-32`, `lower/local.rs:6`, `lower/bind_attribute.rs:9`, `lower/attributes.rs:23`) and five of them have L2 verdicts (`build/logs/wave4-final-dom.log:92-96`). **The enumeration document contradicts the code.** |
| `contract/fixtures/corpus/{11-topcoat-if,12-topcoat-match,13-topcoat-for,14-topcoat-local,15-topcoat-bind}/NOTES.md`, each at line 3 | same, at the per-family level: each opens with "NOT-YET-LOWERABLE" |
| `contract/fixtures/corpus/05-conditional/NOTES.md:40-43` | "NOT-YET-LOWERABLE: the memo wrapping itself" is retired; `examples/dom-tests/deltas.json:16` records `memo-hoisting-for-a-conditional` as measured and fixed |
| `contract/fixtures/corpus/deltas.json` | **orphaned.** Loaded by no code path: `scripts/dom-test.sh:47` points `--delta` at `examples/dom-tests/deltas.json` instead, and `examples/dom-tests/deltas.json:5` records the takeover. Its `$status` at `:23` still says "Seeded, not measured. The Rust emitter does not compile these fixtures yet", which has been false for 13 families since wave 4. Either delete it or reduce it to a pointer. |
| `VERDICT.md:182` | the family 07 parenthetical, see gate item 5 |
| `CONTRACT.md:426` | "28 runtime fixtures and 13 JS goldens"; the suites are now 46 and 20 |

## Structural findings, ranked

Ordered by how much they cost if the gate is written without them.

1. **No committed machine-readable per-family L2 status.** The verdict table exists only in `build/logs/*.log`, a build-artifact directory that is regenerated per run and gitignored in spirit. The `verdicts` string is built in memory at `scripts/dom-test.sh:224/230/235` and printed to stdout. So the primary evidence for gate item 1 is not committed anywhere, and reproducing it requires a full dom-test run. This is the single highest-value thing to fix before the gate: a committed status file makes gate item 1 checkable in one read.
2. **L2 cannot fail the suite** (F3). The one MATCH is undefended.
3. **Nothing keeps a refusal and its assertion in sync** (gate item 2). Six gaps exist because the mechanism allows them; closing the six without closing the mechanism reproduces the seventh.
4. **The corpus's own status prose is two waves behind the measured state.** See Stale documents. A reader who trusts it will mis-state the gate in the safe direction, which is still mis-stating it.
5. **Three case asymmetries with no written justification**: `05-conditional` `jsx.jsx:15` `withMarkup` has no `view.rs` counterpart, `06-insert-children` `jsx.jsx:29` `array` has none, and `17-topcoat-signal` `view.rs:51` `two_signals` has no `jsx.jsx` counterpart. Families 06, 08 and 10 document their drops; these three do not. Small, but each is a case someone will ask about.

## The counts, and how to reproduce each

The numbers a verdict quotes should come from a command someone can run.

| suite | command, from `spike/rustc-codegen-js/` | fixture set | count |
|---|---|---|---|
| runtime | `./scripts/test.sh` | `examples/tests/*.rs` (19) + `examples/core-tests/*.rs` minus `prelude.rs` (27) | 46 |
| emit goldens | `./scripts/emit-test.sh` | `examples/emit/*.rs` | 20 |
| module | `./scripts/module-test.sh` | same, minus any `NN.core` fixture (`examples/emit/17_alloc_shim.core`) | 19 |
| dom | `./scripts/dom-test.sh` | `examples/dom-tests/*.rs` | 20 |
| extern | `./scripts/extern-test.sh` | `examples/extern-tests/*.rs` (1) plus a synthetic vectors entry (`scripts/extern-test.sh:141-155`) | 2 |
| cargo | `cargo test` | 7 workspace members | 401 |
| hydration parity | `contract/parity/check.sh` | `contract/parity/fixtures/*` | 39 |
| contract drift | `node contract/harness/run-all.mjs --check` | `contract/fixtures/` | 14 steps |

**The 9 flag modes are not encoded anywhere.** There is no CI file, no Makefile and no matrix script in the spike. The modes are run by hand as `JS_EXTRA_ARGS='<mode>' ./scripts/test.sh`; the option list is `CONTRACT.md:226-237` and seven of the names are `VERDICT.md:78`. A verdict claiming "46/46 across 9 modes" is claiming the result of nine manual invocations. Either write the matrix script or say it is manual.
