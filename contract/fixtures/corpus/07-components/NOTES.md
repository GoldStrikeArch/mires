# 07 - components with eager props

**PARTLY LOWERABLE** as of wave 1. The old "a component invocation is not yet supported by the dom emitter" error is gone: `view-dom/src/lower/component.rs` lowers a component call and `view-dom/src/dom_writer.rs:215-230` constructs `HoleKind::Component`, which no code had constructed before. What is still missing is the backend's `_$createComponent` emission (behind `backend/src/abi.rs`'s `HoleKind::Component => zombie(...)`) and the SSR `Formatter::enter_component` that has to agree with it. Two call-site forms are still rejected with spanned errors: child nodes of a component (`lower/component.rs:24-32`, so `with_children` does not compile yet) and a `$(...)` runtime expression as a prop value (`lower/component.rs:49-56`).

This family had no defined equivalence until the eager-props decision. It has one now, and this file is where it is written down.

## The decision

A client component is a plain Rust fn taking exactly one argument: a props struct, named after the component in PascalCase with `Props` appended, which the fn destructures on its first line. Props are eager -- each field is an ordinary Rust value, evaluated once where the component is called, in source order, before the call. A prop that must stay reactive is passed explicitly as a `view_abi::Sig<T>` or a closure, and the tracking read lives in the callee's own `$()` hole. Child content is an eager `view_abi::Node` field named `child`.

The alternative was to synthesise Solid's getter objects. Rust has no implicit laziness, so a getter prop has to be a closure, which means getter props in Rust are explicit closures with extra steps: one closure allocation per prop at the call site, and a generic parameter or a trait object per prop in the callee. Making the closure visible in the signature costs the same and says what it does. Getter props only pay for themselves with spread and dynamic props, which are out for G2.

### Why a struct rather than positional arguments

A call site writes props by NAME (`badge(label: "Active")`), and the emitter that lowers that call cannot see the order the callee declared its props in. Positional emission would therefore require source order to equal declaration order, which nothing checks: two props of the same type written in the other order would silently swap. The struct makes the mapping order-independent, turns a missing prop into "missing field `label` in initializer" and an unknown one into "no field named X", and it is the same struct the server half already takes, so exactly one definition of the name exists per target.

This does not weaken the equivalence below. The props struct is a Rust value built INSIDE the thunk the hole carries, and it is never an argument to `createComponent`: the hole's value is `move || badge(BadgeProps { label: ("Active"), })` (`lower/component.rs:62-64`), a zero-argument thunk. `deltas.json`'s `component-props-object` rule states exactly that as its precondition.

The struct borrows for `'__tc_props` if any prop borrows and takes no lifetime at all otherwise (`view-dom/src/client_component.rs:196-260`). Corpus `badge` and `card` borrow; `live_count` does not.

## The equivalence, precisely

A Topcoat client component call is a Solid `createComponent` call whose props object is destructured at the callee's boundary.

That is the whole claim, and it is why every component in `jsx.jsx` destructures. Destructuring is how a Solid component reads each prop once instead of on every access. Solid users are warned off it because it loses reactivity; Topcoat loses exactly the same reactivity, structurally, and buys it back by passing an accessor (`count`) rather than a read (`count()`), which is what the `Sig<f64>` parameter is.

Case for case:

| view.rs | jsx.jsx | what it pins |
|---|---|---|
| `badge(BadgeProps { label: &str })` | `Badge({ label })` | a prop that is a plain value inside the body |
| `card(CardProps { title, child: Node })` | `Card({ title, children })` | child content as an ordinary field |
| `live_count(LiveCountProps { count: Sig<f64> })` | `LiveCount({ count })` | a reactive prop as an accessor read inside the callee |
| `readout(ReadoutProps<R> { read: R })` | `Readout({ read })` | a reactive prop as a closure |
| `static_props` | `staticProps` | a static prop, and a component inside an element |
| `dynamic_prop` | `dynamicProp` | a prop taken from the surrounding scope |
| `with_children` | `withChildren` | trailing view nodes desugaring to `child:` |
| `reactive_prop` | `reactiveProp` | a signal declared in the caller, read in the callee |
| `closure_prop` | `closureProp` | a derivation passed as a closure |
| `sibling_components` | `siblingComponents` | two calls in one body, so prop evaluation order is recorded |

What is compared:

- **L1**, template strings and walks, in the component bodies and at the call sites. A component call contributes no template of its own. It contributes a hole at the call site, and the callee contributes its own templates. Both sides should agree byte for byte.
- **L2**, the record sequence, once `deltas.json` is applied and with the one enumerated harness artifact below.

What is not compared, because Topcoat has no counterpart: prop laziness, the JS props object, `mergeProps`, `propTraps`, `ref`, and spread on a component (family 16).

## Permanent deltas

**No props object crosses the boundary.** The reference builds `{ label: "Active" }` and hands it to `createComponent` as its second argument. Topcoat builds a props struct too, but it is a Rust value constructed inside the thunk the hole carries -- `move || badge(BadgeProps { label: ("Active"), })` -- so `createComponent` is handed a zero-argument thunk and nothing corresponds to the JS object. `deltas.json` rule `component-props-object` blanks the field on both sides, and states that thunk emission as its precondition; the component's own templates follow immediately in the trace and identify the call, so blanking it does not hide which component ran.

