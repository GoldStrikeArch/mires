# Wave 5 — Contract slice notes (LIVE bank, append-only)

Owner: contract agent. Owns `contract/`. Read-only elsewhere.
Peers: backend (`build/logs/wave5-backend-report.md`), framework (`build/logs/wave5-framework-report.md`).

Resume protocol: read this file top to bottom before doing anything.

## Items

1. `JS-EXTERN.md` encoding section (S, FIRST) — precise grammar for the length-prefixed
   symbol encoding + rationale, plus the wave-5 global-scope root once the backend banks it.
2. Dashboard parity fixture (M) — poll framework report for `DASHBOARD SERVING`; capture,
   fixture, `interact.mjs` with hydration checks + live EventSource loop; budgets with bands.
3. `.d.ts` golden support (S/M) — tsc `--noEmit` if typescript is trivially pinnable under
   `contract/vendor`, else structural golden diffs; wired as a drift step.
4. Wave-6 prep (S) — `contract/G2-CHECKLIST.md`, an audited gate checklist.

## Log

(append-only; newest at bottom)

- STARTED: bank file created as first action. Peer reports both at "init"/"STARTED" — backend has
  not yet banked the js_extern global-scope shape, framework has not yet reported DASHBOARD SERVING.
  Proceeding with item 1's readable half (existing encoding from the wave-4 shared file), item 4
  (G2 checklist audit, no peer dependency), and item 3's vendor probe.

### Item 1: JS-EXTERN.md encoding section — WRITTEN (pending the wave-5 global-scope amendment)

Sources read: `js-extern-macro/src/descriptor.rs` (the `#[path]`-shared file, the ONE definition),
`js-extern-macro/src/lib.rs` (the surface syntax), `backend/src/js_extern.rs` (the lowering),
`backend/src/naming.rs::extern_import`, `backend/src/jsast.rs::ImportSpecifiers`,
`build/logs/wave4-backend-report.md` steps 3/4 (deviations 2 and 3).

Replaced the stub at `contract/JS-EXTERN.md` "## The attribute encoding" with:

- the grammar `rcgjs.ext.<version>.<shape><flags>.<module-len>,<name-len>.<module><name>`, a
  field-by-field table (7 shape letters, `n` the only flag), and the two empty-field meanings
  (empty module = global, empty name = index straight off the receiver);
- "Why the lengths and not a delimiter": no character is safely excluded from BOTH halves
  (`. / - @ :` in specifiers, `$ _ .` in dotted names), so any delimiter needs an escape and an
  escaping bug is silent - it still decodes, just to a different interface, and the program calls
  the wrong thing successfully. Cited the three tests that hold it shut
  (`a_specifier_full_of_delimiters_survives`, `every_malformed_descriptor_is_an_error_and_not_a_guess`,
  `a_length_that_splits_a_character_is_refused`), plus why not a binary codec (skew failure mode is
  a mis-decode, not a refusal) and why the version is decoded first.
- the six answers the stub asked for: (1) module + binding, the path's FIRST segment IS the import
  binding; (2) dotted path, walked one member at a time, member shape roots at arg unless it names
  a module; (3) the optional trailing argument is NOT spelled - arity is the Rust signature's,
  nothing padded, nothing stripped, so a 2-param decl can only be `send-trailing-omitted`;
  (4) `n` flag, `== null` not `===`, so BOTH null and undefined are `None`, plus the two guards
  (declaration-time reject on a value-less shape, zombie if the destination is not an `Option`);
  (5) index is an accessor not a type, so `length` stays a plain `get` and the loop is expressible;
  (6) the three-way split of a bad declaration: spanned compile error / reachability-gated zombie
  carrying the descriptor text / an undetected runtime `TypeError` against the real library.

Also updated the stale `**Status.**` line (said "nothing here is implemented yet") and the
"import specifier is not pinned" open question (the specifier IS now pinned - carried and emitted
verbatim as the `from` of a named import; what is still open is who RESOLVES it).

