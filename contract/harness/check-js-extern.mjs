// Drive the fake chart library through every reference emission and record the
// resulting call vectors.
//
//   node --import ./register-loader.mjs check-js-extern.mjs            compare
//   node --import ./register-loader.mjs check-js-extern.mjs --record   rewrite
//
// WHAT THIS STEP IS FOR
// ---------------------
// `#[js_extern]` has no upstream to extract from -- the descriptor design is
// ours, borrowed in shape from Melange's `External_spec` and not from any
// running code. So this file cannot work the way `extract-abi.mjs` does. It
// works the way `run-trace.mjs` does instead: execute a reference emission
// against a recorder and commit what it did.
//
// That alone would be circular -- a driver cannot disagree with its own
// recording -- so every driver also carries hand-written assertions about the
// SHAPE it is supposed to be demonstrating, and those run on every invocation
// whether or not the file is being rewritten. A driver that stopped exercising
// its case fails here rather than silently re-recording.
//
// For the backend: `compareTrace` is exported. Run a compiled module against
// `fixtures/js-extern/chart-lib.mjs`, take `__trace()`, and compare it with the
// vector of the same id. The emitted JS does not have to match the driver's
// text; it has to be indistinguishable at the trace.

import { readFileSync, existsSync, mkdirSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { FIXTURES } from "./paths.mjs";

const DIR = path.join(FIXTURES, "js-extern");
const VECTORS = path.join(DIR, "vectors.json");

const record = process.argv.includes("--record");

const lib = await import(pathToFileURL(path.join(DIR, "chart-lib.mjs")).href);
const { DRIVERS, env } = await import(pathToFileURL(path.join(DIR, "drivers.mjs")).href);

// ------------------------------------------------------------------ reporting
const failures = [];
let asserted = 0;

function check(what, condition) {
  asserted++;
  if (!condition) failures.push(what);
}

// --------------------------------------------------------------- driver source
/**
 * The driver's own source text, split into lines and dedented.
 *
 * Recorded rather than hand-transcribed alongside it: a prose "the emission is
 * `chart.update(data)`" next to a driver that does something else is exactly the
 * kind of disagreement this directory keeps running into. Here the text in the
 * fixture IS the text that was executed, so it cannot describe a different call.
 *
 * A consequence worth stating: reformatting `drivers.mjs` moves these bytes and
 * `run-all.mjs --check` reports drift. That is correct. The reference emission
 * is a contract artifact and changing it should be visible.
 */
function source(fn) {
  const lines = fn.toString().split("\n");
  if (lines.length === 1) return lines;
  const indents = lines
    .slice(1)
    .filter((l) => l.trim().length > 0)
    .map((l) => l.length - l.trimStart().length);
  const strip = indents.length ? Math.min(...indents) : 0;
  return [lines[0], ...lines.slice(1).map((l) => l.slice(strip))];
}

// -------------------------------------------------------------------- running
function run(driver) {
  lib.__reset();

  // Setup builds whatever the vector needs to operate ON, and is deliberately
  // not part of the measurement: a vector about `update` should contain the
  // update and not the construction that made an instance to update. Labels are
  // computed at record time, so the instance still reads as `chart#0` when it
  // first appears below.
  let subject;
  if (driver.setup) subject = driver.setup(lib, env);
  lib.__reset();

  let returned;
  let threw = null;
  try {
    returned = lib.__value(driver.run(lib, subject));
  } catch (err) {
    threw = `${err && err.constructor ? err.constructor.name : "Error"}: ${err && err.message}`;
    returned = "undefined";
  }

  return {
    id: driver.id,
    shape: driver.shape,
    required: driver.required,
    ...(driver.returnWrapper ? { returnWrapper: driver.returnWrapper } : {}),
    why: driver.why,
    ...(driver.setup ? { setup: source(driver.setup) } : {}),
    js: source(driver.run),
    trace: lib.__trace(),
    returned,
    threw
  };
}

const vectors = [];
for (const driver of DRIVERS) {
  const vector = run(driver);
  vectors.push(vector);

  for (const [what, ok] of driver.assert(vector)) {
    check(`${driver.id}: ${what}`, ok);
  }
}

// Structural checks over the whole set, not over one vector: these are the ones
// that catch a vector being deleted or an id being reused, which no per-vector
// assertion can see.
const ids = vectors.map((v) => v.id);
check("every vector id is unique", new Set(ids).size === ids.length);
for (const shape of ["new", "send", "get", "index"]) {
  const covered = vectors.filter((v) => v.shape === shape && v.required);
  check(`the required shape \`${shape}\` has at least one vector`, covered.length > 0);
}
check(
  "the nullable return wrapper is covered on both arms",
  vectors.filter((v) => v.returnWrapper === "nullToOption").length === 2
);
check(
  "null and undefined are told apart across the two fallible reads",
  vectors.find((v) => v.id === "nullable-none").returned === null &&
    vectors.find((v) => v.id === "index-out-of-range").returned === "undefined"
);

// ------------------------------------------------------- the non-vacuity proof
//
// Every assertion above passed. So would an assertion that said nothing. The
// question a recorded fixture always has to answer is whether its claims are
// SPECIFIC to the thing recorded, and the cheap way to answer it is to run each
// vector's assertions against every OTHER vector's recording: if `send-one-arg`
// holds for the trace of `index-length`, it was never a claim about `send`.
//
// A driver whose assertions throw on foreign data counts as having rejected it.
// Throwing is how "the record I indexed into is not there" surfaces, and that is
// a rejection, not an error.
function holds(driver, data) {
  try {
    return driver.assert(data).every(([, ok]) => ok);
  } catch {
    return false;
  }
}

const confusable = [];
for (const driver of DRIVERS) {
  for (const other of vectors) {
    if (other.id === driver.id) continue;
    if (holds(driver, other)) confusable.push(`${driver.id} also holds for ${other.id}`);
  }
}
check(
  `no vector's assertions hold for another vector's recording${
    confusable.length ? `\n    ${confusable.join("\n    ")}` : ""
  }`,
  confusable.length === 0
);

// ------------------------------------------------------------------ the fixture
const fixture = {
  $generatedBy: "contract/harness/check-js-extern.mjs",
  $doNotEditByHand:
    "Re-run the driver instead. Edit fixtures/js-extern/drivers.mjs, then re-record.",
  $whatThisIs:
    "The expected JS-level call trace for each `#[js_extern]` call shape, produced by executing the reference emission in `js` against fixtures/js-extern/chart-lib.mjs.",
  $notDerivedFromUpstream:
    "Unlike every other fixture here, nothing on this page comes from a pinned upstream package. The descriptor design is ours; the fake library is ours. What keeps it honest is the assertions in drivers.mjs, which run on every check.",
  $howTheBackendUsesThis: [
    "Compile a module whose `#[js_extern]` declarations describe chart-lib.mjs.",
    "Resolve its import specifier for the library to fixtures/js-extern/chart-lib.mjs (the way run-trace.mjs resolves `r-dom` to the recording stub).",
    "Drive it so it performs the operation named by a vector's `id`.",
    "Compare `__trace()` with that vector's `trace` using compareTrace() from check-js-extern.mjs.",
    "The emitted JS need not match `js`. It must be indistinguishable at the trace."
  ],
  $shapes: {
    new: "construct: `new B(...)`. Negative arm included -- a plain call throws.",
    send: "a method on an instance: `x.m(...)`. Argument count is observable.",
    get: "a property read, rooted at an instance or at the module binding, one or more scope steps.",
    set: "a property assignment. Not required this wave.",
    index: "an element read out of an indexed collection, by a runtime index.",
    call: "a function reached through a scope path: `B.m(...)`. Not required this wave."
  },
  $env: env,
  vectors
};

if (record) {
  if (!existsSync(DIR)) mkdirSync(DIR, { recursive: true });
  const { writeFileSync } = await import("node:fs");
  writeFileSync(VECTORS, `${JSON.stringify(fixture, null, 2)}\n`);
  process.stdout.write(`wrote ${path.relative(FIXTURES, VECTORS)}\n`);
} else if (!existsSync(VECTORS)) {
  failures.push(`${VECTORS} does not exist; run with --record`);
} else {
  const committed = readFileSync(VECTORS, "utf8");
  const fresh = `${JSON.stringify(fixture, null, 2)}\n`;
  check("the committed vectors match what the drivers produce now", committed === fresh);
}

/**
 * Compare a recorded trace against a vector's expected trace.
 *
 * Deliberately NOT the LCS alignment `compare-trace.mjs` does. That comparator
 * exists because two emitters can order independent runtime calls differently
 * and still be correct; here the trace is the sequence of operations one
 * expression performed on one object, and reordering it changes what happened.
 * So this compares position by position and reports the first divergence with
 * both sides, plus any length difference.
 *
 * Returns an array of human-readable differences; empty means equal.
 */
export function compareTrace(actual, expected) {
  const diffs = [];
  const n = Math.max(actual.length, expected.length);
  for (let i = 0; i < n; i++) {
    const a = actual[i];
    const e = expected[i];
    if (a === undefined) {
      diffs.push(`[${i}] missing: expected ${JSON.stringify(e)}`);
      continue;
    }
    if (e === undefined) {
      diffs.push(`[${i}] unexpected: ${JSON.stringify(a)}`);
      continue;
    }
    const as = JSON.stringify(a);
    const es = JSON.stringify(e);
    if (as !== es) diffs.push(`[${i}] expected ${es}\n      actual   ${as}`);
  }
  return diffs;
}

/** Look a vector up by id, for the backend's own runner. */
export function vector(id) {
  const found = vectors.find((v) => v.id === id);
  if (!found) throw new Error(`no js-extern vector with id ${id}`);
  return found;
}

process.stdout.write(`${vectors.length} vectors, ${asserted} assertions\n`);
for (const shape of ["new", "send", "get", "set", "index", "call"]) {
  const n = vectors.filter((v) => v.shape === shape).length;
  process.stdout.write(`  ${shape}: ${n}\n`);
}

if (failures.length) {
  for (const f of failures) process.stdout.write(`FAIL ${f}\n`);
  process.stdout.write(`${failures.length} failure(s)\n`);
  process.exit(1);
}
process.stdout.write("all js-extern vectors hold\n");
