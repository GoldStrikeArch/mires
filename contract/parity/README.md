# parity

The SSR to client hydration parity harness. It asks one question: when the browser hydrates a server rendered island, does it adopt the DOM the server sent, or does it quietly build a new one that looks the same?

That question cannot be answered by comparing HTML. A node replaced by an identical node serializes identically. So this harness runs hydration in a live DOM with a MutationObserver attached and reports what actually changed.

## The idea

`demo-app/smoke/check.mjs` and `demo-app/smoke/runtime.mjs` are the rung below this one. They prove the compiled island module works: driven against a hand written stub, its signal, its handlers and its interpolation all behave. They cannot prove that hydration happens, because the stub has no hydration registry, no `data-hk`, and no hydrating branch. It answers a different question than a browser does.

This harness puts the real pieces in one room:

| piece | where it comes from |
|---|---|
| the server's HTML | `fixtures/<name>/ssr.html`, captured from the running demo-app |
| the `_$HY` bootstrap | the inline script in that HTML, executed by jsdom as a real script |
| the runtime | whatever `dom.rs` serves at `/demo/topcoat-dom.js`, read off disk |
| the compiled island | the island's own chunk, resolved through the page's import map by `topcoat-island/<name>` |
| the hydrate bracket | the `LOADER` constant in `dom.rs`, run as a module |
| the DOM | jsdom |

Nothing in that list is a copy this harness maintains. The delivery constants and the route table are parsed out of `demo-app/src/dom.rs`, so a harness that still passed while the served page broke is not expressible. That is not a hypothetical: between this harness being started and being finished, `dom.rs` went from serving the runtime as three import-map-stitched modules to serving one self-contained bundle, and nothing here needed changing.

What `dom.rs` is allowed to look like has widened twice since, and both are worth knowing before reading `lib/delivery.mjs`. A delivery constant can be a build artifact rather than a literal, which is what the import map became once the islands were chunked, since the build is what knows how many chunks there are. And a route can carry a `{parameter}` and serve a whole directory, which is what the chunks are served by. A parameterized route reads as one entry per name it answers, keyed by the URL a browser actually requests, because the alternative is one route standing in for six modules and the first `include_str!` in it quietly speaking for all of them.

A constant the harness cannot parse is a failure. A constant it can parse but whose build artifact is absent is a missing build, and the two are told apart on purpose: the unit tests skip the group that reads the real `dom.rs`, loudly and with the reason, and go on proving everything that does not need a build.

## Running it

```sh
cd contract/parity
npm install          # first time only
./check.sh --check   # unit tests, capture drift gate, parity run, size budgets
```

`./check.sh` without the flag re-captures `fixtures/*/ssr.html` from the demo-app binary instead of failing on a difference. Same UPDATE and CHECK split as `harness/run-all.mjs` one directory up, with one difference in kind: the contract harness regenerates from pinned upstream sources, so its drift report answers "did upstream change". This one regenerates from demo-app's own server, so its drift report answers "did the server's response change", which after a rebuild it should.

The pieces run on their own too:

```sh
node test.mjs                  # the harness's own unit tests
node capture-ssr.mjs           # re-capture the SSR bytes
node run.mjs counter           # one fixture
node run.mjs --no-negative     # skip the negative controls
node run.mjs --verbose         # print every mutation record
node budgets.mjs               # the size budgets, all subjects
node budgets.mjs --update      # re-record the baselines that moved
```

Neither `run.mjs` nor `test.mjs` needs a server or cargo. `capture-ssr.mjs` needs a built demo-app binary, and deliberately does not build one.

## jsdom

`contract/README.md` records that the contract harness has no jsdom, for two reasons. jsdom 30 requires node `^24.15` and this environment is 24.5. And jsdom depends on parse5 8, which would put a second parse5 major next to the 7.3.0 that the reference babel plugin parses templates with, and the tree oracle has to agree with that plugin about HTML parsing.

