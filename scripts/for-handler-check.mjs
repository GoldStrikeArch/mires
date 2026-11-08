// Asserts which row a `@click` inside a `for` loop actually acts on.
//
//   node scripts/for-handler-check.mjs
//
// The dom suite cannot show this. `contract/harness/trace.mjs` records that a handler was installed
// -- as a `$$click` property -- and never invokes one, and it records an `insert` without ever
// calling the accessor it was handed, so neither the row a closure captured nor the rows of a second
// render are observable there. That harness is the contract's, so this check goes around it rather
// than changing it, exactly as scripts/event-accessor-check.mjs and scripts/reactive-for-check.mjs
// do.
//
// What it does instead: a run directory of its own, holding a DOM module that wraps three of the
// stub's entry points before re-exporting the rest.
//
//   * `insert` keeps the value it was handed. A static loop hands over the array of row nodes, so
//     the rows are reachable by hand; a reactive one hands over an accessor, so they are reachable
//     by calling it. Either way the row's `<button>` is the row node's `firstChild`, which is where
//     the emitted code put the `$$click`.
//   * `createSignal` keeps the `[get, set]` pair, so what a handler wrote can be read from here
//     without the fixture having to export a getter for it.
//   * `setAttribute` keeps what was written to which node. The stub only records the call, so the
//     row's `value=` is not readable back off the node the way a browser would read it.
//
// A handler is then called the way solid's delegation calls it, with one event object, and the
// signal is read. The event is a plain object with a `target`: `target_value` compiles to
// `event.target.value || ""`, so an object with those members is as good an event as a real one for
// the purpose of proving which element the value came from.
//
// WHAT IS ASSERTED
// ----------------
// Both patterns are the contract. Pattern B is one handler on the container reading the row's
// `value`; pattern A is one handler per row, and each one acts on the row it was built for. A
// thunk built inside a loop takes its environment through an immediately invoked wrapper
// (`(($c) => ($a0) => f($c, $a0))(env)`), so every turn of the loop hands its handler a binding of
// its own rather than sharing the one function-scoped `let` the prelude declares.
//
// The per-row checks below are the only place that difference is observable: which row a closure
// captured is invisible to a trace that never calls a handler.
//
// The fixture is examples/dom-tests/23_for_row_handlers.rs; build/domtest/23_for_row_handlers.js is
// what scripts/dom-test.sh leaves behind, so run that first.

import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/domtest");
const run = path.join(root, "build/forhandlercheck");

const program = path.join(from, "23_for_row_handlers.js");
if (!fs.existsSync(program)) {
  console.log("FAIL  build/domtest/23_for_row_handlers.js is missing; run scripts/dom-test.sh first");
  process.exit(1);
}

// A run directory of its own, at the same depth below the root as build/domtest, so that the
// relative path to the contract harness below still resolves.
fs.mkdirSync(run, { recursive: true });
fs.writeFileSync(path.join(run, "package.json"), '{ "type": "module" }\n');
fs.copyFileSync(path.join(from, "shim.js"), path.join(run, "shim.js"));
fs.copyFileSync(program, path.join(run, "23_for_row_handlers.js"));

// The DOM module the compiled program imports. An explicitly exported name wins over a `export *`,
// so the three wrapped here replace the stub's and everything else comes through unchanged.
fs.writeFileSync(
  path.join(run, "topcoat-dom.js"),
  `import * as trace from "../../contract/harness/trace.mjs";
export * from "../../contract/harness/trace.mjs";

globalThis.__inserts = [];
export function insert(parent, accessor, marker, initial) {
  globalThis.__inserts.push({ parent, accessor, marker });
  return trace.insert(parent, accessor, marker, initial);
}

globalThis.__attributes = [];
export function setAttribute(node, name, value) {
  globalThis.__attributes.push({ node, name, value });
  return trace.setAttribute(node, name, value);
}

globalThis.__signals = [];
export function createSignal(initial) {
  let value = initial;
  const pair = [
    () => value,
    (next) => {
      value = typeof next === "function" ? next(value) : next;
      return value;
    }
  ];
  globalThis.__signals.push(pair);
  return pair;
}
`
);

const compiled = await import(path.join(run, "23_for_row_handlers.js"));

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

// Each view is built on its own, so the inserts, attributes and signals it made are unambiguous.
const build = (entry) => {
  globalThis.__inserts = [];
  globalThis.__attributes = [];
  globalThis.__signals = [];
  const node = entry();
  return {
    node,
    inserts: globalThis.__inserts,
    attributes: globalThis.__attributes,
    signals: globalThis.__signals
  };
};

// The `<button>` of each row: the row template is `<li><button></button></li>` and the emitted code
// walks to `firstChild` before assigning the handler.
const buttons = (rows) => rows.map((row) => row.firstChild);

