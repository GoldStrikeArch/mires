// Family 05 -- conditional expressions.
// Derived from upstream __dom_fixtures__/conditionalExpressions, reduced to the
// four shapes that decide whether the plugin wraps in a memo: static test with
// static branches, dynamic test with a dynamic branch, a logical AND, and a
// nested ternary chain.

const staticTest = <div>{simple ? good : bad}</div>;

const dynamicTest = <div>{state.dynamic ? good() : bad}</div>;

const logicalAnd = <div>{state.dynamic && good()}</div>;

const chain = <div>{state.a ? "a" : state.b ? "b" : "fallback"}</div>;

const withMarkup = (
  <div>
    {state.dynamic ? <span>yes</span> : <span>no</span>}
  </div>
);