Both objections are about one dependency tree, not about jsdom. So this is a separate package with its own `node_modules` and its own lockfile. `contract/vendor` is untouched and keeps parse5 7.3.0 as the only parse5 the oracle sees; `contract/parity` gets parse5 8.0.1 as jsdom's private dependency. Verified by reading both trees.

jsdom is pinned to 29.1.1, the last line whose engines field admits node 24.5 (`^20.19.0 || ^22.13.0 || >=24.0.0`). MutationObserver delivers characterData, childList and attribute records there, which is the whole reason for wanting a real DOM.

## Where the SSR bytes come from

The demo-app server, captured to a committed file. There is no rendering crate here, and the reason is worth stating.

A rendering crate would need its own copy of the counter's view body. The claim this harness makes is that one view body compiled twice agrees with itself, so a second transcription of that view would be a third thing to keep in step, and the day it drifted the harness would report parity between two copies that agree with each other and not with the app.

demo-app already renders the real island through the real macro at the real route. `capture-ssr.mjs` runs the binary demo-app's own build already produced, fetches the route, and writes the response. No cargo runs at any point.

The cost of a committed capture is that it can go stale against a rebuilt emitter. That is covered rather than hoped about. `run.mjs` compares the client's template string against the captured markup node for node, so a capture that no longer matches the compiled module fails the run.

## What it asserts

Per fixture, in order:

| group | assertions |
|---|---|
| delivery | the page's bootstrap equals `fixtures/hydration-script.js` built for the island's event list; the captured response carries it verbatim; the captured response carries the import map `dom.rs` now serves |
| module graph | every specifier the compiled module imports is resolved by the import map |
| the bootstrap ran | `_$HY` exists with `events`, `completed`, `r` and `fe`, and with no `done` yet |
| the server's own claims | the seeds in `data-ts` are the island's arguments; the render id is `data-tk` plus a separator; the first hydration key is that render id's ordinal zero |
| key scheme | every key the server wrote is one the allocation scheme can produce, and the boundaries they imply are the components the source has |
| tree parity | the client's template describes the tree the server sent, node for node, with `data-hk` and filled holes allowed |
| the module | it exports the island's entry point, with the island's arity |
| the observer | it reports a probe mutation, so a zero below cannot mean a deaf observer |
| hydration | the hydrate bracket does not throw and logs nothing; its mutation count equals the fixture's number; every key the server wrote was claimed off the server's own node; the island root is the same node object it was before; no node under the mount was replaced; the mount serializes as it did before |
| clone identity | every key the server wrote still resolves to the same node **object**, on both sides of every component boundary |
| size budgets | the fixture's island chunk and the runtime bundle are inside their recorded gzip bands, and every module the page delivers has a budget at all |
| the wire | if the island calls a procedure, the fixture's declared wire is the one `../fixtures/procedure-wire.json` specifies, vector for vector |
| the live island | a real dispatched click on `+1` renders the next value, costs the fixture's number of mutations, and touches only the count element; the bound `disabled` property tracks the signal to zero and back |

Four of those are worth expanding.

**Registry consumption** is proved from the far end. `sharedConfig` is not exported, so the registry cannot be read directly. But `getNextElement` adds every node it claims to `_$HY.completed` and deletes that key from the registry in the same breath, so "in `completed`" and "gone from the registry" are the same fact. Asserting it against the node object the capture was parsed into also proves the claim was the server's node and not an identical clone, which no HTML comparison can do.

**Tree parity** compares parsed trees, not strings. The template writes its holes with the short marker syntax (`<!$>`, a bogus comment the parser turns into a comment node with data `$`) while the server writes the long one (`<!--$-->`), and the server's markers have the rendered value between them. Both differences are correct, and on parsed trees both disappear. `data-hk` is the one attribute the server may add.

**Clone identity** is a `data-hk -> node object` map captured before hydration and re-resolved after it. It is here because the wave-2 backend report flagged it as a limit of the dom-test stub and delegated it: `harness/trace.mjs` labels a node by the template it was cloned from, so three reused nodes and three fresh clones read identically there. This harness holds real node objects, so it can just ask.