GAP FOUND (for G2-CHECKLIST): **`new-default` has no measured emission.** Imports are only ever
emitted as named specifiers (`jsast::import_named` -> `ImportSpecifiers::Named`, which prints
`{ exported as local }`). `import { default as x }` IS exactly a default import and
`is_js_name("default")` passes, so `#[js(new = "default")]` is the spelling the encoding admits -
but nothing in any suite compiles that declaration and runs it against the `new-default` vector.
Documented as "what the encoding admits rather than what a vector measured" and listed as a gap
rather than claimed. Note `new-named` vs `new-default` is the ONLY thing those two vectors differ
on (`via`), so an unproven default binding leaves half of the `new` shape unmeasured end to end.

STILL OPEN on item 1: the global-scope root the backend adds THIS wave (their item 2,
"js_extern global-scope shape (S, dashboard-blocking)"). Today a global is emitted as the bare
identifier (`jsast::id(head)`, `js_extern.rs:151-155`) and imports nothing. Polling
`build/logs/wave5-backend-report.md` for the change; the section gets amended when banked.

### Item 3: .d.ts golden harness — LANDED (armed, waiting on the backend's emission)

**typescript IS trivially pinnable, so the tsc path is the one that shipped.** Probe: the npm
registry is reachable from this environment. `typescript@5.9.3` has ZERO dependencies and is pure
JavaScript, so it pins exactly like every other upstream artifact: one tarball, one sha512.
Pinned at `contract/upstream.lock` (new `npm.typescript` entry) and fetched with fetch.sh's own
verify-then-unpack steps into `contract/vendor/upstream/typescript/`; `node .../bin/tsc --version`
reports 5.9.3 and the sha512 matched the lock.

Two deliberate deviations from the obvious placement, both recorded in the lock entry's notes:

- **NOT typescript 7.** 7.x is the native port and ships its compiler as ~20 platform-specific
  optional deps (`@typescript/typescript-darwin-arm64` and friends), so one sha512 could not cover
  it and the pin would resolve differently per machine. 5.9.3 is the last pure-JS zero-dep line.
- **NOT added to `vendor/package.json`.** That would rewrite `yarn.lock`, which is load-bearing for
  byte-matching (it forces `@babel/traverse`/`@babel/generator` to upstream's exact versions). A
  tool only ever spawned as a subprocess does not belong in the module graph. Nothing resolves
  through `upstream/typescript`; it is invoked as a program.

**Shape: `contract/parity/dts.mjs`**, two modes (`node dts.mjs` records, `--check` fails on drift),
matching capture-ssr.mjs. Wired into `contract/parity/check.sh` as a step between the parity run
and the budgets, and it forwards `"$@"` so `check.sh --check` gates it.

Two layers, because they catch disjoint failures:

1. STRUCTURAL. `dts/structure.json` is the declared surface reduced to kinds, names, type
   parameters, parameter names+types, members. Parsed with typescript's OWN parser (a regex over
   declaration text is a parser that is wrong about generics and overloads); a degraded regex path
   exists for a missing vendor tree and SAYS SO in the summary rather than pretending. Sorted, so a
   reordered emission is not a diff. Verbatim copies land in `dts/emitted/` so bytes are diffable
   too. Drift = a JSON/byte diff, caught by the same record-vs-check idiom as the SSR captures.
2. TYPE. `tsc --noEmit` over the .d.ts files PLUS one consuming `.ts` per island. This is the
   layer a golden cannot reach: a .d.ts can be byte-stable and still be nonsense (naming a type it
   never declares, exporting a function whose parameter type does not exist). None of that ever
   shows up as drift, because none of it changes once it is wrong. `skipLibCheck` is OFF on purpose
   (the declaration files ARE the subject) and `types: []` keeps stray @types out of the program.

