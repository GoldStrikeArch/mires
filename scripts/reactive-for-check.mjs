// Asserts that a `for` over a signal RE-RENDERS, and that a keyed one keeps its row nodes.
//
//   node scripts/reactive-for-check.mjs
//
// The dom suite's own trace cannot show this. `contract/harness/trace.mjs` records an `insert` and
// stops: it never calls the accessor it was handed, so a second render never happens, and it labels
// a node by the template it was cloned from, so three fresh rows read the same as three reused
// ones. That harness is the contract's, so this check goes around it rather than changing it.
//
// What it does instead: a run directory of its own, holding a DOM module that wraps two of the
// stub's entry points before re-exporting the rest, and an instrumented copy of the shim.
//
//   * `insert` keeps the accessor it was handed, so it can be called again by hand. That is the
//     whole reactive-`for` claim: a loop over a signal fills its hole with a function the runtime
//     subscribes to, not with an array it renders once.
//   * `createSignal` keeps the `[get, set]` pair, so the signal can be written from here without
//     the fixture having to export a setter for it.
//   * `__rt.keyed_row` records the node the eager Rust loop just built beside the node actually
//     contributed, which is what makes identity across a re-render observable.
//
// The order of the shim instrumentation matters and is the same trap `keyed-identity-check.mjs`
// documents: the shim installs `globalThis.__rt` itself, so a wrapper put in place before the
// import is thrown away by it. It is injected ahead of the generated `export const { .. }` clause
// instead.
//
// The fixture is examples/dom-tests/22_reactive_for.rs; build/domtest/22_reactive_for.js is what
// scripts/dom-test.sh leaves behind, so run that first.

import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/domtest");
const run = path.join(root, "build/reactivecheck");

fs.mkdirSync(run, { recursive: true });
fs.writeFileSync(path.join(run, "package.json"), '{ "type": "module" }\n');

const shim = fs.readFileSync(path.join(from, "shim.js"), "utf8");
const clause = shim.indexOf("export const {");
if (clause === -1) {
  console.log("FAIL  the generated shim has no export clause to inject ahead of");
  process.exit(1);
}
const instrument = `
globalThis.__keyedCalls = [];
{
  const real = globalThis.__rt.keyed_row;
  globalThis.__rt.keyed_row = (site, key, node) => {
    const got = real(site, key, node);
    globalThis.__keyedCalls.push({ site, key, built: node, got });
    return got;
  };
}
`;
fs.writeFileSync(
  path.join(run, "shim.js"),
  shim.slice(0, clause) + instrument + shim.slice(clause)
);

