# G2 final: the completed gate

> **Scope.** This gate record predates the compiler's extraction into its own repository. The `view!` lowering (`view-dom`), `dom-tests`, the demo app, the parity harness and the procedure wire it audits are the Topcoat integration and are not in this repository.

The single input to `VERDICT.md`'s G2 section. It carries every row of
[`G2-CHECKLIST.md`](G2-CHECKLIST.md) — the wave-5 audit — through to its final
disposition, and it adds the numbers, the size table, the standing OUT list and
the delta ledger.

Completeness and accuracy over prose. Every claim carries the file that
establishes it. A row with no file reference would be worth nothing, so there are
none.

**Read order.** `G2-CHECKLIST.md` is the audit and is deliberately frozen at its
2026-07-30 wave-5 state. This file is what happened next. Where the two disagree,
this one is current and the other one is the record of what was found.

**Provenance of every number below.** Numbers marked ⚙ are recomputed live and
were re-run while writing this file. Numbers marked ▣ come from a suite that
needs a compiler build or a running server, and are quoted from the wave-6
backend sweep at `build/logs/wave6-backend-report.md`. Nothing here is
transcribed from an older wave.

---

## 1. The gate items, final

### Gate item 1 — every corpus family MATCH or attributed

**PASS, with the ratio stated.** 18 families: **1 MATCH, 13 DIFFERS-attributed,
2 EXCLUDED, 2 GAP.** ⚙

The audit's F2 asked that the ratio be stated rather than hidden behind the word
"attributed", and it is: only family 01 is a true trace match. Every one of the
thirteen differences has a written cause, and as of this wave that is enforced
rather than asserted — see gate item 7.

The verdict is no longer prose in a rotating build log. It is
**`contract/fixtures/l2-status.json`**, one row per family, with the measured
state, the difference count, the mechanically derived cause signature, and the
named written causes it is attributed to.

| family | state | differences | attributed to |
|---|---|---|---|
| 01-simple-elements | **MATCH** | 0 | — |
| 02-text-interpolation | DIFFERS | 25 | `anchor-placement`, `fixture-coverage` |
| 03-attribute-expressions | DIFFERS | 9 | `write-once-vs-always-reactive` |
| 04-event-expressions | DIFFERS | 8 | `delegate-events-placement`, `fixture-coverage` |
| 05-conditional | DIFFERS | 6 | `fixture-coverage` |
| 06-insert-children | DIFFERS | 20 | `anchor-placement`, `fixture-coverage` |
| 07-components | DIFFERS | 47 | `eager-child-content-…`, `the-getter-children-double-clone`, `fixture-coverage` |
| 07b-nested-components | DIFFERS | 42 | `eager-child-content-…`, `fixture-coverage` |
| 08-fragments | DIFFERS | 7 | `fragment-roots-must-be-elements`, `memo-for-a-fragment-member` — **closed this wave, GAP → DIFFERS** |
| 09-svg | **GAP** | — | `TemplateData` carries no template-construction flags; exact spec banked, see below |
| 10-custom-elements | **GAP** | — | same ABI field, same `cloner_key` change; exact spec banked, see below |
| 11-topcoat-if | DIFFERS | 11 | `different-reference-program` |
| 12-topcoat-match | DIFFERS | 14 | `different-reference-program`, `memo-hoisting-for-a-match`, `fixture-coverage` |
| 13-topcoat-for | DIFFERS | 36 | `keyed-list-vs-a-rust-loop` |
| 14-topcoat-local | DIFFERS | 16 | `write-once-vs-always-reactive`, `fixture-coverage` |
| 15-topcoat-bind | DIFFERS | 26 | `anchor-placement`, `delegate-events-placement`, `fixture-coverage` |
| 16-topcoat-spread | EXCLUDED | — | no Rust type to write a spread's value with; pins emitted shape, not behaviour |
| 17-topcoat-signal | EXCLUDED | — | the reference module is a different program |

The reference side is green for all 18 independently: every
`expected.reference.trace` header reports `"status":"completed","error":null`, and
all 84 distinct template strings pass upstream's validator
(`contract/README.md:97-99`).

**Reproduce in one second, with no build:**

```sh
cd contract/harness
node --import ./register-loader.mjs check-l2-status.mjs
```

#### The three GAPs, resolved: one closed, two banked with an exact spec

**08-fragments: CLOSED, and it needed no emitter change at all.** ▣ A view body
with several top-level nodes IS a fragment, because `DomWriter::close_scope`
already returns one template per root as a Rust tuple. The family was a GAP only
because nobody had ever compiled it. All four cases of `corpus/08-fragments/view.rs`
are now ported to `examples/dom-tests/08_fragments.rs` (dom suite 20/20 → **21/21**),
L2 went 18 lines → **7**, and all three templates MATCH byte for byte once the
standard `template-closing-tags` normalisation is applied. The seven remaining lines
are two new written causes; see the delta ledger.

**09-svg and 10-custom-elements: STAY GAP, with the work specified.** ▣ Both need
the same thing and it is bigger than it looks: **`TemplateData` has to carry the
template-construction flags, which is an ABI bump (5 → 6)**. A grep for
`isSVG|isImportNode|namespace|xmlns` across `view-dom/src`, `backend/src/template.rs`
and `view-abi/src` returns **zero hits** — no flag, no wrapper, no `setAttributeNS`,
no namespace table.

