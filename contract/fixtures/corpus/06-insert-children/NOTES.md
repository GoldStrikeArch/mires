# 06 — inserted children and where their anchors land

The family that decides walk paths. Everything else in the corpus depends on
getting this one right.

## Why the two are equivalent

`{children}` and `(children)` are the same hole in the same position. The cases
are chosen so that each one moves the anchor somewhere different: sole child,
after text, before an element, between two elements, and two adjacent holes.

## Deliberate differences

**JSX child spread (`{...children}`) has no Topcoat equivalent.** Upstream's
fixture leans on it heavily (`template13` through `template19`). `view!` has no
child-spread syntax at all — a single `(expr)` can produce several nodes only by
the value's own `NodeViewParts` impl, which is a runtime decision, not a syntactic
one. Those cases are out.

**`children={...}` as an attribute has no Topcoat equivalent either.** Topcoat
passes child content as the `child` field of a component's props struct, written
as trailing view nodes at the call site, which is family 07.

**`{children()()}` and `{expression(), "static"}` are dropped.** Double call and
comma-expression are JS-isms with no Rust reading.

## What the reference does that is worth pinning

```
<div>                          only        — no anchor: sole child needs none
<div>Hello                     afterText   — no anchor: hole is last
<div><span>                    before      — no anchor: hole is first
<div><span></span><span>       between     — no anchor either
```

Every one of these emits **no `<!>` anchor at all**. The plugin emits a
placeholder only on two independent conditions (CONTRACT-DOM 4.1), and "the hole
is the first or last child" is not one of them — the walk reaches
`firstChild`/`nextSibling` and `insert` is given `null` or the following element
as its marker. `view-dom`'s `at_sole_child` check covers only the narrowest of
these, so the anchor rule is the first thing to verify against the recorded
traces.

Adjacent holes DO share one anchor, except when hydrating (CONTRACT-DOM 4.2) —
which is why `expected.reference.hydratable.js` is worth diffing against
`expected.reference.js` for this family specifically.

## view-dom features needed

- `(expr)` child holes with anchor insertion and walk-path shifting —
  **present**
- `$(expr)` reactive child holes — **present**
- **NOT-YET-LOWERABLE:** nothing. This family should compile today.
