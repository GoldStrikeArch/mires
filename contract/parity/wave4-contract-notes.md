# Wave-4 contract + platform slice — running notes

Owner: contract agent. Scope: `contract/` (all) + `mires-build/` (ONE targeted fix, item 2).
Read-only elsewhere. May run the already-built demo-app binary for captures (capture
guard: no dev server up). `cargo test -p mires-build` allowed. No git commit.
Started: 2026-07-30.

## Items
1. Fake chart lib + js_extern vectors (M, FIRST — backend polls for it). Bank
   "CHART-FIXTURE READY". Plus a JS-EXTERN.md contract page.
2. mires-build staleness fix (S) — dylib mtime/stamp vs sources, Unmet/Fix, tests.
3. Search parity fixture goes live (M) — poll `build/logs/wave4-framework-report.md`
   for "SEARCH ISLAND SERVING".
4. Per-chunk budgets (S) — once the framework's chunked demo lands.
5. Drift + docs — `run-all.mjs --check` zero, READMEs current.

## Status
- [x] 1 chart fixture — **CHART-FIXTURE READY** (see below)
- [x] 2 mires-build staleness — LANDED, and it caught a LIVE stale dylib on first run
- [x] 3 search live — **LIVE. 61/61, every scaffolded claim held.**
- [x] 4 per-chunk budgets — recorded, banded, green; four chunk numbers match the framework agent's independent measurement exactly.
- [x] 5 drift + docs — green, READMEs current

## Log
- (init) Bank created as first action. Wave-3 notes read: harness map, budgets design,
  the `$ifTheIslandDivergesFromThis` precedent, the "validate the rule then apply it"
  wire lesson. Next: read wave4 backend/framework reports, then build item 1.

### Orientation

Peer banks at the time of reading: `wave4-backend-report.md` STARTING, all three items
NOT STARTED (item 2 is `#[js_extern]`, which is what waits on me);
`wave4-framework-report.md` STARTING, all four NOT STARTED, no "SEARCH ISLAND SERVING"
and no chunked demo. So my items 3 and 4 both block on them and item 1 goes first,
which is also the order the plan asks for.

### Item 1 — CHART-FIXTURE READY

**FOR THE BACKEND AGENT. Everything below is on disk and green.**

- `contract/fixtures/js-extern/chart-lib.mjs` — the fake library, recorder style.
- `contract/fixtures/js-extern/drivers.mjs` — the reference emission per vector, plus
  the shared `env` inputs and the hand-written assertions.
- `contract/fixtures/js-extern/vectors.json` — 17 recorded vectors.
- `contract/harness/check-js-extern.mjs` — the runner. Exports `compareTrace(actual,
  expected)` and `vector(id)` for your own runner. Wired into `run-all.mjs` as step 14.
- `contract/JS-EXTERN.md` — the contract page. **Its "The attribute encoding" section
  is a STUB addressed to you**, with the six questions the vectors raise, in the order
  they raise them. Fill it there or answer in `CONTRACT.md` and link.

THE SURFACE (modelled on how Chart.js is actually used, derived from nothing):

| binding | shape |
| --- | --- |
| `Chart` / `default` | constructor, TWO DISTINCT function objects |
| `Chart.register(plugin)` / `Chart.version` | static call / static get |
| `chart.update(data)` | send, 1 arg |
| `chart.resize(w, h)` | send, optional trailing arg, branches on `arguments.length` |
| `chart.destroy()` | send, 0 args |
| `chart.getPoint(i)` | send returning an object or `null` |
| `chart.data` -> `.datasets[i]` / `.length` | get, then a scoped get, then index |
| `chart.title` | get + set |

VECTORS: 17 across 6 shapes (new 3, send 6, get 3, set 1, index 3, call 1). Required
this wave: new/send/get/index. `set` and `call` pinned anyway, marked `required: false`.

FOUR DESIGN DECISIONS, each because it makes something observable that otherwise is not:

1. **The constructor is exported twice as two DISTINCT function objects**, each
   tagging its records `via: "named" | "default"`. Exporting one object twice would
   make the import-binding choice unobservable and the trace would agree with either
   answer, which is the same failure mode as a test that passes for the wrong reason.