The four steps, in order: (1) `TemplateData` gains `is_import_node` and `is_svg`;
(2) `Template::finish` sets `is_import_node` when any element has a dashed tag name
and `is_svg` when the root is an SVG-only element; (3) `backend/src/template.rs`
reads both and **adds both to `cloner_key`**, or two templates with the same HTML and
different flags collide onto one cloner; (4) **SVG only, and the part not to rush**:
the synthetic `<svg>` wrapper. CONTRACT-DOM 2.2 — `isSVG` makes the runtime unwrap
TWO levels, because the plugin wraps the template string in a literal `<svg>` first,
so the wrapper must go into the HTML *and* `Tree::parse` must root the walk at the
wrapper's first child, or every walk in that template lands one level too high.
CONTRACT-DOM 2.2's own note: "Emitting `isSVG` without also prefixing `<svg>` to the
template string, or the reverse, silently yields the wrong root node." `rootRect` is
also the only corpus template with an explicit trailing `</svg>`, which the existing
`template-closing-tags` rule strips from side b — so that rule needs a family-09
exception or the two sides can never agree.

**Neither can ever be a MATCH even once the flags land**, and a verdict should not
imply otherwise. Family 10: the reference emits `_el$._$owner = _$getOwner()` on
every custom-element root (six `getOwner` records) which no Topcoat program produces,
and it compiles `some-attr=` to a plain JS **property write** — leaving no trace
record at all — where ours is `_$setAttribute`. Family 09's `dynamic_svg`: the
reference's trace shows `effect` + `effect.threw` ("state is not defined") and never
reaches its three `setAttribute` calls, where ours writes three attributes once —
the standing `write-once-vs-always-reactive` rule.

Banked rather than landed because an ABI bump plus a walk-rooting change, in the last
construction wave, without the budget to re-verify demo-app and the parity harness
afterwards, is exactly the risk that "do not rush namespace correctness" names.
Steps 1–3 are mechanical; step 4 has a silent failure mode.

### Gate item 2 — every grammar module covered or expect_fail'd

**PASS. Fully enumerated, and the mechanism that allowed the gaps is closed.**
▣ `build/logs/wave6-backend-report.md`, item 4; `cargo test -p view-dom` 130 → **136**.

Final coverage, every variant of every enum reachable from `view-dom`:

| enum | variants | lowered | refused | unreachable |
|---|---|---|---|---|
| `view::Node` | 14/14 | 11 | 1 | 2 |
| `attributes::AttributeNode` | 11/11 | 4 | 7 | 0 |
| `ElementName` | 3/3 | 2 | 1 | 0 |

The audit's six resolved into three different answers, and two of them change what
the gate should claim:

- **Two of the six were not gaps.** `Node::Continue` and `Node::Break` are refused
  by the GRAMMAR'S OWN PARSER (`crates/topcoat-view/grammar/src/view/node.rs:139-148`),
  so no `view!` body can produce one and `view-dom`'s arms for them are
  unreachable. Measured, not read: `syn::parse_str::<View>("<div>continue;</div>")`
  fails at parse. **A refusal nobody can reach is not an untested branch**, and the
  gate should say so rather than counting them.
- **Three were real and are now asserted**: `AttributeNode::{Continue, Break, Block}`,
  whose attribute-list forms DO parse and DO reach the refusal.
- **G-6, `ElementName::LitStr`, was the dangerous one and it was worse than
  "untested".** `<"tag">` was accepted, lowered, and its text written into the
  template HTML **unread**. Decision: kept and validated, because a literal names a
  tag statically the same way an identifier does and refusing it would make the dom
  emitter reject a view the grammar accepts. Two new checks, neither of which an
  `Ident` can fail because an identifier cannot spell a way to: the text must be a
  tag name (`<"><script>">` closed the element and opened another — **markup
  injection from a literal**, now a spanned error), and it must not be a void
  element (`is_void_element` answers `false` for every literal, so the grammar had
  already parsed `<"br">` as a Normal element with a closing tag and the emitter
  would have written `<br></br>`).

**The mechanism is closed, which was the audit's actual structural finding:**
"closing the six without closing the mechanism means the seventh appears the same
way". `view-dom/src/dom_writer.rs`, `mod coverage`. Three locks, and a variant must
pass all three:

1. a **wildcard-free `match` per enum** mapping every variant to a classification —
   a variant added upstream stops the file COMPILING at the arm that must classify it;
2. **a row per variant**, with the row count asserted against a constant beside each
   match — fixing lock 1 without adding a row fails here, which is precisely the
   step that used to be skipped;
3. **the row's source must actually produce the variant it claims** — the source is
   parsed and the variant recovered from the AST, so a plausible-looking row cannot
   be satisfied by a source exercising something else.

A third classification fell out of the work and is why this is better than a bigger
table: `Coverage::Unreachable`, whose assertion is of a different KIND — the source
must fail to PARSE, with the grammar's own message. If the grammar ever starts
accepting `continue` in a view body, that assertion fires and points straight at the
arm in `lower/node.rs` that has just become live.

**Every lock was verified by breaking it**, not assumed; the three failure messages
are recorded in the backend report.

One more silent-mis-render class was closed on the way: `in_the_dom` also lost its
wildcard, whose default for an unknown variant was `true` — so a node added upstream
would have counted as a rendered child, and a wrong child count moves every anchor
after it. That is a bug findable only in a browser.

Two grammar modules are structurally OUT and now have a written record saying so:
`class` (`grammar/src/class.rs`) and `props` (`grammar/src/props.rs`). Neither is
reachable from `view-dom`, which never sees those macro bodies. See section 4.

Worth knowing before anyone proposes closing these with an `expect_fail` file:
that mechanism exists only in `scripts/test.sh` (documented `:37-39`, implemented
`:210-235` and `:360-385`, one instance). **It does not exist in
`scripts/dom-test.sh`, `emit-test.sh`, `module-test.sh` or `extern-test.sh`**, so
a dom fixture cannot be marked "must fail".

