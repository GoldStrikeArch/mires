# Wave-1 contract slice — running notes (banking discipline)

Owner: contract agent (owns `contract/` only; everything else read-only; no cargo; no git commit).
Brief: (1) nested-component hydration-key oracle from pinned solid/dom-expressions,
(2) corpus family 07 redefinition for the eager-props equivalence + 07b nested fixture,
(3) standing debts (CONTRACT-DOM.md §6.3 "base 53" vs 54-char alphabet; `run-all.mjs --check` drift).

Status legend: TODO / WIP / DONE.

| Item | Status |
|---|---|
| 1. Nested-context key oracle | DONE |
| 2. Family 07 redefinition + 07b | DONE |
| 3. Standing debts + drift check | DONE |

## Log

- (start) Bank file created. Read the plan: Wave-1 contract column = "keys.json nested-context
  oracle + family-07 redefinition"; Track A §A1 needs the oracle for the SSR `Formatter`'s
  `enter_component()`, which must reproduce solid's exact nested-prefix scheme. Risk #5 in the
  plan: "must match solid's nextHydrateContext exactly; oracle-extend keys.json BEFORE building
  (the prefix-free-letter lesson)."

### Item 1 — the nested-key scheme, read off the pinned sources

Pins: solid-js 1.9.14, dom-expressions 0.40.8 (see `contract/upstream.lock`).

Two independent implementations of the SAME scheme, one per side:

CLIENT
- `solid-js/dist/solid.js:121-132` — `sharedConfig` = `{context, registry, effects, done,
  getContextId(), getNextContextId()}`.
- `:133-137` — `getContextId(count)` = `sharedConfig.context.id + letter + String(count)` where
  `letter = len ? String.fromCharCode(96+len) : ""`, `len = String(count).length - 1`.
- `:141-147` — `nextHydrateContext()` = `{...sharedConfig.context, id: sharedConfig.getNextContextId(), count: 0}`.
- `:1274-1285` — `createComponent(Comp, props)`: if `hydrationEnabled && sharedConfig.context`,
  save `c = context`, `setHydrateContext(nextHydrateContext())`, `untrack(() => Comp(props||{}))`,
  then `setHydrateContext(c)`.
- `dom-expressions/src/client.js:615-617` — `getHydrationKey() { return sharedConfig.getNextContextId(); }`
- `client.js:264-280` — `getNextElement(template)` calls `getHydrationKey()` once per template root
  (only while `isHydrating()`; `client.js:331`).
- `client.js:244-262` — `hydrate()` seeds the root context `{ id: options.renderId || "", count: 0 }`.

SERVER
- `solid-js/dist/server.js:369-379` — same `sharedConfig`, but the two getters THROW outside a
  hydrating context instead of returning junk.
- `:380-384` — `getContextId` byte-identical to the client's.
- `:388-394` — `nextHydrateContext()`, same, plus an `undefined` guard when there is no context.
- `:398-407` — `createComponent`: nests when `sharedConfig.context && !sharedConfig.context.noHydrate`
  (the client gates on `hydrationEnabled` instead; `noHydrate` is the `NoHydration` flag, §11.3).
- `dom-expressions/src/server.js:522-525` — `getHydrationKey()` = `context && !context.noHydrate && getNextContextId()`.
- `server.js:421-424` — `ssrHydrationKey()` wraps it as ` data-hk="…"`.
- `server.js:37-42` / `:141-146` — root context `{ id: renderId || "", count: 0 }`.

THE SCHEME (what `Formatter::enter_component` must reproduce)

A hydration context is `(id: String, count: u32)`. Every hydratable template root consumes one
slot: `key = id + letter(digits(count)-1) + count`, then `count += 1`. `letter(0) = ""`,
`letter(n) = (b'a' + n - 1)` i.e. 1 digit -> "", 2 -> "a", 3 -> "b", 4 -> "c", …

A component boundary consumes ONE slot of the PARENT context and turns that slot's key into the
child context's id:

    enter_component():                       // returns the saved parent context
        child_id   = key_of(parent.id, parent.count)   // the parent's next key
        parent.count += 1                              // the slot is spent
        push { id: child_id, count: 0 }
    exit_component(saved):
        restore the parent context (whose count already advanced)

Three precision points, each a trap:
1. The parent's counter advances by exactly ONE per component, and the advance SURVIVES the
   restore. In JS this is because `nextHydrateContext` mutates the parent object in place via
   `this.context.count++` and `createComponent` restores the same object reference (not a copy).
   A Rust implementation that saves a *copy* of the parent context and restores it would replay
   the component's slot for the next sibling and shift every later key.
