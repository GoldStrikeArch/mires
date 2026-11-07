// Shared absolute paths + the pin manifest. Every extractor imports this so
// there is exactly one place that knows the layout.

import { fileURLToPath } from "node:url";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import path from "node:path";

export const HARNESS = path.dirname(fileURLToPath(import.meta.url));
export const CONTRACT = path.resolve(HARNESS, "..");
export const VENDOR = path.join(CONTRACT, "vendor");
export const UPSTREAM = path.join(VENDOR, "upstream");
export const FIXTURES = path.join(CONTRACT, "fixtures");

export const DOM_EXPRESSIONS = path.join(UPSTREAM, "dom-expressions");
export const DOM_SRC = path.join(DOM_EXPRESSIONS, "src");
export const SOLID = path.join(UPSTREAM, "solid-js");
export const GIT = path.join(UPSTREAM, "git");
export const PLUGIN_TESTS = path.join(
  GIT,
  "packages",
  "babel-plugin-jsx-dom-expressions",
  "test"
);

export const lock = JSON.parse(readFileSync(path.join(CONTRACT, "upstream.lock"), "utf8"));

/** Fail loudly and early if fetch.sh has not been run. */
export function requireUpstream() {
  if (!existsSync(DOM_SRC)) {
    throw new Error(
      `vendor/upstream/ is missing. Run contract/vendor/fetch.sh first.\n  expected: ${DOM_SRC}`
    );
  }
}

/**
 * Assert the node_modules copy of a package agrees with upstream.lock. The
 * hash-verified tarball in upstream/ and the yarn-installed copy in
 * node_modules/ are two copies of the same pin; this keeps them from drifting
 * apart silently.
 */
export function assertPinnedVersion(name, actualVersion) {
  const want = lock.npm[name]?.version;
  if (!want) throw new Error(`upstream.lock has no pin for ${name}`);
  if (actualVersion !== want) {
    throw new Error(
      `${name} version drift: upstream.lock pins ${want}, resolved copy is ${actualVersion}`
    );
  }
}

/** Deterministic JSON: sorted keys where we build objects, 2-space indent, trailing newline. */
export function writeJson(relPath, value) {
  const dest = path.join(FIXTURES, relPath);
  writeFileSync(dest, `${JSON.stringify(value, null, 2)}\n`);
  return dest;
}

export function writeText(relPath, text) {
  const dest = path.join(FIXTURES, relPath);
  writeFileSync(dest, text);
  return dest;
}

/**
 * Standard header every generated fixture carries, so a reader can tell what
 * produced it and from which pin.
 */
export function provenance(extractor) {
  return {
    $generatedBy: `contract/harness/${extractor}`,
    $doNotEditByHand:
      "Re-run the extractor instead. A diff here after a version bump IS the drift report.",
    $extractedFrom: {
      "dom-expressions": lock.npm["dom-expressions"].version,
      "solid-js": lock.npm["solid-js"].version,
      "babel-plugin-jsx-dom-expressions": lock.npm["babel-plugin-jsx-dom-expressions"].version,
      gitSha: lock.git.sha
    },
    $extractionDate: lock.extractionDate
  };
}
