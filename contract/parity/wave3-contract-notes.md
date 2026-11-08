# Wave-3 contract slice — running notes

Owner: contract agent. Scope: `contract/` only (read-only elsewhere). No cargo, no git commit.
Started: 2026-07-30.

## Items
1. Flip nested parity fixture live (poll `build/logs/wave3-framework-report.md` for "NESTED ISLAND SERVING")
2. Search parity fixture (`/island/search`) — fixture.json + interact.mjs, jsdom full loop
3. A6 budget wiring — per-fixture gzip chunk-size budgets w/ tolerance bands
4. Dom-stub gaps — verify parity harness observes nested hydration keys + clone identity

## Status (FINAL)
- [x] 1 nested flip — LIVE, 48/48, every declared key HELD. 3 findings, all attributed.
- [x] 2 search fixture — landed as a validated scaffold + reconciled to the real
      /demo/search wire. Island's CLIENT half blocked on two things the framework
      agent does not own (opaque `view_abi::Event`; non-reactive `for`), so it stays
      `pending` by their deliberate choice. Its whole WIRE is checked every run.
- [x] 3 budgets — baselines recorded, banded, green; composition-aware. Per-CHUNK
      budgets still wait on backend C1 (not landed; no DYLIB STABLE / C1 marker).
- [x] 4 stub gaps — gap A already covered (recorded why); gap B closed with `IDENTITY`.

FINAL VERIFICATION: `parity/check.sh --check` green (138 unit / counter 43 / nested 48 /
search 7, four control runs each failing on their named check, `deepKey` skipped-with-
reason for counter, all budget subjects inside their bands);
`harness/run-all.mjs --check` 13 steps green, **no drift**.
Footprint: `contract/` only. No cargo. No git commit.

## Log
- (init) Created bank. Next: read wave2-contract-notes.md, wave3 framework/backend reports.

### Orientation (done)

Peer banks at time of reading:
- `build/logs/wave3-framework-report.md` — STARTING, all 4 items NOT STARTED. No
  "NESTED ISLAND SERVING" marker. So item 1 BLOCKS on them; item 2 blocks on
  `/island/search`.
- `build/logs/wave3-backend-report.md` — Step 0 orientation only. C1 per-island
  chunking not landed, so item 3's per-chunk budgets block; the per-FIXTURE budgets
  (counter, nested, search, runtime artifact) do NOT block and go first.

Harness map (read, not guessed):
- `parity/run.mjs` — `runFixture(name, {control, verbose})`; `pendingFixture()` for
  the scaffold path; named checks `CLAIMED`/`TREE`/`SCHEME`; `CONTROLS = {key, tree}`.
  Single-island assumption at run.mjs:339 `checks.is("the compiled module declares
  one template per island root", templates.length, 1)` — THE line the nested fixture
  must drop (nested declares `templates: 3`).
- `parity/lib/delivery.mjs` — `delivery()` parses demo-app/src/dom.rs for IMPORT_MAP /
  BOOTSTRAP / LOADER + the `#[route(GET "..")]` table; `locateOutDirArtifact` finds
  OUT_DIR build artifacts (this is where a chunk-size budget must read its bytes);
  `captureFreshness()`; `moduleTemplates()`.
- `parity/capture-ssr.mjs` — owns `fixture()`/`fixtureNames()`; the port-squat guard
  is at the top of `withServer` (refuses if anything answers 127.0.0.1:3000 first).
- `parity/lib/check.mjs` — `Checks` with `.is/.ok/.note`, `.failures`, `.report()`.
- `parity/lib/keys.mjs` — nested-key library (validated against keys-nested.json).

Plan order (dependency-driven): item 4 verdict (pure read) -> item 3 per-fixture
budgets (no peer dep) -> poll -> item 1 nested flip -> item 2 search fixture ->
item 3 per-chunk budgets once C1 lands -> final `run-all.mjs --check` + `check.sh`.

