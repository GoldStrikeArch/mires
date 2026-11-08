// Execute a reference-compiled module against the recording stub and print the
// normalized trace as JSON lines.
//
//   node --import ./register-loader.mjs run-trace.mjs --fixture <family>/<name>
//   node --import ./register-loader.mjs run-trace.mjs --file compiled.js
//   node --import ./register-loader.mjs run-trace.mjs --record            (writes fixtures/reference-traces/)
//
// HOW THE MODULE MAPPING WORKS  (documented per the milestone's "your choice")
// ---------------------------------------------------------------------------
// Compiled output imports from the plugin's configured `moduleName` -- "r-dom"
// for the DOM presets. We do NOT rewrite the compiled source. Instead we write
// it to a temp file and let harness/loader.mjs's bare-specifier fallback catch
// "r-dom", which we pre-register to resolve to trace.mjs.
//
// An import REWRITE was the alternative. It was rejected because rewriting the
// text under test means the thing traced is no longer byte-identical to the
// thing verify-reference.mjs proved faithful -- the proof and the trace would
// cover different artifacts. Module mapping keeps the compiled bytes untouched.
//
// The compiled fixtures reference free variables (component names, props, etc.)
// that were never defined in the fixture source. Those are expected to throw;
// the stub records the throw and continues, and the trace up to that point is
// still the contract-relevant part. Traces record a `status` field saying
// whether the module completed or stopped early.

import { compileUpstreamFixture } from "./compile-reference.mjs";
import { FIXTURES, provenance } from "./paths.mjs";
import { writeFileSync, mkdirSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { tmpdir } from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";

const argv = process.argv.slice(2);
const flag = (name) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : null;
};

/**
 * A stand-in for a free identifier the fixture source never declared (component
 * names, props, loop variables). Callable as a component, indexable as an
 * object, and coercible to a string, so it survives whatever the compiled code
 * does with it.
 *
 * `iterableResult` is an ESCALATION, applied only after a first, plain heal has
 * demonstrably failed on a destructuring call -- `const [a, b] = createSignal(0)`,
 * the idiomatic Solid spelling, which appears in the corpus. It is withheld by
 * default rather than always on, because turning `apply` from "returns
 * undefined" into "returns something" would change what every already-recorded
 * trace saw at every call of a healed binding.
 */
function makeFreeBinding(name, iterableResult = false) {
  const fn = function () {
    return undefined;
  };
  Object.defineProperty(fn, "name", { value: name });
  return new Proxy(fn, {
    get(target, key) {
      if (key === Symbol.toPrimitive) return () => `<free:${name}>`;
      if (key === "toString") return () => `<free:${name}>`;
      // Only present on an ESCALATED binding -- see makeIterableFreeBinding.
      // Withheld by default so a binding that is never destructured behaves
      // exactly as it did before this was added.
      if (key === Symbol.iterator && iterableResult) {
        return function* () {
          for (let i = 0; ; i++) yield makeFreeBinding(`${name}[${i}]`);
        };
      }
      if (key in target) return Reflect.get(target, key);
      return makeFreeBinding(`${name}.${String(key)}`);
    },
    // A plain healed binding returns undefined when called. An escalated one
    // returns a destructurable binding instead.
    apply: () => (iterableResult ? makeFreeBinding(`${name}()`, true) : undefined)
  });
}

