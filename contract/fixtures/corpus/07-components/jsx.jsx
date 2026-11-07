// Family 07 -- components with eager props.
//
// Derived from upstream __dom_fixtures__/components, reduced to the call shapes
// Topcoat's view! component syntax can express, and written in the Solid idiom
// that means what Topcoat's eager props mean: every component DESTRUCTURES its
// props at the boundary. Destructuring is how a Solid component reads a prop
// once instead of on every access, so it is the exact Solid spelling of a Rust
// fn taking ordinary arguments. Solid users are told not to destructure because
// it loses reactivity; Topcoat loses the same reactivity structurally and buys
// it back explicitly, by passing a signal accessor or a closure as the prop.
//
// Two deliberate choices in this file, both about making the trace mean
// something:
//
//   1. The components are DEFINED here rather than left as free bindings, so
//      their bodies compile and the corpus records the templates and holes
//      inside them. That is what the Rust side emits for the same components.
//   2. Nothing is left undefined. run-trace.mjs heals a free binding into a
//      Proxy, and a Proxy reached through trace.mjs's `value()` normalizer comes
//      out as a function, which JSON.stringify then DROPS from the record. Under
//      free bindings the `comp` field of every createComponent record and the
//      `accessor` field of a reactive insert silently disappear, which is
//      exactly the information this family exists to compare.

// A signal, stubbed locally for the reason above. What matters to the compiler
// is only that `count` is an accessor and `setCount` a setter.
function createSignal(initial) {
  let current = initial;
  return [() => current, (next) => (current = next)];
}

const [count, setCount] = createSignal(0);

// Stands in for the Rust `dynamic_prop(label: &str)` parameter.
const label = "Active";

// ------------------------------------------------------------------ components

// One static prop. Destructured, so `label` is a plain string by the time the
// body runs, exactly like the Rust `label: &str` parameter.
function Badge({ label }) {
  return <span class="badge">{label}</span>;
}

// Child content. `children` is destructured too, which forces the call site's
// `get children()` getter at entry.
function Card({ title, children }) {
  return (
    <section class="card">
      <h2>{title}</h2>
      {children}
    </section>
  );
}

// A reactive prop: the ACCESSOR is the prop, and the tracking read `count()` is
// inside this body. Topcoat's `Sig<f64>` parameter plus `$(count.get())` is the
// same arrangement with the same reactivity graph.
function LiveCount({ count }) {
  return <p class="count">count {count()}</p>;
}

// A reactive prop as a plain closure, for a derivation rather than a signal.
function Readout({ read }) {
  return <p class="readout">{read()}</p>;
}

// ------------------------------------------------------------------ call sites

const staticProps = (
  <div id="main">
    <Badge label="Active" />
  </div>
);

const dynamicProp = (
  <div id="main">
    <Badge label={label} />
  </div>
);

const withChildren = (
  <Card title="Profile">
    <p>Account details</p>
  </Card>
);

const reactiveProp = (
  <div class="panel">
    <LiveCount count={count} />
    <button onClick={() => setCount(count() + 1)}>+1</button>
  </div>
);

const closureProp = (
  <div class="panel">
    <Readout read={() => count() * 2} />
  </div>
);

const siblingComponents = (
  <div class="row">
    <Badge label="One" />
    <Badge label="Two" />
  </div>
);
