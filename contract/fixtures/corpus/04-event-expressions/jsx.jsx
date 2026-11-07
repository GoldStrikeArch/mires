// Family 04 -- event expressions.
// Derived from upstream __dom_fixtures__/eventExpressions, reduced to the three
// dispatch paths the compiler distinguishes: a delegated event (click), a
// non-delegated event (change), and an explicit listener (on:).

const delegated = <button onClick={() => console.log("delegated")}>Click</button>;

const delegatedBound = <button onClick={handler}>Click</button>;

const nonDelegated = <button onChange={() => console.log("bound")}>Change</button>;

const listener = <button on:custom-event={() => console.log("listener")}>Custom</button>;

const many = (
  <div id="main">
    <button onClick={() => count.set(count() + 1)}>+1</button>
    <input onInput={e => query.set(e.target.value)} />
  </div>
);