### Item 4 — the two dom-stub gaps, located and judged

The wave-2 backend report's two flagged limits, found and read rather than taken
from the summary:

- GAP A, nested hydration keys (`wave2-backend-report.md:379-390`):
  `contract/harness/trace.mjs`'s `getNextElement` numbers nodes from a FLAT
  `hydrationCounter` (trace.mjs:373-375) and the stub implements no `sharedConfig`
  context at all, so nested keys are invisible to it whatever the emitter emits.
  Explicitly delegated: "the parity harness ... **Flagged for them: this is where
  client-side nested keys get checked.**"
- GAP B, clone identity (`wave2-backend-report.md:573-590`):
  `trace.mjs`'s `nodeName` (:159-171) labels a node by the TEMPLATE it was cloned
  from, so every clone of one template reads as `tmpl#1:root` and three reused
  rows are indistinguishable from three fresh ones. Backend went around it with
  `scripts/keyed-identity-check.mjs` (10/10) rather than change a contract-owned
  file. Same class as gap A.

VERDICT (before adding anything):

- GAP A is genuinely covered, and structurally so. The parity harness runs the
  REAL pinned runtime with a real `sharedConfig`, not the stub, and reads real
  `data-hk` attributes off a jsdom document. `run.mjs:318-323` runs
  `nestedKeyParity(mount, {renderId, expected, components})` under the named check
  `SCHEME`, and `fixtures/nested/interact.mjs:49-64` additionally pins the card's
  and the badge's exact `data-hk` per boundary. Nothing to add.
- GAP B is covered only in the aggregate, and I judged that NOT good enough for
  the nested fixture, which is the fixture the gap was flagged for. What exists:
  `CLAIMED` (run.mjs:394-395) proves each keyed element object is in
  `_$HY.completed`, which a clone could not be; the root-identity check
  (run.mjs:396-400) covers the ROOT only; "every node the server sent is still the
  node that is there" (run.mjs:401-405) catches new nodes appearing. What is
  missing is a positive per-node identity assertion ACROSS a boundary -- and
  `fixtures/nested/interact.mjs`'s own header promises group 2, "hydration adopted
  them, node identity intact ACROSS the boundary", while the body jumps from group
  1 to group 3. The group was never written.
  => adding it (see below), so gap B is observed per boundary and not only summed.

### Item 3 — per-fixture budgets LANDED and green. Baselines below.

New files, all in `parity/`:
- `lib/budget.mjs` — pure measurement. `sizes()` (raw + gzip), `band()`, `measure()`
  (a subject is the SUM of its artifacts), `judge()` (five states).
- `budgets.json` — the recorded baselines, the policy, and the `chunking` block.
- `budgets.mjs` — the standalone report + `--update` re-baselining, plus
  `budgetChecks()` which run.mjs calls per fixture.
Wired: `run.mjs` (per fixture), `check.sh` (a new `=== size budgets` step after the
parity run), `test.mjs` (18 new unit checks).

BASELINES, all MEASURED on 2026-07-30, node v24.5.0, zlib 1.3.1-470d3a2, gzip level 9:

| subject | gzip | raw | artifacts |
| --- | --- | --- | --- |
| island:counter | 969 | 3220 | /demo/islands.js |
| island:nested | (pending) | | no island yet |
| island:search | (pending) | | no island yet |
| runtime | 9106 | 23971 | /demo/topcoat-dom.js |
| loader | 307 | 477 | /demo/island-loader.js |
| page:island | 10382 | 27668 | the three above, summed |
| /demo/islands.js.map | 1581 | 5144 | measured, NOT enforced |

DESIGN DECISIONS, each with the argument, because a budget nobody believes gets
raised until it means nothing:

1. **A BAND, not a ceiling** — the spike's own `NN.maxbytes` (`scripts/test.sh:270`)
   is one-sided, correctly, because it watches for a lost dead-code elimination and
   that only grows output. What ships to a browser needs the other side: a chunk
   that SHRANK unexpectedly means something that was in it is not in it, and a
   ceiling calls that a pass. So `under` is a distinct failing state with its own
   message.