2. Evaluation order inside `{...ctx, id: getNextContextId(), count: 0}`: the spread snapshots the
   parent's fields FIRST (pre-increment), then `id` is computed (incrementing the parent), then
   `count: 0` overrides the spread's copied count. So the child inherits the parent's other
   fields (e.g. `noHydrate`) but always starts counting at 0.
3. The child's FIRST element key is `child_id + "0"` — the concatenation of the parent's spent
   slot key and `0`. The letter is what keeps this unambiguous (§9.1); with all counters at 0 a
   depth-3 chain yields "0", "00", "000" and the first leaf key "0000".

Not-a-trap-but-worth-recording: the client nests unconditionally on `sharedConfig.context` and
does NOT consult `sharedConfig.done`, so contexts keep nesting after a hydration mismatch;
`enableHydration()` must have run or `createComponent` does not nest at all (`:1275`).

### Item 1 DONE

New: `harness/extract-nested-keys.mjs` -> `fixtures/keys-nested.json` (369 KB, generated).
Registered in `harness/run-all.mjs` STEPS right after `extract-keys.mjs`, so the `--check` drift
gate covers it.

How it is driven (both sides, same trees):
- CLIENT: `dom-expressions/src/client.js` `getNextElement` with a recording registry that answers
  every lookup, so it takes the real "found the node" branch; component boundaries via solid's
  client `createComponent` after `enableHydration()`. sharedConfig comes from `harness/rxcore.mjs`,
  i.e. the SAME solid instance client.js's "rxcore" seam resolves to, so there is one shared
  context object. Root context seeded exactly as `hydrate()` does.
- SERVER: `solid-js/web/dist/server.js` `renderToString` over `ssr(["<div", "></div>"],
  ssrHydrationKey())` elements plus its own `createComponent`. The `data-hk` attributes in document
  order are the server's answer.
- Per case the two key sequences are compared by sha256. Any mismatch exits non-zero.

Numbers: 102 cases, 17799 allocations of which 180 are component boundaries, 47 distinct
(parentContextId, parentCount) -> childContextId triples, 11 asserted boundary checks, 0
client/server mismatches. Runs in ~0.1 s.

Case groups: flat (5), one-component (40: pad x inner matrix), siblings (8), depth (13),
shape (10), renderId (5, incl. renderId "0" which looks like a count), boundary (13: parent
counters 9/10/99/100/999/1000/10000 x inner 10/100), inner-boundary (12: the CHILD context's
counter crossing at a nested boundary).

Worked examples now pinned:
- `C(e)` -> ["00"]; `C(C(C(e)))` -> ["0000"]; `C(e) e` -> ["00", "1"]; `C() e` -> ["1"].
- `C(e) C(e) C(e)` -> ["00", "10", "20"] (the spend-survives-restore case).
- `e*10 C(e*11) e` -> [..., "a100".."a109", "a10a10", "a11"]: both counters cross the letter
  boundary at once. "a10a10" = child context id "a10" + letter "a" + count 10.
- `C(e*10 C(e) e) e` -> [..., "0a100", "0a11", "1"].
- renderId "z" on `C(e C(e))` -> ["z00", "z010"].

CONTRACT-DOM.md: added 9.5 (a boundary spends one parent slot and the spend outlives the
component, with the Rust pseudocode and the exit_component copy-restore trap), 9.6 (the letter
comes from the counter of the allocating context, not depth and not child index), 9.7 (the two
implementations agree, so the oracle is two-sided), 9.8 (an empty component still spends a slot),
9.9 (nesting continues after a hydration mismatch). Also corrected 9.2's `nextHydrateContext`
citation from `:143-149` to the actual `:141-147`, and extended 9.2/9.4 "Pinned by" to cite the
new fixture. CONTRACT-DOM.md stays ASCII-only with no em-dashes (verified: 0 non-ASCII bytes).

### Item 2 DONE

Rewrote `fixtures/corpus/07-components/` and added `fixtures/corpus/07b-nested-components/`.

THE EQUIVALENCE, in one sentence (now in 07/NOTES.md): a Topcoat client component call is a
Solid `createComponent` call whose props object is DESTRUCTURED at the callee's boundary.
Destructuring is how a Solid component reads a prop once rather than on every access, so it is
the exact Solid spelling of a Rust fn taking ordinary arguments. Solid users are warned off it
because it loses reactivity; Topcoat loses the same reactivity structurally and buys it back by
passing an accessor (`count`) instead of a read (`count()`), which is the `Sig<f64>` parameter.
Every component in both jsx.jsx files destructures, which is what makes the two sides comparable.

