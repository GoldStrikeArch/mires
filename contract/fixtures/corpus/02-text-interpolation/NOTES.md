---
{
  "deltas": [
    {
      "id": "text-gt-escaping",
      "kind": "accept-diff",
      "op": "template",
      "field": "html",
      "a": "<span>Hi&lt;script>alert();&lt;/script>",
      "b": "<span>Hi&lt;script&gt;alert();&lt;/script&gt;",
      "why": "Topcoat's HtmlContext::Text escapes & < AND > (TEXT_ESCAPES in crates/topcoat-view/src/escape.rs); solid's text mode escapes only & and <. This is the `injection` case, where a JS string literal child is folded into the template through solid's escape(). A superset, so strictly safer and never unsafe, but the template strings are not byte-identical. Recorded in fixtures/escaping.json $topcoatDelta. See the hydration warning below before accepting this permanently."
    }
  ]
}
---

# 02 — text interpolation

Where a text node ends and a hole begins, and how the text is escaped on the way
into the template string.

## Why the two are equivalent

Each JSX text run maps to one quoted Topcoat text node, and each `{expr}` maps to
one `(expr)`. The emitted template is the same in every case here.

## Deliberate differences

**JSX whitespace folding has no counterpart, and that is a simplification.** JSX
trims leading and trailing whitespace on lines and collapses a line break plus
indentation into nothing — which is why upstream's fixture needs eight
`/* prettier-ignore */` variants to pin the behaviour. Topcoat quotes every text
node, so the text is exactly what is between the quotes and nothing about source
formatting can change it. The `multiLine*` upstream cases therefore have no
Topcoat equivalent at all: they exercise a rule Topcoat does not have.

**HTML entities are not decoded.** JSX turns `&nbsp;` and `&lt;` into the
characters they name. Topcoat's text is a Rust string literal, so the fixture
writes the character directly (`\u{a0}`) and lets the escaper decide how it
appears in the template.

## The escaping delta (accepted, with a caveat)

`injection` is the case that makes the standing divergence measurable. The
reference emits:

```
<span>Hi&lt;script>alert();&lt;/script>
```

`<` became `&lt;`, `>` stayed bare. Topcoat escapes `>` as well, so it would emit
`&lt;script&gt;alert();&lt;/script&gt;`. The front matter above accepts this for
trace comparison.

**The rule must be an `accept-diff`, not a `rewrite`.** The obvious way to write
it is "replace `&gt;` with `>` on the Topcoat side". That is wrong, and running
it is how we found out. This same fixture's `escape` case has the **reference**
emitting

```
<span>&nbsp;&lt;Hi&gt;&nbsp;
```

— the plugin copies JSX text through verbatim without decoding entities, so a
source `&gt;` stays `&gt;` and is not an escape at all. A blanket rewrite mangles
that template on one side only and would then hide any genuine divergence in it.
Escaping deltas get pinned to exact value pairs.

**It is accepted for the DOM template string, not for hydration.** If Rust
renders the SSR pass with `&gt;` and solid's runtime later re-renders the same
text client-side with a bare `>`, the two disagree on a text node that hydration
expects to match. That is a real bug waiting in a case this corpus does not yet
cover, and it is the reason the delta carries a `why` rather than being silently
normalised away.

## view-dom features needed

All present.

- literal text, including adjacent literals merging into one text node
- `(expr)` child holes
- text-position escaping — but see the delta above: the escape SET differs
