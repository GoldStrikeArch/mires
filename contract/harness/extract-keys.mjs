// Pin solid's hydration-key allocation by DRIVING the real sharedConfig.
//
//   node --import ./register-loader.mjs extract-keys.mjs
//
// Writes fixtures/keys.json.
//
// The scheme (solid-js/dist/solid.js:133-137):
//
//   function getContextId(count) {
//     const num = String(count), len = num.length - 1;
//     return sharedConfig.context.id + (len ? String.fromCharCode(96 + len) : "") + num;
//   }
//
// The letter is what makes the encoding prefix-free. Without it, parent id "0"
// with child count 1 ("01") would collide with parent id "0" child count 1 at a
// different nesting. The letter encodes the DIGIT COUNT of the number that
// follows: 1 digit -> no letter, 2 digits -> "a", 3 digits -> "b", ... so a
// reader can always tell where one segment ends and the next begins.
//
// We do not reimplement it. We set sharedConfig.context and call the real
// getNextContextId, recording (contextId, count) -> key triples.

import { writeJson, provenance, SOLID } from "./paths.mjs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const solid = await import(pathToFileURL(path.join(SOLID, "dist", "solid.js")).href);
const { sharedConfig } = solid;

/** Drive the real allocator for one (contextId, count) pair. */
function keyFor(contextId, count) {
  sharedConfig.context = { id: contextId, count };
  return sharedConfig.getNextContextId();
}

const triples = [];
const record = (contextId, count) => {
  const before = count;
  const key = keyFor(contextId, count);
  triples.push({
    contextId,
    count: before,
    key,
    // after the call the context's counter has advanced -- record that too,
    // since the emitter must reproduce the increment, not just the encoding
    countAfter: sharedConfig.context.count
  });
};

// Context ids to sweep: the empty root, single chars, and multi-segment ids
// that themselves came out of this scheme.
const CONTEXT_IDS = ["", "0", "1", "9", "a0", "0a10", "b100", "0a10b100"];

// Counts: every 1-digit value, a spread of 2-digit, a spread of 3-digit, and
// the boundaries either side of each digit-length change.
const COUNTS = [
  ...Array.from({ length: 10 }, (_, i) => i), // 0..9      1 digit
  10, 11, 12, 19, 20, 42, 50, 98, 99, // 2 digits
  100, 101, 123, 200, 500, 998, 999, // 3 digits
  1000, 1001, 4096, 9999, // 4 digits ("c")
  10000, 99999, // 5 digits ("d")
  100000 // 6 digits ("e")
];

for (const contextId of CONTEXT_IDS) {
  for (const count of COUNTS) {
    record(contextId, count);
  }
}

// ------------------------------------------------------- boundary assertions
// The properties the emitter must reproduce, checked here so a version bump
// that changes the scheme fails loudly.
const checks = [];
const expect = (name, actual, wanted) => {
  checks.push({ name, actual, expected: wanted, ok: actual === wanted });
};

expect("1-digit count gets no letter", keyFor("x", 7), "x7");
expect("2-digit count gets 'a'", keyFor("x", 42), "xa42");
expect("3-digit count gets 'b'", keyFor("x", 100), "xb100");
expect("4-digit count gets 'c'", keyFor("x", 1000), "xc1000");
expect("boundary 9 -> no letter", keyFor("x", 9), "x9");
expect("boundary 10 -> 'a'", keyFor("x", 10), "xa10");
expect("boundary 99 -> 'a'", keyFor("x", 99), "xa99");
expect("boundary 100 -> 'b'", keyFor("x", 100), "xb100");
expect("empty context id", keyFor("", 5), "5");

const failed = checks.filter((c) => !c.ok);
if (failed.length) {
  console.error("Key-scheme drift:");
  for (const f of failed) console.error(`  ${f.name}: got ${f.actual}, expected ${f.expected}`);
  process.exit(1);
}

// ------------------------------------------------------ nesting demonstration
// nextHydrateContext (solid.js:143-149) derives a CHILD context whose id is the
// parent's next key and whose count restarts at 0. This is what makes ids
// hierarchical. Reproduced here by hand-driving the same two steps, because
// nextHydrateContext is not exported.
const nesting = [];
{
  sharedConfig.context = { id: "", count: 0 };
  let ctx = sharedConfig.context;
  for (let depth = 0; depth < 4; depth++) {
    const childId = sharedConfig.getNextContextId();
    nesting.push({
      depth,
      parentId: ctx.id,
      parentCountBefore: depth === 0 ? 0 : nesting[depth - 1].parentCountAfter,
      childId,
      parentCountAfter: sharedConfig.context.count
    });
    // descend: child context id = the key just allocated, count resets to 0
    sharedConfig.context = { id: childId, count: 0 };
    ctx = sharedConfig.context;
  }
}

writeJson("keys.json", {
  ...provenance("extract-keys.mjs"),
  $source: "solid-js/dist/solid.js:121-149 (sharedConfig, getContextId, nextHydrateContext)",
  $algorithm: [
    "key = context.id + letter(digits(count) - 1) + String(count)",
    "letter(0) = ''  letter(n>0) = String.fromCharCode(96 + n)  // 1->'a', 2->'b', ...",
    "getNextContextId() returns the key for the CURRENT count, then post-increments count.",
    "getContextId() returns the key for the current count WITHOUT incrementing.",
    "A child context is { ...parent, id: parent.getNextContextId(), count: 0 }."
  ],
  $prefixFree:
    "The letter encodes the digit-length of the numeric suffix, so concatenated " +
    "segments are unambiguously separable -- that is the whole point of the letter.",
  tripleCount: triples.length,
  boundaryChecks: checks,
  nesting,
  triples
});

console.log(`keys.json  ${triples.length} triples, ${checks.length} boundary checks all green`);
console.log(`  sample: ${triples.slice(0, 3).map((t) => `(${t.contextId || "''"},${t.count})->${t.key}`).join("  ")}`);