// The rows of a static loop: the one insert that was handed an array rather than an accessor.
const staticRows = (view) => view.inserts.find((entry) => Array.isArray(entry.accessor));

// One click, delivered the way solid's delegation delivers it. `value` is what the element the
// click came from carries, which for a row button is the row's own name.
const click = (button, value) => button.$$click({ target: value === undefined ? {} : { value } });

// -------------------------------------------------- pattern A: a handler per row

const perRow = build(compiled.per_row);
const perRowList = staticRows(perRow);
check(
  "a static loop hands `_$insert` the array of rows it built",
  perRowList !== undefined && perRowList.accessor.length === 3,
  perRowList === undefined ? "no array insert" : `${perRowList.accessor.length} rows`
);

const perRowButtons = buttons(perRowList.accessor);
check(
  "every row carries its own `$$click`",
  perRowButtons.every((button) => typeof button.$$click === "function"),
  perRowButtons.map((button) => typeof button.$$click).join(",")
);
check(
  "and they are three distinct function objects",
  new Set(perRowButtons.map((button) => button.$$click)).size === 3
);

// A row handler acts on its own row: each wrapper was handed the environment of the turn that
// built it, rather than reading the binding the next turn overwrites.
const [readPicked] = perRow.signals[0];
const picks = [];
for (const button of perRowButtons) {
  click(button, "");
  picks.push(readPicked());
}
check(
  "every row's handler acts on its own row",
  String(picks) === "10,20,30",
  `${picks.join(",")} (a shared environment would be 30,30,30)`
);

// ------------------------------- pattern B: one handler on the container

const container = build(compiled.container);
// The view is `<div><p></p><ul></ul></div>`, so the list is the second child of the root.
const list = container.node.firstChild.nextSibling;
check("the container carries a single `$$click`", typeof list.$$click === "function", typeof list.$$click);

const containerRows = staticRows(container);
const containerButtons = buttons(containerRows.accessor);
check("the container's rows carry no handler of their own", containerButtons.every((button) => button.$$click === undefined));
check(
  "each row names itself in its button's `value`",
  String(container.attributes.map((entry) => `${entry.name}=${entry.value}`)) ===
    "value=alpha,value=beta,value=gamma",
  container.attributes.map((entry) => `${entry.name}=${entry.value}`).join(",")
);
check(
  "on the button of that row, which is the element a click lands on",
  container.attributes.every((entry, index) => entry.node === containerButtons[index])
);

// A delegated click's `target` is the element it came FROM, so a click on a middle row reaches the
// container's handler carrying that row's value.
const [readNamed] = container.signals[0];
list.$$click({ target: { value: "beta" } });
check(
  "the container's handler reads the row the click came from",
  readNamed() === "beta",
  JSON.stringify(readNamed())
);
list.$$click({ target: { value: "gamma" } });
check("and again for another row", readNamed() === "gamma", JSON.stringify(readNamed()));

// A click that lands on the container itself has no `value` to read. It must be the empty string
// rather than undefined, so a grid can tell "no cell" apart from a cell.
list.$$click({ target: {} });
check(
  "a click on the container itself reads the empty string, not undefined",
  readNamed() === "",
  JSON.stringify(readNamed())
);

// ------------------------- pattern A in a list that re-renders, which is a grid

const reactive = build(compiled.per_row_reactive);
// Two accessor inserts: the `$(picked.get())` hole first, then the list. The list is the last.
const reactiveList = reactive.inserts.filter((entry) => typeof entry.accessor === "function").at(-1);
check(
  "a loop over a signal hands `_$insert` an accessor, so its rows are rebuilt per render",
  reactiveList !== undefined && Array.isArray(reactiveList.accessor())
);

const [, setLimit] = reactive.signals[0];
const [readReactive] = reactive.signals[1];

const firstRender = buttons(reactiveList.accessor());
check("the first render is every row", firstRender.length === 3, firstRender.length);

// Re-render with a narrower list, which is what a grid does on every step.
setLimit(20);
const narrowed = buttons(reactiveList.accessor());
check("writing the signal renders the narrowed list", narrowed.length === 2, narrowed.length);
check(
  "whose rows are fresh nodes carrying fresh handlers",
  narrowed.every((button, index) => button.$$click !== firstRender[index].$$click)
);

// The same answer one nesting level deeper: the loop that rebuilds the rows lives inside the
// accessor, and each row of a render still captures its own environment.
const rebuilt = [];
for (const button of narrowed) {
  click(button, "");
  rebuilt.push(readReactive());
}
check(
  "a rebuilt list captures per row too",
  String(rebuilt) === "10,20",
  `${rebuilt.join(",")} (a shared environment would be 20,20)`
);

console.log();
console.log(failures === 0 ? "all checks passed" : `${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
