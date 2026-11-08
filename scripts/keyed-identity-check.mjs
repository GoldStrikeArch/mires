// Asserts that a keyed `for` preserves row node IDENTITY across renders.
//
//   node scripts/keyed-identity-check.mjs
//
// The dom suite's own trace cannot show this. `contract/harness/trace.mjs` labels a node by the
// template it was cloned from (`nodeName`, :159-171), so every clone of one template reads as
// `tmpl#1:root` and three fresh rows are indistinguishable from three reused ones. That harness is
// the contract's, so this check goes around it rather than changing it.
//
// What it does instead: `__rt.keyed_row` is reached through the global `__rt` object at call time,
// not through an import binding, so it can be wrapped once the module is loaded and before its
// entry point runs. The order matters: the compiled program imports the shim, and the shim installs
// `globalThis.__rt` itself, so a wrapper put in place BEFORE the import is thrown away by it. Every
// call records the site, the key, the node the eager Rust loop just built, and the node actually
// contributed. That is exactly the fact worth pinning:
//
//   * a key seen for the first time contributes the node just built;
//   * a key seen before contributes the node it contributed the first time, and the freshly built
//     one is discarded -- which is the documented cost of keying an eager loop.
//
// The fixture is examples/dom-tests/20_keyed_for.rs; build/domtest/20_keyed_for.js is what
// scripts/dom-test.sh leaves behind, so run that first.

import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/domtest");
const run = path.join(root, "build/keyedcheck");

// A run directory of its own, holding an INSTRUMENTED copy of the shim. The instrumentation goes in
// just before the generated `export const { .. }` clause, because that clause destructures
// `globalThis.__rt` at the end of the file: a wrapper installed before it is the one that gets
// exported, and the compiled program's `import * as __rt` then sees it. Patching after the import
// cannot work -- a module namespace object's bindings are not writable.
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

// The program resolves both of its imports relative to itself, so both come along. The dom stub
// reaches the contract harness with a path relative to its own directory, and this directory is the
// same depth below the root as the one it was written for, so the path still resolves.
fs.copyFileSync(path.join(from, "topcoat-dom.js"), path.join(run, "topcoat-dom.js"));
fs.copyFileSync(path.join(from, "20_keyed_for.js"), path.join(run, "20_keyed_for.js"));

const compiled = await import(path.join(run, "20_keyed_for.js"));
compiled.rust_entry();

const calls = globalThis.__keyedCalls ?? [];
if (calls.length === 0) {
  console.log("FAIL  the instrumented shim saw no `keyed_row` call at all");
  process.exit(1);
}

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

// The three `keyed()` renders are the first nine calls: ids 1,2,3 then 3,1,2 then 2,4,1.
const renders = [calls.slice(0, 3), calls.slice(3, 6), calls.slice(6, 9)];
const keysOf = (render) => render.map((call) => call.key);

check("the first render is keys 1,2,3", String(keysOf(renders[0])) === "1,2,3", keysOf(renders[0]));
check("the second render is keys 3,1,2", String(keysOf(renders[1])) === "3,1,2", keysOf(renders[1]));
check("the third render is keys 2,4,1", String(keysOf(renders[2])) === "2,4,1", keysOf(renders[2]));

check(
  "every row of the first render contributes the node just built",
  renders[0].every((call) => call.got === call.built)
);

// The identity claim. Same key, same node object, across a reorder.
const firstByKey = new Map(renders[0].map((call) => [call.key, call.got]));
check(
  "the reordered render contributes the SAME node object for each key",
  renders[1].every((call) => call.got === firstByKey.get(call.key)),
  renders[1].map((call) => (call.got === firstByKey.get(call.key) ? "same" : "DIFFERENT")).join(",")
);
check(
  "and discards the row the eager loop rebuilt for it",
  renders[1].every((call) => call.built !== call.got)
);

// A key never seen before is new; the keys around it are still cached.
const grown = renders[2];
check(
  "a key seen for the first time contributes its own new node",
  grown.filter((call) => call.key === "4").every((call) => call.got === call.built)
);
check(
  "beside cached nodes for the keys that were seen",
  grown
    .filter((call) => call.key !== "4")
    .every((call) => call.got === firstByKey.get(call.key))
);

// Two loops must not share a cache: `keyed` and `keyed_by_str` are different sites.
const sites = new Set(calls.map((call) => call.site));
check("each keyed loop has a site of its own", sites.size === 2, [...sites].join(" "));
check(
  "and a site is qualified by the crate that compiled it",
  [...sites].every((site) => site.startsWith("20_keyed_for#")),
  [...sites].join(" ")
);

console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