2. **`argc` is recorded separately from the argument list.** `resize(800)` and
   `resize(800, undefined)` are two vectors with different traces. That is Melange's
   trailing-`undefined` stripping turned into a measurement instead of a footnote, and
   it is where a lowering that pads optional arguments shows up.
3. **The two out-of-range reads DISAGREE on purpose.** `getPoint(99)` answers `null`;
   `datasets[9]` answers `undefined`. Both are real library behaviour, they are on the
   same object graph, and a return wrapper that tests only one passes every other
   vector on the page. Asserted globally, not just per vector.
4. **Symbol keys are never recorded.** The indexed collection is the one Proxy in the
   file and a Proxy sees every symbol probe anything makes (`Symbol.toPrimitive`,
   node's inspector, an `await` looking for `then`). A trace that moves because
   something logged the object is worthless. Everything with a known property set uses
   defined getters instead, so there is no trap to fire.

WHY THE VECTORS ARE NOT CIRCULAR, proved rather than argued. A recording cannot
disagree with the code that produced it, so three things run on every check:
- each driver's hand-written assertions about the shape it demonstrates (69);
- every required shape has at least one vector, and the nullable wrapper has both arms;
- **no vector's assertions hold for any OTHER vector's recording** — 272 (driver,
  foreign recording) pairs, zero confusable. Measured that this bites: a deliberately
  weak assertion ("the trace is an array") holds for all 17 and would be flagged.
Also recorded: the driver's own source text is taken from `Function.prototype
.toString()` rather than transcribed beside it, so the `js` field in the fixture IS the
text that ran and cannot describe a different call. Consequence, stated so it is not
mistaken for a bug: reformatting `drivers.mjs` moves the fixture and `run-all --check`
reports drift. That is correct; the reference emission is a contract artifact.

HOW TO USE IT: compile a module whose `#[js_extern]` declarations describe
`chart-lib.mjs`, resolve its import specifier to that file the way `run-trace.mjs`
resolves `r-dom` to the recording stub (do NOT rewrite the compiled text — same
argument as theirs), drive it to perform the operation named by a vector `id`, and
compare `__trace()` with that vector's `trace` via `compareTrace`. Your emitted JS does
not have to match `js`; it has to be indistinguishable at the trace. `compareTrace`
compares position by position and NOT by `compare-trace.mjs`'s LCS alignment,
deliberately: that comparator exists because two emitters can order independent runtime
calls differently and both be correct, whereas here the trace is the operations one
expression performed on one object and reordering it changes what happened.

STATE: `run-all.mjs --check` 14 steps green, **no drift**. The new step runs 69
assertions under `--record` and 70 standalone (the extra one compares committed bytes).


### Item 2 — mires-build staleness. LANDED, and it is not hypothetical.

**FIRST RESULT, before any argument about the design: the checkout is stale RIGHT
NOW.** `target/release/libview_dom_macro.dylib` is dated 14:58 and five inputs are
newer than it:

    view-abi/src/lib.rs
    view-dom/src/dom_writer.rs
    view-dom/src/lower/control_flow.rs
    view-dom/src/lower/node.rs
    view-dom/src/lower/signal_declaration.rs

That is the wave-3 defect happening again, live, while the wave-4 backend agent edits
`view-dom/src/lower`. Any client expansion run against this checkout today expands
through the macro as it was at 14:58. **I did not rebuild it**: `view-dom/` is theirs
this wave and the dylib is shared state. Reported instead; the command is
`cd <spike> && cargo build --release -p view-dom-macro`.

THE MECHANISM: modification times, not a content stamp. The argument, because the
stamp was the other candidate and the sysroot next door uses one:

- A stamp only works when whatever performs the rebuild writes it. That holds for the
  sysroot because `build_sysroot.sh` is the only way to build one and it writes
  `.js-args`. It does NOT hold here: the command mires-build itself prints for this step
  is a bare `cargo build --release -p view-dom-macro`, which anyone can run by hand
  and which will never write a stamp. A stamp that a hand-run rebuild left behind
  would assert freshness that is not there, and **a stale stamp is worse than no check
  at all** -- it converts "unknown" into "verified".
- Times need no cooperation from the rebuilder, and they are what cargo itself decides
  freshness by, so the check agrees with the tool that actually does the rebuild.
- The failure direction is the safe one: touching a file without changing it reports
  stale, and the cost is one cargo build that finds nothing to do. The opposite error
  is compiling silently through a macro that no longer matches its source.

WHAT CHANGED, all in `mires-build/`:
- `Toolchain::view_sources()` -> `[view-abi, view-dom, view-dom-macro]`, public,
  because both the precondition and the rerun list need the same answer.
- `unmet_view_macros` now has two reasons, told apart in the message: "are not built
  at <path>" and "are older than <file>, so a `view!` would expand through the macro
  as it was built rather than as it is written". Same `Fix` either way. The message
  names the newest file, so the reader knows what they changed.
- `collect_sources` -> `collect_inputs`, now also collecting `Cargo.toml` (a feature or
  dependency change alters what the macro does) and skipping `target/` and dotted
  directories (nothing under `target` is an input, and walking it on a warm checkout
  is a few hundred thousand files instead of a few dozen).
- **The rerun list was the other half of the same defect and it was broken too.**
  `Rerun::views` watched `view-abi/src/lib.rs` and the dylib and NOTHING under
  `view-dom/`. So even with the precondition fixed, editing the lowering would leave
  cargo with no reason to rerun the build script, and the precondition would never get
  a chance to fire. Now the three crate directories and their inputs are watched, plus
  each directory itself, which is what covers a file being added or removed. Counted as
  part of the one fix because it IS the fix: a check that never runs is not a check.

Ties in `newest_input` break by path, deliberately: a fresh checkout gives a whole
directory one timestamp, and a message naming a different file on each run reads as a
different problem each run.

TESTS: 71 -> 82, all green (`cargo test -p mires-build`). New ones cover current /
stale / equal-instant (equal is CURRENT, so a rebuild that lands in the same second is
not a false alarm), an edited manifest, missing-vs-stale told apart, a checkout with no
view sources at all not being called stale, the tie-break, the `target/`+dotted skips,
the manifest collection, and that `Rerun::views` reaches a `view-dom` source.

DEVIATION: `cargo clippy -p mires-build` cannot run here -- clippy is not installed for
the pinned `nightly-2026-07-27`. Tests and `cargo test` are green; lints unverified.
KNOWN GAP, recorded rather than fixed: the macros also depend on
`crates/topcoat-view/grammar`, which is OUTSIDE the spike checkout `Toolchain` is
rooted at. An edit there still goes unnoticed. Fixing it means reaching out of the
spike root, which is a bigger decision than this fix, and it is not the case that bit
anyone in wave 3.

### Items 3 + 4 — BLOCKED, behind two agents. What is prepared meanwhile.

The chain: backend is mid-B2 (`#[js_extern]`, using my item-1 fixture) and has written no
`DYLIB STABLE` line for wave 4; the framework agent will not build demo-app until they do
(their own stated order). So `SEARCH ISLAND SERVING` and the per-chunk measurements are
both downstream of a rebuild that has not happened. Binary and macro dylib are both 14:58.

**Their item-status table was STALE AGAIN** -- the wave-3 F3 lesson repeating. The report
says items 1-4 "not started"; the tree says items 2 and 3 are landed in SOURCE:
`dom.rs` now serves `/demo/chunks/{chunk}` with per-island chunks plus a shared chunk,
`build.rs` calls `Chunking::new().island("counter").island("nested")`, and the LOADER has
a `reportLoss` dev warning. So I read the tree, not the table, again.

Worth recording for them: `reportLoss` is a direct answer to my wave-3 finding that a
hydration key miss is silent -- it takes `[...mount.querySelectorAll("[data-hk]")]`
BEFORE hydrating and warns if one is no longer contained afterwards. That is the right
observable (node identity, not markup), and it is the thing my `IDENTITY` check measures
from the outside.

THREE THINGS THE CHUNKED DOM.RS BREAKS IN MY HARNESS, all found by reading and all fixed:

**B1. `IMPORT_MAP` is no longer a literal.** It is now
`include_str!(concat!(env!("OUT_DIR"), "/islands.importmap.json"))`, because the build is
what knows how many chunks there are. `stringConst` knew two forms and threw on this one
with a message about neither being a raw string nor a `concat!`, which is true and
useless. It now reads all four forms (raw, `concat!`, OUT_DIR artifact, committed file),
and a missing artifact names the CONSTANT that went looking for it, since
"no <hash>/out/x" alone does not say which constant is unbuildable.

**B2. `/demo/chunks/{chunk}` is ONE route serving SIX files** (three chunks, three source
maps), each a match arm. The old reader took the first `include_str!` in a route body, so
this route would have read as a single module whose bytes were `chunks/counter.js` --
silently, and the other five would have been invisible to the coverage check that exists
precisely to notice new modules. A parameterized route now expands to one entry per arm,
keyed by the URL a browser actually requests (`/demo/chunks/counter.js`), which is what
lets budget subjects name a chunk directly and lets the source maps go to `$notBudgeted`.
A parameterized route with no arms is reported unreadable rather than contributing
nothing.

**B3. The loader's render id moved** from `${mount.dataset.tk}.` to a local `${key}.`.
The old check pinned the whole expression, so a rename would have read as a hydration
bug. Split into the two claims that were being made at once: the id carries the
separator (a regex over the shape), and the id is the mount's instance id
(`mount.dataset.tk` appears). Both are real, and neither pins the spelling.

I did NOT touch budgets.json's numbers. Every baseline there has to be MEASURED and the
build does not exist; `chunkingState` will fire correctly the moment it does, because
`/demo/islands.js` is no longer served at all and that is its first branch --
"budgets.json says chunking has not landed and names /demo/islands.js as the one island
module, but dom.rs does not serve it". The prepared steps in `chunking.$landed` still
apply, plus one they did not anticipate: the three `chunks/*.js.map` URLs are new and
belong in `$notBudgeted` beside the existing map entry.

I also did NOT build demo-app. It is not mine, the framework agent is mid-edit on it, and
a build now would compile through the stale macro dylib item 2 just found.

**test.mjs no longer dies when demo-app is unbuilt.** It ran 138 checks and then threw on
`delivery()`, so a missing build blinded every unit test in the file -- including the
ones that need nothing. The delivery-reading group is now skipped LOUDLY with the reason,
and the skip is guarded: only "build demo-app first" is skippable, and a dom.rs this
harness cannot PARSE still fails. Those are different findings and conflating them is how
a harness goes quiet exactly when someone is changing the thing it watches.

### Item 5 — drift + docs

- `harness/run-all.mjs --check`: 14 steps green, **no drift**.
- `cargo test -p mires-build`: 82 + 5 doc-tests green.
- `parity/test.mjs`: 142 checks green (129 -> 138 -> 142 across the wave), delivery group
  skipped with its reason. `check.sh` end to end still needs the build.
- READMEs: `contract/README.md` gains JS-EXTERN.md in the intro, the layout table and the
  fixture table, says fourteen steps, and rewrites the "one fixture cannot work that way"
  paragraph into the two that cannot and why they are gated from opposite sides.
  `parity/README.md` gains the two ways dom.rs widened and the parse-vs-unbuilt split.
  Stale check counts corrected in both (129 -> 142).

### Item 1, the outcome: the fixture changed the backend's descriptor model

Recorded because it is the only evidence that a contract artifact was worth writing
BEFORE the implementation rather than after. From `wave4-backend-report.md:245-263`,
their own section heading: "The chart fixture landed mid-step and changed the model".

Three of my required vectors did not fit the one-member descriptor they were building:
`get-static` (rooted at the module binding, not at an argument), `get-scoped`
(`instance.data.datasets`, two `get` ops), and `index-in-range` (a walk then an index,
three ops). Their DEVIATION 3 is the consequence: a descriptor's name is a dotted PATH
and the shape only decides where the path is rooted. They note the collapse works
"because the fake records every step of a walk" -- which is the design decision from my
side, made for a different stated reason (a walk is several real property reads the
library sees, so a lowering cannot collapse it), and it turned out to be the thing that
let one declaration cover a walk.

That is the fixture doing the job it was for: it was cheaper for them to meet three
vectors than to discover the same three cases against a real chart library, where a walk
and a rooted path both just work and neither would have argued for anything.

OPEN, for the next contract check: `JS-EXTERN.md`'s encoding section is still the stub I
left. Their report does not mention the page. Their encoding now EXISTS
(`rcgjs.ext.<version>...`, `js-extern-macro/`), so the six questions the stub asks are
answerable and should be answered there or in `CONTRACT.md` with a link. Their
DEVIATION 3 rooting rule is the answer to questions 1 and 2 already.

## UNBLOCKED — items 3 and 4 completed

The framework agent banked `SEARCH ISLAND SERVING` and built the chunked demo. Both
items are done. Everything below was measured, not predicted.

### Item 3 — THE SEARCH FIXTURE IS LIVE. 61/61.

**The headline: every claim the scaffold made two waves ago held on the first run
against the real island, and the island's SOURCE SHAPE had changed completely in the
meantime.** `keys: ["0"]`, `components: 0`, `templates: 2`, `islandTemplate: 0`,
`hydrateMutations: 0`, `seeds: []` (arity 0, the case nothing else pinned), the
debounce window, the whole wire, both result sets, and the error path -- exact.

The shape it was written against (`signal results = Vec<String>`, `for item in
$(results.get())`) is NOT what shipped. What shipped is `query` plus `replies: f64`,
an ANSWER COUNTER, with `for item in hits(query.get(), replies)` and the reply left as
parsed JSON on the page side, read one row at a time -- forced by the island crate
being `#![no_std]` with no heap and by a `$( )` not being able to spell a host call.
The fetch, the 150ms timer and the JSON parse all live in `demo-app/src/island-rt.mjs`
rather than in compiled Rust.

None of that moved an assertion, and the reason is worth keeping: **the fixture was
written against the DOM and the bytes on the socket, not against how the island holds
its state.** A fixture written against the source shape would have had to be rewritten
and would have proved nothing. Recorded in the fixture's new `$landed.divergences`
rather than quietly edited away.

FOUR THINGS I HAD TO FIX, three of them defects in MY harness that the chunked, lazy,
async delivery exposed. All three were passing before by being unable to fail.

**D1. `stage()` could not link the graph at all.** The loader holds
`const CHUNK = "topcoat-island/"` and imports `CHUNK + name`, so the specifier does not
exist as text and no rewrite can find it -- which is what an import map is FOR. Fixed
by giving node its own import map: the chunks are staged inside
`node_modules/topcoat-island/` with an `exports` entry per island, so the specifier
really resolves, at runtime, with nothing rewritten. That put the chunks in a
subdirectory, so specifier rewriting is now per-importer (`path.relative` from each
file's own location) instead of assuming one flat directory. Every module is still
staged EXACTLY ONCE, which is load-bearing: a chunk reachable at two paths is two
module instances with two `sharedConfig`s, i.e. two hydration cursors.

**D2. `await import(loader)` was no longer the same as hydrating.** The loader queues
each island on a promise chain (hydration is a single global cursor, so islands take it
one at a time) and returns with the work pending. The run drained its MutationObserver
immediately and measured an untouched document -- **and `hydrateMutations` is 0, so it
would have PASSED by being early.** Now waits on the loader's own `data-tl-hydrated`
signal rather than on a timer, with a new check that the island was hydrated at all,
because a lazy loader legitimately might not.

**D3. "the loader logged nothing" was VACUOUS.** It read the page's VirtualConsole, but
the loader and every chunk are node modules whose `console` is node's, so nothing they
wrote could ever reach the list being asserted on. `lib/dom.mjs` now captures both
realms into one list. It failed immediately and correctly -- with D1's real error,
which is how D1 was found.

**D4. The bootstrap is PER PAGE now.** dom.rs's `BOOTSTRAP` constant is only the
default page's `["click"]`; `/island/search` serves `["input", "click"]`. Checking
every page against the constant would hold for the two that take the default and be
wrong about the one that does not -- which is the only page where it matters. Now
compared against the captured page for the fixture's OWN declared event list.

Also: the `data-tl-hydrated` attribute the loader sets is a real mutation and is NOT
hydration, so it is excluded from the headline count by name and asserted separately.
Folding it in by declaring `hydrateMutations: 1` would have destroyed the distinction
the number exists to make.

**A FINDING, promoted from a note to a check.** PROCEDURES.md records a defect: the
existing browser client throws `Procedure call failed: ${status} ${statusText}` and
DISCARDS the body, so the server's argument index never reaches the developer. My
wave-3 note said "if the message carries no part of the body this client inherited it".
MEASURED: it does not inherit it. The logged message is
`topcoat: search failed: 400 bad request: invalid JSON value: invalid type: integer
`7`, expected a string (at `[0]`)` -- the whole body, argument index included. Now
asserted, against the spec's `$pathRule`-derived suffix rather than serde's wording.

**The search `dead()` probe was asserting something no longer true**, and only ever
passed because it had never run. It was written for a loader where a key miss left the
island inert and NO REQUEST was made. Measured now: the island **still searches, still
renders, and is right** -- a broken key costs hydration and nothing else, with no
throw, no console output and identical markup. Rewritten to assert the working symptom.
Note the loader's new `reportLoss` would report exactly this, but it is written only
under `topcoat dev` and these captures carry no `data-tl-dev`, so in a production page
it stays silent and this harness is still the only thing that sees it.

### Item 4 — PER-CHUNK BUDGETS. Recorded, banded, green.

| subject | gzip | raw | artifacts |
| --- | --- | --- | --- |
| island:counter | 1072 | 3159 | chunks/counter.js + shared.js |
| island:nested | 1289 | 3882 | chunks/nested.js + shared.js |
| island:search | 1547 | 3942 | chunks/search.js + shared.js |
| chunk:shared | 227 | 338 | chunks/shared.js |
| runtime | 9106 | 23971 | topcoat-dom.js (unmoved, as it should be) |
| loader | 1483 | 3320 | island-loader.js (was 307/477) |
| island-rt | 1346 | 2729 | island-rt.js (new) |
| page:island | 11661 | 30450 | runtime + loader + counter + shared |
| page:search | 13482 | 33962 | + island-rt + search instead of counter |

Per-chunk alone: counter 845, nested 1062, search 1320, shared 227.

**CROSS-VALIDATION, unplanned and the most reassuring number here: all four per-chunk
gzip figures match the framework agent's independently measured `smoke/budgets.json`
EXACTLY** (845 / 1062 / 1320 / 227). Two suites, two authors, same bytes.

**One methodological difference, named so nobody calls it a discrepancy.** Their
`page:counter` is 919; my `island:counter` is 1072 over the same two files. Theirs
gzips the concatenation; mine sums the artifacts compressed separately, which is
argued in `lib/budget.mjs` and is what a browser actually pays -- it fetches and
inflates two responses and gets no cross-file redundancy. Both are right about what
they measure; only mine is the delivery cost.

`containsIslands` is now honest for the first time: each island subject reads exactly
its own island, where before chunking all three subjects were one number wearing three
names. That independence is the concrete thing C1 bought and it is visible in the diff.

The loader's 307 -> 1483 gzip is the largest move and it is explained, not absorbed:
lazy hydration with an IntersectionObserver, the dynamic chunk import, the hydration
queue and the dev-mode `reportLoss` all landed in it this wave. Raw went 477 -> 3320.

COVERAGE: all 12 served modules are named. Nine subjects plus five in `$notBudgeted`
(four chunk source maps, and `/demo/chunks.importmap.json` -- served for harnesses,
INLINED in the page, so no browser ever fetches it).

### A silent under-read I found in my own reader, worth its own note

`/demo/chunks/{chunk}` is one route with eight match arms. rustfmt had wrapped the
`counter.js` arm in braces because the line was too long, my `ARM` regex required
`=> Module::`, and so **the counter chunk was dropped from the route table silently** --
eleven of twelve modules read, no error. Half-reading a route is worse than failing to
read it: the modules that did parse look like the whole delivery, and a served module
nothing knows about is invisible to the coverage check whose entire job is to notice a
new module. Fixed both ways: the arm regex accepts a brace, and `expandParameterized`
now returns the arms it could NOT read so `delivery()` reports them by name. Pinned by
tests, including the braced arm and the named-miss.

### FINAL VERIFICATION

- `parity/check.sh --check`: **green end to end.** 161 unit checks; captures clean, no
  drift; counter 44/44, nested 49/49, search 61/61; every negative control fails on its
  named check (`deepKey` skipped-with-reason for the two component-free fixtures);
  every budget subject inside its band.
- `harness/run-all.mjs --check`: 14 steps green, **no drift**.
- `cargo test -p mires-build`: 82 + 5 doc-tests green.
- READMEs current: counts 129 -> 161, all three fixtures live, the compiled island is
  now "the island's own chunk, resolved through the page's import map".

Footprint: `contract/` and the one agreed `mires-build/` fix. No cargo build of demo-app,
no git commit. Capture guard honoured: nothing was answering on 127.0.0.1:3000.
