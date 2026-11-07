// Drives the compiled executor fixtures and asserts what they did.
//
//   node scripts/async-check.mjs
//
// The fixtures are examples/async-tests/*.rs and build/asynctest/*.js is what scripts/async-test.sh
// leaves behind, so run that first.
//
// Two kinds of check. The SHAPE checks read the emitted text: that the scheduler is a microtask,
// that the owner is captured through the DOM module rather than invented, that the promise bridge
// is the shim's. Running the program cannot tell you those -- a scheduler built out of `setTimeout`
// would still pass every behavioural check below, one frame later.
//
// The BEHAVIOUR checks run it. Each fixture calls `note(step)` at the points that matter and the
// driver compares the sequence, because the whole subject here is ORDER: whether a body ran inline
// or in a microtask, whether three wakes cost one poll or three, which of two tasks resumed first.

import { pathToFileURL } from "node:url";
import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/asynctest");

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

/** Everything a fixture recorded, in order. */
let steps = [];
/** The owners a fixture recorded, by tag. */
let owners = new Map();
/** Promises the fixture asked for and the driver has not settled yet. */
let held = [];

globalThis.note = (step) => {
  steps.push(step);
};
globalThis.noteOwner = (tag, owner) => {
  owners.set(tag, owner);
};
globalThis.resolved = (value) => Promise.resolve(value);
globalThis.rejected = (reason) => Promise.reject(reason);
globalThis.plain = (value) => value;
globalThis.pending = () => {
  let settle;
  const promise = new Promise((resolve) => {
    settle = resolve;
  });
  held.push(settle);
  return promise;
};

const reset = () => {
  steps = [];
  owners = new Map();
  held = [];
};

/**
 * Lets every queued microtask run, several turns deep.
 *
 * One `await` drains one turn, and a task that resumes queues the next one, so a chain of N
 * suspensions needs N turns. Ten is far more than any fixture here uses and still finite, so a
 * fixture that never settles fails rather than hanging.
 */
const settleAll = async () => {
  for (let turn = 0; turn < 10; turn += 1) {
    await Promise.resolve();
  }
};

// An unhandled rejection anywhere in this process is a failure of fixture 03's whole point, so it
// is watched for the entire run rather than around one call.
let unhandled = [];
process.on("unhandledRejection", (reason) => {
  unhandled.push(reason);
});

const text = (name) => fs.readFileSync(path.join(from, `${name}.js`), "utf8");
const load = (name) => import(pathToFileURL(path.join(from, `${name}.js`)).href);

// ---------------------------------------------------------------- the shape

