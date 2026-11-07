# 09 — SVG

## Why the two are equivalent

Element and attribute names are written identically. Topcoat's attribute names
already allow `-` and `:`, and element names are taken verbatim, so
`linearGradient`, `gradientTransform` and `stroke-width` all survive with no
escaping or renaming. The templates match character for character.

## Deliberate differences

None in syntax. Every difference here is in what the *emitter* must infer, and
that is the point of the family.

## What the reference does that is worth pinning

The four templates are not treated alike:

```
_$template(`<svg width=400 height=180><rect stroke-width=2 x=50 y=20 width=150 height=150>`)
_$template(`<svg width=400 height=180><rect width=150 height=150>`)
_$template(`<svg><linearGradient gradientTransform=rotate(25)><stop offset=0%>`)
_$template(`<svg><rect x=50 y=20 width=150 height=150></svg>`, false, true, false)
```

Three facts fall out:

1. **A template rooted at `<svg>` needs no namespace flag.** `<svg>` is an
   ordinary HTML element as far as the parser is concerned, so the first three
   get no boolean arguments at all — consistent with CONTRACT-DOM 2.4, which
   says the three booleans are appended only when at least one is true.
2. **A template rooted at an SVG-only element is wrapped and flagged.**
   `rootRect` becomes `<svg><rect …></svg>` with `isSVG: true`. The wrapper is
   synthetic: it exists so `innerHTML` parses the fragment in the SVG namespace,
   and the extra unwrap depth is CONTRACT-DOM 2.2. An emitter that walks this
   template without knowing about the wrapper lands one level too high.
3. **The closing `</svg>` is written out**, unlike every other template in the
   corpus, which leaves trailing tags open. The wrapper is closed because the
   unwrap logic counts on it.

Dynamic SVG attributes go through `setAttribute`, not `setProperty` — three of
them share one `effect` and a `_p$` record (CONTRACT-DOM 6.2), with keys `e`,
`t`, `a` from the base-53 frequency-ordered alphabet (6.3).

## view-dom features needed

- static and dynamic attributes on SVG elements — **present** syntactically
- namespace detection — **present.** `TemplateData` carries `is_svg`,
  `Template::finish` sets it when the root is an SVG-only element and prefixes
  the literal `<svg>` wrapper, and the backend roots the walk at the wrapper's
  first child so the runtime's extra unwrap lines up. All four templates match
  the reference byte for byte, flags included.
- **NOT-YET-LOWERABLE:** `setAttributeNS` for `xlink:` and `xml:` names. No case
  in this family needs it.
- **NOT-YET-LOWERABLE:** the shared-`effect` / `_p$` record form for multiple
  dynamic attributes on one element. view-dom emits one `::view_abi::effect` per
  hole, and records the grouping in `Hole::effect_group` for the backend to
  honour later.
