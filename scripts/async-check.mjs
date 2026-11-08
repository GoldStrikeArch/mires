// Drives the compiled executor fixtures and asserts what they did.
//
//   node scripts/async-check.mjs [<dir>] [<fixture>...]
//
// The fixtures are examples/async-tests/*.rs and <dir>/<fixture>.js is what scripts/async-test.sh
// leaves behind, so run that first. <dir> defaults to build/asynctest/$ASYNC_OPT_MODE (`off` when
// unset), and ASYNC_OPT_MODE says which executor path the shape checks expect. With no fixture
// named, every fixture this repository can build is checked; 06_procedure needs the Topcoat
// framework's crates (see scripts/async-test.sh) and is checked only when named. Each fixture's
// verdict is printed as a `summary <fixture> <ok> <failed>` line once its checks are done.
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
const [dirArgument, ...named] = process.argv.slice(2);
const from = path.resolve(
  dirArgument ?? path.join(root, "build/asynctest", process.env.ASYNC_OPT_MODE ?? "off")
);
const BUILDABLE = [
  "01_microtask",
  "02_owner",
  "03_rejected",
  "04_coalesced",
  "05_concurrent",
  "07_cancellation",
  "08_subscription",
];
const selected = named.length > 0 ? named : BUILDABLE;

let failures = 0;
/** The fixture the checks below are about, and every fixture's `[ok, failed]` tally. */
let current = null;
const tally = new Map();
/** `count` is how many assertions one line stands for: the shared checks report theirs. */
const check = (label, ok, detail, count = 1) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  const counts = tally.get(current) ?? [0, 0];
  counts[ok ? 0 : 1] += ok ? count : 1;
  tally.set(current, counts);
  if (!ok) {
    failures += 1;
  }
};
/**
 * Prints the finished fixture's verdict as soon as the next one starts, so a check that throws out
 * of a host callback and ends the process still leaves the verdicts that were reached.
 */