07 view.rs is the CLIENT half the dom emitter sees: plain `fn ... -> view_abi::Node`, props as
ordinary args, `child: view_abi::Node` for child content, `count: view_abi::Sig<f64>` and
`read: impl Fn() -> f64` for the two reactive-prop spellings. Ten fns: badge, card, live_count,
readout, static_props, dynamic_prop, with_children, reactive_prop, closure_prop,
sibling_components. Case-for-case table in NOTES.md.

07 jsx.jsx: components DEFINED locally (not free bindings) so their bodies compile and the
corpus records their templates and holes. Also nothing left undefined at all, which fixed a real
measurement bug: run-trace heals a free binding into a Proxy, and trace.mjs's `value()` turns a
Proxy into a function, which JSON.stringify then DROPS -- so under free bindings the `comp` field
of every createComponent record and the `accessor` field of a reactive insert silently vanished.
The old 07 trace had exactly that. New trace: 45 records, freeBindingsHealed [], every field
present. Regenerated with gen-corpus.mjs. 9 templates in both presets.

The plugin's getter rule, probed: a static string prop, a bare-identifier prop, an accessor prop
and an arrow prop are all PLAIN props; only a JSX-element child becomes `get children()`. There
is no JSX spelling of eager element children written inline -- `children={<p/>}` becomes a getter
too. `children={kid}` with the child hoisted into a const IS plain, and that is the exact eager
equivalent.

PERMANENT DELTAS with whys (full prose in 07/NOTES.md): no props object; a prop evaluates exactly
once at the call site in argument order; reactivity is in the callee's signature not at the call
site; child content is eager so it is built BEFORE the boundary; child content can be used once
(`Node` is not Clone); `child` is a reserved parameter name; components are called not tagged; no
`untrack` wrapper (a component call is not inside a tracking scope).

07b is the hydration-key half. view.rs has 5 call-site fns each citing its keys-nested.json case:
nested_in_body ["00","010"], children_component ["00","10"], depth_three ["00","010","0110"],
siblings_in_body ["00","010","020"], nested_in_element ["0","10","110"]. jsx.jsx uses the hoisted
`children={kid}` spelling so 07b is fully L2-comparable; 49 records, freeBindingsHealed [], no
getters anywhere.

THE ONE REAL DIVERGENCE FROM SOLID, quantified and driven (keys-nested.json field
`getterChildren`, 5 rows):
  lazy, body allocates own root then reads props.children:  <div data-hk="00"><span data-hk="01">
  lazy, body destructures at entry:                         <div data-hk="01"><span data-hk="00">
  eager (Topcoat):                                          <div data-hk="10"><span data-hk="0">
  eager + sibling after:                                    ... plus <span data-hk="2">
  lazy + sibling after:                                     ... plus <span data-hk="1">
Same source, same props: under the getter model the child's key depends on WHERE IN ITS BODY the
callee reads the prop, which a compiler cannot see. Under eager child content the order is fixed
by the source, which is what lets a Rust Formatter reproduce the emitted JS's keys without
running it. Accepted because Topcoat owns both sides: the obligation is server/client agreement
(CONTRACT-DOM 9.7), not byte-equality with solid's layout.

DELTAS PRE-WRITTEN in fixtures/corpus/deltas.json (2 per family, 4 total, each why-carrying):
- `component-props-object` (ignore-field createComponent.props): no props object exists on the
  Topcoat side. States the PRECONDITION -- assumes `_$createComponent(thunk)` emission -- and says
  to delete the rule if the emitter synthesises a props object, since compare-trace reports an
  unmatched rule as STALE.
- `component-fn-identity` (ignore-field createComponent.comp): the reference passes the component
  function itself, so N calls to one component alias to one fn id; a thunk per call site does not.
  Verified in the new traces: Badge is `fn#0` three times in 07 and `fn#1` three times in 07b.
Both load correctly through `loadDeltas` (checked: 2 rules per fixture).

NOT expressible as a rule, so recorded in deltas.json `$notExpressibleAsRules` and in 07/NOTES.md:
family 07 `withChildren` will report exactly one `missing-in-b` for `template.clone` of the
children template. The reference has TWO clones because trace.mjs's `value()` (:186-190) forces
`get children()` when recording the props object and the destructure then forces it again (getters
are not memoised). Real solid clones once; Topcoat will clone once. It is a HARNESS artifact, not
a compiler divergence. Two ways out are written down (teach `value()` not to force getters, which
rebaselines every committed trace carrying a props object; or add a `drop-record` rule kind);
neither taken, deliberately.