What it adds over the checks either side of it is **attribution, not detection**, and that is worth being exact about. A clone swapped in for a server node also fails registry consumption, because the original is not in `_$HY.completed`, and also fails "every node the server sent is still the node that is there", because the clone is a node that was not there before. Neither of those says *which* node. For a one-template island there is only one candidate and it hardly matters. For a nesting island "some node changed" is not a usable diagnosis, and "the node at key `i7.110`, the badge inside the card, is not the object the server sent" is -- it names the boundary. That is the difference between the nested fixture finding a component-context bug and reporting a symptom of one.

**Size budgets** are described in their own section below.

## Negative controls

A parity harness that cannot fail is the failure mode, so each run is followed by three controls. Each perturbs one thing and passes only if one **named** check fails. Inverting on "something failed" would let a control that broke for an unrelated reason look like a working one.

| control | perturbation | must fail |
|---|---|---|
| `key` | the island root's `data-hk` becomes a key under the same render id that the client never asks for | every key the server wrote was claimed off the server's own node |
| `deepKey` | the same, but on the *deepest* nested key, staying inside its own parent context | every key the server wrote was claimed off the server's own node |
| `tree` | a `<span>` is appended to the island root | the client template is the tree the server sent, node for node |

The `tree` control is caught by tree parity and by nothing else: the extra node sits past every walk the counter performs, so hydration reports zero mutations, claims its key, leaves the DOM byte identical, and the island still works. Tree parity is the check that sees drift before it becomes a bug.

`deepKey` exists because running `key` against the nested fixture showed that fixture could not test what it was built to test. `perturb` takes the *first* `data-hk`, which is the island's own root, so the island is rebuilt whole -- the loudest available failure. The nested fixture was built for the quiet one: break a key *below* a component boundary and see how far the damage spreads. Depth is measured by walking the key chain rather than by string length, because a key's length also grows with the ordinal it encodes, so sorting on length would sometimes pick a shallow node with a big ordinal and quietly turn this back into the `key` control. It is skipped, with a reason printed, for a fixture that declares no components -- there the deepest key *is* the root, and a duplicate control makes a suite look broader than it is.

### What a key miss actually does

Worth its own heading because the answer recorded here through wave 2 was the **opposite** of what happens now, and the correction is more instructive than either state.

The old description: the fallback clone is never inserted, so a key miss costs zero mutations, leaves a byte-identical DOM, and produces a completely inert island. That was true of a loader which handed the runtime `[...mount.childNodes]` as a workaround for the zero-sized return type. `dom.rs`'s `LOADER` no longer does -- it calls `hydrate(() => entry(...seeds), mount, { renderId })` -- so upstream's own insert semantics apply and the clone **is** inserted.

Measured now: a key miss is not death, it is **silent loss of hydration**. `getNextElement` misses the registry, falls back to `template()`, and the fresh clone replaces the server's node. On the counter the island then *works*: clicking renders the next value, costs the same single mutation a hydrated island costs, and the markup is byte-identical because it is the same template rendered from the same seed. No throw, no console output. Every signal a person would think to check says everything is fine, and what was actually lost is the server's DOM. Better functionally than an inert island; considerably worse to notice.

And the substantive result about component contexts, which needed both key controls to see: **a hydration key is an independent claim in both directions.** Under `deepKey` the badge is rebuilt and loses its `data-hk` while the card above it and the island root keep theirs -- one childList record, contained at the boundary. Under `key` the root loses its claim and is rebuilt, while the card and the badge are still claimed off the server's own nodes and moved into the rebuilt root. Neither control alone shows that.

## Current results

The counter fixture passes all 43 checks and the **nested fixture passes all 48 against the real island**; the `search` scaffold passes its 7. Every control fails on its named check, `deepKey` is skipped with a reason for the counter, and the controls are skipped for the pending fixture, because a control over bytes that do not exist would pass vacuously.

`node test.mjs` is 161 unit checks over the harness's own moving parts, of which the nested-key group runs against all 108 cases in `../fixtures/keys-nested.json`.

