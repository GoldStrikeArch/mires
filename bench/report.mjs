#!/usr/bin/env node
//
// bench/results-raw/*.json -> bench/RESULTS.md
//
//   node bench/report.mjs
//
// Run by bench/run.sh as its last step, and safe to re-run on its own: it reads
// only files, and rewrites RESULTS.md from scratch every time.
//
// WHY THE MEDIANS ARE RECOMPUTED HERE
// -----------------------------------
// Every result file already carries a `median` next to the raw `values`, and
// this script ignores it and computes its own from the array -- then checks the
// two agree and says so in the footer. That is not distrust of upstream's
// arithmetic; it is that a table of numbers whose provenance is "a field in a
// file" cannot be argued with, while one whose provenance is "the median of
// these fifteen values, which are printed in results-raw/" can. The check is
// what makes the recomputation worth anything: if it ever disagreed, that would
// be a fact about the harness worth knowing rather than a silent divergence.
//
// WHY THE GEOMEAN IS NOT THE OFFICIAL ONE
// ---------------------------------------
// krausest's published ranking is a weighted geometric mean over values
// normalised against the p90 of the whole published population -- hundreds of
// implementations measured on the maintainer's machine. That population does
// not exist locally, and inventing a weighting would be worse than not having
// one. So this reports the plain geometric mean of per-benchmark slowdowns
// against vanillajs-keyed, which is a different statistic with a different
// meaning, and says so under the table rather than letting the number be
// mistaken for a leaderboard position.
//
// WHY OUR ENTRIES GET A SECOND SIZE SECTION
// -----------------------------------------
// The harness's size benchmark is one brotli number for everything the page
// loads, which is the right number for comparing implementations and useless
// for understanding one: it cannot say how much of our payload is the runtime,
// how much is the island chunk and how much is the shim. So our entry
// also get a per-artifact breakdown measured with the spike's own conventions
// (gzip at a pinned level 9, summed per artifact -- see bench/lib/
// budget.mjs). The two tables are NOT comparable with each other, and the
// section says so where someone reading only that section will see it.
//
// PARTIAL DATA IS A SUPPORTED STATE
// ---------------------------------
// A `run.sh --quick` produces a handful of benchmarks for some of the
// implementations, and that draft has to render. Anything absent is left out
// and reported as absent -- never zero, never a blank that reads as "fast".

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

import { GZIP_LEVEL, sizes } from "./lib/budget.mjs";

const bench = dirname(fileURLToPath(import.meta.url));
const root = join(bench, "..");
const rawDir = join(bench, "results-raw");
const outFile = join(bench, "RESULTS.md");

/**
 * The six implementations, in table order, each with the shape of the
 * `framework` string the harness writes into its result files.
 *
 * Matched by predicate rather than by literal, because the string embeds a
 * version (`solid-v1.9.3-keyed`) that moves whenever the pin does, and a table
 * that silently loses a column on a version bump would be worse than one that
 * fails loudly.
 */
const SLOTS = [
	{
		dir: "keyed/vanillajs",
		label: "vanillajs keyed",
		baseline: true,
		matches: name => name === "vanillajs-keyed",
	},
	{
		dir: "keyed/solid",
		label: "solid keyed",
		matches: name => /^solid-/.test(name) && name.endsWith("-keyed"),
	},
	{
		dir: "keyed/leptos",
		label: "leptos keyed",
		matches: name => /^leptos-/.test(name) && name.endsWith("-keyed"),
	},
	{
		dir: "non-keyed/vanillajs",
		label: "vanillajs non-keyed",
		matches: name => name === "vanillajs-non-keyed",
	},
	{
		dir: "non-keyed/mires-vanilla",
		label: "mires-vanilla",
		ours: { source: join(bench, "vanilla", "dist-krausest"), note: "Entry B: standalone crate, DOM ops written by hand in Rust over `#[js_extern]`." },
		matches: name => name.startsWith("mires-vanilla"),
	},
];

/** The CPU benchmarks, with the throttle the harness applies to each. */
const CPU = [
	{ id: "01_run1k", label: "create 1,000 rows" },
	{ id: "02_replace1k", label: "replace all 1,000 rows" },
	{ id: "03_update10th1k_x16", label: "partial update (every 10th, x16)", throttle: 4 },
	{ id: "04_select1k", label: "select row", throttle: 4 },
	{ id: "05_swap1k", label: "swap rows", throttle: 4 },
	{ id: "06_remove-one-1k", label: "remove row", throttle: 2 },
	{ id: "07_create10k", label: "create 10,000 rows" },
	{ id: "08_create1k-after1k_x2", label: "append 1,000 to 10,000" },
	{ id: "09_clear1k_x8", label: "clear 1,000 rows", throttle: 4 },
];

