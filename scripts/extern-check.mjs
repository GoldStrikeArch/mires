// Runs the compiled `#[js_extern]` fixture against the contract's fake chart library and compares
// its call trace with the contract's vectors.
//
//   node scripts/extern-check.mjs
//
// `contract/fixtures/js-extern/chart-lib.mjs` records the JavaScript-level operations performed on
// it; `vectors.json` holds, per call shape, the trace the reference emission left behind, produced
// by executing that emission rather than by transcribing it. `contract/harness/check-js-extern.mjs`
// exports `vector(id)` and `compareTrace(actual, expected)` for exactly this, and importing it
// re-runs the contract's own assertions about its drivers on the way past, which is worth having:
// a vector that stopped exercising its case fails here too.
//
// The emitted JavaScript does not have to match the reference emission's text. It has to be
// indistinguishable from it at the trace, which is what a descriptor is ABOUT: `new Chart(el, cfg)`
// and a `Chart(el, cfg)` factory both produce a chart, and only a recorder tells them apart.
//
// The fixture is examples/extern-tests/01_chart.rs; build/externtest/01_chart.js is what
// scripts/extern-test.sh leaves behind, so run that first.

import { fileURLToPath, pathToFileURL } from "node:url";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const from = path.join(root, "build/externtest");
const harness = path.join(root, "contract/harness");
const fixtures = path.join(root, "contract/fixtures/js-extern");

// The compiled program imports the library by the specifier its declarations named. The run
// directory satisfies that specifier with a re-export of the contract's file, which is the same
// trick scripts/dom-test.sh uses for the DOM runtime: one module instance, so `__trace()` read
// from here sees what the compiled program did.
//
// `default` is re-exported by name because `export *` does not carry it, and the library exports
// its constructor twice as two DISTINCT function objects on purpose: a descriptor has to say which
// binding it imports, and identical objects would make that choice unobservable.
fs.writeFileSync(
  path.join(from, "chart-lib.mjs"),
  `export * from "../../contract/fixtures/js-extern/chart-lib.mjs";
export { default } from "../../contract/fixtures/js-extern/chart-lib.mjs";
`
);

const lib = await import(pathToFileURL(path.join(from, "chart-lib.mjs")).href);
const { env } = await import(pathToFileURL(path.join(fixtures, "drivers.mjs")).href);
const { vector, compareTrace } = await import(
  pathToFileURL(path.join(harness, "check-js-extern.mjs")).href
);
const js = await import(pathToFileURL(path.join(from, "01_chart.js")).href);

let failures = 0;
const check = (label, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"}  ${label}${detail === undefined ? "" : `: ${detail}`}`);
  if (!ok) {
    failures += 1;
  }
};

/** Build an instance the way the vectors' `setup` does, without recording it. */
const setup = () => {
  lib.__reset();
  const chart = js.v_new_named(env.canvas, env.config);
  lib.__reset();
  return chart;
};

/**
 * Drive one vector and compare.
 *
 * `run` returns what the compiled entry point answered. `returned` says how to check it: omit it
 * where the entry hands back the library's own value and the vector's `returned` is the claim, or
 * give a predicate where the Rust entry answers something of its own.
 */
const run = (id, drive, returned) => {
  const expected = vector(id);
  let answer;
  let threw = null;
  try {
    answer = drive();
  } catch (error) {
    threw = error;
  }

  const trace = lib.__trace().map((record) => JSON.parse(JSON.stringify(record)));
  const diffs = compareTrace(trace, expected.trace);
  check(`${id}: the trace is the vector's`, diffs.length === 0, diffs.join("\n      ") || undefined);

  if (expected.threw !== null && expected.threw !== undefined) {
    check(`${id}: it throws, as the vector says`, threw !== null, threw === null ? "it did not" : undefined);
    return;
  }
  check(`${id}: it does not throw`, threw === null, threw === null ? undefined : String(threw));
  if (threw !== null) {
    return;
  }

  if (typeof returned === "function") {
    check(`${id}: the value reached Rust`, returned(answer), JSON.stringify(lib.__value(answer)));
    return;
  }
  const normalized = JSON.stringify(lib.__value(answer));
  check(
    `${id}: the value handed back is the vector's`,
    normalized === JSON.stringify(expected.returned),
    normalized
  );
};

// -------------------------------------------------------------------------------- new
run("new-named", () => {
  lib.__reset();
  return js.v_new_named(env.canvas, env.config);
});
run("new-default", () => {
  lib.__reset();
  return js.v_new_default(env.canvas, env.config);
});
// The negative arm: a `new` shape that lowered to a plain call would be caught here, because the
// library refuses to be constructed without `new`.
run("new-without-new", () => {
  lib.__reset();
  return js.v_new_without_new(env.canvas, env.config);
});

// ------------------------------------------------------------------------------- send
run("send-one-arg", () => {
  const chart = setup();
  js.v_send_one_arg(chart, env.nextData);
  return undefined;
});
run("send-zero-args", () => {
  const chart = setup();
  js.v_send_zero_args(chart);
  return undefined;
});
// The two `resize` declarations differ only in arity, and `argc` in the vector is what tells them
// apart: a lowering that padded the shorter call with an explicit `undefined` fails here.
run("send-trailing-omitted", () => {
  const chart = setup();
  js.v_send_trailing_omitted(chart);
  return undefined;
});
run("send-trailing-undefined", () => {
  const chart = setup();
  js.v_send_trailing_undefined(chart, undefined);
  return undefined;
});

// --------------------------------------------------------------------------- nullable
// `getPoint(0)` answers an object and `getPoint(99)` answers `null`. The Rust entries read through
// the `Option` rather than handing it back, so what is checked is that the wrapper produced the
// right variant and that a `Some` carries the value and not just the tag.
run(
  "nullable-some",
  () => {
    const chart = setup();
    return js.v_nullable_some(chart);
  },
  (answer) => answer === env.config.data.points[0].x
);
run(
  "nullable-none",
  () => {
    const chart = setup();
    return js.v_nullable_none(chart);
  },
  (answer) => answer === true
);

// -------------------------------------------------------------------------------- get
run("get-property", () => {
  const chart = setup();
  return js.v_get_property(chart);
});
// A walk: two property reads, and the vector holds both.
run("get-scoped", () => {
  const chart = setup();
  return js.v_get_scoped(chart);
});
run("get-static", () => {
  lib.__reset();
  return js.v_get_static();
});

// -------------------------------------------------------------------------------- set
run("set-property", () => {
  const chart = setup();
  js.v_set_property(chart, "Revenue, restated");
  return undefined;
});

// ------------------------------------------------------------------------------ index
run("index-in-range", () => {
  const chart = setup();
  return js.v_index_in_range(chart);
});
run("index-length", () => {
  const chart = setup();
  return js.v_index_length(chart);
});
// Out of range answers `undefined`, which is NOT `null`: the two sit side by side in the fixture so
// a descriptor cannot conflate them, and this vector is the half that is not nullable.
run("index-out-of-range", () => {
  const chart = setup();
  return js.v_index_out_of_range(chart);
});

// ------------------------------------------------------------------------------- call
run("call-static", () => {
  lib.__reset();
  js.v_call_static(env.plugin);
  return undefined;
});

console.log(failures === 0 ? "\nall checks passed" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