**A prop is evaluated exactly once, at the call site.** In the reference a getter prop is evaluated on first read, zero times if it is never read, and again on every later read. In Rust an argument is evaluated once, before the call, left to right. For a pure prop this is unobservable. For a prop with side effects the two are simply different, and Rust's answer is the one a Rust programmer already expects from a function call.

**Reactivity is in the callee's signature, not at the call site.** Solid can make any prop reactive without the callee knowing. Topcoat cannot: `Sig<f64>` and `impl Fn() -> f64` are types, so a prop that needs to update has to be declared that way. The upside is that a Topcoat component's signature says which of its props can change. The downside is that turning a static prop reactive is a breaking change to the component. Accepted: this is the trade every language without implicit laziness makes.

**Child content is eager, so it is built before the component is called.** The reference emits `get children()` at the call site, so the child is built when the callee reads the prop, inside the callee. Topcoat builds it as an argument, before the boundary. This is not only an ordering curiosity: it changes hydration keys, because the child's template roots take slots in the caller's context instead of the callee's. Family 07b quantifies it, and `keys-nested.json` cases `07b-children-component-eager` and `07b-children-component-getter` drive both layouts through the real runtimes.

**Child content can be used once.** `view_abi::Node` is a one-word handle with no `Clone` (`view-abi/src/lib.rs:122-127`), so a component cannot render its children twice. Solid's `props.children` getter can be read twice and builds the child twice, which is why Solid ships the `children()` helper to memoise it. Topcoat needs no helper and has no way to ask for a second copy.

**`child` is a reserved prop name.** Trailing view nodes desugar to it (`crates/topcoat-view/grammar/src/view/component.rs:84-90`), so the field of that name cannot mean anything else. In JSX `children` is an ordinary prop that can also be passed explicitly.

**Components are called, not tagged.** JSX tells a component from an element by capitalisation. Topcoat tells them apart by shape: `<name>` is always an element and `name(...)` is always a component. There is no lowercase component and no capitalised element, which removes a class of ambiguity, but it also means the two grammars are not mechanically translatable, only semantically.

**No `untrack` wrapper.** The reference's `createComponent` wraps the body in `untrack` so a synchronous read in the body does not subscribe the caller's effect (`solid/dist/solid.js:1279`). A Topcoat component call happens once, during template construction, and not inside a tracking scope, so there is nothing to untrack. If a component call ever lands inside a reactive hole, this stops being true and needs revisiting.

## One harness artifact, not a compiler difference

The `withChildren` case produces **two** `template.clone` records for `<p>Account details` in `expected.reference.trace`, one either side of the `createComponent` record. Real Solid produces one. The doubling is `harness/trace.mjs`'s `value()` normaliser (`:186-190`) reading every key of the props object in order to record it, which forces `get children()` before the component runs; the destructure inside `Card` then forces it again, and a getter is not memoised.

A Topcoat trace will therefore be short by exactly one record here, reported as one `missing-in-b` for `template.clone` of that template. It is enumerated so wave 2 recognises it instead of hunting for a bug. It is not expressible as a `deltas.json` rule, because no rule kind drops a single record from one side. `deltas.json` field `$notExpressibleAsRules` records the option of teaching `value()` to record a getter without forcing it, and why that was not done here.

## A prop whose type is `impl Trait` needs a generic parameter

`readout` is the one case whose props struct is NOT a verbatim copy of the declared parameter list, and this is the only place the corpus writes something today's `#[component(client)]` does not yet emit.

`view-dom/src/common.rs:24-42` accepts any parameter type and `view-dom/src/client_component.rs:167-175` copies it straight into a struct field. `impl Trait` is not legal in field position, so `#[component(client)] async fn readout(read: impl Fn() -> f64)` currently expands to `struct ReadoutProps { read: impl Fn() -> f64 }`, which does not compile. Argument-position `impl Trait` is already defined as sugar for a generic parameter, so the fix is to hoist it onto both items, which is what view.rs writes:

```rust
struct ReadoutProps<R: Fn() -> f64> { read: R }

fn readout<R: Fn() -> f64>(props: ReadoutProps<R>) -> view_abi::Node {
    let ReadoutProps { read } = props;
    dom_view! { <p class="readout">$(read())</p> }
}
```

Call sites need no change: `readout(read: (|| count.get() * 2.0))` lowers to `move || readout(ReadoutProps { read: (|| count.get() * 2.0), })` and inference fills `R` in. The generic parameter also has to be threaded onto the props path the same way the lifetime is, i.e. NOT written at the use site, since `lower/component.rs:38-41` strips the call path's generic arguments when it renames the last segment.

The alternative -- a boxed `Box<dyn Fn() -> f64>` field -- keeps the struct non-generic but demands trait objects from the backend, which is a far larger ask than monomorphising one closure, and it allocates per call site.

## view-dom features needed

- **STILL MISSING:** the backend's `_$createComponent` emission (`backend/src/abi.rs`, `HoleKind::Component => zombie(...)`) and the SSR `Formatter::enter_component` that has to agree with it (CONTRACT-DOM.md 9.5). Lowering itself landed in wave 1.
- **STILL REJECTED:** child nodes of a component (`lower/component.rs:24-32`), which is `with_children`; and `$(...)` as a prop value (`lower/component.rs:49-56`).
- `Sig<T>` as a prop: the signal handle already crosses a fn boundary as a `Copy` u32, so this needs nothing new from `view-abi`.
- A closure as a prop: monomorphises, so the reactive hole in the callee sees a concrete closure. Needs the generic hoist above. Untested through the backend.
