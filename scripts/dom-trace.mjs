// Runs a compiled client program against the contract's recording stub and prints its trace.
//
//   node scripts/dom-trace.mjs <program.js> <label>
//
// The program is an ES module this backend compiled: it imports the dom-expressions runtime from
// the specifier `-Cllvm-args=js-dom-module` named, and it exports `rust_entry`. scripts/dom-test.sh
// lays out a directory where that specifier resolves to a re-export of contract/harness/trace.mjs,
// so the calls the program makes are recorded rather than performed.
//
// The output is the same JSON Lines shape contract/harness/run-trace.mjs writes -- a header record
// followed by one record per runtime call -- so contract/harness/compare-trace.mjs reads it beside
// a reference trace without knowing which side produced which.
//
// A reference module runs at import time, because every fixture case is a module level `const`. A
// compiled Rust program has no module level side effects at all, so the entry point is called
// explicitly; that call is what the recorded trace is of.

import path from "node:path";
import { pathToFileURL } from "node:url";

const [program, label] = process.argv.slice(2);
if (!program) {
  console.error("usage: dom-trace.mjs <program.js> [label]");
  process.exit(2);
}

const here = path.dirname(new URL(import.meta.url).pathname);
const tracePath = path.resolve(here, "..", "contract", "harness", "trace.mjs");

// The same module instance the program's dom module re-exports: node keys the cache by resolved
// URL, so `__trace()` here sees what the program's calls recorded.
const trace = await import(pathToFileURL(tracePath).href);

let status = "completed";
let error = null;

trace.__reset();
try {
  const compiled = await import(pathToFileURL(path.resolve(program)).href);
  if (typeof compiled.rust_entry !== "function") {
    throw new Error("the compiled program exports no `rust_entry`");
  }
  compiled.rust_entry();
} catch (err) {
  status = "stopped-early";
  error = String((err && err.message) || err);
}

const records = trace.__trace();
const lines = [
  JSON.stringify({
    $record: "header",
    $generatedBy: "scripts/dom-trace.mjs",
    $what:
      "Normalized runtime-call trace of a rustc_codegen_js-compiled client module executed " +
      "against contract/harness/trace.mjs.",
    fixture: label ?? path.basename(program, ".js"),
    label: label ?? path.basename(program, ".js"),
    freeBindingsHealed: [],
    status,
    error,
    recordCount: records.length
  })
];
for (const record of records) lines.push(JSON.stringify(record));
process.stdout.write(`${lines.join("\n")}\n`);

if (status !== "completed") process.exitCode = 1;