2. **gzip, level 9, pinned** — and pinned at the extreme rather than a realistic 6
   so a moved number means moved CONTENT, not a differently-tuned compressor.
   MEASURED while deciding: topcoat-dom.js is 9105 at level 6 and 9106 at level 9.
   Deflate is not monotone in effort, which is the argument for writing the level
   down instead of trusting a default. node+zlib versions recorded alongside, with
   the note that several subjects moving a few bytes AT ONCE is a zlib change.
3. **slack = max(tolerance * baseline, floorBytes)**, 5% / 128 bytes. Proportional
   alone is unusable at both ends of this range: 5% of the 307-byte loader is 15
   bytes, one reworded comment in dom.rs. Tolerance is TIGHT because these builds
   are deterministic -- the band absorbs intentional changes, not noise; there is
   no noise.
4. **A subject is a SUM, summed per artifact, not gzipped as one stream.** The
   browser fetches and decompresses each module separately, so its cost is the sum;
   concatenating first would credit the delivery with cross-file redundancy no
   transport exploits. Written as a sum NOW so an island's post-chunking subject
   (own chunk + shared chunk) needs no shape change.
5. **"unmeasured" is a FAILING state, not a pass.** The first green run after an
   island lands is supposed to stop and make someone record the number.

THE COVERAGE CHECK is the one that makes this hard to leave stale: every `.js` route
dom.rs serves must be named in budgets.json, as a subject or in `$notBudgeted` with
a reason. A new module otherwise leaves every subject green while the page grows.

MY OWN FIRST ATTEMPT AT IT WAS WRONG, recorded because it is the useful half: I
detected chunking by pattern-matching route names for something chunk-shaped, and
`/^\/demo\/islands?[-.]/` matched `/demo/island-loader.js`, so the first run
reported "per-island chunking HAS landed" against an unchunked tree. Asking from the
other side -- "is this route one budgets.json names" -- needs no guess about what a
chunk will be called, which matters because the backend's own wave-3 design
(`wave3-backend-report.md` step 1) puts chunks in a directory whose name is a build
option. Both the misfire and the fix are pinned by unit tests.

PREDICTED BASELINE MOVE, named in advance so it is not mistaken for a regression:
the framework agent's wave-3 item 1 (move-closure emission) makes `count[0] = ..`
become plain `count` in the counter's compiled module, so `island:counter` will go
**UNDER** its band. That is the two-sided band doing the job it was argued for, with
a cause that can be named -- re-baseline with `node budgets.mjs --update` when it
lands, and the diff records what it was.

### Item 4 — LANDED

- Named check `IDENTITY` in run.mjs, backed by exported `movedKeys(mount, keyed)`:
  captures `data-hk -> node object` BEFORE hydration and reports which keys no
  longer resolve to that object. Per key, so a failure names the BOUNDARY.
- Honest about what it adds: ATTRIBUTION, not detection. A swapped clone also fails
  CLAIMED and also fails "every node the server sent is still the node that is
  there"; neither says WHICH node, and for a nesting island "some node changed" is
  not a usable diagnosis. Written that way in the code so nobody reads more into it.
- Not vacuous, proved by unit test rather than argued: a jsdom document with a
  nested keyed pair, the INNER node replaced by its own clone -> `movedKeys` returns
  `["i0.10"]` while the serialized markup is byte-identical. Plus: a vanished key
  counts as moved.
- Gap A needed nothing. Recorded why, above.
- Also dropped run.mjs's hardcoded single-template assumption (`expected.templates`,
  default 1) and added `expected.islandTemplate` + a declaration-order diagnostic,
  because "the template does not describe the server's tree" and "the templates are
  declared in a different order than the fixture assumed" are different findings and
  the nested fixture declares `templates: 3`.

