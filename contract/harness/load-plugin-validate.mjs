// Load the babel plugin's OWN markup validator from the pinned git tree and
// expose it as a normal function.
//
// src/shared/validate.js is ESM-with-require: it has `export function` but also
// a top-level `const parse5 = require("parse5")`. Node can load it as neither
// CJS (the exports) nor ESM (the require). Rather than reimplement the
// validator -- which is the one piece of upstream logic that decides whether a
// template string is safe to walk -- we compile the module wrapper away and run
// the ORIGINAL BODY verbatim in a CJS scope.
//
// Only the module syntax is rewritten. Not one line of the validation logic is
// touched, so this stays a driver, not a reimplementation.

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { GIT, VENDOR } from "./paths.mjs";

const VALIDATE_PATH = path.join(
  GIT,
  "packages",
  "babel-plugin-jsx-dom-expressions",
  "src",
  "shared",
  "validate.js"
);

const source = readFileSync(VALIDATE_PATH, "utf8");

// The only transformation: turn the ESM export declarations into plain
// declarations and collect them onto module.exports. Asserted, so a change in
// upstream's export style fails loudly instead of silently exporting nothing.
const exported = [...source.matchAll(/^export\s+function\s+([A-Za-z0-9_$]+)/gm)].map((m) => m[1]);
if (exported.length === 0) {
  throw new Error(`no exports found in ${VALIDATE_PATH} -- the module shape changed`);
}

const cjs = `${source.replace(/^export\s+function\s+/gm, "function ")}
module.exports = { ${exported.join(", ")} };`;

const vendorRequire = createRequire(path.join(VENDOR, "package.json"));
const module_ = { exports: {} };
// eslint-disable-next-line no-new-func
new Function("require", "module", "exports", "__filename", "__dirname", cjs)(
  vendorRequire,
  module_,
  module_.exports,
  VALIDATE_PATH,
  path.dirname(VALIDATE_PATH)
);

/**
 * Upstream's own check. Returns null/undefined when the markup round-trips
 * through the HTML parser unchanged, or { html, browser } describing the
 * divergence when the parser restructured it.
 *
 * Note it expects `templateWithClosingTags` -- the closed, attribute-free form
 * the plugin maintains in parallel with the emitted template string.
 */
export const { isInvalidMarkup } = module_.exports;
export const VALIDATE_SOURCE_PATH = VALIDATE_PATH;
export const EXPORTED_NAMES = exported;