### Gate item 3 — budgets enforced

**PASS.** Two independent budget systems, both enforcing.

**Size budgets**: `contract/parity/budgets.mjs` over `contract/parity/budgets.json`,
run last by `contract/parity/check.sh`. **11 subjects** ⚙ — the nine the audit
counted plus `island:dashboard` and `page:dashboard`, which the milestone island
added. Every baseline is a measured first-green number rather than a chosen one
(`budgets.json` `$comment`); gzip is pinned at level 9 rather than a realistic 6,
so a change in a number means a change in CONTENT and not a change in how hard the
compressor tried; the tolerance is 5% proportional with a 128-byte floor, and it
is deliberately tight because the builds are deterministic and there is no
measurement noise to absorb. Coverage is itself checked: `unbudgetedRoutes` catches
a module appearing in the delivery with no budget at all, and `$notBudgeted` is the
written exclusion list (five source maps and the standalone import map, each with a
reason).

**Emission budgets**: per-fixture `NN.maxbytes` files under `examples/emit/` and
`examples/core-tests/`, enforced by `scripts/emit-test.sh`. This is the precedent
the L2 gate's "an improvement fails too" rule is modelled on.

The audit's one qualification — that the dashboard was the coming fourth and
largest island and had no budget — is closed: it has both subjects, measured.

### Gate item 4 — the milestone islands

| island | parity fixture | state |
|---|---|---|
| counter | `contract/parity/fixtures/counter/` | live |
| nested | `contract/parity/fixtures/nested/` | live |
| search | `contract/parity/fixtures/search/` | live; the first island carrying a procedure call |
| **dashboard** | `contract/parity/fixtures/dashboard/` | **live, the G2 milestone**: a chart through five `#[js_extern]` declarations (three global-scope) and a movers list driven by SSE. Both budget subjects measured. Both negative controls trip. |

The parity run is deliberately excluded from `contract/harness/run-all.mjs`
(`contract/README.md:50`): run-all regenerates from pinned upstream, the parity
harness regenerates from demo-app's own server. Two different drift questions, two
different gates.

Harness unit tests: **176/176** ⚙ (`node contract/parity/test.mjs`).

### Gate item 5 — the standing OUT list

See section 4. The audit's three amendments are all applied, and one entry
changed KIND rather than wording — see the keyed `for` row.

### Gate item 6 — `#[js_extern]` and the `.d.ts` emission

`#[js_extern]` landed in wave 4 and gained the global-scope root in wave 5
(descriptor VERSION 1 → 2, a `g` flag). The encoding is documented at
`contract/JS-EXTERN.md`, read out of `js-extern-macro/src/descriptor.rs`, which is
the one definition of the format: the macro compiles it to encode, the backend
includes the same file with `#[path]` to decode. **17 vectors** ⚙ in
`contract/fixtures/js-extern/vectors.json`, driven by
`contract/harness/check-js-extern.mjs`.

**Checked and NOT a gap: `new-default` is measured end to end.** Raised as a
suspected gap during the audit and disproved. `examples/extern-tests/01_chart.rs:53`
declares `#[js(new = "default")]`, `01_chart.js.expected:5` shows the emitted
`import { default as ext$default$… }`, and `scripts/extern-check.mjs:115` drives the
compiled entry point against the `new-default` vector. Recorded because `via` is the
only thing `new-named` and `new-default` differ on, so a gate reader will want to
re-check it — and it has been.

**Standing GAP J-2: the global root is proven next door, not on this page's
vectors.** All 17 vectors drive the chart library, which is a module, so none
exercises a global. The backend covers eight global shapes in
`examples/extern-tests/02_globals.rs` via `scripts/globals-check.mjs` (17 checks,
including one that reads the emitted text to assert a global imported nothing,
which running the program cannot tell you). The behaviour is measured; what is
missing is a vector for it in `contract/fixtures/js-extern/`. The eight ids are
proposed in `contract/JS-EXTERN.md` and `globals-check.mjs` becomes their driver
unchanged. Low risk — but `JS-EXTERN.md` must not be read as having pinned the
global root.

**Open, not a gap:** who resolves the import specifier, and **callbacks**, which
`contract/JS-EXTERN.md` calls "the largest hole in this page and it is deliberate":
nothing covers handing JavaScript a compiled Rust closure.