**The nested fixture's declared expectations all held.** Its keys -- `["0","10","110"]`, written and validated against the key oracle before the island existed -- are exactly what the server wrote: the card's own root carries `10`, the badge's carries `110`, and the two boundaries they imply are the two components the source has. `hydrateMutations` is 0, so a component boundary costs nothing at hydration; `clickMutations` is 1 and `clickHtml` matched, both of which were predictions. Clone identity holds on both sides of both boundaries, and no mutation lands inside a component on a click. Nothing about the emitter, the SSR side, or the key scheme was wrong. What the flip did find was two things about the harness itself -- the template count and the declaration order, both recorded below -- and one stale fact about the product, above.

The headline number is that **the hydrate bracket produces zero mutations**. Hydration adopts the server's DOM completely: same root object, no node replaced, byte identical markup, and every hydration key claimed.

One click costs one mutation, which is what upstream costs. It used to cost three; see the section below.

## The getNextMarker defect, and what fixing it changed

**Fixed, and this section is kept as the worked example of the harness catching something.** It is the clearest illustration of what this harness is for, so it is recorded rather than deleted.

The defect: the compiled `islands.js` called `_$getNextMarker($t3)` where `$t3` was the `<!--$-->` marker itself, and handed both the returned end marker and the claimed node list to `_$insert`. The claimed list was therefore the opening marker, the value AND the closing marker, where upstream claims only the value. The fix, in `backend/src/abi.rs`, names a hole's closing `<!/>` as a walk step so a later step can re-base on it.

What it cost, and where: **nothing at hydration, one extra pair of mutations per update.** `fixtures/counter/fixture.json` now records `clickMutations: 1` and a `clickHtml` with both marker comments preserved; it used to be 3 and a `<p>` with no markers left in it. `hydrateMutations` was 0 throughout and the fix could not change it.

The lesson worth keeping is the one about the wrong prediction. The wave 4 ledger predicted the defect would cost one mutation per dynamic text hole AT HYDRATION. It did not, and the reason is in the runtime: `insertExpression` returns early for a string or number hole while hydrating, so the claimed range is never written during hydration no matter how wrong it is. The prediction came from `demo-app/smoke/stub.mjs`, which has no hydrating branch and so could not show that. The defect was real; it was an update-time defect, and only a harness running the real hydrating runtime could say which.


## Nested keys

`lib/keys.mjs` is the half of the comparison library that only matters once components are involved. `lib/tree.mjs` asks whether the client's template describes the server's tree; this one asks whether the keys are keys the allocation scheme can produce, and whether they nest the way the source does.

It works because a key is self-describing. Reading left to right from the render id, the first character of what remains fixes the next slot's width: a digit is a one-digit count, and a letter promises exactly `letter - 'a' + 2` digits. So `keyChain` walks a key into the chain of component contexts it was allocated through, deterministically, with no search. That is what the letter in a key is for, and it is the inverse of `hydrationKey`, which lives in the same file so one implementation serves both directions.

Two things it will not assume, both because `../fixtures/keys-nested.json` proves they are false:

- **Document order is not allocation order.** With eager child content a component's child is built before the boundary is spent, so it carries a LOWER key while sitting INSIDE the element with the higher one: `<div data-hk="10"><span data-hk="0"></span></div>`. A checker that demanded a rising sequence in document order would fail the one layout Topcoat emits. Comparison is against a declared allocation order, recovered from the key strings; document order is reported alongside as information.
- **Context nesting is not DOM nesting**, in either direction, for the same reason.

And one thing it is honest about: parsing alone is weak, because a key of nothing but digits always parses, as a key several boundaries deep. What makes the check tight is `components`, the boundary count the fixture's source has, plus the rule that a slot spent by an element cannot also be the context a deeper key nests under. Both directions are asserted in `test.mjs`, including the limit.

## Size budgets

`budgets.json` records what each part of the island delivery costs a browser, `lib/budget.mjs` measures, and `budgets.mjs` judges. Every number in the file was measured, never chosen: a baseline is the first green measurement of a subject, recorded with the date it was taken.

