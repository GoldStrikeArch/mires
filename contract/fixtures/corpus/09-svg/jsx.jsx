// Family 09 -- SVG.
// Derived from upstream __dom_fixtures__/SVG, reduced to the three cases that
// decide namespace handling: a static svg subtree, dynamic attributes inside
// one, and an svg-only element used as a template root.

const staticSvg = (
  <svg width="400" height="180">
    <rect stroke-width="2" x="50" y="20" width="150" height="150" />
  </svg>
);

const dynamicSvg = (
  <svg width="400" height="180">
    <rect stroke-width={state.width} x={state.x} y={state.y} width="150" height="150" />
  </svg>
);

const camelCased = (
  <svg>
    <linearGradient gradientTransform="rotate(25)">
      <stop offset="0%"></stop>
    </linearGradient>
  </svg>
);

const rootRect = <rect x="50" y="20" width="150" height="150" />;
