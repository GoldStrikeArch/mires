# 17 — signals

**LOWERS.** `lower/signal_declaration.rs` claims its `KeySite::Signal` ordinal
so later sites are numbered consistently, and emits
`let <ident> = ::view_abi::signal(<ordinal>, <init>);` — the initializer is no
longer dropped, so the signal has storage. `$()` reads and `@click=$()` writes
lower as a reactive `Child` hole and a `DelegatedEvent` hole respectively.

This family is nonetheless **EXCLUDED** from the L2 comparison, for a reason that
is not about lowering at all: the reference module for family 17 is a different
program, so a trace comparison against it would not be measuring anything
(`examples/dom-tests/17_signal.rs:9-10`). The row and its `why` are in
`contract/fixtures/l2-status.json`; the L1 goldens still run.

## Why the two are equivalent

| Topcoat | Solid |
|---|---|
| `signal count = 0.0;` | `const [count, setCount] = createSignal(0)` |
| `$(count.get())` | `{count()}` |
| `@click=$(\|_e\| count.set(…))` | `onClick={() => setCount(…)}` |

Same reactive loop: a read in a dynamic position subscribes, a write in a handler
re-runs it.

## Deliberate differences

**The compiler never sees the declaration.** This is the delta worth stating
plainly. `createSignal` is an ordinary function call as far as
babel-plugin-jsx-dom-expressions is concerned — it does not appear in the
compiled output's imports, it produces no runtime call, and the plugin's only
involvement is that `count()` happens to sit in a dynamic position. Topcoat's
`signal` is a *declaration in the view grammar*: the macro sees it, gives it a
key site, and is expected to serialise the initial value into the page.

So the reference output has nothing to compare the declaration against. What can
be compared is everything downstream of it — the reactive hole, the effect, the
handler, the `delegateEvents` call — and that is what `expected.reference.trace`
records.

**One accessor versus a getter/setter pair.** Solid destructures into two values;
Topcoat's `count` is one binding with `.get()` and `.set()`. This is why
`run-trace.mjs` needed a destructuring-capable healed binding to trace this
family's JSX at all — noted in the harness report, not a corpus fact.

**`.increment()` / `.toggle()` / `.push_str()` have no Solid spelling.** They are
sugar for a read-modify-write; the JSX file spells them out.

## What the reference does that is worth pinning

```
<div><button>+1</button><p>Count:
<div><input><p>Hello, <!>!
```

The second one earns its `<!>` anchor: the hole is between text and text, so it
is neither first nor last child. Compare family 06, where no case needed one.

`delegateEvents(["click"])` is emitted **once at module scope** (CONTRACT-DOM
7.4), not per element — so the per-template `events` set `view-dom` already
collects has to be unioned across the whole module before it is emitted.

## view-dom features needed

- `$(expr)` reactive child holes — **present**
- `@click=$(closure)` delegated event holes — **present**
- **PARTIAL:** `signal` declarations lower to nothing. No storage is allocated,
  no initial value is serialised, so `.get()` reads state that does not exist and
  `.set()` writes nowhere.
- **NOT-YET-LOWERABLE:** the module-scope `delegateEvents` union.
