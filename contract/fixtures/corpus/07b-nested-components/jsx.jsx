// Family 07b -- components nested inside components.
//
// Same conventions as family 07: every component destructures its props, which
// is Solid's spelling of Topcoat's eager props, and nothing is left as a free
// binding so the trace keeps every field.
//
// The child-content case is written as `children={kid}` with the child hoisted
// into a const, NOT as `<Card><Badge/></Card>`. A hoisted JSX element reaches the
// props object as a plain value, so it is built once, before the outer component
// is called, which is exactly what Topcoat's eager `Node` argument does. The
// idiomatic nesting compiles to a `get children()` getter instead and is
// deliberately absent here: it would add reference-only records in the middle of
// the module and break the whole-module trace alignment this family is held to.
// What the getter form does is recorded in family 07's `withChildren` case, and
// its key layout in `keys-nested.json` case `07b-children-component-getter`.

// ------------------------------------------------------------------ components

function Badge({ label }) {
  return <span class="badge">{label}</span>;
}

function Card({ title, children }) {
  return (
    <section class="card">
      <h2>{title}</h2>
      {children}
    </section>
  );
}

// Calls another component from its own body: the pure nesting case.
function LabelledCard({ title }) {
  return (
    <section class="card">
      <h2>{title}</h2>
      <Badge label="Active" />
    </section>
  );
}

function DoubleCard({ title }) {
  return (
    <section class="card">
      <h2>{title}</h2>
      <Badge label="One" />
      <Badge label="Two" />
    </section>
  );
}

function OuterCard({ title }) {
  return (
    <section class="outer">
      <h2>{title}</h2>
      <LabelledCard title="Inner" />
    </section>
  );
}

// ------------------------------------------------------------------ call sites

const nestedInBody = <LabelledCard title="Profile" />;

const kid = <Badge label="Active" />;
const childrenComponentEager = <Card title="Profile" children={kid} />;

const depthThree = <OuterCard title="Profile" />;

const siblingsInBody = <DoubleCard title="Profile" />;

const nestedInElement = (
  <div id="main">
    <LabelledCard title="Profile" />
  </div>
);
