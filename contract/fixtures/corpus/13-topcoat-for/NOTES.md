# 13 — `for pat in expr` with a markup body

**LOWERS.** `for pat in expr` with a markup body is a reactive child hole
(`lower/node.rs:25-28`): `view_abi::list` starts the list, the loop runs where it
is written, `view_abi::push` appends each row, and one `_$insert` takes the list.
Only the ATTRIBUTE-LIST form is still refused (`lower/attributes.rs:41`), as is a
dynamic attribute NAME (`lower/attribute.rs:13`), which `attr_for` also needs.

The per-family L2 verdict is `contract/fixtures/l2-status.json`, enforced by
`contract/harness/check-l2-status.mjs`. It is not restated here: a status
sentence in a NOTES.md is exactly what went two waves stale before wave 6.

## Why the two are equivalent

All three forms render the body once per item into the same parent. Both JSX
forms are recorded because they are not interchangeable:

- **`.map()`** is the structural match. It builds an array eagerly and hands it
  to `insert`, which is what a Rust `for` loop over an iterator does.
- **`<For each={…}>`** is what a Solid author writes and what
  `builtIns: ["For", "Show"]` auto-imports from `moduleName`. It is keyed and
  reconciling: on a change it moves existing DOM nodes rather than rebuilding
  them.

## Deliberate differences

**Topcoat's `for` is not keyed and does not reconcile.** It runs once per render,
server-side. So `for` ≡ `.map()` and `for` ≢ `<For>`, and the two JSX files
compile to visibly different output — `_$createComponent(For, …)` versus an
array `insert`. Calling `<For>` the equivalent would be wrong; it is recorded as
the idiomatic-Solid reference point, not as the target.

**This is permanent, decided in wave 6, and it is why this family can never reach
MATCH.** The obvious repair is to make `view_abi::push_keyed` update the retained
node from the freshly built row. It was examined and rejected on three findings,
recorded in full in
`contract/parity/fixtures/dashboard/fixture.json`'s `expected.$notKeyed`. The
deciding one is that the repair is aimed at the wrong thing: `<For>` clones a
row's template once per row EVER, an eager Rust loop clones once per row PER
RENDER, and that happens before any reconcile is reached. So the standing
`keyed-list-vs-a-rust-loop` cause is not a deferral waiting on a fix — it is a
consequence of `for` being a Rust loop, and `contract/fixtures/l2-status.json`
records this family as DIFFERS attributed to exactly that.

The sharper rule the same investigation produced, which belongs with this family
because it is the one people get wrong: a keyed row's changing parts must be
reactive holes **of the row's own template**. A plain `( )` hole in a keyed row is
`Fill::Once` — it emits a bare `_$insert` with no effect — so it is written once
to a node the reconcile then discards.

**`fallback` has no equivalent.** `<For each={x} fallback={<li>None</li>}>`
renders the fallback when the list is empty. In Topcoat that is an `if` around
the loop — a different construct, in a different family.

**No item index.** `<For>`'s callback receives `(item, index)`. Topcoat's `for`
binds only the pattern; an index needs `.enumerate()`, which changes the
iterator's item type and therefore the pattern.

**Attribute-position `for` has no JSX form whatsoever.** `attr_for` in `view.rs`
emits zero or more *attributes* from a loop, naming each one
(`(name)=(value)`). The nearest JSX construct is a spread of a prebuilt object,
which is opaque to the compiler where the loop body is not — so the JSX file
records a spread and says so, rather than pretending.

## view-dom features needed

- **LANDED:** `for` in a view body; a per-iteration template instance; the array
  `insert` form.
- **STILL REFUSED:** `for` in an attribute list (`lower/attributes.rs:41`) and
  `(name)=(value)`, a dynamic attribute *name* (`lower/attribute.rs:13`), so
  `attr_for` still needs two features.
- **THE OPEN QUESTION FOR THIS FAMILY** is not lowering, it is keying: the
  recorded reference is JSX's keyed `<For>` and Topcoat's `for` is a Rust loop.
  See "Deliberate differences" above and `contract/G2-CHECKLIST.md`, "The
  unkeyed list".