**A band, not a ceiling**, and that is the one design decision worth arguing for. The spike's own budgets (`examples/*/NN.maxbytes`, enforced at `scripts/test.sh:270`) are one-sided, and for what they watch that is right -- they exist to catch a lost dead-code elimination, which only ever makes output bigger. What ships to a browser needs the other side too. A chunk that *shrank* unexpectedly is not good news: it means something that was in it is no longer in it, and the two ways that happens are a lowering that silently stopped emitting and a dead-code pass that reached too far. Both produce a smaller file and a broken island, and a ceiling calls that a pass. So `under` is a distinct failing state with its own message.

**gzip, at a pinned level 9.** Raw bytes are what the ceilings above watch, because they are measuring the emitter's output as an artifact; these are measuring what a user downloads, and no server sends this uncompressed. The level is pinned at the extreme rather than a realistic 6 so that a moved number means moved *content*, not a differently tuned compressor. Measured while deciding: `topcoat-dom.js` is 9105 bytes at level 6 and 9106 at level 9. Deflate is not monotone in effort, which is exactly why the level has to be written down instead of left to a default. Raw is recorded alongside, because when a subject moves it is the first thing worth reading. The node and zlib versions are recorded too: if several subjects move by a handful of bytes at once, that is a zlib change and not a regression.

**Slack is `max(tolerance * baseline, floorBytes)`**, 5% and 128 bytes. Proportional alone is unusable at both ends of this range: 5% of the 307-byte loader is 15 bytes, which one reworded comment in `dom.rs` would break, while 5% of the 9 KB runtime is 456, which is the right order. The tolerance is tight because these builds are deterministic -- the band absorbs intentional changes, not measurement noise, because there is no noise.

**A subject is a sum, summed per artifact.** The browser fetches and decompresses each module separately, so its cost is the sum of the parts; gzipping a concatenation first would credit the delivery with cross-file redundancy no transport exploits. It is written as a sum now so that an island's post-chunking subject -- its own chunk plus the shared chunk it imports -- needs no shape change.

**"Unmeasured" is a failing state, not a pass.** A subject with no baseline reports its measurement and what to run. The first green run after an island lands is supposed to stop and make someone record the number.

The check that keeps the file honest is **coverage**: every module `dom.rs` serves must be named in `budgets.json`, either as a subject or in `$notBudgeted` with a reason. Without it a new module can appear in the delivery while every recorded subject stays green and the page gets bigger. It is also how per-island chunking announces itself, and the first attempt at that got it wrong in an instructive way: it pattern-matched route names for something chunk-shaped, and `/^\/demo\/islands?[-.]/` matched `/demo/island-loader.js`, so the first run reported chunking as landed against an unchunked tree. Asking from the other side needs no guess about what a chunk will be called, which matters because the backend's design puts chunks in a directory whose name is a build option. Both the misfire and the fix are pinned by unit tests.

Before per-island chunking lands there is one island module, so the three island subjects all resolve to the same file and are not independent: adding any island moves all of their baselines at once. That non-independence is itself the measurement that argues for chunking, and it is recorded in the file's `chunking` block rather than left to be discovered.

## The procedure wire

A fixture whose island calls a procedure has to stub the call, and a stub is a second statement of the wire. `../fixtures/procedure-wire.json` is the first one.

So the fixture does not restate it. Its `wire` block **names** the vectors it honours, and `lib/wire.mjs` reads the values off them and fails with the name of the vector when they disagree. The argument for that indirection is a scar rather than a principle: in wave 2 the rendered argument path in a decode error was written `1` in the spec and observed `[1]` by the server, and a fixture that had copied the value would have gone on stubbing the stale one into a stub that accepted it happily. The check runs on the pending path *and* the live path, so the agreement is asserted every run rather than at authoring time, and the run surfaces the vector's `$corrected` flag as a note so nobody re-derives the old spelling.