{
  const emitted = text("02_owner");
  check(
    "shape: the scheduler is a microtask through the shim",
    /__rt\.microtask\(\(\) =>/.test(emitted),
    emitted.match(/.*__rt\.microtask.*/)?.[0]?.trim()
  );
  check(
    "shape: the promise bridge is the shim's `settled`",
    /__rt\.settled\(/.test(emitted),
    emitted.match(/.*__rt\.settled.*/)?.[0]?.trim()
  );
  check(
    "shape: the owner is captured through the DOM module",
    /import \{ getOwner as _\$getOwner \} from "\.\/mires-dom\.js";/.test(emitted)
  );
  check(
    "shape: and re-entered through it",
    /import \{ runWithOwner as _\$runWithOwner \} from "\.\/mires-dom\.js";/.test(emitted) &&
      /_\$runWithOwner\(/.test(emitted),
    emitted.match(/.*_\$runWithOwner\(.*/)?.[0]?.trim()
  );
  // The waker's data is a task id and never an address: a `RawWaker` built from a heap pointer is
  // the one shape this value model has a miscompile history with.
  check(
    "shape: the waker vtable is a plain record of four functions",
    /VTABLE\$?\w* = \{ clone: /.test(emitted),
    emitted.match(/.*VTABLE.* = \{ clone.*/)?.[0]?.trim()?.slice(0, 120)
  );
}

// ------------------------------------------------------- 01: once, and deferred

{
  reset();
  const js = await load("01_microtask");
  js.run();
  check(
    "01: the body did NOT run inline",
    JSON.stringify(steps) === JSON.stringify(["before", "after"]),
    JSON.stringify(steps)
  );
  await settleAll();
  check(
    "01: it ran in a microtask, after the spawning stack unwound",
    JSON.stringify(steps) === JSON.stringify(["before", "after", "body"]),
    JSON.stringify(steps)
  );
  await settleAll();
  check(
    "01: and exactly once, however many turns pass",
    steps.filter((step) => step === "body").length === 1,
    JSON.stringify(steps)
  );
}

// ------------------------------------------------- 02: the owner, and the write

{
  reset();
  const dom = await import(pathToFileURL(path.join(from, "mires-dom.js")).href);
  const js = await load("02_owner");
  // The island's own owner. `run` is called under it, the way an island's setup is.
  const island = { name: "island" };
  const signal = dom.runWithOwner(island, () => js.run());

  check("02: the signal starts at its initial value", signal[0]() === 0, String(signal[0]()));
  check("02: the owner was captured at the spawn", owners.get("spawn") === island);
  check("02: nothing resumed yet", owners.has("resume") === false);
  // Nothing is running now, so an owner leaking out of the executor would show as a non-null one.
  check("02: the owner is restored after the synchronous stack", dom.getOwner() === null);

  await settleAll();
  check(
    "02: the continuation ran under the owner the spawn captured",
    owners.get("resume") === island,
    String(owners.get("resume") && owners.get("resume").name)
  );
  check(
    "02: the resolved value crossed as itself",
    JSON.stringify(steps) === JSON.stringify(["reply"]),
    JSON.stringify(steps)
  );
  check("02: the write landed on the signal", signal[0]() === 1, String(signal[0]()));
  check("02: and the owner is restored again afterwards", dom.getOwner() === null);
}

// --------------------------------------------- 03: a rejection is a value, not a rejection

{
  reset();
  const js = await load("03_rejected");
  js.run_rejected();
  js.run_resolved();
  await settleAll();
  check(
    "03: the rejection arrived inside the poll, as a value",
    steps.includes("boom"),
    JSON.stringify(steps)
  );
  check(
    "03: and the resolved arm still arrives through the other one",
    steps.includes("fine"),
    JSON.stringify(steps)
  );
  check(
    "03: nothing reached the host as an unhandled rejection",
    unhandled.length === 0,
    JSON.stringify(unhandled.map(String))
  );
}

// ------------------------------------------------------- 04: wakes are coalesced

{
  reset();
  const js = await load("04_coalesced");
  js.run();
  await settleAll();
  check(
    "04: three wakes before the queued poll cost ONE further poll",
    steps.length === 2,
    JSON.stringify(steps)
  );
  check(
    "04: and the task finished",
    JSON.stringify(steps) === JSON.stringify(["poll", "poll"]),
    JSON.stringify(steps)
  );
}

// -------------------------------------------------------- 05: two tasks at once

{
  reset();
  const js = await load("05_concurrent");
  js.run();
  await settleAll();
  check(
    "05: both bodies started, in spawn order",
    JSON.stringify(steps) === JSON.stringify(["first-start", "second-start"]),
    JSON.stringify(steps)
  );
  check("05: both are suspended on their own promise", held.length === 2, String(held.length));

  // Settled in the OPPOSITE order to the spawns: a table that answered the wrong task would
  // resume the wrong one, which reordering is what makes visible.
  held[1]("second");
  await settleAll();
  check(
    "05: the second task resumed, and only it",
    JSON.stringify(steps) === JSON.stringify(["first-start", "second-start", "second-end"]),
    JSON.stringify(steps)
  );

  held[0]("first");
  await settleAll();
  check(
    "05: then the first, so neither evicted the other",
    JSON.stringify(steps) ===
      JSON.stringify(["first-start", "second-start", "second-end", "first-end"]),
    JSON.stringify(steps)
  );

  reset();
  js.run_plain();
  check("05: awaiting a plain value does not resolve inline", steps.length === 0);
  await settleAll();
  check(
    "05: it settles in a microtask, successfully",
    JSON.stringify(steps) === JSON.stringify(["here"]),
    JSON.stringify(steps)
  );
}

console.log(`${failures === 0 ? "PASS" : "FAIL"}: ${failures} failed`);
process.exit(failures === 0 ? 0 : 1);
