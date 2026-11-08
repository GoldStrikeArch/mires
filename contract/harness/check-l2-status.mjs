// The L2 gate: per-corpus-family trace parity, committed and enforced.
//
//   node --import ./register-loader.mjs check-l2-status.mjs            # check (default)
//   node --import ./register-loader.mjs check-l2-status.mjs --check
//   node --import ./register-loader.mjs check-l2-status.mjs --record   # rewrite the status
//   node --import ./register-loader.mjs check-l2-status.mjs --json
//
// WHY THIS EXISTS
// ---------------
// L2 is the comparison between a corpus family's reference trace and the trace of
// the same family compiled by this backend. `scripts/dom-test.sh` runs it and
// PRINTS it, deliberately: a difference between two emitters is a finding, not
// automatically a regression in this one (dom-test.sh's own header says so, and
// its exit code counts only L1 failures). The cost of that choice is that the L2
// verdict lives in `build/logs/`, is regenerated per run, and can change in either
// direction without anything noticing. The one family that MATCHES today
// (01-simple-elements) is therefore undefended: it could become DIFFERS and every
// suite would still pass.
//
// This script is the defence. `contract/fixtures/l2-status.json` is the committed
// verdict, one row per family, and this script recomputes it and fails on ANY
// difference -- in either direction. A regression fails because it is a
// regression. An IMPROVEMENT fails too, with a message saying to commit the better
// status, which is the `NN.maxbytes` precedent from `scripts/emit-test.sh`: a
// budget you beat is a budget you re-baseline, not one you leave loose.
//
// WHAT IT COMPARES, AND WHY IT NEEDS NO BUILD
// -------------------------------------------
// Not the freshly compiled trace -- the COMMITTED one,
// `examples/dom-tests/NN.trace.expected`. That is sound, and it is the load-bearing
// design decision here:
//
//   dom-test.sh already diffs the freshly produced trace against NN.trace.expected
//   and FAILS the suite on any difference (its step 2, "trace changed"). So on a
//   green dom run the committed trace and the emitted trace are byte identical.
//   Comparing the committed one against the reference therefore yields exactly the
//   L2 verdict dom-test.sh would print -- and yields it from committed bytes alone,
//   with no rustc, no cargo, no backend build, in about a second.
//
// The consequence worth stating plainly: this gate cannot catch an emitter change
// on its own, because an emitter change that is not also committed to
// NN.trace.expected is caught by dom-test.sh first. It catches the thing
// dom-test.sh cannot -- a re-baselined trace that silently moved a family's L2
// verdict, and a reference trace or delta rule that moved underneath one.
//
// THE THREE NON-MEASURED STATES
// -----------------------------
// A family with no `.family` file is never compared, so it needs a declared state
// and the declaration has to be checkable or it is just prose:
//
//   EXCLUDED  deliberately not measured. Requires `why`. The script asserts the
//             family really is unmeasured -- if a `.family` file starts naming it,
//             the exclusion is stale and this fails.
//   GAP       neither measured nor excluded. Requires `why`. Same assertion. This
//             is the state that is a gate item, and it fails LOUDLY the moment a
//             fixture appears for it, because that is the moment its real status
//             becomes recordable.
//
// and every DIFFERS row requires `attribution`: one or more named causes, each of
// which must exist as a key in `examples/dom-tests/deltas.json`'s
// `$whatIsDeliberatelyNotHere`. That is what makes "ATTRIBUTED" mean something
// mechanical rather than being a word in a table.

