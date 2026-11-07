# 14 — `let` bindings inside a view body

**LOWERS.** `let` in a view body is compile-time plumbing (`lower/local.rs:6`):
it introduces a Rust binding scoped to the rest of the body and contributes no
DOM, and it claims **no** key — it is a binding, not a reactive scope, so it does
not consume a `KeySite` ordinal. Only the ATTRIBUTE-LIST form is still refused
(`lower/attributes.rs:48`).

The per-family L2 verdict is `contract/fixtures/l2-status.json`, enforced by
`contract/harness/check-l2-status.mjs`. It is not restated here: a status
sentence in a NOTES.md is exactly what went two waves stale before wave 6.

## Why the two are equivalent

A `let` introduces a Rust binding scoped to the rest of the body. JSX has no
statement position at all, so the two equivalents are:

- **an IIFE** — `(() => { const t = …; return <article>…</article>; })()`, which
  matches the scoping exactly: the binding exists only for the markup;
- **a hoisted `const`** above the expression, which is what an author would
  actually write but leaks the binding into the enclosing scope.

Both are recorded. They compile to the same template and the same holes; only
where the `const` lands differs.

## Deliberate differences

**`let` should be invisible in the output.** This is the whole claim of the
family: a `let` is compile-time plumbing, so the expected lowering is a template
byte-identical to one written with the expression inlined at each use. If the
emitter produces anything else — an extra hole, a changed walk — that is a bug
the fixture is designed to catch.

**Ordering matters in the attribute list.** `attr_local` binds `href` and then
uses it in the attribute *that follows*. Attribute-list `let` therefore has a
sequencing constraint that JSX's hoisted `const` does not, and an emitter that
reorders attributes (see family 16 — the spread rule does reorder them) has to
keep bindings ahead of their uses.

## view-dom features needed

- **LANDED:** `let` in a view body (`lower/local.rs:6`). Every hole in this
  family is an ordinary `(expr)` the emitter already handled, which is why
  nothing else was needed.
- **STILL REFUSED:** `let` in an attribute list (`lower/attributes.rs:48`).
