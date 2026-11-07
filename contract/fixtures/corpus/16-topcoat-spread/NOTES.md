# 16 — attribute spreads

**LOWERS, AND IS DELIBERATELY NOT COMPARED.** `lower/attributes.rs:23`
constructs `HoleKind::Spread` for `<div (attrs)>`. What a spread's VALUE is has
no answer yet: the corpus writes `topcoat::view::Attributes`, which the spike does
not build. So `examples/dom-tests/16_topcoat_spread.rs` pins the emitted shape
and not runtime behaviour, and it carries no `.family` file for that reason —
this family's row in `contract/fixtures/l2-status.json` is `EXCLUDED`, with that
as the written `why`. The L1 goldens still run.

## Why the two are equivalent

`<div (attrs)>` and `<div {...attrs}>` both apply a collection of attributes
whose names are not known at compile time. The reference compiles both to
`_$spread(el, props, isSVG, skipChildren)`.

## Deliberate differences

**Topcoat's spread carries an `Attributes` value, not a plain object.**
`topcoat::view::Attributes` is a map-like collection with unique keys, built by
the `attributes!` macro. JSX spreads any object. The practical consequence is
that Topcoat's spread cannot contain event handlers or children, where a JSX
props spread can — upstream's `insertChildren` fixture relies on
`{...dynamic}` carrying a `children` key, which Topcoat's `Attributes` has no
way to express.

## The ordering rule — the load-bearing finding

The reference does **not** treat a spread as opaque to the template. Static
attributes before the first spread stay baked into the template string; static
attributes after it are hoisted out into `mergeProps`:

| source | emitted template | spread argument |
|---|---|---|
| `{...attrs}` | `<div>` | `attrs` |
| `{...attrs} id="main"` | `<div>` | `mergeProps(attrs, {id: "main"})` |
| `id="main" {...attrs}` | `<div id=main>` | `attrs` |
| `class="base" {...attrs} id="main"` | `<div class=base>` | `mergeProps(attrs, {id: "main"})` |

Read the last row carefully: `class` stayed in the template and `id` did not,
from the same element. The rule is positional — everything after the first
spread must be applied at runtime so it can win over whatever the spread
contains — and it is exactly the kind of thing an emitter gets wrong by treating
attributes as an unordered set. `topcoat-view`'s `Attributes` documentation says
"do not rely on render order for attributes", which is true of the *collection*
but emphatically not true of the spread's position in the attribute list.

**`skipChildren` is the fourth argument** and is `true` only for
`spreadWithChildren`, where the element has static children the spread must not
clobber.

**A spread on a component is a different call entirely**: `spreadOnComponent`
compiles to `_$createComponent(Child, _$mergeProps(props, {name: "John"}))`, with
no `_$spread` at all. So spread support is really two features, and the component
one is blocked behind family 07.

## view-dom features needed

- **LANDED:** attribute spread; `HoleKind::Spread` construction.
- **STILL MISSING:** `mergeProps` emission; the positional split above;
  `skipChildren`; component spread; and a Rust type to write a spread's value
  with, which is what keeps the family out of the L2 comparison.
