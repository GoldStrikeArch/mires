# 10 — custom elements

## Why the two are equivalent

Dashed element names and dashed attribute names are ordinary Topcoat syntax —
`<my-widget data-widget-id="profile">` needs no escape hatch. The static cases
map exactly.

## Deliberate differences

**`prop:` and `attr:` have no Topcoat spelling.** JSX uses them to override the
compiler's guess about whether a name is a DOM property or an HTML attribute.
Topcoat lowers every `name=(expr)` to `HoleKind::Attribute`, full stop. So:

- `attr:my-attr={data}` → written as plain `my-attr=(data)`, which happens to
  agree because the guess for a dashed name is "attribute" anyway.
- `prop:someProp={data}` → **has no Topcoat equivalent and is omitted from
  `view.rs`.** Writing `someProp=(data)` would compile to a `setAttribute` where
  the JSX compiles to a `setProperty`, so it would be a silently wrong pairing
  rather than a documented one.
- `bool:` (upstream's `template42`–`template61`) is likewise absent.
- `is="my-element"` and `<slot>` are static attributes and elements, so they need
  no special handling and are not interesting here.

**`contextToCustomElements` changes the template call.** All three templates in
this family are emitted as:

```
_$template(`<my-element>`, true, false, false)
```

The leading `true` is `isImportNode`. The preset sets
`contextToCustomElements: true`, which forces `document.importNode` instead of
`cloneNode` so a custom element upgrades correctly on insertion. This is the only
family in the corpus where the flag appears.

The same option has a second half with no Topcoat meaning: the reference also
writes `_el$._$owner = _$getOwner()` on every custom-element root, six times over
this family. That is solid's own back-channel for handing a custom element its
reactive owner, and nothing on the Topcoat side would read it.

## view-dom features needed

- dashed element and attribute names — **present**
- `isImportNode` on the template call — **present.** `Template::finish` sets
  `is_import_node` when any element in the template has a dashed tag name, and
  all three templates match the reference byte for byte on all four
  `_$template` arguments.
- **NOT-YET-LOWERABLE:** the attribute-versus-property decision, including any
  Topcoat syntax for overriding it (there is none today). Every `name=(expr)` is
  `HoleKind::Attribute`, so ours is a `setAttribute` where the reference writes a
  property.
