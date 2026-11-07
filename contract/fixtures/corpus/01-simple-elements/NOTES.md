# 01 — simple elements

Fully static markup. No holes, no reactivity, no runtime calls beyond cloning.

## Why the two are equivalent

Both sides describe the same element tree. Topcoat writes HTML element names,
void elements, and attribute names exactly as HTML does, so the mapping is
character-for-character apart from quoting text nodes.

| JSX | view! |
|---|---|
| `<input id="entry" type="text" />` | `<input id="entry" type="text">` |
| `<span />` | `<span></span>` |
| `<style>{"div { color: red; }"}</style>` | `<style>"div { color: red; }"</style>` |

## Deliberate differences

- **Text must be quoted.** `Welcome` in JSX is `"Welcome"` in `view!`. A Rust
  macro cannot see unquoted prose as one token, so this is a syntax tax, not a
  semantic one.
- **`<span />` has no view! spelling for non-void elements.** Topcoat requires a
  matching close tag on anything that is not an HTML void element. The emitted
  template is identical either way — the reference compiler writes
  `<div><span><a></a></span><span>` for both.
- **The JSX comment node (`{/* … */}`) in the upstream fixture is dropped.**
  `view!` has no child-position comment, and the reference compiler erases it
  from the template anyway, so nothing is lost.

## Emitted templates

```
<div id=main><h1>Welcome</h1><label for=entry>Edit:</label><input id=entry type=text>
<div><span><a></a></span><span>
<div><style>div { color: red; }
```

Note what the reference compiler does and does not close. Trailing tags are left
open on purpose (CONTRACT-DOM 2.5); attribute values are emitted unquoted when
they contain no character that would need quoting. The Rust emitter must match
both choices, since the template string is compared byte for byte.

## view-dom features needed

All present. This family should be the first one the emitter compiles.

- literal text
- static elements, including void and raw-text (`<style>`) elements
- static attributes baked into the template HTML