**`.d.ts`: 6/6, including a real `tsc` pass.** ⚙ `contract/parity/dts.mjs` reads
both emission sources (`build/dtstest/` and demo-app's `OUT_DIR`), records verbatim
goldens plus a structural summary, and runs `tsc --noEmit` over the declarations
together with a scaffolded consuming `.ts` per module — 4 files type-checked.
`typescript@5.9.3` is pinned in `contract/upstream.lock` (zero dependencies, one
tarball, one sha512). So the `.d.ts` goldens are NOT structural-only, which both
the backend's wave-5 report and the earlier drafts of the checklist assumed.

### Gate item 7 — the L2 gate (new; the audit's top structural finding)

**CLOSED.** The audit's two highest-ranked structural findings were that there is
no committed machine-readable per-family L2 status, and that L2 cannot fail the
suite — so family 01's MATCH was undefended and could silently become DIFFERS.

Two files close both:

- **`contract/fixtures/l2-status.json`** — the committed verdict.
- **`contract/harness/check-l2-status.mjs`** — recomputes it and fails on ANY
  difference, in either direction.

Wired into both contract-side entry points, so neither can miss it:
`contract/harness/run-all.mjs` (step 15, in `--check` mode) and
`contract/parity/check.sh` (after the harness unit tests, before the SSR capture).

**Why it needs no build, and why that is sound.** It compares the COMMITTED trace,
`examples/dom-tests/NN.trace.expected`, against the reference — not a freshly
emitted one. `scripts/dom-test.sh` already diffs the emitted trace against
`NN.trace.expected` and fails the suite on any difference (its step 2), so on a
green dom run the committed trace IS the emitted trace. **Proven, not assumed**:
recomputing families 02, 13 and 07b from committed bytes reproduces
`build/logs/dom-*.l2.log` from the last real dom run byte for byte, differing only
in the `b:` path line.

The consequence, stated plainly: this gate cannot catch an emitter change on its
own, because dom-test.sh catches that first. It catches what dom-test.sh cannot —
a re-baselined trace that silently moved a family's verdict, and a reference trace
or delta rule that moved underneath one.

**It does not change `scripts/dom-test.sh`.** That script's choice to report L2
rather than fail on it is still right for the reason its own header gives: a
difference between two emitters is a finding, not automatically a regression in
this one. The gate is the answer instead of changing it.

**What it enforces beyond equality:**

| rule | why |
|---|---|
| a `DIFFERS` row must carry `attribution` | "attributed" becomes mechanical instead of being a word in a table |
| every cited cause must be a key of `$whatIsDeliberatelyNotHere` in `examples/dom-tests/deltas.json` | catches a typo and an unwritten cause with the same check |
| an `EXCLUDED`/`GAP` row must carry `why` AND must really be uncompared | **this is the tripwire for families 08/09/10**: the moment a `.family` file names one, its declaration is stale and the check fails with "committed GAP, measured …" |
| a `MATCH` row may not load a delta rule that matched nothing | a stale rule is how an accepted delta quietly becomes an unnoticed regression |
| an IMPROVEMENT fails too | the `NN.maxbytes` precedent: a budget you beat is one you re-baseline, not one you leave loose |

Directions reported: `REGRESSED`, `IMPROVED`, `NEW`, `REMOVED`, `UNATTRIBUTED`,
`UNKNOWN-CAUSE`, `UNJUSTIFIED`, `STALE-RULE`.

**It caught something real, in its first wave, unprompted.** The backend's wave-6
sweep ran `contract/parity/check.sh` and it stopped at the new gate:

```
IMPROVED      08-fragments: committed GAP, measured DIFFERS
UNATTRIBUTED  08-fragments DIFFERS with no `attribution`
```

Family 08 had gone from GAP to DIFFERS that same wave and had no row yet. That is
the tripwire in the "an EXCLUDED/GAP row must really be uncompared" rule firing
exactly as designed, on the exact family it was designed for — and it fired on an
IMPROVEMENT, which is the half of the design most likely to be argued with. The row
was then recorded with its two attributed causes, and the same command is green.
This is the difference between a status file and a gate.

**Negative controls, all six run and tripped:**

```
IMPROVED      01-simple-elements: committed DIFFERS, measured MATCH
REGRESSED     02-text-interpolation.differences: committed 24, measured 25
IMPROVED      05-conditional: committed GAP, measured DIFFERS
IMPROVED      06-insert-children.differences: committed 99, measured 20
UNATTRIBUTED  13-topcoat-for DIFFERS with no `attribution`
UNKNOWN-CAUSE 14-topcoat-local cites "no-such-cause"
```

---

## 2. Suite counts

Every number comes from a command someone can run, named beside it.

| suite | command, from `spike/rustc-codegen-js/` | fixture set | count |
|---|---|---|---|
| runtime | `./scripts/test.sh` | `examples/tests/*.rs` (19) + `examples/core-tests/*.rs` minus `prelude.rs` (28) | ▣ **47/47** |
| runtime, 8 further flag modes | `./scripts/matrix.sh` | trampoline, queue-off, switch-flat, scoped-lets, line-comments, source-map, minify, minify-locals | ▣ **47/47 each** |
| runtime, `js-names=mangled` | `./scripts/matrix.sh` | the standing OUT mode, PINNED at its broken count rather than skipped | ▣ **33/47** |
| emit goldens | `./scripts/emit-test.sh` | `examples/emit/*.rs` | ▣ **20/20** |
| module | `./scripts/module-test.sh` | same, minus any `NN.core` fixture | ▣ **19/19** |
| dom | `./scripts/dom-test.sh` | `examples/dom-tests/*.rs` | ▣ **21/21** (+`08_fragments`) |
| extern | `./scripts/extern-test.sh` | `examples/extern-tests/*.rs` (2) + a synthetic vectors entry | ▣ **4/4** |
| async | `./scripts/async-test.sh` | `examples/async-tests/*.rs` | ▣ **7/7** |
| dts emission | `./scripts/dts-test.sh` | `examples/dts-tests/*.rs` | ▣ **2/2** |
| keyed identity | `node scripts/keyed-identity-check.mjs` | — | ▣ **10/10** |
| event accessors | `node scripts/event-accessor-check.mjs` | — | ▣ **8/8** |
| reactive `for` | `node scripts/reactive-for-check.mjs` | — | ▣ **18/18** |
| **root workspace** | `cargo test --release` | workspace members | ▣ **420** (+6 `view-dom`, +6 `mires-build`) |
| demo-app | `cargo test` in `demo-app/` | — | ▣ **15** |
| demo-app smoke | `node demo-app/smoke/check.mjs` | — | ▣ **99** |
| **L2 corpus parity** | `node --import ./register-loader.mjs check-l2-status.mjs` | 18 corpus families | ⚙ **18 rows, 0 unexplained** |
| hydration parity | `contract/parity/check.sh` | `contract/parity/fixtures/*` (4) | ▣ **217**: counter 44, dashboard 63, nested 49, search 61; every negative control fails as it must |
| parity harness units | `node contract/parity/test.mjs` | — | ⚙ **176/176** |
| `.d.ts` goldens | `node contract/parity/dts.mjs --check` | `contract/parity/dts/` | ⚙ **6/6**, incl. a real `tsc --noEmit` over 4 files |
| size budgets | `node contract/parity/budgets.mjs` | 11 subjects | ▣ **every subject inside its band** |
| contract drift | `node contract/harness/run-all.mjs --check` | `contract/fixtures/` | ⚙ **15 steps** |

**The flag matrix is now a script, and this was the audit's last structural
finding.** ▣ It said: "The 9 flag modes are not encoded anywhere. There is no CI
file, no Makefile and no matrix script in the spike … A verdict claiming '46/46
across 9 modes' is claiming the result of nine manual invocations. Either write the
matrix script or say it is manual."

`scripts/matrix.sh` was written. **Ten modes, one table**, and it does two things a
hand run does not:

- it **pins `js-names=mangled` at its broken count** (33/47) rather than skipping it,
  so the standing OUT entry keeps a number attached instead of becoming a habit;
- it **always restores the sysroot to the default flags** as its last step. A matrix
  run leaves `build/sysroot/.js-args` at whichever mode ran last, and wave 5 lost
  time to exactly that — a demo-app build refusing with `found "js-names=mangled",
  wanted ""`.

So a verdict may now claim the matrix result as a single reproducible command. The
option list is `CONTRACT.md:226-237`.

---

## 3. Size table

Raw and gzip, gzip pinned at level 9. Every number measured 2026-07-30, from
`contract/parity/budgets.json`, enforced within 5% (128-byte floor). ⚙

### Islands — a chunk plus the shared chunk

| subject | raw | gzip |
|---|---|---|
| `island:counter` | 3,159 | 1,072 |
| `island:nested` | 3,882 | 1,289 |
| `island:search` | 3,942 | 1,547 |
| `island:dashboard` | 9,385 | 2,630 |

### Chunks and runtime

| subject | raw | gzip |
|---|---|---|
| `chunk:shared` | 338 | 227 |
| `runtime` (`mires-dom.js`, solid-js/web + core, one reactive graph) | 23,971 | 9,106 |
| `loader` (`island-loader.js`) | 3,320 | 1,483 |
| `island-rt` (`island-rt.js`) | 8,345 | 3,570 |

### Pages — everything a visitor actually downloads

| subject | raw | gzip | contents |
|---|---|---|---|
| `page:island` | 30,450 | 11,661 | runtime + loader + counter + shared |
| `page:search` | 39,687 | 15,740 | + island-rt + search |
| `page:dashboard` | **57,258** | **21,077** | + chart-lib + dashboard |

The dashboard page is the milestone's real cost: **21 KB gzipped for a fully
hydrated chart-and-live-feed page**, of which 9,106 is the vendored reactive
runtime, shared by every page.

Not budgeted, with reasons written at the exclusion: five `.js.map` source maps
(served, but no browser fetches one unless devtools are open) and
`/demo/chunks.importmap.json` (the page inlines the map in a
`<script type="importmap">`, so a browser never fetches this route).

---

## 4. The standing OUT list

Inherited from `VERDICT.md:180-188`, with the audit's three amendments applied and
one entry whose KIND changed.

| out | reason | status |
|---|---|---|
| components / `#[component(client)]` | emission still missing | **the parenthetical is dropped.** `VERDICT.md:182` qualifies this with "family 07 — props equivalence undefined". Props equivalence IS defined: `contract/fixtures/corpus/07-components/NOTES.md:5` and `corpus/README.md`. Keep the entry, drop the qualifier. |
| `#[procedure]` from compiled code | — | unchanged |
| **keyed `for`** | `each` renders index-keyed lists | **kind changed: this is a design consequence, not a deferral.** See below. |
| nested islands | — | unchanged |
| client-side text conversion for non-primitive hole values | a `&&str` hole inserts its slot record | unchanged |
| `js-names=mangled` with core+alloc | crate-dependent symbols; readable names are the tool | unchanged |
| memo for `match` arms | deliberate: an arm selects on a pattern, and its body reads what the pattern bound, so there is no test to split out without matching twice | unchanged |
| per-island chunking and lazy hydration | stage 4 | unchanged |
| **grammar modules `class` and `props`** | neither is reachable from `view-dom`, which never sees those macro bodies | **added** (audit amendment 2). They are structurally out of scope and were previously left to look like omissions. |
| **corpus families 09 and 10** (SVG, custom elements) | `TemplateData` carries no template-construction flags: an ABI bump plus, for SVG, a walk-rooting change with a silent failure mode | **added** (audit amendment 2, narrowed). 08 closed instead. Both stay GAP with an exact four-step spec banked; **neither can ever be a MATCH even once the flags land**, for reasons recorded under gate item 1. |
| ~~`<"tag-name">` element names~~ | — | **REMOVED from OUT.** Supported, validated, and now read: two refusals (not a tag name; a void element) plus a test that the literal lowers identically to the identifier spelling. See gate item 2. |
| a vector for the `#[js_extern]` global root | measured next door by `globals-check.mjs`; no vector in `contract/fixtures/js-extern/` | standing GAP J-2 |
| `#[js_extern]` callbacks | nothing covers handing JavaScript a compiled Rust closure | deliberate, `contract/JS-EXTERN.md` |
| pairing a template with its hydration key | `lib/keys.mjs`'s `rootsByKey` finds the other roots; the pairing is the work left | `contract/parity/README.md` known limits |
| `fixtures/search/`'s `islandTemplate: 0` | labelled unverified; `nested` showed the emitter declares component-body templates first, so a loop-body template is likely declared first here too | `contract/parity/README.md` known limits |
| one island asserted per parity fixture | the loader hydrates every island on the page and the run watches the whole document, so a second island would be hydrated but not asserted about | `contract/parity/README.md` known limits |

**The deferred-shapes list is separate and stays separate.** `contract/CONTRACT-DOM.md`
section 13 marks Suspense/resources/lazy, Portal, seroval serialization, streaming
SSR, and assets/request context as out of scope, each with a Topcoat delta
explaining why, plus async hydration. That list is about the dom-expressions runtime
contract, not about this compiler's coverage; merging the two would make the OUT
list look longer and less decided than it is.

The one literal "out for G2" in the tree is
`contract/fixtures/corpus/07-components/NOTES.md:11`: getter props, which "only pay
for themselves with spread and dynamic props, which are out for G2".

### Keyed `for`: why this OUT entry is not waiting on a fix

The audit called this "the single most consequential finding of wave 5" and left
the decision open. **Wave 6 decided it: `push_keyed` stays identity-only, the
dashboard is not re-keyed, and the parity fixture's inverted assertion stands with a
second cause added.** Backend wave-6 report, item 1. Three findings:

- **K1 — there is no disposal bug to fix.** A plain `( )` hole in a row is
  `Fill::Once` and emits a bare `_$insert` with **no effect at all**; a `$( )` hole
  IS an effect but is owned by the ISLAND, not the row, because a Rust loop opens no
  reactive scope. So the rule is sharper than "keyed is for static rows": **a keyed
  row's changing parts must be reactive holes OF THE ROW'S OWN TEMPLATE**, and a
  plain `( )` hole in a keyed row is written once to a node the reconcile then
  discards. The dashboard's movers list is exactly that — a `$( )` there cannot
  carry the free function call the price formatting needs.
- **K2 — both repairs are unsound.** Re-running the fills against the cached node
  silently DUPLICATES an anchored child hole: upstream's `insert` seeds
  `current = []` when `initial` is undefined, so `cleanChildren` inserts a second
  text node before the anchor on every render. Morphing the cached node from the
  fresh one cannot see `HoleKind::Event` or `HoleKind::Property`, and there is no
  DOM implementation anywhere in the spike to verify a morph against — the three
  stubs that exist disagree about where a hole's value even lives.
- **K3 — the deciding one.** Even a perfect fix does not move family 13 to MATCH.
  The reference is solid's `<For>`, which clones a row's template once per row
  EVER; ours is an eager Rust loop that clones once per row PER RENDER, before
  `push_keyed` is reached. **The divergence is the loop, not the reconcile** —
  which is what the standing rule is named after, `keyed-list-vs-a-rust-loop`.

So family 13 is ATTRIBUTED for a structural reason that outlives any version of
`push_keyed`, and `l2-status.json` records it that way.

The parity fixture asserts the OPPOSITE of the obvious claim, deliberately:
`contract/parity/fixtures/dashboard/interact.mjs` section 4 asserts the rows are
REBUILT and the list element is not, because asserting keyed identity would pass for
the broken version and fail for the correct one. **That assertion has flipped twice
and its full history is now written into the file's header**, with the three findings
above, so a third flip has to defeat K3 rather than just K1 and K2.

---

## 5. The delta ledger: every DIFFERS cause

**Eleven** named causes, each a key of `$whatIsDeliberatelyNotHere` in
`examples/dom-tests/deltas.json`, each with one written paragraph there. Every
`attribution` in `l2-status.json` cites one of these and the gate fails if it cites
anything else.

| cause | what it is | families |
|---|---|---|
| `anchor-placement` | outside hydratable mode the reference emits a `<!>` only for an expression sandwiched between two text nodes and lets adjacent expressions share one; view-dom anchors every dynamic child of a multi-child element. Changes the template string, the walk and the `_$insert` marker together, so it is one divergence showing up as many records. **view-dom's, not the backend's.** | 02, 06, 15 |
| `write-once-vs-always-reactive` | JSX has one syntax for a dynamic attribute and wraps every non-static one in an effect. Topcoat has two: `name=(expr)` is written once, `:name=$(expr)` is reactive. A **language** difference, not an emitter defect. | 03, 14 |
| `different-reference-program` | the reference modules for 11 and 12 use solid's `<Show>`, so their traces carry `createComponent` and `Show` records no Topcoat program can produce. Topcoat's `if` is a Rust conditional, not a component. Not the same program; reported for what it is. | 11, 12 |
| `delegate-events-placement` | the reference emits one `delegateEvents` at the end of the module carrying the union over every template; this backend emits one per instantiated template that needs one. Both register the same listeners. | 04, 15 |
| `fixture-coverage` | some cases of a family have no counterpart here — the macro still refuses them, or the spike has no type to write them with. Each fixture's module docs say which. Reported as ONLY IN A. | 02, 04, 05, 06, 07, 07b, 12, 14, 15 |
| `memo-hoisting-for-a-match` | a `match`, and an `if` whose test binds, reach the backend as one closure and get no memo: an arm selects on a pattern and its body reads what the pattern bound, so there is no test to hand over separately. `view_abi::cond` documents it. | 12 |
| `keyed-list-vs-a-rust-loop` | the recorded reference is JSX's `<For>`, a keyed reconciling component; Topcoat's `for` is a Rust loop that runs once where it is written and hands one list to one `_$insert`. See section 4. | 13 |
| `eager-child-content-and-the-cases-it-makes-unwritable` | child content of a client component is an eager `view_abi::Node` field of the props struct, built BEFORE the component boundary, taking slots in the CALLER's key context; the reference builds it inside the callee from a `get children()` getter. Permanent, quantified by `keys-nested.json`'s `07b-children-component-eager` vs `-getter`. Today view-dom rejects component children outright, so those cases have no counterpart at all. **Not expressible as a rule: every rule kind preserves record counts.** | 07, 07b |
| `the-getter-children-double-clone` | family 07's `withChildren` shows TWO `template.clone` records where real solid produces one, because `harness/trace.mjs`'s `value()` reads every key of the props object and so forces `get children()` before the component runs. A **harness** artifact. Subsumed by the entry above, since the case is unwritable anyway. | 07 |
| `fragment-roots-must-be-elements` | **new this wave.** `DomWriter::template` refuses bare text and a bare `(expr)` outside an element, so upstream's bare `{inserted}` fragment member is written `<div>(inserted)</div>` here — adding a real element, a real template and a real `_$insert` (4 of family 08's 7 lines). `corpus/08-fragments/NOTES.md:18-25` calls it an undecided LANGUAGE question — can a view body start with text? — not a missing feature. | 08 |
| `memo-for-a-fragment-member` | **new this wave.** A dynamic member of a JSX fragment is an ARRAY ELEMENT, so the plugin wraps it in `_$memo`; nothing else would subscribe. Ours is a child of the `<div>` that wraps it, so `_$insert` takes the accessor and subscribes itself — the same reactivity with one fewer allocation. 1 line. | 08 |

