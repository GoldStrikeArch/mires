// Run every extractor and every proof, in dependency order.
//
//   node --import ./register-loader.mjs run-all.mjs          run everything
//   node --import ./register-loader.mjs run-all.mjs --check   fail if anything changed
//
// --check is the drift gate: it runs the full pipeline and then reports any
// fixture whose committed bytes differ from what was just regenerated. A clean
// run means the committed fixtures are exactly what the pinned upstream
// produces today.

import { execFileSync } from "node:child_process";
import { readFileSync, existsSync, readdirSync, statSync } from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { HARNESS, FIXTURES, requireUpstream } from "./paths.mjs";

const check = process.argv.includes("--check");

requireUpstream();

// Order matters in two places: extract-bootstrap reads delegated-events.json,
// and gen-tree-oracle reads the corpus's compiled output, so gen-corpus has to
// have written it first. Everything else needs nothing but the vendor install.
const STEPS = [
  ["extract-constants.mjs", []],
  ["extract-abi.mjs", []],
  ["extract-keys.mjs", []],
  ["extract-nested-keys.mjs", []],
  ["extract-escaping.mjs", []],
  ["extract-bootstrap.mjs", []],
  ["gen-corpus.mjs", []],
  ["gen-tree-oracle.mjs", []],
  ["import-upstream-fixtures.mjs", []],
  ["verify-reference.mjs", ["--all"]],
  ["run-trace.mjs", ["--record"]],
  ["test-compare-trace.mjs", []],
  // Not an extractor and not a proof about upstream: procedure-wire.json is
  // hand-written, because the procedure wire is Topcoat's own and there is no
  // upstream to derive it from. This step re-reads the framework source lines the
  // file cites and fails if a cited value moved, so the hand-written spec is
  // gated the same way a generated fixture is.
  ["check-procedure-wire.mjs", []],
  // Also not an extractor. `#[js_extern]`'s descriptor design is ours and has no
  // upstream, so its vectors are RECORDED by driving a checked-in fake library
  // through reference emissions, the way run-trace.mjs records the reference
  // compiler's runtime calls. The circularity that would otherwise create --
  // a driver cannot disagree with its own recording -- is closed by the
  // assertions in fixtures/js-extern/drivers.mjs, which run on every invocation.
  ["check-js-extern.mjs", ["--record"]],
  // The L2 gate, and the third step here that is neither an extractor nor a proof
  // about upstream. It recomputes every corpus family's trace-parity verdict and
  // fails if it differs from the committed fixtures/l2-status.json IN EITHER
  // DIRECTION. It runs in --check mode rather than --record so the failure carries
  // its own directional message instead of surfacing only as a drift hash.
  //
  // It reads two things: the reference traces in fixtures/corpus/, which the steps
  // above regenerate from pinned upstream, and examples/dom-tests/NN.trace.expected,
  // which is ours. So it is also the one step that fails when an upstream bump moves
  // a reference trace underneath a recorded verdict -- which is exactly the drift
  // question this script exists to ask, one level up from bytes.
  ["check-l2-status.mjs", ["--check"]]
];

/** sha256 of every file under a directory, keyed by relative path. */
function snapshot(dir) {
  const out = new Map();
  if (!existsSync(dir)) return out;
  const walk = (d) => {
    for (const entry of readdirSync(d, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name)
    )) {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) {
        out.set(
          path.relative(dir, full),
          createHash("sha256").update(readFileSync(full)).digest("hex")
        );
      }
    }
  };
  walk(dir);
  return out;
}

const before = check ? snapshot(FIXTURES) : null;

let failed = 0;
for (const [script, args] of STEPS) {
  const label = [script, ...args].join(" ");
  process.stdout.write(`\n=== ${label}\n`);
  try {
    const out = execFileSync(
      process.execPath,
      ["--import", path.join(HARNESS, "register-loader.mjs"), path.join(HARNESS, script), ...args],
      { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], cwd: HARNESS }
    );
    process.stdout.write(
      out
        .split("\n")
        .filter(Boolean)
        .map((l) => `  ${l}`)
        .join("\n") + "\n"
    );
  } catch (err) {
    failed++;
    process.stdout.write(`  FAILED (exit ${err.status})\n`);
    if (err.stdout) process.stdout.write(`${err.stdout}`.split("\n").map((l) => `  ${l}`).join("\n"));
    if (err.stderr) process.stderr.write(`${err.stderr}`);
  }
}

if (check) {
  const after = snapshot(FIXTURES);
  const changed = [];
  const added = [];
  const removed = [];
  for (const [rel, hash] of after) {
    if (!before.has(rel)) added.push(rel);
    else if (before.get(rel) !== hash) changed.push(rel);
  }
  for (const rel of before.keys()) if (!after.has(rel)) removed.push(rel);

  // Traces embed no timestamps, and every extractor writes deterministic
  // output, so any difference here is real drift.
  process.stdout.write("\n=== drift check\n");
  if (!changed.length && !added.length && !removed.length) {
    process.stdout.write("  no drift: committed fixtures match regenerated output exactly\n");
  } else {
    for (const r of changed) process.stdout.write(`  CHANGED ${r}\n`);
    for (const r of added) process.stdout.write(`  ADDED   ${r}\n`);
    for (const r of removed) process.stdout.write(`  REMOVED ${r}\n`);
    failed++;
  }
}

process.stdout.write(`\n${failed ? `${failed} step(s) failed` : "all steps green"}\n`);
process.exit(failed ? 1 : 0);
