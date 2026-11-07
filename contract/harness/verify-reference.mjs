// G0 proof 3: prove the pinned plugin + options reproduce upstream's OWN
// expected fixture outputs.
//
//   node --import ./register-loader.mjs verify-reference.mjs [--all] [--diff]
//
// By default checks the four families named in the milestone spec across
// __dom_hydratable_fixtures__. --all sweeps every fixture in every family we
// have a preset for, which is the stronger claim.
//
// A byte-match here means: our babel version, our plugin version, our plugin
// options, and our formatter all agree with the tree that produced the
// committed output.js. If this passes, the pin is faithful and the fixtures
// downstream of it can be trusted.

import { compileUpstreamFixture, FIXTURE_PRESETS } from "./compile-reference.mjs";
import { PLUGIN_TESTS, writeJson, provenance } from "./paths.mjs";
import { readdirSync, existsSync } from "node:fs";
import path from "node:path";

const argv = process.argv.slice(2);
const all = argv.includes("--all");
const showDiff = argv.includes("--diff");

// The four families the milestone requires, all in the hydratable DOM set.
const REQUIRED = [
  ["__dom_hydratable_fixtures__", "simpleElements"],
  ["__dom_hydratable_fixtures__", "textInterpolation"],
  ["__dom_hydratable_fixtures__", "eventExpressions"],
  ["__dom_hydratable_fixtures__", "insertChildren"]
];

function fixturesIn(family) {
  const dir = path.join(PLUGIN_TESTS, family);
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true })
    .filter((d) => d.isDirectory() && existsSync(path.join(dir, d.name, "code.js")))
    .map((d) => d.name)
    .sort();
}

const targets = all
  ? Object.keys(FIXTURE_PRESETS).flatMap((f) => fixturesIn(f).map((n) => [f, n]))
  : REQUIRED;

/**
 * Compare, and also report whether a mismatch is only trailing whitespace.
 * We report both so any normalization we rely on is explicit, not hidden.
 */
function classify(expected, actual) {
  if (expected === actual) return { match: "exact", normalization: "none" };
  if (expected.replace(/\s+$/, "") === actual.replace(/\s+$/, "")) {
    return { match: "trailing-whitespace", normalization: "stripped trailing whitespace" };
  }
  if (expected.replace(/[ \t]+$/gm, "") === actual.replace(/[ \t]+$/gm, "")) {
    return { match: "line-trailing-whitespace", normalization: "stripped per-line trailing whitespace" };
  }
  return { match: "differs", normalization: null };
}

const results = [];
let failures = 0;

for (const [family, name] of targets) {
  let entry;
  try {
    const { expected, actual } = compileUpstreamFixture(family, name);
    const c = classify(expected, actual);
    entry = { family, fixture: name, ...c };
    if (c.match === "differs") {
      failures++;
      if (showDiff) {
        console.error(`\n--- ${family}/${name} EXPECTED ---\n${expected}`);
        console.error(`--- ${family}/${name} ACTUAL ---\n${actual}`);
      }
    }
  } catch (err) {
    failures++;
    entry = { family, fixture: name, match: "error", error: err.message };
  }
  results.push(entry);
}

const byMatch = results.reduce((acc, r) => {
  acc[r.match] = (acc[r.match] ?? 0) + 1;
  return acc;
}, {});

for (const r of results) {
  const mark = r.match === "exact" ? "ok  " : r.match === "differs" || r.match === "error" ? "FAIL" : "~   ";
  console.log(`${mark} ${r.family}/${r.fixture}  ${r.match}${r.error ? `: ${r.error}` : ""}`);
}

console.log(`\n${results.length} fixtures: ${JSON.stringify(byMatch)}`);

if (all) {
  writeJson("reference-parity.json", {
    ...provenance("verify-reference.mjs"),
    $what:
      "G0 proof 3. Each entry is one upstream fixture compiled by our pinned " +
      "toolchain and compared against upstream's committed output.js.",
    $normalization:
      "match:'exact' means byte-identical with NO normalization. Any other value " +
      "names the normalization that was required.",
    summary: byMatch,
    total: results.length,
    results
  });
}

if (failures) {
  console.error(
    `\n${failures} fixture(s) did not reproduce. The pin is NOT faithful -- ` +
      `check plugin options in compile-reference.mjs PRESETS against the upstream spec files.`
  );
  process.exit(1);
}
console.log("all target fixtures reproduced");
