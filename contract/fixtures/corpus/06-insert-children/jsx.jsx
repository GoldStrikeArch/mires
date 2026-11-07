// Family 06 -- inserted children.
// Derived from upstream __dom_fixtures__/insertChildren, reduced to child holes
// in the positions that change the emitted anchor: only child, after text,
// before an element, between two elements, and two adjacent holes.

const only = <div>{children}</div>;

const afterText = <div>Hello {children}</div>;

const beforeElement = (
  <div>
    {children}
    <span />
  </div>
);

const between = (
  <div>
    <span />
    {children}
    <span />
  </div>
);

const adjacent = <div>{first}{second}</div>;

const dynamicCall = <div>{children()}</div>;

const array = <div>{tiles}</div>;