// The DOM module the compiled program imports. An explicitly exported name wins over a `export *`,
// so the two wrapped here replace the stub's and everything else comes through unchanged. The
// relative path to the contract harness resolves because this directory sits at the same depth
// below the root as the one the stub was written for.
fs.writeFileSync(
  path.join(run, "topcoat-dom.js"),
  `import * as trace from "../../contract/harness/trace.mjs";
export * from "../../contract/harness/trace.mjs";

globalThis.__inserts = [];
export function insert(parent, accessor, marker, initial) {
  globalThis.__inserts.push({ parent, accessor, marker });
  return trace.insert(parent, accessor, marker, initial);
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

fs.copyFileSync(path.join(from, "22_reactive_for.js"), path.join(run, "22_reactive_for.js"));

const compiled = await import(path.join(run, "22_reactive_for.js"));

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

// Each view is built on its own, so the inserts and the signal it made are unambiguous.
const build = (entry) => {
  globalThis.__inserts = [];
  globalThis.__signals = [];
  globalThis.__keyedCalls = [];
  entry();
  return {
    inserts: globalThis.__inserts,
    signal: globalThis.__signals[0],
    keyed: globalThis.__keyedCalls
  };
};

const titles = (rows) => rows.map((row) => String(row));

// ---------------------------------------------------------------- the accessor

const filtered = build(compiled.filtered);
// The list's insert is the one whose value is a function. The input's handler is a property write,
// not an insert, so there is exactly one insert in this view.
const listInsert = filtered.inserts.at(-1);
check(
  "a `for` over a signal hands `_$insert` an accessor, not an array",
  typeof listInsert.accessor === "function",
  typeof listInsert.accessor
);

const fixed = build(compiled.fixed);
// Two inserts: the `$(count.get())` hole's accessor, and the static list's array.
const staticList = fixed.inserts.find((entry) => Array.isArray(entry.accessor));
check(
  "a `for` that names no signal still hands over the array it built",
  staticList !== undefined && staticList.accessor.length === 4,
  staticList === undefined ? "no array insert" : `${staticList.accessor.length} rows`
);
check(
  "and the `$(..)` beside it is still its own accessor",
  fixed.inserts.some((entry) => typeof entry.accessor === "function")
);

// ------------------------------------------------------------- the re-render

const [, setQuery] = filtered.signal;
check(
  "and renders nothing until it is called: the rows are the accessor's, not the setup's",
  filtered.keyed.length === 0,
  filtered.keyed.length
);

globalThis.__keyedCalls = [];
const first = listInsert.accessor();
check("the first render of the filtered list is every row", first.length === 4, first.length);

const firstByKey = new Map(globalThis.__keyedCalls.map((call) => [call.key, call.got]));
check(
  "keyed by the four ids",
  String([...firstByKey.keys()]) === "1,2,3,4",
  [...firstByKey.keys()].join(",")
);
check(
  "every row of the first render contributes the node just built",
  globalThis.__keyedCalls.every((call) => call.got === call.built)
);

globalThis.__keyedCalls = [];
setQuery("al");
const narrowed = listInsert.accessor();
check(
  "writing the signal and re-running the accessor renders the FILTERED list",
  narrowed.length === 2,
  `${narrowed.length} rows`
);
check(
  "which is the two titles that match",
  String(globalThis.__keyedCalls.map((call) => call.key)) === "1,3",
  globalThis.__keyedCalls.map((call) => call.key).join(",")
);

// The identity claim: a key that survived the filter keeps the node it already had.
check(
  "every surviving row is the SAME node object it was before",
  globalThis.__keyedCalls.every((call) => call.got === firstByKey.get(call.key)),
  globalThis.__keyedCalls
    .map((call) => (call.got === firstByKey.get(call.key) ? "same" : "DIFFERENT"))
    .join(",")
);
check(
  "and the row the eager loop rebuilt for it is discarded",
  globalThis.__keyedCalls.every((call) => call.built !== call.got)
);
check(
  "the rendered array holds those same nodes, in order",
  narrowed.every((row, index) => row === globalThis.__keyedCalls[index].got)
);

// Widening again brings the filtered-out rows back, and they are the original nodes too: the cache
// is per call site and is never pruned, which is the documented cost and here the documented gain.
globalThis.__keyedCalls = [];
setQuery("");
const widened = listInsert.accessor();
check("clearing the signal renders every row again", widened.length === 4, widened.length);
check(
  "including the rows that had been filtered out, as their original nodes",
  globalThis.__keyedCalls.every((call) => call.got === firstByKey.get(call.key))
);

// ------------------------------------------------------- what keying is for

// The same reactive list without a key clause re-renders just as well, and builds new nodes every
// time. That is the delta keying buys, asserted rather than claimed.
const unkeyed = build(compiled.unkeyed);
const unkeyedInsert = unkeyed.inserts.at(-1);
check(
  "an unkeyed loop over a signal is reactive too",
  typeof unkeyedInsert.accessor === "function"
);
const before = unkeyedInsert.accessor();
const after = unkeyedInsert.accessor();
check("it renders the same number of rows each time", before.length === after.length, before.length);
check(
  "but every row is a FRESH node, which is what a key clause is for",
  before.every((row, index) => row !== after[index]),
  titles(before).length ? undefined : "no rows"
);
check("and it reaches no keyed cache at all", unkeyed.keyed.length === 0, unkeyed.keyed.length);

console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