**Consumers are scaffolded, and the scaffold admits it.** Islands are not known until emitted, so a
consumer is generated on first record: it imports the module and pins each export to a local of its
own declared type, which proves the file resolves and names types that exist, and proves nothing
about whether they are the RIGHT types. It carries a `// SCAFFOLD: ... not yet reviewed.` marker
line; dts.mjs never rewrites a consumer whose marker has been removed, so reviewed files stay
reviewed and generated ones stay generated.

**Self-arming, so it cannot pass forever.** no emission + no goldens -> PENDING, exit 0 (nothing
recorded, nothing to be wrong about); emission + no goldens -> records, and the goldens now exist;
goldens + no emission -> FAILS. The day the backend emits its first .d.ts a record run arms the
check with no further edit here.

**Proved the type layer actually catches things** rather than shipping an unexercised tsc call.
Added a `.d.ts` section to `contract/parity/test.mjs` (helpers exported from dts.mjs; `main()` now
runs only under `import.meta.main`). It asserts, against temp files: a well-formed .d.ts + consumer
passes; a .d.ts naming an undeclared type FAILS and the message names it; a consumer misusing a
correct declaration FAILS and the CONSUMER is blamed. `node test.mjs` -> **all 173 checks passed**
(was 160; +13). `node dts.mjs` and `--check` both report PENDING and exit 0 against today's tree.

Also added `outDirFilesBySuffix(suffix)` to `contract/parity/lib/delivery.mjs`, next to
`locateOutDirArtifact` and under the same documented OUT_DIR PATH CONTRACT. The harness DISCOVERS
the .d.ts files rather than naming them, because what the backend calls each file is the backend's
to decide and a harness that named them would report "nothing emitted" the day the naming changed.
Newest OUT_DIR wins; only one is read, so two emitters' output is never merged into one surface.

### Item 4: contract/G2-CHECKLIST.md — WRITTEN (audited, not assembled)

Two fan-out audits (corpus families; grammar-module coverage) plus my own spot-checks of every
load-bearing ref. Verified by hand before writing: `view-dom/src/lower/node.rs:41-46` (Continue and
Break refusals exist), `lower/attributes.rs:50-64` (Block/Continue/Break refusals exist),
`dom_writer.rs:1305-1316` (the node table has exactly TWO rows, DOCTYPE and expression-name, so the
five refusals above are genuinely unasserted), `lower/element.rs:20` (`string_name()` accepts the
LitStr form silently), `scripts/dom-test.sh:263-264` (exits only on `$failures`), and that
`contract/fixtures/corpus/deltas.json` is referenced ONLY in prose from
`examples/dom-tests/deltas.json` and loaded by no code path.

**Gate item 1 (every corpus family MATCH or attributed): 1 MATCH, 12 ATTRIBUTED, 2 EXCLUDED, 3 GAP.**
- GAPs: 08-fragments, 09-svg, 10-custom-elements have NO `examples/dom-tests/` fixture, no
  `.family`, no verdict in any log. Each has a written blocker in its NOTES.md, so the gate choice
  is three fixtures or three EXCLUDED rows; leaving them unstated is the only wrong answer.
- Only family 01 is a true trace match. "MATCH or attributed" is satisfied at a ratio of 1:12, and
  a verdict that does not say so is true and misleading.
- **L2 cannot fail the suite** (`dom-test.sh:20-23` by design, exit at `:263-264`), so the single
  MATCH is undefended: family 01 could silently become ATTRIBUTED.

