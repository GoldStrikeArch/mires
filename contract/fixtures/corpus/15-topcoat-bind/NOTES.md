# 15 — `:name=$(expr)` bind attributes

**LOWERS.** `:name=$(expr)` lowers through `lower/bind_attribute.rs:9`, the name
decides the sink, and a `$(...)` value is reactive where a `(...)` value is
written once. Only a dynamic bind-attribute NAME is refused
(`lower/bind_attribute.rs:16`).

The per-family L2 verdict is `contract/fixtures/l2-status.json`, enforced by
`contract/harness/check-l2-status.mjs`. It is not restated here: a status
sentence in a NOTES.md is exactly what went two waves stale before wave 6.

## Why the two are equivalent

A bind attribute is an attribute kept in sync with a reactive expression: the
server renders the initial value into the template, the browser re-applies it
when a signal the expression read changes. That is exactly what a Solid dynamic
attribute does, so `:<name>=$(expr)` ≡ `<name>={expr}`.

## Deliberate differences

**`:` is what makes the attribute reactive; JSX has no such marker.** In JSX
every `{expr}` attribute is reactive by default and the compiler decides whether
to wrap it in an effect by looking at the expression. Topcoat is explicit:
`name=(expr)` is evaluated once, `:name=$(expr)` re-applies. The explicit form is
easier to lower — no dynamism analysis — but it means the two languages disagree
about the *default*, and a mechanical translation in either direction has to look
at the expression to get it right.

## What the reference does that is worth pinning

This family exists mostly to pin the **sink** each name gets, because that is
what `HoleKind` names and `view-dom` does not yet decide:

| name | reference sink | why |
|---|---|---|
| `hidden` | `setBoolAttribute` | in `BooleanAttributes` |
| `value` | `setProperty` | in `Properties` |
| `checked` | `setProperty` | in `Properties` |
| `class` | `className` | aliased |
| `style` | `style` | dedicated helper |
| `disabled` | `setBoolAttribute` | in `BooleanAttributes` |

Six names, five different runtime functions. `view-dom`'s `attribute_hole`
produces `HoleKind::Attribute` for all of them, so every row here is currently
wrong in a different way. The `properties.json` fixture is the table to drive
this from — it is already extracted and pinned.

Also worth noting from `two_way`: the two dynamic sites on the same `<input>`
(the `:value` bind and the `@input` handler) do **not** share an effect, but two
dynamic *attributes* on one element would (CONTRACT-DOM 6.2).

## view-dom features needed

- **LANDED:** bind attributes (`lower/bind_attribute.rs:9`); reactive attribute
  holes — `$(...)` is reactive and `(...)` is written once, which
  `dom_writer.rs`'s `a_bind_attribute_is_reactive_only_when_written_with_a_dollar`
  pins; the per-name sink table, `class` and `style` reaching the diffing helpers
  and a property name being assigned; reactive holes on one element sharing an
  effect group (CONTRACT-DOM 6.2).
- **STILL REFUSED:** a dynamic bind-attribute name
  (`lower/bind_attribute.rs:16`).
- The one-syntax-per-reactivity split is a language difference from JSX, not an
  emitter defect, and it is the standing `write-once-vs-always-reactive` cause in
  `examples/dom-tests/deltas.json`.
