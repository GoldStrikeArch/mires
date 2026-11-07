# 12 — `match` with markup arms

**LOWERS.** A `match` with markup arms is a reactive child hole
(`lower/node.rs:29-32`), and a block groups sibling nodes without opening a scope
of its own (`:21`), so a multi-node arm is expressible. Only the ATTRIBUTE-LIST
form is still refused (`lower/attributes.rs:45`).

The per-family L2 verdict is `contract/fixtures/l2-status.json`, enforced by
`contract/harness/check-l2-status.mjs`. It is not restated here: a status
sentence in a NOTES.md is exactly what went two waves stale before wave 6.

## Why the two are equivalent

A `match` over N arms and a chain of N−1 nested ternaries select one subtree from
several. The reference compiles the chain to nested memos, one per test, so the
emitted structure is a tree of conditionals — which is also the natural lowering
for `match`.

## Deliberate differences

**Pattern matching has no JSX counterpart.** The JSX file compares strings
(`status === "draft"`); `view.rs` matches enum variants and *binds* from them
(`Status::Published { title } => …(title)`). The binding is the interesting part:
it is a value the arm's markup can use that does not exist outside the arm. No
ternary chain can express that, so the JSX version has to reach for `post.title`
from an outer scope. The two produce the same DOM, by different means.

**Match guards have no counterpart.** `Status::Archived if show_archived` folds
into the ternary chain as an extra `&&`, but only because this fixture's guard
happens to be a simple boolean.

**Exhaustiveness is a real difference, not a cosmetic one.** Rust requires the
match to cover every variant; the ternary chain needs a final `else`. That means
a Topcoat `match` can have *no fallback branch at all* when the patterns are
exhaustive, where the JSX equivalent always has one. An emitter that assumes
"there is always a final else" will be wrong.

**Multi-node arms need two features, not one.** `multi_node_arm` wraps two
siblings in a block, and blocks are *separately* unsupported ("a block in a view
body"). The JSX side has to build an array literal, which the reference compiles
to an array `insert` rather than a conditional — a genuinely different emission.

**Attribute-position `match` has no JSX form**, for the same reason as family
11's attribute `if`: each arm emits attributes, not a value.

## view-dom features needed

- **LANDED:** `match` in a view body; block-bodied arms; per-arm templates plus
  a shared anchor.
- **STILL REFUSED:** `match` in an attribute list (`lower/attributes.rs:45`).
- **DELIBERATELY NOT COMING:** memo wrapping. An arm selects on a pattern rather
  than on a boolean and its body reads what the pattern bound, so there is no
  test to hand over separately; splitting one out would mean matching twice,
  which is a different program. `view_abi::cond` documents this, and it is the
  standing `memo-hoisting-for-a-match` cause in
  `examples/dom-tests/deltas.json`.