async function traceModule(code, label) {
  const trace = await import("./trace.mjs");

  // Upstream fixture sources reference identifiers they never define -- they
  // were only ever meant to be COMPILED, not executed. Rather than give up at
  // the first ReferenceError, define the missing name as an inert binding and
  // retry, so the trace reaches the end of the module. Every name healed this
  // way is recorded in the trace header, since it is an assumption a reader
  // should be able to audit.
  const healed = [];
  const MAX_RETRIES = 100;

  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    trace.__reset();

    // A fresh filename each attempt: node caches modules by URL, so a retry
    // would otherwise re-serve the failed instance without re-executing it.
    const file = path.join(tmpdir(), `topcoat-trace-${randomUUID()}.mjs`);
    writeFileSync(file, code);

    try {
      await import(pathToFileURL(file).href);
      return { label, status: "completed", error: null, healed, records: trace.__trace() };
    } catch (err) {
      const message = String((err && err.message) || err);
      const missing = /^(\w[\w$]*) is not defined$/.exec(message);
      if (missing && attempt < MAX_RETRIES) {
        const name = missing[1];
        if (!healed.includes(name)) {
          globalThis[name] = makeFreeBinding(name);
          healed.push(name);
          continue;
        }
      }

      // Second heal rule: an already-healed binding that was CALLED and whose
      // result was then destructured. Re-heal it as an iterable-returning
      // binding and retry. Recorded in the header as `name (iterable)` so the
      // escalation is visible rather than silent.
      const notIterable =
        /^(\w[\w$]*) is not a function or its return value is not iterable$/.exec(message);
      if (notIterable && attempt < MAX_RETRIES) {
        const name = notIterable[1];
        const mark = `${name} (iterable)`;
        if (!healed.includes(mark)) {
          globalThis[name] = makeFreeBinding(name, true);
          healed.push(mark);
          continue;
        }
      }
      return {
        label,
        status: "stopped-early",
        error: message,
        healed,
        records: trace.__trace()
      };
    }
  }
  return { label, status: "stopped-early", error: "retry limit exceeded", healed, records: [] };
}

/** Serialize a trace as JSON Lines: a header, then one line per record. */
function toJsonl(result, extra = {}) {
  const lines = [
    JSON.stringify({
      $record: "header",
      ...extra,
      label: result.label,
      freeBindingsHealed: result.healed,
      status: result.status,
      error: result.error,
      recordCount: result.records.length
    })
  ];
  for (const r of result.records) lines.push(JSON.stringify(r));
  return `${lines.join("\n")}\n`;
}

// The families recorded as committed reference traces. DOM (non-hydratable and
// hydratable) only: the SSR presets emit string concatenation, not runtime
// calls, so a call trace says nothing about them.
const RECORD_TARGETS = [
  ["__dom_fixtures__", "simpleElements"],
  ["__dom_fixtures__", "textInterpolation"],
  ["__dom_fixtures__", "attributeExpressions"],
  ["__dom_hydratable_fixtures__", "simpleElements"],
  ["__dom_hydratable_fixtures__", "textInterpolation"],
  ["__dom_hydratable_fixtures__", "insertChildren"],
  ["__dom_hydratable_fixtures__", "eventExpressions"]
];

if (argv.includes("--record")) {
  const dir = path.join(FIXTURES, "reference-traces");
  mkdirSync(dir, { recursive: true });
  const p = provenance("run-trace.mjs");

  const summary = [];
  for (const [family, name] of RECORD_TARGETS) {
    const { actual } = compileUpstreamFixture(family, name);
    const result = await traceModule(actual, `${family}/${name}`);
    const file = path.join(dir, `${family.replace(/^__|__$/g, "")}.${name}.trace`);
    writeFileSync(
      file,
      toJsonl(result, {
        ...p,
        $what:
          "Normalized runtime-call trace of a reference-compiled fixture executed " +
          "against harness/trace.mjs. Node identities are renamed in first-appearance " +
          "order; functions become fn#N; templates become tmpl#N.",
        fixture: `${family}/${name}`
      })
    );
    summary.push({ fixture: `${family}/${name}`, status: result.status, records: result.records.length });
    console.log(
      `  ${path.basename(file)}  ${result.records.length} records  (${result.status})`
    );
  }

  console.log(`\nwrote ${summary.length} traces to fixtures/reference-traces/`);
  const completed = summary.filter((s) => s.status === "completed").length;
  console.log(`  ${completed}/${summary.length} ran to completion`);
} else {
  const fixture = flag("fixture");
  const file = flag("file");
  if (!fixture && !file) {
    console.error(
      "usage: run-trace.mjs (--fixture <family>/<name> | --file <compiled.js> | --record)"
    );
    process.exit(2);
  }

  let code;
  let label;
  if (fixture) {
    const [family, name] = fixture.split("/");
    code = compileUpstreamFixture(family, name).actual;
    label = fixture;
  } else {
    code = readFileSync(path.resolve(file), "utf8");
    label = path.basename(file);
  }

  const result = await traceModule(code, label);
  process.stdout.write(toJsonl(result));
}