const MEMORY = [
	{ id: "21_ready-memory", label: "ready memory" },
	{ id: "22_run-memory", label: "memory after adding 1,000 rows" },
	{ id: "25_run-clear-memory", label: "memory after clearing 1,000 rows" },
];

const SIZE = [
	{ id: "42_size-compressed", label: "transferred (brotli)", unit: "KB" },
	{ id: "41_size-uncompressed", label: "transferred (uncompressed)", unit: "KB" },
	{ id: "43_first-paint", label: "first paint", unit: "ms" },
];

// ------------------------------------------------------------------ reading

/** @returns {{byFramework: Map<string, Map<string, object>>, files: number, unmatched: string[]}} */
function readRaw() {
	const byFramework = new Map();
	if (!existsSync(rawDir)) return { byFramework, files: 0, unmatched: [] };

	let files = 0;
	for (const name of readdirSync(rawDir).sort()) {
		// `_aggregate.json` is createResultJS's roll-up of everything below it, a
		// different shape entirely; the per-benchmark files are the source here.
		if (!name.endsWith(".json") || name.startsWith("_")) continue;
		let result;
		try {
			result = JSON.parse(readFileSync(join(rawDir, name), "utf8"));
		} catch (error) {
			console.warn(`report.mjs: skipping ${name}: ${error.message}`);
			continue;
		}
		if (!result?.framework || !result?.benchmark) {
			console.warn(`report.mjs: skipping ${name}: no framework/benchmark fields`);
			continue;
		}
		files += 1;
		if (!byFramework.has(result.framework)) byFramework.set(result.framework, new Map());
		byFramework.get(result.framework).set(result.benchmark, result);
	}

	const unmatched = [...byFramework.keys()].filter(name => !SLOTS.some(slot => slot.matches(name)));
	return { byFramework, files, unmatched };
}

/** The median of an array, computed the way the harness computes it. */
function median(values) {
	const sorted = [...values].sort((a, b) => a - b);
	const half = sorted.length / 2;
	return sorted.length % 2 === 0 ? 0.5 * (sorted[half - 1] + sorted[half]) : sorted[Math.trunc(half)];
}

const disagreements = [];

// Only the CPU benchmarks say anything about how thorough a run was: the memory,
// size and startup benchmarks run once by upstream's default, so counting their
// single iteration as "fewer than 15" would label every full run a draft.
const cpuIterationCounts = new Set();

/**
 * One number for one implementation and one benchmark, or null.
 *
 * `key` picks the series: CPU results carry total/script/paint, everything else
 * a single DEFAULT.
 */
function value(result, key) {
	const series = result?.values?.[key];
	if (!series || !Array.isArray(series.values) || series.values.length === 0) return null;
	if (result.type === "cpu") cpuIterationCounts.add(series.values.length);
	const ours = median(series.values);
	if (typeof series.median === "number" && Math.abs(ours - series.median) > 1e-9) {
		disagreements.push(`${result.framework} ${result.benchmark} ${key}: recomputed ${ours}, file says ${series.median}`);
	}
	return ours;
}

// ------------------------------------------------------------------ tables

const fixed = (n, digits) => (n === null || n === undefined ? "--" : n.toFixed(digits));

/** A markdown table from a header row and body rows, left-aligned first column. */
function table(header, rows) {
	const widths = header.map((cell, i) => Math.max(cell.length, ...rows.map(row => String(row[i]).length)));
	const line = cells => `| ${cells.map((cell, i) => String(cell).padEnd(widths[i])).join(" | ")} |`;
	const rule = `|${widths.map((width, i) => (i === 0 ? ":" : " ") + "-".repeat(width) + (i === 0 ? " " : ":")).join("|")}|`;
	return [line(header), rule, ...rows.map(line)].join("\n");
}

function geomean(factors) {
	if (factors.length === 0) return null;
	return Math.exp(factors.reduce((total, factor) => total + Math.log(factor), 0) / factors.length);
}

// ------------------------------------------------------ per-artifact sizes

/** Every file under a delivered entry directory that the browser could fetch. */
function artifactsOf(dir) {
	const skip = new Set(["node_modules", ".git", ".DS_Store", "package.json", "package-lock.json"]);
	const found = [];
	const walk = (current, prefix) => {
		for (const name of readdirSync(current).sort()) {
			if (skip.has(name)) continue;
			const full = join(current, name);
			const rel = prefix ? `${prefix}/${name}` : name;
			if (statSync(full).isDirectory()) walk(full, rel);
			else found.push({ rel, full });
		}
	};
	walk(dir, "");
	return found;
}