**Gate item 2 (every grammar module covered or expect_fail'd): FAILS AS WRITTEN.** "Grammar module"
= a node of the un-forked `topcoat-view-grammar`, mirrored file-for-file in `view-dom/src/lower/`
(`view-dom/src/lower.rs:1-5`). 6 of ~42 nodes are in neither bucket: `Node::Continue`,
`Node::Break`, `AttributeNode::{Continue,Break,Block}` (refusal in code, NO assertion) and
`ElementName::LitStr` (silently accepted, no test at all - the dangerous one, untested BEHAVIOUR
rather than an untested refusal). Five close as five table rows at `dom_writer.rs:1305`/`:1358`.
Structural finding behind it: **nothing keeps a refusal and its assertion in sync** - no registry,
no index, no exhaustiveness check - which is why the six exist and why a seventh will appear.
Also: `scripts/dom-test.sh`/`emit-test.sh`/`module-test.sh`/`extern-test.sh` have NO `expect_fail`
mechanism (only `scripts/test.sh` does), so that is not available to close them.
Two grammar modules (`class`, `props`) are structurally unreachable from view-dom and have no
written record saying so; they belong on the OUT list rather than looking like omissions.

**Gate item 3 (budgets): PASS**, qualified. Size budgets `parity/budgets.mjs` over `budgets.json`
(9 subjects, measured-not-chosen baselines, gzip pinned at level 9, coverage self-checked by
`unbudgetedRoutes` + `$notBudgeted`); emission budgets are `examples/emit/NN.maxbytes` under
`emit-test.sh`. Qualification: the dashboard will be the fourth and largest island and has no
budget yet.

**Gate item 4 (milestone islands):** counter/nested/search present; **dashboard PENDING**.

**Gate item 5 (standing OUT list):** inherited from `VERDICT.md:180-188` with three amendments -
one stale entry (the family-07 "props equivalence undefined" parenthetical is false per
`07-components/NOTES.md:5`), additions from this audit, and a note to keep CONTRACT-DOM.md
section 13's deferred shapes SEPARATE (it is about the runtime contract, not this compiler's
coverage; merging makes the OUT list look longer and less decided than it is).

**Gate item 6 (added; Stage 4 names it and the old wording does not): `#[js_extern]` + .d.ts.**
Carries GAP J-1, the unmeasured `new-default` binding from item 1.

**STALE DOCUMENTS (6), the part that saves the most wave-6 time.** Chief among them:
`corpus/README.md:45-64` and `:115-152` still list families 11-15 as not lowerable and seven
constructs as blocking errors, all of which now lower and five of which have L2 verdicts - **the
enumeration document contradicts the code**. And `contract/fixtures/corpus/deltas.json` is
ORPHANED, with a `$status` at `:23` still claiming no comparison has ever run.

**Top structural finding: there is NO committed machine-readable per-family L2 status.** The
verdict table exists only in `build/logs/*.log`, a regenerated build-artifact directory; the
`verdicts` string is built in memory in `dom-test.sh` and printed to stdout. So the primary
evidence for gate item 1 is not committed anywhere and reproducing it needs a full dom-test run.
Ranked #1 to fix before the gate.

Also recorded: 3 undocumented case asymmetries (05 `withMarkup`, 06 `array`, 17 `two_signals`),
and that **the 9 flag modes are not encoded anywhere** - no CI file, no Makefile, no matrix script;
"46/46 across 9 modes" is the result of nine manual invocations, and the verdict should either get
a matrix script or say it is manual.

### Item 1 AMENDED: the wave-5 global-scope root is banked and documented

Backend banked it (`build/logs/wave5-backend-report.md:36-176`). Amended `contract/JS-EXTERN.md`:

- **VERSION 1 -> 2.** The grammar did NOT move (the flags field was already "a run of letters");
  the vocabulary did. Documented with their reasoning: a v1 decoder meeting `g` refuses it as
  "not a flag", which is safe either way, but the version check refuses by NAMING the version,
  which is the more useful message. Nothing persists a descriptor across a build, so no migration.
- **`g` = rooted at the global scope**, second flag after `n`. `encode` writes them in that order,
  `decode` accepts any order. Updated both worked examples to v2 and verified them against
  `descriptor.rs::the_encoding_is_the_documented_one` (`rcgjs.ext.2.n-.8,5.chart.jsChart`,
  `rcgjs.ext.2.gn.0,5.width`). Added the warning that the `g` in `gn` is the SHAPE letter for a get
  and the `g` flag is a different field, since that collision is genuinely confusing on the page.
