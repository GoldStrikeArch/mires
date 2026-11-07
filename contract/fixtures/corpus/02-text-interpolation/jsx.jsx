// Family 02 -- text interpolation.
// Derived from upstream __dom_fixtures__/textInterpolation. Kept to the cases
// where whitespace handling and expression adjacency actually decide the
// emitted template string.

const trailing = <span>Hello </span>;
const leading = <span> John</span>;

const trailingExpr = <span>Hello {name}</span>;
const leadingExpr = <span>{greeting} John</span>;

/* prettier-ignore */
const multiExpr = <span>{greeting} {name}</span>;

/* prettier-ignore */
const multiExprTogether = <span> {greeting}{name} </span>;

const escape = <span>&nbsp;&lt;Hi&gt;&nbsp;</span>;

const injection = <span>Hi{"<script>alert();</script>"}</span>;
