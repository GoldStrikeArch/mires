# 08 — fragments

## Why the two are equivalent

Topcoat has no fragment syntax because it does not need one. A `view!` body with
several top-level nodes *is* a fragment; `view-dom` lowers it to one `Template`
per root and returns a tuple, so the function's return type states the arity:

```rust
fn multi_static() -> (view_abi::Node, view_abi::Node)
```

The reference compiles `<>…</>` to a plain JS array of the same elements, so the
two agree on what is produced, if not on how it is typed.

## Deliberate differences

**A top-level non-element has no equivalent.** `DomWriter::template` rejects bare
text and a bare `(expr)` outside an element — "the dom emitter only lowers nodes
inside an element". Upstream's `singleExpression` (`<>{inserted}</>`),
`firstStatic`, `lastStatic` and the trailing `After` text in `multiExpression`
all put a non-element at fragment top level, so the Topcoat file wraps each in a
`<div>`. That is not a cosmetic change: it adds a real element to the tree and
changes the emitted templates. It is recorded here rather than hidden because
"can a view body start with text?" is a language question someone has to answer.

**Fragments with keys have no equivalent.** JSX allows `<>` with a `key`; Topcoat
has no key concept anywhere.

**A tuple is not an array.** The reference produces a JS array whose elements can
be reordered, sliced, and spread by the surrounding code. A Rust tuple's arity is
fixed at compile time. For emission that is an advantage — the arity is known —
but it means a `view!` body whose node count depends on control flow cannot be a
tuple, which is one reason families 11–13 are harder than they look.

## view-dom features needed

- multiple roots, one `Template` each, tuple-typed expansion — **present**
- **NOT-YET-LOWERABLE:** a non-element node at the top level of a view body. Not
  a missing feature so much as an undecided one.
