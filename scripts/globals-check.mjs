// Runs the compiled `#[js_extern]` globals fixture against a recorder installed as the globals it
// declared, and asserts the operations the emission performed.
//
//   node scripts/globals-check.mjs
//
// The counterpart of scripts/extern-check.mjs for the third root. That one drives declarations
// rooted at a module's imported binding and at an argument, against the contract's fake library;
// this one drives declarations rooted at the GLOBAL SCOPE, which no module can stand in for: the
// whole point of a global is that nothing is imported to reach it.
//
// The recorder is here rather than in contract/fixtures because contract/ is the contract agent's.
// The encoding and the shapes are banked for them in build/logs/wave5-backend-report.md; when a
// vector set lands beside the chart library's, this file becomes its driver.
//
// The fixture is examples/extern-tests/02_globals.rs; build/externtest/02_globals.js is what
// scripts/extern-test.sh leaves behind, so run that first.

import { fileURLToPath, pathToFileURL } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/externtest");
const compiled = path.join(from, "02_globals.js");

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

// ------------------------------------------------------------------- the recorder

/** Every operation the compiled program performed, in order. */
let trace = [];
const record = (op, detail) => {
  trace.push({ op, ...detail });
};
const reset = () => {
  trace = [];
};

/**
 * `EventSource`, as a class rather than a function, so a lowering that emitted a plain call where
 * the descriptor said `new` throws the way the browser's own does. That negative arm is the reason
 * this is a class: `new X(u)` and `X(u)` are otherwise indistinguishable at the trace.
 */
class EventSource {
  static CONNECTING = 0;

  constructor(url) {
    this.url = url;
    this.closed = false;
    record("new EventSource", { url, argc: arguments.length });
  }

  close() {
    this.closed = true;
    record("close", { url: this.url, argc: arguments.length });
  }
}

globalThis.EventSource = EventSource;
globalThis.dashLog = function dashLog(message) {
  record("dashLog", { message, argc: arguments.length });
};
globalThis.dash = {
  // A property with a getter and a setter, so a read and a write are both observable and a `get`
  // lowered as a zero-argument call would find a non-function.
  get status() {
    record("get status", {});
    return this._status;
  },
  set status(value) {
    record("set status", { value });
    this._status = value;
  },
  _status: "idle",
  ticks: [10, 20, 30],
  // `missing` is absent on purpose: reading it answers `undefined`, which is the half of the
  // nullable rule `null` does not cover.
};

const js = await import(pathToFileURL(compiled).href);

// ------------------------------------------------------------------ nothing is imported

// A global is reached through no module at all, so the compiled program must import nothing but
// its own shim. An emission that bound a global with an `import` would still run here, because the
// binding would resolve; only the text says whether anything was imported.
const text = fs.readFileSync(compiled, "utf8");
const imports = [...text.matchAll(/^import .*? from "(.*?)";$/gm)].map((match) => match[1]);
check(
  "a global imports nothing",
  imports.every((specifier) => specifier === "./shim.js"),
  imports.join(", ")
);

// ------------------------------------------------------------------------------- new

reset();
const source = js.v_new_global("/events");
check("new-global: it constructed an EventSource", source instanceof EventSource, typeof source);
check("new-global: the url crossed as itself", source?.url === "/events", source?.url);
check(
  "new-global: one construction was recorded",
  JSON.stringify(trace) === JSON.stringify([{ op: "new EventSource", url: "/events", argc: 1 }]),
  JSON.stringify(trace)
);

// The negative arm, and the one this fixture exists for: a `new` lowered as a plain call throws,
// exactly as it does against the contract's chart library.
reset();
let threw = null;
try {
  EventSource("/events");
} catch (error) {
  threw = error;
}
check("new-global: a plain call would have thrown", threw !== null, String(threw));

// ------------------------------------------------------- a method on what a global returned

reset();
js.v_new_then_close("/ticks");
check(
  "new-then-close: the method landed on the instance the constructor answered",
  JSON.stringify(trace) ===
    JSON.stringify([
      { op: "new EventSource", url: "/ticks", argc: 1 },
      { op: "close", url: "/ticks", argc: 0 },
    ]),
  JSON.stringify(trace)
);

// ------------------------------------------------------------------------------ call

reset();
js.v_call_global("hello");
check(
  "call-global: a bare global function was called with its argument",
  JSON.stringify(trace) === JSON.stringify([{ op: "dashLog", message: "hello", argc: 1 }]),
  JSON.stringify(trace)
);

check("call-global-path: Math.max(3, 7) is 7", js.v_call_global_path(3, 7) === 7);
// A path whose last step lost its receiver would be `max(3, 7)`, which is not a global.
check(
  "call-global-path: the path kept its receiver",
  /Math\.max\(/.test(text),
  text.match(/return .*max.*/)?.[0]
);

// ------------------------------------------------------------------------- get and set

reset();
globalThis.dash._status = "streaming";
check("get-global: the property was read", js.v_get_global() === "streaming");
check(
  "get-global: through the getter, not as a call",
  JSON.stringify(trace) === JSON.stringify([{ op: "get status" }]),
  JSON.stringify(trace)
);

reset();
js.v_set_global("stopped");
check("set-global: the property was written", globalThis.dash._status === "stopped");
check(
  "set-global: through the setter",
  JSON.stringify(trace) === JSON.stringify([{ op: "set status", value: "stopped" }]),
  JSON.stringify(trace)
);

// ---------------------------------------------------------------------------- index

check("index-global: in range reads the element", js.v_index_global(1) === 20);
check("index-global: the index crossed as a number", js.v_index_global(2) === 30);
check("index-global: out of range is undefined", js.v_index_global(9) === undefined);

// -------------------------------------------------------------------------- nullable

// The whole point of `== null`: a missing property is `undefined`, not `null`, and a wrapper that
// tested only `null` would answer `Some(undefined)` here.
check("nullable-global: a missing property is None", js.v_nullable_global() === true);

console.log(`${failures === 0 ? "PASS" : "FAIL"}: ${failures} failed`);
process.exit(failures === 0 ? 0 : 1);