/**
 * Which of the files in a delivered directory the benchmark page actually
 * loads, if the entry says so.
 *
 * It has to be able to say so. An entry that ships one chunk per island (the
 * island entry does) has a directory full of code the benchmark page never
 * fetches, and summing the directory would report a payload several times the
 * real one -- next to a brotli column measuring only what was fetched, which is
 * how a size table stops being evidence and starts being an error. Deriving the
 * set here instead is not an option worth taking: it would mean this script
 * reimplementing an import-map-driven loader well enough to be trusted.
 *
 * So the contract is a file: `bench-artifacts.json` at the root of the
 * delivered directory, an array of directory-relative paths, in load order.
 * Without it the whole directory is measured and the section says loudly that
 * it is a directory sum.
 */
function manifestOf(dir) {
	const path = join(dir, "bench-artifacts.json");
	if (!existsSync(path)) return null;
	try {
		const parsed = JSON.parse(readFileSync(path, "utf8"));
		const list = Array.isArray(parsed) ? parsed : parsed?.artifacts;
		if (!Array.isArray(list) || list.some(entry => typeof entry !== "string")) {
			console.warn(`report.mjs: ${relative(root, path)} is not an array of paths; ignoring it`);
			return null;
		}
		return list;
	} catch (error) {
		console.warn(`report.mjs: could not read ${relative(root, path)}: ${error.message}`);
		return null;
	}
}

function sizeTable(dir, entries) {
	const rows = [];
	let rawTotal = 0;
	let gzipTotal = 0;
	for (const rel of entries) {
		const full = join(dir, rel);
		if (!existsSync(full)) {
			rows.push([`\`${rel}\``, "missing", "missing"]);
			continue;
		}
		const { raw, gzip } = sizes(readFileSync(full, "utf8"));
		rawTotal += raw;
		gzipTotal += gzip;
		rows.push([`\`${rel}\``, raw.toLocaleString("en-US"), gzip.toLocaleString("en-US")]);
	}
	rows.push(["**total**", `**${rawTotal.toLocaleString("en-US")}**`, `**${gzipTotal.toLocaleString("en-US")}**`]);
	return table(["artifact", "raw", `gzip -${GZIP_LEVEL}`], rows);
}

function breakdown(slot) {
	const dir = slot.ours.source;
	if (!existsSync(dir)) {
		return [`_Not measured: \`${relative(root, dir)}\` is not present, so this entry's artifacts could not be read._`];
	}
	const onDisk = artifactsOf(dir).map(artifact => artifact.rel);
	if (onDisk.length === 0) return [`_Not measured: \`${relative(root, dir)}\` is empty._`];

	const manifest = manifestOf(dir);
	if (manifest === null) {
		return [
			"> Measured over **every file in the delivered directory**, because it carries no",
			"> `bench-artifacts.json` saying which of them the benchmark page loads. If the entry ships",
			"> code the page never fetches -- a per-island chunk, a source map -- this total is larger",
			"> than the payload, and larger than the brotli column above for a reason that is not size.",
			"",
			sizeTable(dir, onDisk),
		];
	}

	const loaded = new Set(manifest);
	const alsoPresent = onDisk.filter(rel => !loaded.has(rel));
	const out = [`The ${manifest.length} artifacts the page loads, per \`bench-artifacts.json\`:`, "", sizeTable(dir, manifest)];
	if (alsoPresent.length > 0) {
		out.push(
			"",
			`Also in the directory and **not** counted above (${alsoPresent.length} files: ` +
				alsoPresent.slice(0, 8).map(rel => `\`${rel}\``).join(", ") +
				(alsoPresent.length > 8 ? ", ..." : "") +
				"). These are served but never fetched by the benchmark page, so they are not part of",
			"its payload -- and the harness's brotli column does not count them either.",
		);
	}
	return out;
}

// -------------------------------------------------------------- provenance

function chromeVersion() {
	try {
		return execFileSync("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", ["--version"], { encoding: "utf8" }).trim();
	} catch {
		return "unknown (could not run the Chrome binary)";
	}
}

function pin() {
	try {
		return readFileSync(join(bench, "KRAUSEST_PIN"), "utf8").trim();
	} catch {
		return "unrecorded";
	}
}

// -------------------------------------------------------------------- main

const { byFramework, files, unmatched } = readRaw();

