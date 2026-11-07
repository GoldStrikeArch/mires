// Assert the built bundle exports exactly the dom-expressions client ABI plus
// the declared reactive additions.
//
//   node check-exports.mjs
//
// Exits non-zero on any difference in either direction: a missing name means
// the emitter cannot link, an extra name means the artifact carries surface we
// have not pinned.
//
// The expected list is the union of two pinned sources, and it has to be:
// abi.json is extracted from upstream's client.js and cannot describe a name
// upstream does not export, while the emitter also needs `createSignal`. That
// name is pinned in reactive-additions.mjs instead.
//
// The bundle targets browsers, so importing it under node needs a few DOM
// globals to exist at module-evaluation time. We install the minimum inert
// stand-ins rather than pulling in jsdom -- nothing here exercises behavior, it
// only reads the export list.

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { reactiveAdditions } from "./reactive-additions.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const distPath = path.join(here, "dist", "mires-dom.js");
const abiPath = path.resolve(here, "..", "..", "contract", "fixtures", "abi.json");

const abi = JSON.parse(readFileSync(abiPath, "utf8"));
const abiNames = abi.exports.map((e) => e.name);
const additions = Object.keys(reactiveAdditions);
const expected = [...abiNames, ...additions].sort();

// --- minimal browser globals, so a browser-targeted bundle can be evaluated
const noop = () => {};
const fakeElement = () => ({
  appendChild: noop,
  removeChild: noop,
  setAttribute: noop,
  addEventListener: noop,
  removeEventListener: noop,
  style: { setProperty: noop, removeProperty: noop },
  content: { firstChild: null },
  firstChild: null,
  nextSibling: null,
  childNodes: []
});

globalThis.document ??= {
  createElement: fakeElement,
  createElementNS: fakeElement,
  createTextNode: () => ({}),
  createComment: () => ({}),
  addEventListener: noop,
  removeEventListener: noop,
  querySelectorAll: () => [],
  head: fakeElement(),
  body: fakeElement()
};
globalThis.window ??= globalThis;
globalThis.navigator ??= { userAgent: "node" };

const mod = await import(pathToFileURL(distPath).href);
const actual = Object.keys(mod).sort();

const missing = expected.filter((n) => !actual.includes(n));
const extra = actual.filter((n) => !expected.includes(n));

console.log(
  `expected          ${expected.length} exports ` +
    `(${abiNames.length} from abi.json + ${additions.length} reactive addition(s))`
);
console.log(`bundle provides   ${actual.length} exports`);

if (missing.length) {
  console.error(`\nMISSING from the bundle (${missing.length}):`);
  for (const n of missing) console.error(`  ${n}`);
}
if (extra.length) {
  console.error(`\nEXTRA in the bundle (${extra.length}):`);
  for (const n of extra) console.error(`  ${n}`);
}

if (missing.length || extra.length) {
  console.error("\nexport set does NOT match the pinned surface");
  process.exit(1);
}

// Also confirm every name the emitter is specified to call is genuinely
// callable/present, not merely exported as undefined.
const required = [...abi.emitterRequired.names, ...additions];
const notUsable = required.filter((n) => mod[n] === undefined);
if (notUsable.length) {
  console.error(`\nemitter-required exports present but undefined: ${notUsable.join(", ")}`);
  process.exit(1);
}

// The additions only mean anything if they share one reactive graph with the
// ABI's own reactivity: an external signal cannot notify these effects. Reading
// a signal inside `effect` and writing it has to re-run the effect.
const [read, write] = mod.createSignal(1);
const observed = [];
mod.effect(() => observed.push(read()));
write(2);
if (observed.length < 2 || observed[observed.length - 1] !== 2) {
  console.error(
    `\ncreateSignal is not in the same reactive graph as the ABI's effect: ` +
      `effect saw ${JSON.stringify(observed)}, expected it to re-run and see 2`
  );
  process.exit(1);
}

console.log(`\nexport set matches the pinned surface exactly (${expected.length} names)`);
console.log(`all ${required.length} emitter-required names are defined`);
console.log(`createSignal and effect share one reactive graph`);