### Retired deltas — measured, and they did not occur

Recorded because a delta that stops occurring and is left standing is how an
accepted difference quietly becomes an unnoticed regression.
`examples/dom-tests/deltas.json` `$measuredAndRetired`:

| retired | outcome |
|---|---|
| `text-gt-escaping` | predicted from Topcoat escaping `>` where solid does not. **Does not occur**: the reference plugin constant-folds a string-literal child into the template and escapes it on the way in, while view-dom treats `(expr)` as a dynamic hole whatever it holds — our template is `<span>Hi<!></span>` and the text never enters the HTML, so no escaping happens to differ over. Removed rather than left standing. |
| `template-declaration-order` | **fixed, not accepted.** A cloner carries the source position of the `view!` that built it; where several views build the same template the EARLIEST position wins. Pins: family 01 went from seven differences to none, family 11 from 38 lines to 11. |
| `node-is-zero-sized` | **fixed in the ABI (version 3).** `view_abi::Node` is a `repr(transparent)` one-word handle, so a template root survives being returned and being passed through `content`. Pins: families 11 and 12 emit an `insert` per branch, and the island entry point returns its root. |
| `memo-hoisting-for-a-conditional` | **fixed.** `view_abi::cond` hands the test over as its own closure and the backend emits `_$memo(() => test())`. Both spellings go through it. Pins: family 05 lost three `memo`-only lines, family 11 lost all of its. |