// Bind each slot to the framework string that actually turned up, if any.
for (const slot of SLOTS) {
	slot.name = [...byFramework.keys()].find(name => slot.matches(name)) ?? null;
	slot.results = slot.name ? byFramework.get(slot.name) : new Map();
}
const present = SLOTS.filter(slot => slot.name !== null);
const absent = SLOTS.filter(slot => slot.name === null);
const baseline = SLOTS.find(slot => slot.baseline && slot.name !== null) ?? null;

const out = [];
out.push("# krausest js-framework-benchmark: mires, a Rust->JS compiler");
out.push("");

if (files === 0) {
	out.push("No results. `bench/results-raw/` is empty -- run `bench/run.sh` (or `bench/run.sh --quick`) first.");
	writeFileSync(outFile, out.join("\n") + "\n");
	console.log(`report.mjs: no results in ${relative(root, rawDir)}; wrote a placeholder RESULTS.md`);
	process.exit(0);
}

out.push(
	"Entries compiled from Rust by the `rustc_codegen_mires` backend, measured by the",
	"real upstream harness on this machine against vanillajs (keyed and non-keyed), solid and",
	"leptos. Both entries are submitted as **non-keyed**; the harness's own `isKeyed` detector",
	"gates that before anything is measured.",
	"",
);

if (absent.length > 0) {
	out.push(
		"> **Partial run.** No results for " + absent.map(slot => `\`${slot.dir}\``).join(", ") + ".",
		"> Those columns are left out below rather than shown empty.",
		"",
	);
}
if (unmatched.length > 0) {
	out.push(
		"> `results-raw/` also holds results for " + unmatched.map(name => `\`${name}\``).join(", ") +
			", which is not one of the six this report covers; ignored.",
		"",
	);
}

const columns = present.map(slot => slot.label);

// -- CPU ---------------------------------------------------------------

out.push("## Duration", "");
out.push(
	"Median of the harness's iterations, in milliseconds, with the slowdown against",
	baseline ? `\`${baseline.dir}\` in brackets.` : "no baseline available (vanillajs keyed did not run), so no slowdowns.",
	"Lower is better.",
	"",
);

const cpuRows = [];
const factorsBySlot = new Map(present.map(slot => [slot.dir, []]));
let anyCpu = false;

for (const benchmark of CPU) {
	const base = baseline ? value(baseline.results.get(benchmark.id), "total") : null;
	const cells = present.map(slot => {
		const ms = value(slot.results.get(benchmark.id), "total");
		if (ms === null) return "--";
		anyCpu = true;
		if (base === null || base === 0) return fixed(ms, 1);
		const factor = ms / base;
		factorsBySlot.get(slot.dir).push(factor);
		return `${fixed(ms, 1)} (${factor.toFixed(2)})`;
	});
	const label = benchmark.throttle ? `${benchmark.label} [^t${benchmark.throttle}]` : benchmark.label;
	cpuRows.push([label, ...cells]);
}

if (anyCpu) {
	if (baseline) {
		cpuRows.push([
			"**geometric mean of slowdowns** [^geo]",
			...present.map(slot => {
				const mean = geomean(factorsBySlot.get(slot.dir));
				const count = factorsBySlot.get(slot.dir).length;
				return mean === null ? "--" : `**${mean.toFixed(2)}** (n=${count})`;
			}),
		]);
	}
	out.push(table(["benchmark", ...columns], cpuRows), "");
	const throttles = [...new Set(CPU.filter(b => b.throttle).map(b => b.throttle))].sort();
	for (const throttle of throttles) {
		out.push(`[^t${throttle}]: run with a ${throttle}x CPU slowdown applied by the harness.`);
	}
	if (baseline) {
		out.push(
			"",
			"[^geo]: **This is not the ranking number from the published table.** Upstream's is a",
			"    weighted geometric mean over values normalised against the p90 of the entire",
			"    published population, measured on the maintainer's machine; that population is not",
			"    reproducible locally and a locally invented weighting would be worse than none. This",
			`    is the plain geometric mean of the per-benchmark slowdowns against \`${baseline.dir}\`,`,
			"    over the benchmarks that ran (`n`). Comparing it to a published geomean is comparing",
			"    two different statistics.",
		);
	}
	out.push("");
} else {
	out.push("_No CPU benchmarks in this run._", "");
}

// -- memory ------------------------------------------------------------

