// The smallest runtime shim a compiled program can run against.
//
//   node scripts/make-min-shim.mjs <program.js> <out.js>
//
// runtime/shim.js installs about seventy members on `globalThis.__rt`, and a
// program reads a dozen of them. Shipping the whole file would double this
// entry's payload for members nothing calls, so this reads the program, collects
// the members it actually names, and writes a standalone script that installs
// those and their dependencies and nothing else.
//
// The scan is the one scripts/make-esm-shim.mjs and mires-build's shimcheck use:
// every `__rt.<name>` in the text, ignoring a match whose `__rt` is the tail of
// a longer identifier. It runs again over each definition it pulled in, because
// a member may call another one, and repeats until nothing new appears.
//
// A member the program names and runtime/shim.js does not define is an error
// here rather than a `TypeError` in a browser: the program would call it.
//
// The output is a plain script (never a module), so it concatenates ahead of a
// program compiled without `js-modules=esm`, exactly as runtime/shim.js does.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const shimPath = path.resolve(here, "..", "..", "..", "runtime", "shim.js");

const [, , programPath, outPath] = process.argv;
if (!programPath || !outPath) {
  console.error("usage: make-min-shim.mjs <program.js> <out.js>");
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Reading runtime/shim.js
// ---------------------------------------------------------------------------

// Where the next token starts, given that `at` is the start of one: a string, a
// template literal, a comment or an ordinary character, each skipped whole.
//
// A `/` is a comment opener or a division; runtime/shim.js has no regular
// expression literals, and one carrying a brace or a quote would be misread as
// text. The depth walk below refuses to close in that case rather than guessing,
// so the failure is loud.
function skip(text, at) {
  const c = text[at];
  if (c === "/" && text[at + 1] === "/") {
    const end = text.indexOf("\n", at);
    return end === -1 ? text.length : end;
  }
  if (c === "/" && text[at + 1] === "*") {
    const end = text.indexOf("*/", at + 2);
    return end === -1 ? text.length : end + 2;
  }
  if (c === '"' || c === "'" || c === "`") {
    let i = at + 1;
    while (i < text.length) {
      if (text[i] === "\\") {
        i += 2;
        continue;
      }
      // A `${` opens ordinary code again, which may itself hold the quote that
      // closes nothing. Walk it with the same depth rule.
      if (c === "`" && text[i] === "$" && text[i + 1] === "{") {
        let depth = 1;
        i += 2;
        while (i < text.length && depth > 0) {
          const next = skip(text, i);
          if (next > i + 1) {
            i = next;
            continue;
          }
          if (text[i] === "{") depth += 1;
          if (text[i] === "}") depth -= 1;
          i += 1;
        }
        continue;
      }
      if (text[i] === c) return i + 1;
      i += 1;
    }
    return text.length;
  }
  return at + 1;
}

// The index just past the `;` that ends the statement starting at `from`, with
// brackets, braces, parentheses, strings and comments skipped.
function statementEnd(text, from) {
  let depth = 0;
  let i = from;
  while (i < text.length) {
    const next = skip(text, i);
    if (next > i + 1) {
      i = next;
      continue;
    }
    const c = text[i];
    if (c === "{" || c === "(" || c === "[") depth += 1;
    else if (c === "}" || c === ")" || c === "]") depth -= 1;
    else if (c === ";" && depth === 0) return i + 1;
    i += 1;
  }
  return -1;
}

// `name -> the text that installs it`, in the order runtime/shim.js installs
// them, because a member built out of another one (`__rt.box` reads
// `__rt._boxes`) has to come after it.
function definitions(source) {
  const out = new Map();

  // The head is an object literal assigned to `__rt` whole. Its members are
  // split out so that a program reaching one log helper does not drag in six.
  const head = source.indexOf("globalThis.__rt = {");
  if (head === -1) {
    console.error("make-min-shim.mjs: runtime/shim.js does not open with `globalThis.__rt = {`");
    process.exit(1);
  }
  const open = source.indexOf("{", head);
  const headEnd = statementEnd(source, head);
  if (headEnd === -1) {
    console.error("make-min-shim.mjs: the opening object literal does not close");
    process.exit(1);
  }
  const close = source.lastIndexOf("}", headEnd);
  let i = open + 1;
  let key = null;
  let valueFrom = -1;
  let depth = 0;
  while (i < close) {
    const next = skip(source, i);
    if (next > i + 1) {
      i = next;
      continue;
    }
    const c = source[i];
    if (c === "{" || c === "(" || c === "[") depth += 1;
    else if (c === "}" || c === ")" || c === "]") depth -= 1;
    else if (depth === 0 && c === ":" && key === null) {
      const before = source.slice(0, i);
      const match = /([A-Za-z_$][A-Za-z0-9_$]*)\s*$/.exec(before);
      if (match) {
        key = match[1];
        valueFrom = i + 1;
      }
    } else if (depth === 0 && c === "," && key !== null) {
      out.set(key, `globalThis.__rt.${key} =${source.slice(valueFrom, i)};`);
      key = null;
    }
    i += 1;
  }
  if (key !== null) {
    out.set(key, `globalThis.__rt.${key} =${source.slice(valueFrom, close)};`);
  }

  // Everything else is a top level `globalThis.__rt.<name> = <expr>;`.
  const assignment = /^globalThis\.__rt\.([A-Za-z0-9_$]+) =/gm;
  for (const match of source.matchAll(assignment)) {
    const end = statementEnd(source, match.index);
    if (end === -1) {
      console.error(`make-min-shim.mjs: the definition of \`${match[1]}\` does not close`);
      process.exit(1);
    }
    out.set(match[1], source.slice(match.index, end));
  }

  return out;
}

// Every member `text` reads off `__rt`. The guard is mires-build's: `a__rt.foo`
// and `x.__rt.foo` are not reads of the shim. `globalThis.__rt.foo` is, which is
// how runtime/shim.js writes every one of its own, so the qualifier is part of
// the pattern rather than something the guard has to see past.
function members(text) {
  const out = new Set();
  const pattern = /(?:globalThis\.)?__rt\.([A-Za-z0-9_$]+)/g;
  for (const match of text.matchAll(pattern)) {
    const before = match.index === 0 ? "" : text[match.index - 1];
    if (/[A-Za-z0-9_$.]/.test(before)) continue;
    out.add(match[1]);
  }
  return out;
}

// ---------------------------------------------------------------------------
// The closure
// ---------------------------------------------------------------------------

const shim = readFileSync(shimPath, "utf8");
const program = readFileSync(programPath, "utf8");
const defined = definitions(shim);

const wanted = new Set(members(program));
const direct = new Set(wanted);
const seen = new Set();
const missing = new Set();

// Iterate the scan over the bodies pulled in, because a member calls others:
// `__rt.copy` reads `__rt._put`, which reads `__rt._boxed`.
while (wanted.size > seen.size) {
  for (const name of [...wanted]) {
    if (seen.has(name)) continue;
    seen.add(name);
    const body = defined.get(name);
    if (body === undefined) {
      missing.add(name);
      continue;
    }
    for (const next of members(body)) wanted.add(next);
  }
}

if (missing.size > 0) {
  console.error(
    `make-min-shim.mjs: ${path.basename(programPath)} reads ${missing.size} member(s) ` +
      `runtime/shim.js does not install: ${[...missing].sort().join(", ")}`,
  );
  process.exit(1);
}

const kept = [...defined.keys()].filter((name) => wanted.has(name));

const out = [
  "// The runtime shim, cut down to what this program reads.",
  "//",
  "// Generated by scripts/make-min-shim.mjs out of runtime/shim.js. Do not edit:",
  "// change the shim and build again.",
  "globalThis.__rt = globalThis.__rt || {};",
  ...kept.map((name) => defined.get(name)),
  "",
].join("\n");

writeFileSync(outPath, out);

console.log(
  `min-shim: ${kept.length} of ${defined.size} members ` +
    `(${direct.size} named by the program, ${kept.length - direct.size} pulled in)`,
);