Also updated `fixtures/corpus/README.md`: the family table (07 now 9/9/45, new 07b row 4/5/49),
the equivalence-level table (07 and 07b moved from "equivalence undefined" to "L2 expected"), the
paragraph that said 07 had no defined equivalence, the NOT-YET-LOWERABLE ledger's blocks column,
and the accepted-deltas section (1 rule -> 5, plus the `$notExpressibleAsRules` companion).

keys-nested.json grew to 108 cases: group `corpus-07b` (6 cases, names matching 07b's fn names)
plus the `getterChildren` section. Still 0 client/server mismatches, 11 boundary checks green.

### Item 3 DONE

The "base 53" debt: verified against the pinned source and FIXED. The alphabet
`plugin/src/shared/utils.js:417` is `"etaoinshrdlucwmfygpbTAOISWCBvkxjqzPHFMDRELNGUKVYJQZX_$"` =
54 characters, all distinct (52 letters in rough English frequency order, then `_` and `$`), so
`base = chars.length` is 54: ids 0..53 are single characters and id 54 rolls over to "te".
CONTRACT-DOM 6.3's heading now says base 54, the quoted code is byte-exact including the blank
lines inside the do-block, and the "Pinned by" line is tightened to
`__dom_fixtures__/attributeExpressions/output.js:183-197`, where the record is initialised as
`{ e: undefined, t: undefined, a: undefined }` and read back as `_p$.e/.t/.a` (verified).

Two more stale facts found and fixed while checking:
- CONTRACT-DOM 9.2 cited `solid/dist/solid.js:143-149` for `nextHydrateContext`; it is `:141-147`.
- contract/README.md's fixture inventory: added the `keys-nested.json` row, corrected "17
  construct families" to 18, and corrected "121 template trees" to 135 (51 hand-written + 84 from
  the corpus) and "All 70 pass the validator" to 84. Verified: the 10 validator rejections are all
  hand-written hazard cases, 0 corpus-derived templates are rejected.

DRIFT: `node --import ./register-loader.mjs run-all.mjs --check` -> all 12 steps green, "no drift:
committed fixtures match regenerated output exactly". The corpus table is 18 families, 18
compiled, 18 traced to completion. extract-nested-keys.mjs is registered in STEPS so the gate
covers keys-nested.json too.

Footprint: only `contract/` touched (12 modified, 4 added). No cargo, no git commit.

### Deviations from the brief

1. The nested oracle is a NEW fixture `keys-nested.json` driven by a NEW `extract-nested-keys.mjs`
   rather than an extension of keys.json (the brief allowed either). keys.json keeps pinning the
   ENCODING with hand-set contexts; the new one pins the NESTING by driving real component trees.
2. 108 cases rather than "~100", because the 07b group and the child-context boundary group were
   worth having.
3. 07b's jsx.jsx writes the child-content case as `children={kid}` (child hoisted into a const, a
   plain prop) rather than inline `<Card><Badge/></Card>`. Reason: the inline form compiles to a
   getter, which adds reference-only records mid-module and breaks the whole-module trace
   alignment, so 07b would drop from L2 to L1. The getter form is instead recorded in 07's
   `withChildren` case and driven in keys-nested.json's `getterChildren` section.
4. 07's jsx.jsx now defines every component locally and leaves NO free binding. This is a change
   of corpus style, made because free bindings were silently deleting trace fields (the Proxy /
   JSON.stringify interaction above). Worth considering for other families.
5. One divergence is not expressible as a deltas.json rule and is recorded in
   `$notExpressibleAsRules` rather than by inventing a `drop-record` rule kind, which would be a
   change to compare-trace's semantics that no real Topcoat trace has justified yet.
6. NEW FINDING the framework agent needs, beyond the brief: eager child content changes the key
   layout, because a child's template roots take slots in the CALLER's context instead of the
   callee's. `card(title: "x", badge(...))` gives keys "00" and "10" (siblings in the root
   context); solid's getter children give "00" and "010" (nested). Both layouts are driven in
   keys-nested.json. Self-consistency between Topcoat's Formatter and its emitted JS is what
   matters, so this is accepted, but `enter_component` must be called AFTER the child argument is
   built, not around the whole call expression.