- **New "The three roots" section**, the wave-5 addition stated separately because the bug it fixed
  was SILENT: rooting used to be the predicate `module.is_empty()`, which could not say "global"
  for a shape that has a receiver. `new EventSource(url)` already worked (no receiver), but
  `#[js(get = "dash.status")]` with no module read a property of ARGUMENT ZERO - `undefined.status`
  with no args, or a property of an unrelated argument. Not a refusal either way. Now a `Root` enum,
  switched on in one place. Documented their three rules with the reasons: call/new need no flag
  (keeps wave-4 declarations compiling, keeps `Math.max` a one-liner); `global` CLEARS the block's
  module and `#[js(global, module = "x")]` is a spanned error naming both roots (a browser global
  declared inside a `module = "chart.js"` block would otherwise emit an import for a name the host
  already has - exactly the dashboard's shape); an index roots at its argument unless the flag says
  otherwise, because re-rooting would change what `index-in-range`/`index-out-of-range` emit.
- **Added a seventh answer** to the six-question list, pointing at the roots section, noting it is
  the one answer that changed AFTER this page's vectors were written and why.
- **New section "What the global root is measured against"**, stating honestly that none of the 17
  vectors exercises a global (they all drive the chart library, which is a module). The behaviour
  is proven NEXT DOOR: `examples/extern-tests/02_globals.rs` + `scripts/globals-check.mjs`, 17
  checks, one of which reads the emitted TEXT to assert a global imported nothing (running the
  program cannot tell you - an import-bound global resolves too). Recorded the eight proposed
  vector ids. This page must not be read as having pinned the global root.

### CORRECTION: my `new-default` gap was WRONG, and is withdrawn

I raised GAP J-1 from reading the backend source alone (imports only ever emitted as named
specifiers, so `#[js(new = "default")]` looked like "what the encoding admits, unmeasured"). It is
measured, end to end, and has been since wave 4:

- `examples/extern-tests/01_chart.rs:53` declares `#[js(new = "default")]`
- `examples/extern-tests/01_chart.js.expected:5` shows `import { default as ext$default$h12f6... }`
- `scripts/extern-check.mjs:115` runs the COMPILED `v_new_default` against `vector("new-default")`

Both `JS-EXTERN.md` and `G2-CHECKLIST.md` corrected. The checklist now records it as "checked and
NOT a gap", with the refs, precisely because `via` is the only thing the two vectors differ on and
a gate reader will want to re-check it. Lesson banked: I verified this before it reached the final
report, but the claim was in the doc for one edit cycle on a source-reading inference alone.

Replaced by **GAP J-2** (the real, smaller one): the global root has no vector in
`contract/fixtures/js-extern/`, only the backend's next-door recorder.

### Item 2: the dashboard parity fixture — SCAFFOLDED PENDING (never reached "DASHBOARD SERVING")

The framework agent never banked DASHBOARD SERVING: the backend has not banked DYLIB STABLE for
wave 5, so demo-app cannot be rebuilt and `/island/dashboard` does not serve. Rather than block, I
delivered it the way THIS HARNESS PRESCRIBES for an island that does not exist yet - a `pending`
fixture (`parity/README.md:202`), which is exactly what `search` was two waves before its island.
A pending fixture is not a skipped one: run.mjs checks everything not needing the server's bytes.

`fixtures/dashboard/{fixture.json,interact.mjs}` + pending `island:dashboard` and `page:dashboard`
budget subjects. Verified live: the pending path runs, its checks pass, negative controls skip.

**Everything is read out of fixture.json** - selectors, tick sequence, expected shapes. So the
island can ship with different class names or a different chart method without an assertion being
rewritten, and an assertion rewritten next to a running island is one that agreed with it.

interact.mjs asserts, in order: hydration (list + canvas survived, list starts empty, zero
mutations, the claimed node is still the same object); the island opened exactly ONE EventSource;
then the five-tick loop - render, **REORDER with keyed node IDENTITY** (tag each row by symbol
before, assert the node carrying each symbol is the same object after - DOM text cannot tell a
moved node from a rewritten one, which is the whole reason this island is the milestone),
membership change with survivors keeping their nodes across an insert/remove, the **error tick**
(no throw, list not blanked, subscription not closed, what it logged RECORDED not required), and
recovery. Plus a `dead` probe for the key-miss control, with the finding that this island is the
WORST case for silent hydration loss: its server markup is an empty list, so a completely
unhydrated dashboard and a correct one are indistinguishable on screen forever.

**THE STREAM IS STUBBED, THE CHART IS NOT.** A stubbed EventSource in the jsdom realm (both realms,
same seam `search` hit with fetch) pushes a deterministic sequence and dispatches BOTH the named
and default paths so an island wired differently than predicted still receives its frame. Stubbing
is also the only way to push a malformed frame on purpose, which a live feed will not produce to
order.

**FINDING while writing it: the framework agent serves MY OWN fixture as the chart library.**
`demo-app/src/dom.rs:293` serves `include_str!("../../contract/fixtures/js-extern/chart-lib.mjs")`
at `/demo/chart-lib.js`, with the import map resolving `topcoat-chart` to it. So the island calls
into the SAME recorder the js-extern vectors were recorded from. I threw away the proxy fake I had
written and read the recorder's own `__trace()` instead, asserting records in their ACTUAL shape
(`{op:"new",ctor,via,argc}` / `{op:"send",target,method,argc}`) rather than a style-alike. Needed
one general addition to run.mjs: `moduleFor(url)` -> the staged file URL, so a fixture imports the
same module INSTANCE the page did (node caches by resolved URL). Importing the file by any other
path would give a second recorder with an empty trace, and every containment assertion would pass
while measuring nothing. Also added `declared` to the context (the whole fixture.json) for blocks
only one fixture has.
Shapes are asserted strictly (that is what the `#[js_extern]` declarations decide); series values
by containment and marked PREDICTED. **Four updates for five ticks** - the missing one is the
assertion: a malformed frame must not reach the chart at all, because a library handed a partly
decoded series draws a WRONG chart rather than failing.

### Two real bugs found by running it, both fixed, neither mine

1. **One unbuilt chunk aborted the ENTIRE parity run with a raw stack trace.** dom.rs already has
   the dashboard arm (framework, in flight) but `out/chunks/dashboard.js` does not exist, so
   `locateOutDirArtifact` threw inside `expandParameterized` and took counter, nested AND search
   down with it. This happens through nobody's mistake - a route lands before its chunk is built.
   Fixed in `lib/delivery.mjs`: an unbuilt artifact is now recorded as `unbuilt` and REPORTED, the
   other arms are still delivered, and delivery returns it so nothing consumes the map believing it
   is the whole delivery. Same argument `missed` makes one level down.
2. **`/demo/chart-lib.js` was served with NO budget.** Caught by the coverage check that exists for
   exactly this ("a module appearing in the delivery with no budget at all"). Added to
   `page:dashboard`'s artifacts - the PAGE fetches it, so a browser pays for it; deliberately NOT
   in `island:dashboard`, which is what a browser fetches to run the island's compiled half. The
   chart is a peer of the island, not part of it.

### Budgets: bands recorded, and two subjects are OVER (not mine to rebaseline)

Bands come from `policy`: the greater of 5% of the gzip baseline and a 128-byte floor. Both
dashboard subjects carry their pending reason and a `$bandWhenMeasured` note.

`node budgets.mjs` reports **island-rt and page:search OVER**: island-rt.mjs grew (3100 gzip) as
the framework agent added the dashboard's stream/chart host code, and page:search carries it
(15236 vs a 13482 baseline, past the +675 band). This is real, expected, in-flight growth caused by
another agent mid-wave. I did NOT rebaseline: budgets.json's own rule is "read the diff before
keeping it", and a contract agent silently re-baselining another agent's in-flight growth is how a
budget stops meaning anything. FLAGGED for whoever closes the wave.

### Final verification (all run at the end, on the tree as it stands)

- `contract/parity/test.mjs`: **174/174** (was 160 entering the wave; +13 .d.ts, +1 coverage)
- `contract/parity/run.mjs`: **exit 0** - "parity holds for every fixture, and the negative controls
  fail as they must", with the dashboard reporting PENDING and its controls skipped
- `contract/parity/dts.mjs --check`: PENDING, exit 0
- `node harness/run-all.mjs --check`: **"no drift: committed fixtures match regenerated output
  exactly", all steps green** - DRIFT ZERO
- `node budgets.mjs`: 2 subjects over, both attributed above

## DASHBOARD SERVING received — fixture FLIPPED LIVE. 62/62.

Coordinator banked it. Rewrote both files against MEASURED ground truth from
`build/logs/wave5-framework-report.md:363-440`; nothing below is predicted.

### The scaffold was wrong in four ways, and one of them inverted the central assertion

1. **KEYED -> UNKEYED.** The headline. `push_keyed` returns the node a key contributed last time
   and DISCARDS the freshly built row, text included, so a keyed version re-orders correctly and
   shows its FIRST prices for ever (measured: a row read 4310 after a tick set it to 4460).
   My scaffold asserted keyed node identity across reorder. **That assertion would PASS for the
   broken version and FAIL for the correct one.** So section 4 of interact.mjs now asserts the
   OPPOSITE - the rows are rebuilt, the list ELEMENT is not - and names the reason in the check
   text so a silent return to keying is caught rather than welcomed.
2. **The list is SERVER-RENDERED.** Predicted an empty list and one key; it renders five rows and
   `expected.keys` is `["0".."5"]`. First fixture where hydration ADOPTS repeated rows.
3. **A frame carries ONE row, not the list**: `{slot,symbol,price,delta}`, and the RANKING is the
   host's (`dash_sink`: set `moved[slot]`, re-sort by |moved| desc with ties by slot asc, then
   write the price). So I DERIVED the five expected row-sets from that rule rather than observing
   them. All four derived orderings passed first run, including the deliberate |150| tie in tick 3.