out.push("## Memory", "");
const memRows = [];
let anyMem = false;
for (const benchmark of MEMORY) {
	const cells = present.map(slot => {
		const mb = value(slot.results.get(benchmark.id), "DEFAULT");
		if (mb !== null) anyMem = true;
		return fixed(mb, 2);
	});
	memRows.push([benchmark.label, ...cells]);
}
out.push(anyMem ? table(["benchmark (MB)", ...columns], memRows) : "_No memory benchmarks in this run._", "");

// -- size and first paint ----------------------------------------------

out.push("## Transfer size and first paint", "");
out.push(
	"Measured by the harness, which counts every response the page pulls (excluding `/css`,",
	"which is served from the harness root and shared by every implementation) and brotli-",
	"compresses the total itself.",
	"",
);
const sizeRows = [];
let anySize = false;
for (const benchmark of SIZE) {
	const cells = present.map(slot => {
		const number = value(slot.results.get(benchmark.id), "DEFAULT");
		if (number !== null) anySize = true;
		return fixed(number, 1);
	});
	sizeRows.push([`${benchmark.label} (${benchmark.unit})`, ...cells]);
}
out.push(anySize ? table(["measure", ...columns], sizeRows) : "_No size benchmark in this run._", "");

// -- our per-artifact breakdown ----------------------------------------

out.push("## What our payload is made of", "");
out.push(
	"The table above gives one brotli number per implementation, which is the right number for",
	"comparing them and no help at all in understanding one. These are the same two entries",
	"broken down per delivered file, measured with the spike's own convention: **gzip at a pinned",
	`level ${GZIP_LEVEL}, summed per artifact** (\`bench/lib/budget.mjs\`), because that is what the`,
	"rest of the spike's budgets are recorded in.",
	"",
	"**These numbers are not comparable with the brotli column above** -- different compressor,",
	"different level, and this counts files on disk rather than responses the browser actually",
	"fetched.",
	"",
);
for (const slot of SLOTS.filter(candidate => candidate.ours)) {
	out.push(`### \`${slot.dir}\``, "", slot.ours.note, "", ...breakdown(slot), "");
}

// -- Melange -----------------------------------------------------------

out.push("## Melange comparison", "");
const melangeReport = join(bench, "melange", "out", "melange-report.md");
if (existsSync(melangeReport)) {
	out.push(readFileSync(melangeReport, "utf8").trim(), "");
} else {
	out.push(
		"_Pending._ The core-logic comparison against Melange (the same non-keyed store logic written",
		"in OCaml, compiled by the Melange playground, against the same logic in Rust compiled by this",
		"backend: sizes, node micro-benchmark, readability) writes",
		`\`${relative(root, melangeReport)}\`, which this section inlines once it exists.`,
		"",
	);
}

// -- provenance --------------------------------------------------------

out.push("## Provenance", "");
const iterations = [...cpuIterationCounts].sort((a, b) => a - b);
out.push(
	`- harness: github.com/krausest/js-framework-benchmark @ \`${pin()}\``,
	`- browser: ${chromeVersion()} (not headless, 1280x800, \`--js-flags=--expose-gc\`)`,
	`- node: ${process.version}`,
	`- generated: ${new Date().toISOString()}`,
	`- result files: ${files} in \`bench/results-raw/\`, ${present.length} of ${SLOTS.length} implementations`,
	`- CPU iterations per benchmark: ${iterations.length ? iterations.join(", ") : "none (no CPU benchmarks in this run)"}`,
	"",
);
if (iterations.some(count => count < 15)) {
	out.push(
		"> Some benchmarks ran fewer than the harness's default 15 iterations, which means this was a",
		"> `--quick` run. **Draft numbers; not the artifact.**",
		"",
	);
}
out.push(
	"Medians here are recomputed from the raw `values` arrays in `bench/results-raw/`, not read off",
	"the `median` field the harness writes beside them" +
		(disagreements.length === 0 ? "; the two agree on every value in this run." : ":"),
	"",
);
if (disagreements.length > 0) {
	out.push("> **The recomputed medians disagree with the recorded ones**, which should not happen:", "");
	for (const line of disagreements.slice(0, 20)) out.push(`> - ${line}`);
	if (disagreements.length > 20) out.push(`> - ...and ${disagreements.length - 20} more`);
	out.push("");
}

writeFileSync(outFile, out.join("\n").replace(/\n{3,}/g, "\n\n") + "\n");
console.log(`report.mjs: ${files} result files, ${present.length}/${SLOTS.length} implementations -> ${relative(root, outFile)}`);
if (absent.length > 0) console.log(`report.mjs: absent: ${absent.map(slot => slot.dir).join(", ")}`);
if (disagreements.length > 0) console.warn(`report.mjs: ${disagreements.length} median disagreements -- see the report footer`);