const reported = new Set();
const summarize = () => {
  if (current !== null && !reported.has(current)) {
    const [ok, failed] = tally.get(current) ?? [0, 0];
    console.log(`summary ${current} ${ok} ${failed}`);
    reported.add(current);
  }
};
/** Whether `name` was asked for and its program exists; a missing program is a failure. */
const want = (name) => {
  if (!selected.includes(name)) {
    return false;
  }
  if (name !== current) {
    summarize();
  }
  current = name;
  const present = fs.existsSync(path.join(from, `${name}.js`));
  if (!present) {
    check(`${name}: compiled program present`, false, path.join(from, `${name}.js`));
  }
  return present;
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

// ------------------------------------------------------- 01: once, and deferred

if (want("01_microtask")) {
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

// ---------------------------------------------------------------- the shape
//
// Release builds are minified, so the patterns allow for the compact spelling. Minification also
// renames the items, so whether the old Rust task table is linked is answered from the item names
// the link step's lane report (`<program>.lane.json`) records, as well as from the text.

if (want("02_owner")) {
  const emitted = text("02_owner");
  let linkedItems = "";
  try {
    const report = JSON.parse(fs.readFileSync(path.join(from, "02_owner.js.lane.json"), "utf8"));
    linkedItems = report.items.map((item) => item.name).join("\n");
  } catch {
    // No lane report: the emitted text is all there is to read.
  }
  const optimized = process.env.ASYNC_OPT_MODE === "on";
  if (optimized) {
    check(
      "shape: the compiler emitted the shared host future driver",
      /__rt\.host_spawn\(/.test(emitted),
      emitted.match(/.*__rt\.host_spawn.*/)?.[0]?.trim()
    );
  } else {
    check(
      "shape: off/report uses the safe Rust microtask executor",
      /__rt\.microtask\(/.test(emitted) && /__rt\.host_local_waker\(/.test(emitted)
    );
  }
  check(
    "shape: the promise bridge is the shim's `settled`",
    /__rt\.settled\(/.test(emitted),
    emitted.match(/.*__rt\.settled.*/)?.[0]?.trim()
  );
  check(
    "shape: the owner is captured through the DOM module",
    /import ?\{ ?getOwner as _\$getOwner ?\} ?from ?"\.\/topcoat-dom\.js";/.test(emitted)
  );
  check(
    "shape: and re-entered through it",
    /import ?\{ ?runWithOwner as _\$runWithOwner ?\} ?from ?"\.\/topcoat-dom\.js";/.test(emitted) &&
      (optimized
        ? /__rt\.host_spawn\([^;]*_\$runWithOwner/.test(emitted)
        : /_\$runWithOwner\(/.test(emitted)),
    emitted.match(/.*_\$runWithOwner.*/)?.[0]?.trim()
  );
  const taskTable = /view_async(?:::|\$)task(?:::|\$)(?:install|lookup|waker_for)|RawWakerVTable|static mut TASKS/;
  check(
    "shape: no Rust task table or RawWaker is linked",
    !taskTable.test(emitted) && !taskTable.test(linkedItems)
  );
}

// ------------------------------------------------- 02: the owner, and the write

if (want("02_owner")) {
  reset();
  const dom = await import(pathToFileURL(path.join(from, "topcoat-dom.js")).href);
  const js = await load("02_owner");
  // The island's own owner. `run` is called under it, the way an island's setup is.
  const island = { name: "island", cleanups: null };
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

// The same compiled owner-aware future also accepts a getter-backed thenable. The
// bridge must call the retrieved method, preserving its receiver, after one lookup.
if (want("02_owner")) {
  reset();
  const dom = await import(pathToFileURL(path.join(from, "topcoat-dom.js")).href);
  const js = await load("02_owner");
  const original = globalThis.resolved;
  const trace = [];
  globalThis.resolved = (value) => {
    const thenable = Object.create({
      get then() {
        trace.push("get");
        return new Proxy(function () {}, {
          get() { throw new Error("unexpected callable property read"); },
          apply(target, receiver, args) {
            trace.push(receiver === thenable ? "receiver" : "wrong receiver");
            args[0](value);
          },
        });
      },
    });
    return thenable;
  };
  try {
    const owner = { name: "thenable", cleanups: null };
    const signal = dom.runWithOwner(owner, () => js.run());
    check("02 thenable: the task is still deferred", trace.length === 0 && signal[0]() === 0);
    await settleAll();
    check("02 thenable: one lookup and the original receiver", JSON.stringify(trace) === JSON.stringify(["get", "receiver"]), JSON.stringify(trace));
    check("02 thenable: the compiled future resumes once", JSON.stringify(steps) === JSON.stringify(["reply"]) && signal[0]() === 1);
    check("02 thenable: owner reentry and restoration", owners.get("resume") === owner && dom.getOwner() === null);
  } finally {
    globalThis.resolved = original;
  }
}

// --------------------------------------------- 03: a rejection is a value, not a rejection

if (want("03_rejected")) {
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

if (want("04_coalesced")) {
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

if (want("05_concurrent")) {
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

// ------------------------------------- 06: a procedure, through the framework's own expansion
//
// Not built by scripts/async-test.sh: it needs the framework's `topcoat_runtime_macro` and
// `topcoat_dom`, which this repository does not have. It runs when named and present.

if (want("06_procedure")) {
  // Every request the compiled program made: the url, the media type and the body. The client
  // half builds the whole `fetch` init itself with a `js!{}` block, so what is recorded here is
  // the real thing rather than the arguments of a host function that stood in for it: the method,
  // the header the block writes under a name Rust cannot spell, and the body.
  let requests = [];
  let answer = null;
  globalThis.fetch = (url, init) => {
    requests.push({
      url,
      method: init?.method,
      contentType: init?.headers?.["content-type"],
      body: init?.body,
    });
    // A `Response` as far as the block reads one: a status it checks and a body it reads.
    return Promise.resolve(answer).then((text) => ({ ok: true, status: 200, text: () => text }));
  };

  reset();
  const js = await load("06_procedure");
  const emitted = text("06_procedure");

  // The route and the media type are the wire's, and they are in the compiled text because the
  // client half builds them: a client that agreed with the server by accident would not be.
  check(
    "06: the route prefix is the wire's",
    emitted.includes('"/_topcoat/procedures/"'),
    emitted.match(/"\/_topcoat\/procedures\/"/)?.[0]
  );
  check(
    "06: on the serde media type, not application/json",
    emitted.includes('"application/topcoat+json"') && !emitted.includes('"application/json"')
  );
  // The `js!{}` block, which is what replaced the one host function this client used to need. The
  // header name is in the compiled text because the client builds the init object itself, and it
  // is the thing a `#[repr(C)]` struct could never have spelled.
  check(
    "06: the fetch init is built by the client, header name and all",
    emitted.includes('"content-type"') && emitted.includes("globalThis.fetch"),
    emitted.match(/globalThis\.fetch[^\n]*/)?.[0]
  );
  check(
    "06: and no host function stands in for it",
    !emitted.includes("topcoatFetch")
  );

  const reply = js.make_signal();
  answer = Promise.resolve('"world"');
  js.on_click("world", reply);
  check("06: nothing was requested inline", requests.length === 0, String(requests.length));

  await settleAll();
  check("06: exactly one request went out", requests.length === 1, JSON.stringify(requests));
  check(
    "06: to the procedure's own id under the route",
    /^\/_topcoat\/procedures\/[0-9a-f-]{36}$/.test(requests[0]?.url ?? ""),
    requests[0]?.url
  );
  check(
    "06: with the serde media type",
    requests[0]?.contentType === "application/topcoat+json",
    requests[0]?.contentType
  );
  check("06: as a POST", requests[0]?.method === "POST", requests[0]?.method);
  // One argument is still an ARRAY. A client that sent the bare value would be understood by
  // nothing, and it is the mistake a hand-written one makes.
  check(
    "06: the argument crossed as a JSON array of one",
    requests[0]?.body === '["world"]',
    requests[0]?.body
  );
  check("06: the call succeeded", JSON.stringify(steps) === JSON.stringify(["ok"]), JSON.stringify(steps));
  check("06: and the reply was written into the signal", reply[0]() === "world", String(reply[0]()));

  // The zero-argument spelling is `[]` and not `null`, which PROCEDURES.md calls a deliberate and
  // unshared asymmetry: `()` serializes to `null`, and the server decodes an array.
  requests = [];
  answer = Promise.resolve('"5"');
  const version = js.make_signal();
  js.on_version(version);
  await settleAll();
  check("06: a zero-argument call sends `[]`, not `null`", requests[0]?.body === "[]", requests[0]?.body);
  check("06: and its reply lands too", version[0]() === "5", String(version[0]()));

  // A failed request comes back as an `Err` the caller can render, not as a host rejection.
  reset();
  requests = [];
  answer = Promise.reject("offline");
  const failed = js.make_signal();
  js.on_click("world", failed);
  await settleAll();
  check("06: a rejected request is an Err", JSON.stringify(steps) === JSON.stringify(["err"]), JSON.stringify(steps));
  check("06: carrying the reason", failed[0]() === "offline", String(failed[0]()));
  check("06: and still nothing reached the host unhandled", unhandled.length === 0, JSON.stringify(unhandled.map(String)));
}

// The two shared checks throw at their first wrong step, with the trace in the message. A throw from
// inside a host callback the program calls ends the process instead; the verdicts printed stand.

if (want("07_cancellation")) {
  const { checkOwnerCancellation } = await import("./async-cancellation-check.mjs");
  try {
    const cancellation = await checkOwnerCancellation(
      await load("07_cancellation"),
      await import(pathToFileURL(path.join(from, "topcoat-dom.js")).href),
    );
    check(`07: owner cancellation (${cancellation.checks} assertions)`, true, undefined, cancellation.checks);
  } catch (error) {
    check("07: owner cancellation", false, error?.message ?? String(error));
  }
}

// Subscription exceptions return through typed Rust error paths under the captured owner.
if (want("08_subscription")) {
  const { checkOwnerSubscription } = await import("./async-subscription-check.mjs");
  try {
    const subscription = await checkOwnerSubscription(
      await load("08_subscription"),
      await import(pathToFileURL(path.join(from, "topcoat-dom.js")).href),
    );
    check(`08: owner subscription (${subscription.rows.length} cases)`, true, undefined, subscription.rows.length);
  } catch (error) {
    check("08: owner subscription", false, error?.message ?? String(error));
  }
}

summarize();
for (const name of selected) {
  if (!reported.has(name)) {
    current = name;
    check(`${name}: checked`, false, "scripts/async-check.mjs has no checks for it");
    summarize();
  }
}
console.log(`${failures === 0 ? "PASS" : "FAIL"}: ${failures} failed`);
process.exit(failures === 0 ? 0 : 1);