4. Selectors: `.chart` -> `.dash-chart`, a row is three spans. Cost nothing - every selector was
   already read out of fixture.json, which is why that design was worth having.

### What the fixture measures (62 checks, all green first run of the final shape)

Hydration adopting five server rows with zero mutations; exactly one EventSource opened on
`/demo/ticks` with a listener on the NAMED `tick` event (a listener on `message` alone would never
fire); four good ticks each re-ranking, re-pricing AND re-classing (`trend()` is a conditional
attribute - asserted separately, because an attribute that stopped updating leaves the text
correct); the inverted identity check; the malformed frame; recovery; and the foreign-call trace.

**A `setup()` hook had to be added to run.mjs.** The island constructs its `EventSource` inside the
list's FIRST EFFECT RUN, which IS hydration - and jsdom has no `EventSource` at all (verified).
`search` gets away with stubbing `fetch` from `interact` only because its call is behind a 150ms
debounce. Without a pre-hydration hook the island throws inside the hydrate bracket and every later
assertion measures the wreckage. Optional and awaited, so no other fixture pays for it.

**The stub catches a throwing listener, because that is what a browser does.** `dispatchEvent` does
not propagate a listener's exception to the dispatcher. A stub that let it escape would fail in the
harness before reaching anything the island decides, making the malformed-frame assertion vacuous.
Measured: `SyntaxError: Unexpected end of JSON input`, subscription stays open, list unchanged,
next frame renders. The check asserts the frame was REJECTED rather than partly decoded - a partly
decoded tick writes a WRONG price, which is worse than no price.

