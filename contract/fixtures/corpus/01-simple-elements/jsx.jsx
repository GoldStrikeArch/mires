// Family 01 -- simple elements.
// Derived from upstream __dom_fixtures__/simpleElements, cut down to the one
// thing this family is for: fully static markup with no dynamic positions.

const nested = (
  <div id="main">
    <h1>Welcome</h1>
    <label for="entry">Edit:</label>
    <input id="entry" type="text" />
  </div>
);

const siblings = (
  <div>
    <span>
      <a></a>
    </span>
    <span />
  </div>
);

const rawText = (
  <div>
    <style>{"div { color: red; }"}</style>
  </div>
);
