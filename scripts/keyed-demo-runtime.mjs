// Derives the keyed runtime module the demo island imports from runtime/shim.js.
//
//   node scripts/keyed-demo-runtime.mjs
//   node scripts/keyed-demo-runtime.mjs --check
//
// The demo has an application-owned `__rt` module for its host functions. A
// keyed island additionally needs the compiler runtime's keyed state machine,
// but copying that implementation into the application would give it two
// sources of truth. Keep a generated minimal module beside the application
// instead, and re-export its compiler-facing entry point from island-rt.mjs.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { minimalShim } from "./minimal-shim.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const shim = readFileSync(path.join(root, "runtime", "shim.js"), "utf8");
const output = path.join(root, "demo-app", "src", "keyed-rt.mjs");

// The DOM bundle calls the insert hook during hydration through globalThis.__rt.
// Include that external root alongside the compiler entry point and metrics;
// the extractor follows both runtime members and their lexical helpers.
// The shared island host also exposes byte-slice initialization and aborts.
const generated = minimalShim(
  [
    "globalThis.__rt.byte_slice_ref;",
    "globalThis.__rt.js_abort;",
    "globalThis.__rt.keyed_dom_insert;",
    "globalThis.__rt.keyed_dom_view;",
    "globalThis.__rt.keyed_metrics_reset;",
    "globalThis.__rt.keyed_metrics_snapshot;",
  ],
  shim,
  { module: true },
).source;

if (process.argv.includes("--check")) {
  let current;
  try {
    current = readFileSync(output, "utf8");
  } catch {
    throw new Error(`${output} is missing; run node scripts/keyed-demo-runtime.mjs`);
  }
  if (current !== generated) {
    throw new Error(`${output} is stale; run node scripts/keyed-demo-runtime.mjs`);
  }
  console.log("keyed demo runtime: current");
} else {
  writeFileSync(output, generated);
  console.log(`keyed demo runtime: ${output}`);
}