**Chart trace: 6 records for 5 frames** - one `new` (via "named") plus five `update`s (one at
hydration, four good ticks). The MISSING sixth update is the assertion: a malformed frame must not
reach the chart, because a library handed a partly decoded series draws a wrong chart rather than
failing. Series asserted by value, keyed `acme..echo`, which is also the assertion that a
`#[repr(C)]` struct crosses as an object keyed by its RUST FIELD NAMES.

### Two harness fixes this forced, both real

1. **The tree oracle could not express a server-rendered loop.** The `for` body is hoisted to its
   own template, so the parent declares an EMPTY `<ul>` while the server sends five `<li>`; there
   are no `$`/`/` markers because element content hydrates by KEY, not by marker. `lib/tree.mjs`
   now reads an extra element child carrying `data-hk` as hole content. **The `data-hk` is what
   keeps it honest**: the `tree` negative control appends an UNKEYED `<span>`, so it still trips -
   verified, both dashboard controls still fail as they must.
2. `/demo/chunks/dashboard.js.map` was unbudgeted -> `$notBudgeted` beside the other three.

### Budgets updated, every subject inside its band

| subject | gzip | note |
|---|---|---|
| `island:dashboard` | 2630 | de-pended and measured. Its `/demo/chunks/dashboard.js` part reads **2403**, byte-for-byte the framework agent's own number; 2630 is that chunk PLUS shared, gzipped together, which is this harness's documented method (raw 9385 = their 9047 + 338 exactly). No discrepancy. |
| `page:dashboard` | 21077 | de-pended and measured. The most expensive page the demo serves. |
| `island-rt` | 3570 (was 1346) | REBASELINED with the cause named: the dashboard's host code (`dash_sink`, `dash_slot`, the ranking) plus three shim helpers the first arithmetic-on-a-host-value island needed. |
| `page:search` | 15740 (was 13482) | REBASELINED - it carries island-rt. |
| `runtime` | 9140 | **band CHECKED as the coordinator asked: 9140 is inside [8650, 9562] of the 9106 baseline, so `runWithOwner` did NOT need a rebaseline.** Left alone deliberately. |