### One artifact of a RULE, not of either emitter

Found while attributing family 03 and recorded as a `note` on its row rather than
silently absorbed: the reference writes a boolean attribute bare as `disabled`,
view-dom writes `disabled=""`, and the `template-attribute-quotes` rewrite rule
turns that into `disabled=`. So one of family 03's nine differences is the rule's,
not the emitter's. A clause for the empty value would close it.
`examples/dom-tests/deltas.json` is backend-owned, so this is a finding rather than
a fix.

---

## 6. Documents brought current

The audit's fourth structural finding was that the repo's own status prose was two
waves behind the measured state and that one document contradicted the code. Every
one of them is a row someone would otherwise re-derive by hand at the gate.

**The audit named six. There were eight** — the two extra were found by reading the
code rather than the audit, and both were claims the code had already falsified.

| document | was | now |
|---|---|---|
| `contract/fixtures/corpus/README.md` families table | a `view-dom status` column, two waves stale | column REMOVED, replaced by a pointer to `l2-status.json` and the command to check it |
| `…/README.md` L0 list | an enumeration of which families compile | pointer — that enumeration is live status |
| `…/README.md` L1/L2 intent table | read as status | kept, relabelled **DESIGN INTENT**, with the two intents measurement has since settled (05's memo gap closed; 13's cannot) |
| `…/README.md` NOT-YET-LOWERABLE ledger | listed `if`, `match`, `for`, `let`, block, bind attribute and attribute spread as blocking errors — **this is the document that contradicted the code** | rewritten from the `unsupported`/`error` call sites, retitled "What `view-dom` refuses", and it keeps a paragraph saying what it USED to claim so a reader can tell a refreshed document from a stale one |
| `…/README.md` accepted deltas | "Five rules are seeded today… nothing measured yet" | names the file the suites actually load, and what each seeded rule measured to |
| `…/{11,12,13,14,15}/NOTES.md` line 3 | "**NOT-YET-LOWERABLE.**" | "**LOWERS.**", what IS still refused, and a pointer — never a restated status |
| `…/{11..17}/view.rs` module docs | the same claim in Rust doc comments | same treatment |
| `…/{05,11,12,13,14,15,16}/NOTES.md` "view-dom features needed" | NOT-YET-LOWERABLE bullets | LANDED / STILL REFUSED / DELIBERATELY NOT COMING |
| **`…/16-topcoat-spread/`** (not in the audit) | "attribute spread … not yet supported"; `HoleKind::Spread` "never constructed" | it IS constructed (`lower/attributes.rs:23`). The family is EXCLUDED for a different reason: no Rust type to write a spread's VALUE with |
| **`…/17-topcoat-signal/`** (not in the audit) | `signal_declaration.rs` "emits nothing at all", "drops the initializer", "the signal has no storage" | it emits `let <ident> = ::view_abi::signal(<ordinal>, <init>);`. Storage exists |
| `…/03-attribute-expressions/NOTES.md` | property sink "named in `view-abi` and never constructed" | partially stale: the table landed (`view-dom/src/contract.rs::is_property`) and the kind IS constructed, but only from `lower/bind_attribute.rs`, so `value=(v)` still reaches `setAttribute` while `:value=$(v)` reaches the property sink |
| `contract/fixtures/corpus/deltas.json` | **orphaned**, and its `$status` still said "Seeded, not measured. The Rust emitter does not compile these fixtures yet" | KEPT and reduced to a pointer — see below |
| `contract/G2-CHECKLIST.md` | would itself have gone stale | banner at the top: it is the frozen wave-5 audit, this file is current |