`fetchStub` records every call before answering it, so a fixture can assert what was sent as well as what came back, and it **throws** on a call past the end of its queue rather than inventing a reply -- an island that made one more call than the fixture planned has learned something and must not be told it passed. It matches the procedure route by *shape*, the prefix plus exactly one non-empty segment, because a procedure's id is a v4 uuid minted at macro-expansion time: it is neither knowable from the client side nor stable across rebuilds.

## Adding a fixture

Three files under `fixtures/<name>/`:

- `fixture.json`: the route, the mount selector, the island name, the bootstrap's event list, and an `expected` block. Every number the run compares against lives here, so a change in behaviour is a one line edit in a reviewable place. A fixture with components should declare `keys` (relative to the render id, in allocation order) and `components`; one with more than one template should declare `templates`; one that calls a procedure should declare a `wire` block naming its vectors.
- `ssr.html`: written by `capture-ssr.mjs`, never by hand.
- `interact.mjs`: exports `interact` for the working island and `dead` for what the `key` control should see. Both are awaited, so either may be `async` -- which it has to be for anything involving time, such as a debounce.

Also add an `island:<name>` subject to `budgets.json`, `pending` until the island exists. `run.mjs` asserts the two pendings agree, so a fixture that went live while its budget stayed pending -- shipping an unmeasured chunk -- fails rather than passing quietly.

Then add the page to demo-app if it is a new route. The run discovers fixture directories, so nothing here needs editing.

### A fixture whose island does not exist yet

Give `fixture.json` a `pending` block with `why`, `mirrors` (a `keys-nested.json` case name, or `null` for an island with no components and so no nesting to mirror), `flipsWhen` (the ordered list of conditions) and `thenRun`. `capture-ssr.mjs` then skips the route instead of 404ing on it, the negative controls are skipped, and `run.mjs` checks everything that does not need the server's bytes: that the declared key sequence is one the scheme can produce, that it is in allocation order, that it implies the declared number of boundaries, that it matches the oracle case it mirrors, that its budget subject is pending too, and -- if the island calls a procedure -- that its whole declared wire agrees with the spec.

That is the point of the mechanism. A pending fixture is not a skipped one, and its expectations are validated before the island lands rather than written in a hurry when it does. `fixtures/nested/` and `fixtures/search/` are the worked examples: `nested` is validated against the key oracle, and `search` -- which has no components and so nothing to nest -- is validated against the procedure wire instead. Delete the `pending` block to go live, and run `node budgets.mjs --update` to record the island's first measured size.

## Known limits

**Templates are attributed to an island, not counted.** They are module-scoped `const tmpl$h… = _$template(…)` bindings shared by every island in the bundle, so counting them answers a different question -- and not academically: the moment a second island landed in demo-app's bundle, the counter fixture's true claim of one template started reading as four, and it was the fixture that failed rather than anything about the counter. `islandTemplates` follows transitive reachability from `__island_<name>` instead. It reads identifiers, never positions, because the emitter's declaration order is its own business.

**Which template is the island's own was a wrong prediction, and is now measured.** `expected.templates` declares the count and `expected.islandTemplate` names which one to compare, defaulting to 0. The scaffold assumed 0; the emitter declares the two *component body* templates before the island's own, so nested's island root is index **2**. That is why the run, when the named template does not match, tries every other one and reports by index which does: "the template does not describe the server's tree" and "the templates are declared in a different order than the fixture assumed" are completely different findings, and telling them apart by hand from a tree diff is miserable. The diagnostic paid for itself on the first live run. `fixtures/search/` still declares `islandTemplate: 0`, labelled as unverified -- given the evidence from `nested`, a loop-body template is likely declared first there too.

**Pairing a template with its hydration key is still open.** The comparison above is against the island root only; `lib/keys.mjs`'s `rootsByKey` is the half that finds the other roots, and the pairing itself is the work left.

**One island per fixture.** The loader hydrates every island on the page, and the run watches the whole document, so a second island on the same page would be hydrated but not asserted about.

**A stale capture is reported, not repaired.** If the demo-app binary is older than its sources, the run says so, skips the one check that depends on the page's own delivery, and names the file to rebuild for. Everything else reads the runtime, the loader and the compiled island from disk, so it is unaffected.