STATE: test.mjs 80 -> 101 checks, counter 39 -> 43, nested 6 -> 7, both negative
controls still fail on their named check. `check.sh --check` green end to end.

### Item 2 — search fixture LANDED as a validated scaffold (island still absent)

`/island/search` does not exist: the framework agent's item 3 is NOT STARTED as of
this writing. So the fixture is landed the way wave 2 landed `nested` -- `pending`,
with everything that does not need the server's bytes checked on every run. For this
fixture that is the WHOLE WIRE, which is most of what the item is about.

New files:
- `parity/lib/wire.mjs` — `wireSpec()`, `wireParity(declared)`, `fetchStub(wire, queue)`.
- `parity/fixtures/search/fixture.json` — declaration, `sourceShape`, `wire`, `expected`.
- `parity/fixtures/search/interact.mjs` — the full loop, plus `dead()`.

THE MECHANISM THAT MATTERS: the fixture does NOT restate the wire. Its `wire` block
NAMES the vectors it honours (`serde-one-arg`, `serde-zero-args`,
`serde-zero-args-rejects-null`, `serde-rejects-json-media-type`, `response-struct`,
`serde-wrong-argument-type`) and `lib/wire.mjs` reads the values off them. The
argument is wave 2's own scar: the rendered argument path was written `1` in
procedure-wire.json and observed `[1]` by the server, and a fixture that had COPIED
the value would have gone on stubbing the stale one into a stub that accepted it
happily. `wireChecks` runs on BOTH the pending and the live path, so the agreement is
asserted every run rather than at authoring time. The run also surfaces the vector's
`$corrected` flag as a note, so nobody re-derives the old spelling.

NOT VACUOUS, proved rather than argued: 9 perturbation tests, one per compared field
(request/response content type, zero-arg body, route prefix, method, error
prefix/suffix/status, error content type), each asserted to produce exactly one
difference that names both sides; plus a nonexistent vector id; plus the check that
the error-suffix failure names `serde-wrong-argument-type` and not just the value.

ASSERTIONS `interact.mjs` MAKES (all written, none yet run):
1. Server state: input and list survived hydration, list starts EMPTY (the server ran
   no query), input empty.
2. DEBOUNCE, the claim about an interval: THREE keystrokes in a row, then zero calls
   synchronously AND zero at 40% of the window (sampled inside the window so the
   assertion is not a race with its own timer), then exactly ONE call after it. Three
   keystrokes because the claim is COALESCING -- one call per keystroke after a delay
   would pass a one-keystroke test and is the exact thing a debounce prevents.
3. WIRE, on the recorded call: procedure route by SHAPE (prefix + exactly one
   non-empty segment -- the id is a uuid minted at macro-expansion time and is not
   knowable or stable), POST, `application/topcoat+json` (parameters stripped), the
   argument as a JSON ARRAY `["ru"]` and not a bare value (the tuple is `(T,)`), and
   explicitly never `null` (the surrogate wire's zero-arg spelling, which this wire
   rejects).
4. Render: one `<li>` per result, inside the SAME `<ul>` element the server sent.
5. Update: a second settled query REPLACES the list rather than appending. The two
   result sets deliberately OVERLAP on one item -- a disjoint pair would make an
   append look like a replace to anyone reading the last item only. Mutations
   asserted to land only inside the list.
6. NEGATIVE: a 400 whose body is built from the error vector (prefix + suffix, middle
   deliberately NOT asserted -- it is serde_json's own wording and a patch release
   would break it). Then: the last good results are still on screen, and the island
   still searches and renders afterwards. What the client DOES with the error is
   RECORDED as a note, not required -- with the PROCEDURES.md defect cited (the
   existing browser client throws `Procedure call failed: ${status} ${statusText}`
   and DISCARDS the body, so the server's argument index never reaches the developer;
   if the logged message carries no part of the body this client inherited it).
7. Bookkeeping: the stub's queue is fully consumed, and `fetchStub` THROWS on a call
   past the end rather than inventing a reply.
