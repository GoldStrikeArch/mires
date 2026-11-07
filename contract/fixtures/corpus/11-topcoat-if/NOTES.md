# 11 — `if` / `else` with markup bodies

**LOWERS.** A markup-bodied `if`/`else if`/`else` is a reactive child hole whose
closure chooses a branch (`lower/node.rs:21-24`). Only the ATTRIBUTE-LIST form is
still refused (`lower/attributes.rs:37`).

The per-family L2 verdict is `contract/fixtures/l2-status.json`, enforced by
`contract/harness/check-l2-status.mjs`. It is not restated here: a status
sentence in a NOTES.md is exactly what went two waves stale before wave 6.

## Why the two are equivalent

Topcoat's `if` with markup bodies and JSX's `{cond ? <a/> : <b/>}` both choose
between two subtrees at one position. The reference compiles the ternary to a
`memo`-wrapped accessor handed to `insert`, and `<Show>` to a `createComponent`
call over the same accessor.

Two JSX forms are recorded rather than one, because they are not the same:

- **the ternary** is the closer structural match — one insert, two branches, no
  component;
- **`<Show>`** is what a Solid author actually writes, and it is what
  `builtIns: ["For", "Show"]` makes the plugin auto-import from `moduleName`.

## Deliberate differences

**Topcoat's `if` is a statement; JSX's conditional is an expression.** In `view!`
each branch emits *nodes into the surrounding body*, so a branch may emit zero
nodes, one, or several with no wrapper. A JSX ternary must produce exactly one
value, so several nodes need an array and zero needs `null`. The `if_only` case
(no `else`) is the clearest instance: `view!` just emits nothing, JSX has to say
`{cond && <a/>}` and yield `undefined`.

**Attribute-position `if` has no JSX form at all.** `attr_if` in `view.rs` emits
*two attributes* (`aria-current` and `class`) from one conditional. The JSX file
can only manage a conditional value for a single named attribute. This is the
first construct in the corpus where the two languages genuinely do not line up,
and it is why `attr_if`'s JSX counterpart is annotated rather than claimed to be
equivalent.

**Reactivity is the unresolved part.** In Solid the conditional is reactive: the
memo re-runs and `insert` swaps the subtree. Topcoat's `if` is evaluated once on
the server. A `$()`-driven `if` — reactive branch selection — has no syntax yet.
So the "equivalence" here is first-render equivalence only.

## view-dom features needed

- **LANDED:** `if` in a view body; one template per branch plus an `insert` at
  the shared anchor; the `memo` wrapping of a dynamic test (CONTRACT-DOM 6.4) —
  `view_abi::cond` hands the test over as its own closure and the backend emits
  `_$memo(() => test())`, recorded as `memo-hoisting-for-a-conditional` under
  `$measuredAndRetired` in `examples/dom-tests/deltas.json`.
- **STILL REFUSED:** `if` in an attribute list (`lower/attributes.rs:37`).
- The emitter claimed the `ReactiveScope` key before erroring, so site numbering
  did not shift when the feature landed. It did not.
