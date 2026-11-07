# 05 — conditionals as a value

A conditional in *expression* position: the hole is unconditional, only its value
is chosen. Markup-bodied `if` is family 11 and is a different construct.

## Why the two are equivalent

`{cond ? a : b}` and `(if cond { a } else { b })` are both "one child hole whose
value is computed by a branch". Rust's `if` is an expression, so this maps
directly and needs no new syntax.

## Deliberate differences

**`&&` becomes `else { "" }`.** Rust has no value-returning `&&` over mixed
types, so `{state.dynamic && good()}` is written `$(if dynamic { good } else { "" })`.
The reference compiles the JS form to a `memo`-wrapped conditional that yields
`undefined` in the false branch, where Topcoat yields an empty string. For
`insert` those differ: `undefined` clears the position, `""` writes an empty text
node. **This is a real behavioural difference and is not accepted as a delta** —
it is a design question the emitter has to answer, and the fixture exists to make
it visible rather than to paper over it.

**Optional chaining (`state?.dynamic`) is dropped.** Rust has no equivalent
operator; `Option` handling is a `match`, which is family 12.

## What the reference does that is worth pinning

The plugin's `wrapConditionals: true` (set in `PRESETS.dom`, deliberately absent
from `PRESETS.domHydratable`) decides whether a conditional gets a `memo`. It
wraps only when **both** the test and the branch are dynamic — CONTRACT-DOM 6.4.
So `static_test` and `dynamic_test` compile to visibly different code from
near-identical source, and the emitter needs the same two-part test rather than
"is there a conditional here".

## view-dom features needed

- `(expr)` and `$(expr)` child holes where the expr happens to be an `if` —
  **present**. Nothing about the conditional is special to view-dom; it is opaque
  Rust inside the hole.
- **LANDED:** the `memo` wrapping. `view_abi::cond` hands the test over as a
  closure of its own and the backend emits `_$memo(() => test())` plus an
  accessor that dispatches on it; both spellings of a conditional go through it,
  the markup-bodied `if`/`else if` of family 11 and a Rust `if` inside `$(...)`.
  Recorded as `memo-hoisting-for-a-conditional` under `$measuredAndRetired` in
  `examples/dom-tests/deltas.json`; the pin is this family losing three
  `memo`-only difference lines and family 11 losing all of its.
- A `memo` record that still reports ONLY IN A for this family is NOT that gap.
  It belongs to the reference-only `withMarkup` case, which has no `view.rs`
  counterpart — one of the three case asymmetries listed in
  `contract/G2-CHECKLIST.md`.
