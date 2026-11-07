# 07b - nested components and the key namespace

**PARTLY LOWERABLE** for the same reasons as family 07: `view-dom/src/lower/component.rs` lowers a component call and `HoleKind::Component` is now constructed, but the backend's `_$createComponent` emission and the SSR `Formatter::enter_component` are still missing. `children_component` is additionally rejected at the call site, because child nodes of a component are not lowered yet (`lower/component.rs:24-32`).

Family 07 is about props. This one is about hydration keys, which is the part of components that fails silently. A wrong key is a plausible string that no node in the document carries, and the only symptom is that hydration does nothing.

## What the key layout is

Every component call opens a child hydration context whose id is the parent slot the call spent, and the parent's counter advances past that slot for good. So the keys inside a component depend on where the call sits, and the keys after it depend on how many slots were spent before it. The scheme is CONTRACT-DOM.md points 9.5 to 9.8, and the oracle is `contract/fixtures/keys-nested.json`, driven through both of Solid's runtimes.

Group `corpus-07b` in that fixture holds the exact tree of each case here, so the expected keys are driven rather than asserted:

| view.rs | keys-nested.json case | element keys |
|---|---|---|
| `nested_in_body` | `07b-component-in-body` | `00`, `010` |
| `children_component` | `07b-children-component-eager` | `00`, `10` |
| `depth_three` | `07b-depth-three` | `00`, `010`, `0110` |
| `siblings_in_body` | `07b-siblings-in-body` | `00`, `010`, `020` |
| `nested_in_element` | `07b-nested-in-element` | `0`, `10`, `110` |

Read `nested_in_body` as: the call to `labelled_card` spends root slot 0, so its context id is `0` and its own section root takes `00`; the `badge` call inside spends slot 1 of that context, so its context id is `01` and the span root takes `010`.

`nested_in_element` is the same tree one slot along: the `<div id="main">` takes root slot 0, so the component opens at slot 1.

## The one real divergence from Solid: eager child content

`children_component` and `nested_in_body` are the same source shape in Solid and different shapes in Topcoat.

In Topcoat, child content is an eager `Node` field of the props struct, so `card(title: "Profile", badge(label: "Active"))` calls `badge` **before** `card`: the struct is a Rust value and its fields are evaluated where the struct is built, which is at the call site, in the caller's context, before the thunk the hole carries can run. The badge boundary spends root slot 0 and the card boundary spends slot 1, which makes the two components siblings in the root context: keys `00` and `10`.

In Solid the same nesting compiles to `get children()`, so the badge is built when `Card` reads the prop, inside the card's context: keys `00` and `010`. The pinned runtimes give exactly that, in `keys-nested.json` case `07b-children-component-getter`, which is byte for byte the layout of `07b-component-in-body`. Under Solid's model, child content and a call in the body are indistinguishable. Under Topcoat's, they are not.

This delta is accepted, for three reasons.

**Topcoat owns both sides.** The obligation is that the SSR `Formatter` and the emitted client code agree with each other, not that either agrees with Solid's layout. CONTRACT-DOM.md point 9.7 is about that agreement, and the fixture proves it for Solid so Topcoat has a reference for the shape of the agreement, not a byte target for the keys.

**Solid's layout is not statically knowable.** Because the child is built when the body reads the prop, the key a child gets depends on how far through its own key allocations the body is at that moment. `keys-nested.json` field `getterChildren` drives it: a component that allocates its own root and then reads `props.children` emits `<div data-hk="00"><span data-hk="01">`, and the same component destructuring at entry emits `<div data-hk="01"><span data-hk="00">`. Same source, same props, different keys, decided by a line of the body a compiler does not get to see. Topcoat's eager order is fixed by the source, which is what lets a Rust `Formatter` produce the same keys as the emitted JS without running it.

**Eager child content is already the decision.** It follows from `Node` being a one-word non-`Clone` handle and from the server needing to stream child content inline. See family 07 for that argument.

## Equivalence level

Everything here is expected to reach **L2**, which is why `jsx.jsx` writes the child-content case as `children={kid}` with the child hoisted into a const. A hoisted JSX element reaches the props object as a plain value, so the reference builds it once, before the outer call, which is exactly what Topcoat does. There is no JSX spelling of eager element children written inline: the plugin turns any JSX-element child into a getter, `<Card><Badge/></Card>` included, and family 07 records what that costs a trace.

The generated `expected.reference.hydratable.js` is where the key allocation order is visible in compiled form: one `_$getNextElement` per template root, in the order the keys are spent, with the component boundaries in between as `_$createComponent` calls.

## Props are a struct here too

Every component in view.rs takes one argument, the props struct both halves declare, and destructures it on its first line; family 07's NOTES.md carries the rule and the argument for it. It does not touch this family's subject. A props struct is built at the call site, so its fields are spent in the caller's context, which is precisely the ordering the eager-children divergence below turns on -- the struct makes that ordering more obvious than positional arguments did, not different.

## Permanent deltas

The two from family 07 apply unchanged and are declared once, for both families, in `deltas.json`: `component-props-object` and `component-fn-identity`.

One more is specific to this family. **Component identity is per call site, not per component.** The reference passes the component function itself to `createComponent`, so the three `Badge` calls in this module all record the same `fn#1`, and `compare-trace.mjs` preserves that aliasing on purpose. If the emitter wraps each call in a thunk to get the context nesting, every call site produces a distinct function and the aliasing is gone. Rule `component-fn-identity` blanks the field; the `why` says what to do if the emitter turns out to pass the component directly, in which case the rule goes STALE and should be deleted.

## view-dom features needed

- Everything family 07 needs, plus:
- `Formatter::enter_component` / `exit_component` on the SSR side, matching CONTRACT-DOM.md 9.5 exactly, including the detail that the parent's counter advance survives the restore.
- The KeyPlan question this family exists to force: a component body is its own `view!` with its own KeyPlan starting at zero, while the server renders it inline continuing the island counter. Nested contexts are how the two are reconciled: the child KeyPlan's ordinals are relative to a context id the caller allocated.