**The recurrence fix, which is the checklist's own lesson applied: no corpus
document restates a per-family status any more.** Every one of them points at
`contract/fixtures/l2-status.json`, which is the only place the status lives and is
now enforced. That is what stops this class of staleness coming back, and it is why
the fix was a pointer rather than a rewrite.

**Why `corpus/deltas.json` was kept rather than deleted.** The audit offered both.
Deleting would have broken two live citations: `examples/dom-tests/deltas.json`
cites four of its rules by id as "an ATTRIBUTED COPY of the rule of the same id"
there, and `CONTRACT.md:1557` cites its prediction for family 07. And its
`$ruleKinds`, `$rejectedRules` and `$notExpressibleAsRules` are written nowhere
else — particularly the rejected blanket-rewrite spelling of the escaping delta,
which was caught by running it. So `$what` now says it is a design record, a new
`$notLoaded` block names the live file and the live status file, and `$status`
records what each seeded rule actually measured to.

### Two stale documents this slice could not fix — REQUIRED CORRECTIONS

Neither is in `contract/`. Both are listed here because the verdict writer will
otherwise repeat them.

| document | correction |
|---|---|
| `VERDICT.md:182` | drop the parenthetical "(family 07 — props equivalence undefined)". Props equivalence IS defined: `contract/fixtures/corpus/07-components/NOTES.md:5`. Keep the entry itself if the emission is still missing. |
| `CONTRACT.md:426` | says "28 runtime fixtures and 13 JS goldens". ▣ The suites are now **47 and 20**. |

