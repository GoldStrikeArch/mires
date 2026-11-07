# 03 — attribute expressions

Static attributes bake into the template HTML; `name=(expr)` becomes an
`Attribute` hole applied after cloning.

## Why the two are equivalent

`id={id}` and `id=(id)` are the same construct: an attribute whose value is
computed. Both sides also carry a static attribute on the same element, so the
fixture pins the split between what stays in the template string and what does
not.

## Deliberate differences

**Upstream's `classList={{...}}` and `style={{...}}` object forms are dropped.**
Topcoat has no object-valued attribute. Its `class!` macro assembles a
space-separated string and its `:style` bind takes a string, so the nearest
Topcoat construct produces a plain attribute value, not a per-key diff. Rather
than write a JSX equivalent that would compile to `_$classList(...)` — code
Topcoat can never emit from this syntax — the cases are simply not in the corpus.
They belong to a later wave that decides whether `class!` should lower to
`className` or to `classList`.

**Boolean attributes are spelled `disabled=""`, not `disabled`.** Topcoat's
`view!` guide is explicit that the literal empty-string form is preferred over
`disabled=(true)` because it folds into the static template. The reference
compiler emits `<input type=text disabled>` from bare `disabled`, and the same
from `disabled=""`, so the template agrees.

**`ref={link}` is absent.** Topcoat has no ref attribute.

## What the reference does that is worth pinning

```
<div id=main class=base>
<input type=text disabled>
<div id=main><a href=/>Welcome
<input>
```

- Attribute values are emitted **unquoted** whenever they contain nothing that
  needs quoting — `id=main`, `href=/`. This is a template-string detail the Rust
  emitter must reproduce exactly, not a formatting preference.
- `<input value={state.value} />` leaves the template as a bare `<input>`:
  `value` is in dom-expressions' `Properties` set, so it is applied as a DOM
  property, never as an attribute. view-dom builds `HoleKind::Attribute`
  unconditionally, so this case will diverge until the name-to-sink table lands.

## view-dom features needed

- static attributes — **present**
- dynamic attribute values (`name=(expr)`) — **present**, written once and not
  reactive. That is not a hardcoded gap any more: Topcoat has two syntaxes where
  JSX has one, and `(...)` means write-once while `:name=$(...)` means reactive
  (`lower/bind_attribute.rs:24-26`). It is why the reference trace carries an
  `effect` record where ours carries a bare `setAttribute`, and it is the
  standing `write-once-vs-always-reactive` cause in
  `examples/dom-tests/deltas.json` rather than a defect.
- **PARTIAL:** the `Properties` / `ChildProperties` / `Aliases` name-to-sink
  decision. The table landed — `view-dom/src/contract.rs::is_property` — and
  `HoleKind::Property` IS constructed, but only from `lower/bind_attribute.rs`.
  `lower/attribute.rs` still emits `HoleKind::Attribute` unconditionally, so
  `value=(v)` on an `<input>` lowers to `setAttribute` where the reference emits
  `setProperty`, while `:value=$(v)` reaches the property sink.