8. `dead()`: a dead island makes NO REQUEST AT ALL -- the loudest of the three
   fixtures' symptoms -- with an EMPTY stub queue so a dead island and a working one
   cannot both produce that result.
Standard hydration assertions (zero mutations, registry consumed, IDENTITY, negative
controls) come from run.mjs and need nothing fixture-specific.

`input` IS THE FIRST NON-CLICK EVENT in the harness: `events: ["input","click"]`, and
run.mjs already compares the served bootstrap against `buildHydrationScript(events)`,
so a server that delegated only `click` fails before any typing happens. Typing goes
through a real dispatched BUBBLING `input` event, because solid's delegation puts one
listener on the document and a non-bubbling event never reaches it.

RUN.MJS CHANGES this required:
- `interact`/`dead` are now AWAITED. A debounce cannot be measured synchronously. The
  two synchronous fixtures return undefined, which awaits to undefined.
- the interact context gains `wire` and `console` (the page's captured console), so
  the error-path note reads the run's own log rather than reimplementing capture.
- `pendingFixture` now tolerates `mirrors: null` (an island with no components has no
  nesting case to mirror; its keys still go through the scheme checks) and runs
  `wireChecks` when the fixture declares a wire.

KNOWN OPEN QUESTION, recorded in the fixture rather than assumed away: the framework
agent's finding that a procedure id is minted PER EXPANSION means one `#[procedure]`
in a file shared by the server and client crates gets TWO different uuids and cannot
address itself, so their island may call a demo-app route instead of the procedure
route. `wire.$ifTheIslandDivergesFromThis` says exactly what to do: move `routePrefix`
and record the divergence, but KEEP the body encoding, media type and error vectors --
those are what the fixture is for and they are the part their design preserves.

STATE after item 2: test.mjs 101 -> 123 checks. search fixture 7 checks (pending).
`check.sh --check` green; `harness/run-all.mjs --check` 13 steps green, no drift.

### Item 1 — NESTED FIXTURE IS LIVE. The declared keys HELD. 48/48.

The framework agent's item-status table said "NOT STARTED" and was STALE: `demo-app/island/nested.rs`, the `#[page("/island/nested")]` in `main.rs`, and a rebuilt
binary were all already on disk. So I checked the tree rather than the table.
No "NESTED ISLAND SERVING" marker was ever written; the evidence was
`__island_nested` in the compiled module plus the page route.

FLIP: read `island/nested.rs` first and confirmed it is my fixture's `sourceShape`
EXACTLY (their D1 records the decision to build to it rather than add child content,
which would have moved every key after the first). Then `capture-ssr.mjs` -> written
nested /island/nested 2440 bytes (new). Deleted `pending`. Ran the suite.

**THE HEADLINE: every declared key held against the REAL island.**
- `keys: ["0","10","110"]` -- exact. The card's own root carries `i0.10`, the badge's
  `i0.110`, derived boundaries `["i0.1","i0.11"]`, 2 components as declared.
- `components: 2`, `templates: 3` -- exact.
- `hydrateMutations: 0` -- exact. A component boundary costs NOTHING at hydration.
- `clickMutations: 1` and `clickHtml` -- both were PREDICTED in wave 2 and both held.
- clone identity holds on both sides of both boundaries; no mutation landed inside a
  component; the card is byte-identical after a click.
48 checks, all green. Nothing about the emitter, the SSR side or the scheme was wrong.

THREE FINDINGS, none of them a key mismatch:

**F1 (harness bug I introduced-by-omission, MINE, fixed). `moduleTemplates` counts the
whole module, and the module now has two islands.** The counter's true claim of ONE
template started reading as FOUR the moment `nested` landed in the shared bundle --
the fixture failed, not the counter. Fixed with `islandTemplates(source, island)` in
`lib/delivery.mjs`: transitive reachability from `__island_<name>` over the module's
own functions, collecting the `const tmpl$h.. = _$template(..)` bindings it reaches.
counter -> 1, nested -> 3. Measured, not guessed. Sub-finding worth keeping: my first
regexes used `\w`, and the emitter's mangled names carry `$` (`tmpl$h<hash>`,
`islands_js_expanded$counter$..`), so they matched NOTHING -- which is the right
direction for a reader to fail in, and is why the "declares no bindings" error exists.

