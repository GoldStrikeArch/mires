// Family 08 -- fragments.
// Derived from upstream __dom_fixtures__/fragments. A view! body with more than
// one top-level node IS a fragment, so this family maps directly; the cases
// kept are the ones where the position of a dynamic node inside the fragment
// changes the emitted array.

const multiStatic = (
  <>
    <div>First</div>
    <div>Last</div>
  </>
);

const multiExpression = (
  <>
    <div>First</div>
    {inserted}
    <div>Last</div>
    After
  </>
);

const firstDynamic = (
  <>
    {inserted()}
    <div />
  </>
);

const lastStatic = (
  <>
    <div />
    {inserted}
  </>
);
