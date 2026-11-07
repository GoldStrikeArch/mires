// Unit tests for harness/compare-trace.mjs.
//
//   node --import ./register-loader.mjs test-compare-trace.mjs
//
// Exits non-zero on the first failure, so run-all.mjs treats it like any other
// step. Plain assertions rather than a test runner: the comparator is the one
// piece of this harness with no upstream to check it against, so its own test
// should not depend on anything either.

import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { readTrace, loadDeltas, compareTraces, normalize } from "./compare-trace.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const data = (name) => path.join(HERE, "testdata", name);

let passed = 0;
const check = (name, fn) => {
  try {
    fn();
    passed++;
    console.log(`  ok    ${name}`);
  } catch (err) {
    console.log(`  FAIL  ${name}`);
    console.log(`        ${err.message.split("\n").join("\n        ")}`);
    process.exitCode = 1;
  }
};

const base = readTrace(data("base.trace"));
const equal = readTrace(data("equal-renumbered.trace"));
const differing = readTrace(data("differing.trace"));
const accepted = readTrace(data("accepted.trace"));

// --------------------------------------------------------------- identity
check("a trace equals itself", () => {
  const r = compareTraces(base, base);
  assert.equal(r.equal, true);
  assert.equal(r.differences.length, 0);
});

// ----------------------------------------------------------- renumbering
check("renumbered ids compare equal", () => {
  const r = compareTraces(base, equal);
  assert.equal(r.equal, true, `expected equal, got ${JSON.stringify(r.differences, null, 2)}`);
  assert.equal(r.differences.length, 0);
});

check("normalisation is what makes them equal, not luck", () => {
  // The raw bytes really do differ; only the canonical form agrees.
  assert.notEqual(
    JSON.stringify(base.records),
    JSON.stringify(equal.records),
    "test fixture is wrong: the two traces are textually identical"
  );
  assert.deepEqual(normalize(base.records), normalize(equal.records));
});

check("normalisation preserves aliasing", () => {
  // #0 appears three times in base.trace as the SAME node. If the renamer lost
  // that, two independent nodes would compare equal to one shared node.
  const n = normalize([
    { op: "x", a: "#0", b: "#0" },
    { op: "y", a: "#1" }
  ]);
  assert.equal(n[0].a, n[0].b, "same id must map to same alias");
  assert.notEqual(n[0].a, n[1].a, "different ids must map to different aliases");
});

check("normalisation keeps the structural path on a labelled node", () => {
  const n = normalize([{ op: "x", node: "tmpl#0:root.firstChild.nextSibling" }]);
  assert.match(n[0].node, /^\$tmpl0:root\.firstChild\.nextSibling$/);
});

// -------------------------------------------------------------- differing
check("real differences are reported", () => {
  const r = compareTraces(base, differing);
  assert.equal(r.equal, false);
  const kinds = r.differences.map((d) => `${d.kind}:${d.op}`);
  assert.ok(kinds.includes("changed:template"), `template change missing from ${kinds}`);
  assert.ok(kinds.includes("extra-in-b:setAttribute"), `setAttribute missing from ${kinds}`);
  assert.ok(
    kinds.includes("changed:addEventListener"),
    `delegate flag change missing from ${kinds}`
  );
});

check("a changed record names the fields that changed", () => {
  const r = compareTraces(base, differing);
  const ev = r.differences.find((d) => d.op === "addEventListener");
  assert.deepEqual(
    ev.fields.map((f) => f.field),
    ["delegate"]
  );
  assert.equal(ev.fields[0].a, true);
  assert.equal(ev.fields[0].b, false);
});

check("alignment does not cascade after an inserted record", () => {
  // differing.trace inserts setAttribute in the middle. Everything after it is
  // unchanged, so index alignment would report 3 spurious diffs; LCS reports 0.
  const r = compareTraces(base, differing);
  assert.ok(
    r.differences.length <= 3,
    `expected at most 3 differences, got ${r.differences.length}: ` +
      JSON.stringify(r.differences.map((d) => `${d.kind}:${d.op}`))
  );
});

// -------------------------------------------------- differing-but-accepted
check("without a delta file, the escaping difference fails", () => {
  const r = compareTraces(base, accepted);
  assert.equal(r.equal, false);
  assert.equal(r.differences.length, 1);
  assert.equal(r.differences[0].op, "template");
});

check("with the delta file, the escaping difference is accepted", () => {
  const deltas = loadDeltas(data("deltas.json"), "testdata/base");
  const r = compareTraces(base, accepted, deltas);
  assert.equal(r.equal, true, `still failing: ${JSON.stringify(r.differences, null, 2)}`);
  assert.equal(r.differences.length, 0);
});

check("the delta does not hide a genuine difference", () => {
  // Same rules, applied to the trace that differs for real. The escaping rule
  // must not make the template-string change disappear.
  const deltas = loadDeltas(data("deltas.json"), "testdata/base");
  const r = compareTraces(base, differing, deltas);
  assert.equal(r.equal, false);
  assert.ok(r.differences.some((d) => d.op === "template"));
});

check("a rule matching nothing is reported as stale", () => {
  const deltas = loadDeltas(data("deltas.json"), "testdata/base");
  const r = compareTraces(base, accepted, deltas);
  const stale = r.unusedRules.map((u) => u.rule.id);
  assert.deepEqual(stale, ["stale-rule-on-purpose"]);
});

// ------------------------------------------------------- delta file shapes
check("front matter in a .md file is read", () => {
  const deltas = loadDeltas(data("NOTES-with-front-matter.md"), "testdata/base");
  assert.equal(deltas.rules.length, 1);
  assert.equal(deltas.rules[0].id, "text-gt-escaping");
  const r = compareTraces(base, accepted, deltas);
  assert.equal(r.equal, true);
});

check("a rule without a `why` is refused", () => {
  assert.throws(
    () => loadDeltas(data("bad-delta-no-why.json"), "testdata/base"),
    /has no "why"/
  );
});

// ------------------------------------------------------------- header facts
check("a status mismatch is surfaced even when records agree", () => {
  const stopped = {
    ...base,
    header: { ...base.header, status: "stopped-early", error: "boom" }
  };
  const r = compareTraces(base, stopped);
  assert.equal(r.equal, true, "records still agree");
  assert.deepEqual(
    r.headerNotes.map((n) => n.field),
    ["status", "error"]
  );
});

console.log(`\n${passed} assertion group(s) passed${process.exitCode ? ", WITH FAILURES" : ""}`);