### The .d.ts harness went from PENDING to a REAL tsc pass

The backend's B3 LANDED and their report banked a wave-6 ask: "if you can pin `tsc` under
`contract/vendor`, `build/dtstest/01_shapes.d.ts` is ready". **I had already pinned it, so this
closed in wave 5 instead.** Taught `dts.mjs` to read `build/dtstest/` as a peer of demo-app's
OUT_DIR. Result: **6/6, with `tsc --noEmit` passing over the two real emissions plus two scaffolded
consumers.** So the `.d.ts` goldens are NOT structural-only, which both the backend's report and my
own earlier draft assumed.

**The tsc layer immediately earned itself by catching a bug in MY OWN scaffold**: it bound members
of nested `declare module "chart.js" { .. }` blocks as if they were exports of the file, and tsc
correctly said no such export exists. Nested members are now marked with their module and excluded,
and only `export`ed declarations are bound. A structural golden diff would never have seen this.

### Final verification, everything, on the tree as it stands

- `parity/test.mjs` **176/176** (160 entering the wave)
- `parity/run.mjs` **exit 0**, 4 fixtures, "parity holds for every fixture, and the negative
  controls fail as they must"; dashboard **62/62**, both its controls tripping
- `parity/budgets.mjs` **every subject is inside its band**
- `parity/dts.mjs --check` **6/6**
- `parity/check.sh --check` end to end: green
- `harness/run-all.mjs --check` **"no drift: committed fixtures match regenerated output exactly",
  all steps green** - DRIFT ZERO
