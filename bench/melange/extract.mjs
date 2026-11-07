// Compiles store.ml with the Melange v7.0.1 playground and reports what the
// output imports.
//
//   node extract.mjs                     # compile store.ml -> out/store.melange.js
//   node extract.mjs --src probe.ml --out out/probe.js
//   node extract.mjs --offline           # skip the browser, re-run the import report
//
// WHY A BROWSER
// -------------
// There is no opam here, so there is no `melc`. The playground at
// https://melange.re/v7.0.1/playground/ is the compiler: the whole of Melange is
// compiled to JavaScript by js_of_ocaml and installed on the page as
// `globalThis.ocaml`, whose `compileML(source)` is the entry point. It is
// client side end to end -- nothing is sent to a server -- so driving the page
// with puppeteer-core and calling that function IS running the compiler, just
// through a window. The bundle is around 23 MB, which is why the waits below are
// measured in minutes rather than seconds.
//
// THE IMPORT REPORT IS THE GATE
// -----------------------------
// Melange's runtime lives in the `melange.js` opam package and is emitted as
// ordinary ES imports (`melange.js/caml_array.mjs` and friends). Those files do
// not exist in any npm registry, so an output that imports one cannot be run or
// honestly measured here: its real size is its own bytes plus a runtime nobody
// can produce, and node would refuse to load it at all.
//
// The answer is not to fake the runtime, it is to not need it. Melange's
// `external` declarations inline into plain JavaScript, so a module written out
// of externals and records emits no imports whatsoever -- which is the same
// discipline the Rust side is held to (global-rooted `#[js_extern]`, no DOM).
// So this script PRINTS every specifier it finds and exits non-zero when there
// is one, and the loop is: adapt store.ml until the report is empty. Only an
// irreducible import gets a hand written stub, and its bytes are then counted on
// Melange's side of the size table.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

const PLAYGROUND = "https://melange.re/v7.0.1/playground/";
const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

// The compiler bundle is ~23 MB of js_of_ocaml output; on a cold cache the load
// and the evaluation of it are both slow, and a timeout here reads as "the
// playground is gone" when it only means "not yet".
const LOAD_TIMEOUT = 180_000;

// --------------------------------------------------------------------- args

const args = process.argv.slice(2);
let src = path.join(here, "store.ml");
let out = path.join(here, "out", "store.melange.js");
let offline = false;

for (let i = 0; i < args.length; i++) {
	switch (args[i]) {
		case "--offline":
			offline = true;
			break;
		case "--src":
			src = path.resolve(here, args[++i]);
			break;
		case "--out":
			out = path.resolve(here, args[++i]);
			break;
		default:
			console.error(`extract.mjs: unknown argument: ${args[i]}`);
			process.exit(2);
	}
}

// ----------------------------------------------------------- the import report

// Four ways a module can name another one. `export ... from` is in the list
// because it is an import that happens to re-export, and a scan that missed it
// would call an output import-free when node still has to resolve a file.
const SPECIFIER_PATTERNS = [
	/\bimport\s+[^;'"]*?\bfrom\s*["']([^"']+)["']/g,
	/\bimport\s*["']([^"']+)["']/g,
	/\bexport\s+[^;'"]*?\bfrom\s*["']([^"']+)["']/g,
	/\brequire\s*\(\s*["']([^"']+)["']\s*\)/g,
	/\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
];

/**
 * Every module specifier the source names, in source order, deduplicated.
 *
 * @param {string} source
 * @returns {string[]}
 */
function specifiers(source) {
	const found = new Map();
	for (const pattern of SPECIFIER_PATTERNS) {
		for (const match of source.matchAll(pattern)) {
			if (!found.has(match[1])) found.set(match[1], match.index ?? 0);
		}
	}
	return [...found.entries()].sort((a, b) => a[1] - b[1]).map(([spec]) => spec);
}

/**
 * Print the report. Returns the specifiers so the caller can decide the exit
 * status -- printing and deciding are separate so `--offline` can print the same
 * thing without re-compiling.
 *
 * @param {string} file
 * @returns {string[]}
 */
function report(file) {
	const source = readFileSync(file, "utf8");
	const specs = specifiers(source);
	console.log("");
	console.log(`==> import report for ${path.relative(here, file)}`);
	if (specs.length === 0) {
		console.log("    clean: no import, export-from, require or dynamic import");
	} else {
		for (const spec of specs) console.log(`    ${spec}`);
		console.log("");
		console.log(`    ${specs.length} specifier(s). Melange's runtime is opam-only and not`);
		console.log("    npm-resolvable, so this output cannot be run or honestly measured.");
		console.log("    Adapt store.ml to eliminate them (externals inline; Stdlib.Array,");
		console.log("    Stdlib string functions and Printf do not). Only if one is");
		console.log("    irreducible, stub it under out/node_modules/ and count the stub's");
		console.log("    bytes on Melange's side of the size table.");
	}
	const lines = source.split("\n").length;
	console.log(`    ${Buffer.byteLength(source, "utf8")} bytes, ${lines} lines`);
	return specs;
}

// ------------------------------------------------------------------- offline

if (offline) {
	const specs = report(out);
	process.exit(specs.length === 0 ? 0 : 1);
}

// ------------------------------------------------------------------ compile

const source = readFileSync(src, "utf8");
console.log(`==> compiling ${path.relative(here, src)} (${Buffer.byteLength(source, "utf8")} bytes)`);
console.log(`    through ${PLAYGROUND}`);

const { default: puppeteer } = await import("puppeteer-core");

const browser = await puppeteer.launch({
	executablePath: CHROME,
	headless: true,
	args: ["--no-sandbox", "--disable-dev-shm-usage"],
});

let result;
try {
	const page = await browser.newPage();
	page.on("pageerror", error => console.error(`    [page error] ${error.message}`));

	await page.goto(PLAYGROUND, { waitUntil: "domcontentloaded", timeout: LOAD_TIMEOUT });
	console.log("    page loaded, waiting for the compiler bundle (~23 MB)");

	await page.waitForFunction(
		// eslint-disable-next-line no-undef
		() => Boolean(globalThis.ocaml) && typeof globalThis.ocaml.compileML === "function",
		{ timeout: LOAD_TIMEOUT, polling: 500 },
	);
	console.log("    globalThis.ocaml.compileML is up");

	result = await page.evaluate(ml => globalThis.ocaml.compileML(ml), source);
} finally {
	await browser.close();
}

// The success shape is `{ js_code, warnings, ... }`. Everything else is a
// failure, and the useful thing to do with it is show it whole rather than guess
// at which of the several error shapes the playground picked.
if (!result || typeof result.js_code !== "string") {
	console.error("extract.mjs: the playground did not return JavaScript.");
	console.error(`  keys: ${result ? Object.keys(result).join(", ") : String(result)}`);
	console.error(JSON.stringify(result, null, 2));
	process.exit(1);
}

if (Array.isArray(result.warnings) && result.warnings.length > 0) {
	console.log("    warnings:");
	for (const warning of result.warnings) {
		console.log(`      ${typeof warning === "string" ? warning : JSON.stringify(warning)}`);
	}
}

mkdirSync(path.dirname(out), { recursive: true });
writeFileSync(out, result.js_code);
console.log(`    wrote ${path.relative(here, out)}`);

const specs = report(out);
process.exit(specs.length === 0 ? 0 : 1);