**F2 (scaffold assumption, MINE, was WRONG -- and the diagnostic I built for it paid
for itself immediately).** The scaffold assumed the island's own template is declared
FIRST (`islandTemplate` default 0). It is declared LAST: the emitter emits the two
COMPONENT body templates before the island's own, so nested's island root is index 2
(`tmpl$h20207ec50a9647a8` badge span, `tmpl$h7cd76cfa254e7097` card section,
`tmpl$h424d16885beec833` island div). Now recorded as `islandTemplate: 2`, labelled
MEASURED. Attributed precisely: SCAFFOLD ASSUMPTION, not emitter and not SSR.

**F3 (a stale recorded fact about the product, NOT mine, and the biggest one).**
Running the `key` control against nested exposed that the control could not test what
the fixture was built for: `perturb` takes the FIRST `data-hk`, which is the island
ROOT, so the island dies whole. The nested fixture's whole reason to exist was the
QUIET partial failure below a boundary, and no control reached it.

Added a THIRD control, `deepKey` + `perturbDeepest`, breaking the DEEPEST key
(depth measured with `keyChain`, not string length -- length also grows with the
ordinal, so sorting on it would sometimes pick a shallow node and silently become the
`key` control again). The replacement stays inside the SAME parent context, so only
the last slot is wrong and the finding is as narrow as possible. Skipped with a
reason for a component-free fixture, where the deepest key IS the root.

That measured three things, and **the description of the degradation recorded in
wave 2 turned out to be the OPPOSITE of what now happens**:
- Wave 2's note said: the clone is never inserted, ZERO mutations, byte-identical
  DOM, a completely INERT island. That described a loader that handed the runtime
  `[...mount.childNodes]` as a workaround for the zero-sized return type.
- `demo-app/src/dom.rs`'s LOADER no longer does that -- it now calls
  `hydrate(() => entry(...seeds), mount, { renderId })`. Upstream's insert semantics
  apply and the clone IS inserted.
- MEASURED now: a key miss is SILENT LOSS OF HYDRATION, not death. Root miss on the
  counter = 1 childList record replacing the root, and **the island then WORKS**:
  clicking renders 6, costs the same 1 mutation, markup is byte-identical (same
  template, same seed), no throw, no console output. Every signal a person would
  check says fine. Better functionally, worse observably.
- And the substantive result about component contexts, which needed BOTH controls:
  **a hydration key is an INDEPENDENT CLAIM in both directions.** deepKey: the badge
  is rebuilt (loses its `data-hk`) while the card ABOVE it and the root keep theirs --
  damage contained to one boundary, one childList record inside the card. Root key:
  the root loses its claim and is rebuilt, while the card and badge are STILL claimed
  off the server's own nodes and moved into the rebuilt root. I first asserted "a root
  miss costs every key below it" and the measurement corrected me.

Fixed as a consequence: run.mjs's degradation note (rewritten from measurement, and it
says the old text was stale so nobody trusts the rest of it), counter's `dead()` (it
asserted an inert island; now asserts the working-client-render symptom and why it is
invisible), nested's `dead()` (branches on `perturbed.which`, which is now handed to
the probe).

FOR THE FRAMEWORK AGENT: the LOADER change is theirs and the positive path is entirely
green, so this is not a regression report. But the observability consequence is worth a
decision: a hydration key miss now costs the user a full re-render of the affected
subtree and NOTHING reports it -- no throw, no console message, identical markup. Only
this harness sees it. Upstream's mismatch throw is dev-only (CONTRACT-DOM 14.6) and
`web.js` has no such branch, so in production it is entirely silent.