import { readFileSync, existsSync, writeFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { CONTRACT, FIXTURES } from "./paths.mjs";
import { readTrace, loadDeltas, compareTraces } from "./compare-trace.mjs";

const ROOT = path.resolve(CONTRACT, "..");
const CORPUS = path.join(FIXTURES, "corpus");
const DOM_TESTS = path.join(ROOT, "examples", "dom-tests");
const DELTAS = path.join(DOM_TESTS, "deltas.json");
const STATUS = path.join(FIXTURES, "l2-status.json");

/** Fields a row declares by hand. --record preserves them; --check requires them. */
const DECLARED = ["attribution", "why", "note"];
/** Fields --record computes. Everything a check compares lives here. */
const MEASURED = ["state", "fixture", "differences", "accepted", "staleRules", "causes", "records"];

/** The prose block a fresh --record seeds the file with. Once committed, the file owns it. */
const SEED = {
  $what:
    "The committed L2 verdict: one row per corpus family, recording how the trace of " +
    "the family compiled by rustc_codegen_js compares against the reference trace in " +
    "contract/fixtures/corpus/<family>/expected.reference.trace, with the accepted " +
    "deltas in examples/dom-tests/deltas.json applied. Recomputed and enforced by " +
    "contract/harness/check-l2-status.mjs.",
  $howToRegenerate: [
    "cd contract/harness",
    "node --import ./register-loader.mjs check-l2-status.mjs           # check, exit 1 on any change",
    "node --import ./register-loader.mjs check-l2-status.mjs --record  # rewrite after reading the diff",
    "It reads only committed bytes -- the reference traces here and examples/dom-tests/NN.trace.expected -- so it needs no build. That is sound because scripts/dom-test.sh already fails if a freshly emitted trace differs from NN.trace.expected, so on a green dom run the committed trace IS the emitted trace."
  ],
  $states: {
    MATCH: "compared, and the traces agree once the accepted deltas are applied.",
    DIFFERS:
      "compared, and they do not. `differences` is the count and `attribution` names the written cause(s). This is what the G2 checklist calls ATTRIBUTED.",
    EXCLUDED: "deliberately not compared. `why` says so. No examples/dom-tests/NN.family names it.",
    GAP: "neither compared nor excluded. `why` records the blocker. The only state that is a gate item."
  },
  $fields: {
    measured:
      "state, fixture, differences, accepted, staleRules, causes, records -- rewritten by --record and compared by a check. `causes` is the mechanically derived signature of the disagreement: the distinct kind:op pairs, sorted.",
    declared:
      "attribution, why, note -- written by hand, carried forward by --record untouched, and REQUIRED: a DIFFERS row with no attribution fails, and so does one citing a cause that is not a key of $whatIsDeliberatelyNotHere in examples/dom-tests/deltas.json."
  }
};

const rel = (p) => path.relative(ROOT, p);

// ------------------------------------------------------------------ inputs

/** Every corpus family, in directory order, which is the corpus's own order. */
function families() {
  return readdirSync(CORPUS, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
}

/**
 * The dom fixture that names each family, from the `NN.family` files. This is the
 * same registration dom-test.sh uses: a fixture is compared iff it declares a
 * family, so reading the declarations is reading the comparison set.
 */
function fixturesByFamily() {
  const out = new Map();
  if (!existsSync(DOM_TESTS)) return out;
  for (const name of readdirSync(DOM_TESTS).sort()) {
    if (!name.endsWith(".family")) continue;
    const stem = name.slice(0, -".family".length);
    const family = readFileSync(path.join(DOM_TESTS, name), "utf8").trim();
    const prior = out.get(family);
    if (prior) throw new Error(`two fixtures claim family ${family}: ${prior.stem}, ${stem}`);
    out.set(family, { stem, trace: path.join(DOM_TESTS, `${stem}.trace.expected`) });
  }
  return out;
}

/**
 * The named causes a DIFFERS row may cite: the keys of deltas.json's
 * `$whatIsDeliberatelyNotHere`, which is the file that already holds one written
 * paragraph per cause. Citing anything else is a typo or an unwritten cause, and
 * both should fail.
 */
function knownCauses() {
  const deltas = JSON.parse(readFileSync(DELTAS, "utf8"));
  return new Set(Object.keys(deltas.$whatIsDeliberatelyNotHere ?? {}));
}

// ------------------------------------------------------------------ measure

/**
 * The signature of a set of differences: the distinct `kind:op` pairs, sorted.
 * Derived mechanically so it can be recorded, and coarse enough to be stable --
 * it changes when the SHAPE of the disagreement changes, which is when a human
 * should look, and not when a record index moves.
 */
function causeSignature(differences) {
  return [...new Set(differences.map((d) => `${d.kind}:${d.op}`))].sort();
}

/** Compare one family, or return null if nothing names it. */
function measure(family, fixture) {
  if (!fixture) return null;
  const reference = path.join(CORPUS, family, "expected.reference.trace");
  if (!existsSync(reference)) {
    return { state: "NO-REFERENCE", fixture: rel(fixture.trace), reference: rel(reference) };
  }
  if (!existsSync(fixture.trace)) {
    return { state: "NO-TRACE", fixture: rel(fixture.trace) };
  }

  const a = readTrace(reference);
  const b = readTrace(fixture.trace);
  const result = compareTraces(a, b, loadDeltas(DELTAS, `corpus/${family}`));

  return {
    state: result.equal ? "MATCH" : "DIFFERS",
    fixture: rel(fixture.trace),
    differences: result.differences.length,
    accepted: result.accepted.length,
    staleRules: result.unusedRules.map((u) => u.rule.id ?? u.rule.kind).sort(),
    causes: causeSignature(result.differences),
    records: { reference: a.records.length, ours: b.records.length }
  };
}

// ------------------------------------------------------------------- build

function build(committed) {
  const byFamily = fixturesByFamily();
  const rows = {};

  for (const family of families()) {
    const prior = committed?.families?.[family] ?? {};
    const measured = measure(family, byFamily.get(family));

    if (measured) {
      rows[family] = { ...measured };
    } else {
      // Not measured. The committed state is the declaration, and a declaration
      // this script cannot recompute is one it must carry forward verbatim --
      // except for `fixture`, which is now provably absent.
      const state = prior.state === "EXCLUDED" ? "EXCLUDED" : "GAP";
      rows[family] = { state, fixture: null };
    }
    for (const key of DECLARED) {
      if (prior[key] !== undefined) rows[family][key] = prior[key];
    }
  }
  return rows;
}

// ------------------------------------------------------------------- check

/** Rows that changed, described in the direction they moved. */
function diffRows(committed, fresh) {
  const problems = [];
  const names = [...new Set([...Object.keys(committed), ...Object.keys(fresh)])].sort();

  for (const family of names) {
    const was = committed[family];
    const now = fresh[family];
    if (!was) {
      problems.push({
        family,
        direction: "NEW",
        text: `${family} is a corpus family with no row in l2-status.json`
      });
      continue;
    }
    if (!now) {
      problems.push({
        family,
        direction: "REMOVED",
        text: `${family} has a row but is no longer a corpus family`
      });
      continue;
    }

    // A declared row (EXCLUDED/GAP) that gained a fixture, or a measured row that
    // lost one, is the sharpest signal this file carries: the family's status just
    // became recordable, or stopped being.
    if (was.state !== now.state) {
      const better =
        (was.state === "DIFFERS" && now.state === "MATCH") ||
        (["GAP", "EXCLUDED"].includes(was.state) && ["MATCH", "DIFFERS"].includes(now.state));
      problems.push({
        family,
        direction: better ? "IMPROVED" : "REGRESSED",
        text: `${family}: committed ${was.state}, measured ${now.state}`
      });
      continue;
    }

    for (const key of MEASURED) {
      if (key === "state" || key === "fixture") continue;
      const a = JSON.stringify(was[key] ?? null);
      const b = JSON.stringify(now[key] ?? null);
      if (a === b) continue;
      const better = key === "differences" && (now[key] ?? 0) < (was[key] ?? 0);
      problems.push({
        family,
        direction: better ? "IMPROVED" : "REGRESSED",
        text: `${family}.${key}: committed ${a}, measured ${b}`
      });
    }
  }
  return problems;
}

/** The declaration rules: a state that is not measured has to justify itself. */
function checkDeclarations(rows, causes) {
  const problems = [];
  for (const [family, row] of Object.entries(rows)) {
    if (row.state === "DIFFERS") {
      const cited = row.attribution ?? [];
      if (!cited.length) {
        problems.push({
          family,
          direction: "UNATTRIBUTED",
          text: `${family} DIFFERS with no \`attribution\`: name the cause(s) from examples/dom-tests/deltas.json $whatIsDeliberatelyNotHere`
        });
      }
      for (const cause of cited) {
        if (!causes.has(cause)) {
          problems.push({
            family,
            direction: "UNKNOWN-CAUSE",
            text: `${family} cites "${cause}", which is not a key of $whatIsDeliberatelyNotHere in examples/dom-tests/deltas.json`
          });
        }
      }
    }
    if (["EXCLUDED", "GAP"].includes(row.state) && !row.why) {
      problems.push({
        family,
        direction: "UNJUSTIFIED",
        text: `${family} is ${row.state} with no \`why\``
      });
    }
    if (row.state === "MATCH" && (row.staleRules ?? []).length) {
      problems.push({
        family,
        direction: "STALE-RULE",
        text: `${family} MATCHes but loads rule(s) that matched nothing: ${row.staleRules.join(", ")}`
      });
    }
  }
  return problems;
}

function tally(rows) {
  const counts = {};
  for (const row of Object.values(rows)) counts[row.state] = (counts[row.state] ?? 0) + 1;
  return counts;
}

// --------------------------------------------------------------------- CLI

const argv = process.argv.slice(2);
const record = argv.includes("--record");
const asJson = argv.includes("--json");

const committed = existsSync(STATUS) ? JSON.parse(readFileSync(STATUS, "utf8")) : null;
if (!committed && !record) {
  console.error(`check-l2-status: ${rel(STATUS)} does not exist. Run with --record to create it.`);
  process.exit(1);
}

const fresh = build(committed);
const counts = tally(fresh);

if (record) {
  const out = {
    ...(committed ?? {}),
    families: fresh,
    $summary: counts
  };
  // Preserve the prose block if one is already committed; seed it if not.
  if (!out.$what) out.$what = SEED.$what;
  if (!out.$howToRegenerate) out.$howToRegenerate = SEED.$howToRegenerate;
  if (!out.$states) out.$states = SEED.$states;
  if (!out.$fields) out.$fields = SEED.$fields;
  // Key order: prose first, then the summary, then the rows.
  const ordered = {
    $what: out.$what,
    $howToRegenerate: out.$howToRegenerate,
    $states: out.$states,
    $fields: out.$fields,
    $summary: out.$summary,
    families: out.families
  };
  writeFileSync(STATUS, `${JSON.stringify(ordered, null, 2)}\n`);
  console.log(`recorded ${rel(STATUS)}`);
  for (const [state, n] of Object.entries(counts).sort()) console.log(`  ${state} ${n}`);
  process.exit(0);
}

const problems = [
  ...diffRows(committed.families ?? {}, fresh),
  ...checkDeclarations(fresh, knownCauses())
];

if (asJson) {
  process.stdout.write(`${JSON.stringify({ counts, problems, families: fresh }, null, 2)}\n`);
}

if (!problems.length) {
  if (!asJson) {
    const line = Object.entries(counts)
      .sort()
      .map(([s, n]) => `${n} ${s}`)
      .join(", ");
    console.log(`L2 status matches ${rel(STATUS)}: ${line}`);
  }
  process.exit(0);
}

if (!asJson) {
  console.log(`L2 status does not match ${rel(STATUS)}:\n`);
  for (const p of problems) console.log(`  ${p.direction}  ${p.text}`);
  console.log("");
  const improved = problems.filter((p) => p.direction === "IMPROVED");
  const regressed = problems.filter((p) => p.direction === "REGRESSED");
  if (regressed.length) {
    console.log(
      "  A regression: a family's trace moved further from its reference. Read\n" +
        "  build/logs/dom-<fixture>.l2.log after ./scripts/dom-test.sh for the records."
    );
  }
  if (improved.length && !regressed.length) {
    console.log(
      "  Only improvements. This still fails, on purpose, the same way an\n" +
        "  examples/emit/NN.maxbytes you came in under still fails: commit the better\n" +
        "  status so the next regression has something to regress FROM.\n" +
        "    node --import ./register-loader.mjs check-l2-status.mjs --record"
    );
  }
  if (problems.some((p) => ["NEW", "REMOVED"].includes(p.direction))) {
    console.log("  A family appeared or vanished. Re-record after adding its declaration.");
  }
  if (problems.some((p) => ["UNATTRIBUTED", "UNKNOWN-CAUSE", "UNJUSTIFIED"].includes(p.direction))) {
    console.log(
      "  A row is missing its written justification. `state` is measured; `attribution`\n" +
        "  and `why` are declared by hand and --record carries them forward untouched."
    );
  }
}
process.exit(1);