---

## 7. The three case asymmetries — still open, still small

The audit's fifth structural finding, carried forward unchanged because each is a
question someone will ask at the gate and none has a written justification:

- `05-conditional` `jsx.jsx:15` `withMarkup` has no `view.rs` counterpart. It is
  the source of family 05's remaining ONLY-IN-A `memo` record, which is **not** the
  retired `memo-hoisting-for-a-conditional` gap — now written into
  `05-conditional/NOTES.md` and into `l2-status.json`'s note, because it is exactly
  the kind of thing that gets misread as a regression.
- `06-insert-children` `jsx.jsx:29` `array` has no `view.rs` counterpart.
- `17-topcoat-signal` `view.rs:51` `two_signals` has no `jsx.jsx` counterpart.

Families 06, 08 and 10 document their drops; these three do not.

---

## 8. Drift

**ZERO.** ⚙ `node contract/harness/run-all.mjs --check`, run at the end of this
slice, on the tree as it stands:

```
=== check-l2-status.mjs --check
  L2 status matches contract/fixtures/l2-status.json: 13 DIFFERS, 2 EXCLUDED, 2 GAP, 1 MATCH

=== drift check
  no drift: committed fixtures match regenerated output exactly

all steps green
```

All **15 steps** green, including the three that are checkers rather than
extractors: `check-procedure-wire.mjs` (90 checks over 19 vectors, 9 pinned by an
existing test and 10 derived from the cited code), `check-js-extern.mjs` (17
vectors, 69 assertions) and the new `check-l2-status.mjs`.

The new step writes nothing, so it cannot perturb the fixture snapshot — which the
clean drift report above confirms rather than assumes. Every file this slice edited
under `contract/fixtures/` is hand-written and regenerated by no step:
`corpus/README.md`, eight `NOTES.md`, seven `view.rs`, `corpus/deltas.json`, and the
new `l2-status.json`. All JSON revalidated; `interact.mjs` re-parsed.

---

## 9. Deviations from the brief for this slice

1. **The stale-document list was six; the real count was eight.** The two extra
   (families 16 and 17) were found by reading the code rather than the audit, and
   both were claims the code had already falsified. A third, family 03's, was
   partially stale in a way that needed a more precise statement rather than a
   correction. Section 6.
2. **`corpus/deltas.json` was reduced to a pointer rather than deleted.** The audit
   offered both. Deleting would have broken two live citations by id and lost three
   prose sections written nowhere else. Section 6.
3. **The L2 gate compares committed traces, not freshly emitted ones.** This was a
   design choice, not a constraint of the slice's tooling: it makes the gate a
   one-second no-build check, and it is sound because `dom-test.sh` already gates
   the committed trace against the emitted one. Its limits are stated in gate item 7
   rather than left to be discovered.
4. **The gate lives in `contract/harness/`, not in `scripts/dom-test.sh`.** Partly
   ownership, but mostly because `dom-test.sh`'s choice not to fail on L2 is still
   right for the reason its own header gives. The gate is the answer instead of
   changing it.
5. **`VERDICT.md:182` and `CONTRACT.md:426` are NOT fixed**, only specified. Neither
   is in `contract/`. They are carried as required corrections in section 6.