STATE: counter 43/43, nested 48/48, search 7/7 (pending), unit tests 129, four control
runs each failing on their named check, `deepKey` skipped-with-reason for counter,
budgets all inside their bands.

### Item 2 reconciliation — the search fixture now targets the wire that EXISTS

The framework agent landed the search island's SERVER half and measured it against my
vectors with live curl. Two reconciliations followed, and the fixture had already
prescribed the first one before it happened.

**R1: the call target is `/demo/search`, NOT the procedure route.** Their finding: a
procedure's id is a uuid minted at MACRO-EXPANSION time, and an island's file is
expanded TWICE (once per crate), so the two expansions mint DIFFERENT ids and a
procedure declared in a shared file cannot address itself from the client. demo-app
serves the same wire at a written-down `#[route(POST "/demo/search")]` decoding
through `topcoat::runtime::serde_args`.
This is exactly what `wire.$ifTheIslandDivergesFromThis` told the next person to do:
move the route, record the divergence, KEEP the body encoding / media type / error
vectors. Done via a new `wire.callTarget` (`kind: "exactPath" | "procedureRoute"`),
and `fetchStub` now reports `onTarget` alongside `isProcedureRoute` so a later change
of mind is visible rather than silent. `routePrefix` is KEPT and still cross-checked,
because the procedure route does exist -- it is just not what a compiled island can
call.

**R2: the error path is `(at `[0]`)`, not `[1]` — and this is the wave-2 lesson
repeating in a new form.** The vector pins a TWO-argument signature so it records
`[1]`; this procedure takes ONE argument, so the server answers `[0]` (measured, live).
Copying the vector's literal -- which is what the fixture originally did -- would have
stubbed an error the real server never sends. Fixed by making `lib/wire.mjs`
**validate the rule, then apply it**: the spec's `$pathRule` (sequence index written
`[n]`) must first reproduce the VECTOR's own recorded suffix at the vector's own
argument index -- a closed loop with no free parameter -- and only then is the rule
applied at this fixture's index. The failure message says explicitly "do not fix this
by editing the fixture", because the tempting fix is the wrong one.
That is strictly stronger than what I shipped an hour earlier, and the trigger was the
same class of mistake wave 2 recorded: a value that is right for the vector and wrong
for the caller.

Also folded in: `expected.results` / `secondResults` are now the bodies the server
ACTUALLY answered (`["ru"] -> ["rust","ruby","trust","crust"]`,
`["st"] -> ["rust","trust","crust"]`), measured rather than invented, and they still
overlap -- on "rust" and "trust" -- so the replace-vs-append assertion stays sharp.

Unit tests for all of it: rule-vs-vector both directions, the "do not edit the
fixture" instruction, both `callTarget` malformations, `onTarget` vs
`isProcedureRoute` told apart, and the positive procedure-route shape case.
test.mjs 129 -> 138.

### For the next wave (per-chunk budgets, item 3's remainder)

Backend C1 has NOT landed (no `DYLIB STABLE`, no C1-complete marker). When it does,
`budgets.mjs` will FAIL on its own coverage check and print what to do, because every
new chunk route is a module budgets.json does not name. The steps are already written
into `chunking.$landed` and the failure message: set `chunking.landed`, point each
island subject at its own chunk PLUS the shared chunk (subjects are already sums, so
no shape change), add a subject for the shared chunk, re-baseline with
`node budgets.mjs --update`.
Their design (wave3-backend-report step 1) puts chunks as siblings in
`OUT_DIR/<dir>/` with cross-chunk imports spelled `./<owner>.js`, and the directory
name is a build option -- which is why the coverage check does not pattern-match route
names. Note also their flagged follow-up: a chunked build needs
`js-shim-module=../shim.js`, or the default `./shim.js` resolves to `chunks/shim.js`
and 404s.
