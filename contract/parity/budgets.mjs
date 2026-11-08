// The chunk-size budget report.
//
//   node budgets.mjs             # judge every subject against its baseline
//   node budgets.mjs --update    # re-record the baselines that moved, with today's date
//
// The per-fixture half of this runs inside `run.mjs`, so a size regression fails
// the same run a hydration regression fails. This script is the whole picture and
// the only way to re-baseline.
//
// It reads the served bytes through `lib/delivery.mjs`, which parses them out of
// demo-app/src/dom.rs and demo-app's OUT_DIR -- the same source the parity run
// measures. So a budget is taken from the module the page serves, not from a
// build artifact that happens to be lying around.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

import { GZIP_LEVEL, islandsIn, judge, sizes } from "./lib/budget.mjs";
import { delivery } from "./lib/delivery.mjs";

const BUDGETS = join(import.meta.dirname, "budgets.json");

/** The recorded budgets. */
export function budgets() {
	return JSON.parse(readFileSync(BUDGETS, "utf8"));
}

/**
 * Every module dom.rs serves that budgets.json does not name.
 *
 * THE COVERAGE CHECK, and it is the one that makes this file hard to leave stale.
 * A budget list is only as good as its completeness: a new module can appear in
 * the delivery and every recorded subject stays green while the page gets bigger,
 * which is the exact failure a size budget exists to prevent.
 *
 * It is also how per-island chunking announces itself. The first attempt at this
 * pattern-matched route names for something chunk-shaped and immediately
 * misfired -- `/demo/island-loader.js` matched a regex meant for
 * `/demo/islands.js` -- which is the argument for asking the question from the
 * other side. "Is this route one budgets.json knows about" needs no guess about
 * what a chunk will be called, and the backend's own wave-3 design has chunks as
 * siblings under a directory whose name is a build option, so guessing would have
 * been wrong anyway.
 *
 * @param {{routes: Map<string,string>}} served
 * @param {object} recorded
 * @returns {string[]}
 */
export function unbudgetedRoutes(served, recorded) {
	const named = new Set([
		...Object.values(recorded.subjects).flatMap(subject => subject.artifacts),
		...Object.keys(recorded.$notBudgeted),
	]);
	return [...served.routes.keys()].filter(url => !named.has(url));
}

/**
 * Is the `chunking.landed` flag still true of what dom.rs serves?
 *
 * Two ways it can go stale, and they read differently: an unbudgeted module has
 * appeared (chunking, or any other new module -- see `unbudgetedRoutes`), or the
 * single island module the flag describes is no longer served at all.
 *
 * @param {{routes: Map<string,string>}} served
 * @param {object} recorded
 * @returns {{agrees: boolean, why: string}}
 */
export function chunkingState(served, recorded) {
	const unbudgeted = unbudgetedRoutes(served, recorded);
	const islandArtifact = recorded.chunking.islandArtifact;
	if (!recorded.chunking.landed && !served.routes.has(islandArtifact)) {
		return {
			agrees: false,
			why: `budgets.json says chunking has not landed and names ${islandArtifact} as the one island module, but dom.rs does not serve it. Find where the island's client half is served now.`,
		};
	}
	if (unbudgeted.length) {
		return {
			agrees: false,
			why: `dom.rs serves ${unbudgeted.length} module(s) budgets.json does not name: ${unbudgeted.join(", ")}.\n`
				+ "       If these are per-island chunks, chunking has landed: set chunking.landed, point each island subject at its own\n"
				+ "       chunk plus the shared chunk, add a subject for the shared chunk, and re-baseline. Otherwise add them as subjects\n"
				+ "       or, if a browser never fetches them, to $notBudgeted with the reason.",
		};
	}
	return {
		agrees: true,
		why: recorded.chunking.landed
			? "chunked, and every module the page serves is budgeted"
			: `one island module (${islandArtifact}), and every module the page serves is budgeted`,
	};
}

/**
 * Judge every subject.
 *
 * @returns {{name: string, verdict: ReturnType<typeof judge>}[]}
 */
export function judgeAll(served, recorded) {
	return Object.entries(recorded.subjects).map(([name, subject]) => ({ name, verdict: judge(served, subject, recorded.policy) }));
}

const MARK = { ok: "ok     ", over: "OVER   ", under: "UNDER  ", unmeasured: "NEW    ", pending: "pending", composition: "MOVED  " };

/**
 * The budget checks a parity run adds for one fixture.
 *
 * Three of them, and each is here for its own reason:
 *
 * - the fixture's OWN island subject, which is the number that moves when the
 *   emitter's output for this island moves;
 * - the `runtime` subject, checked on every fixture rather than once, because it
 *   is the largest thing the page delivers and every fixture loads it. A number
 *   that big is worth re-asserting wherever it is actually being depended on;
 * - the COVERAGE check, which is the one that must not be skippable. A new module
 *   in the delivery leaves every recorded subject green while the page gets
 *   bigger, so it belongs in the run and not only in the standalone report.
 *
 * A subject with no baseline yet FAILS rather than passing quietly. That is the
 * point of separating "unmeasured" from "ok" in `judge`: the first green run after
 * an island lands is supposed to stop and make someone record the number.
 *
 * @param {string} name the fixture name
 * @param {{routes: Map<string,string>}} served
 * @param {import("./lib/check.mjs").Checks} checks
 */
export function budgetChecks(name, served, checks) {
	const recorded = budgets();

	const coverage = chunkingState(served, recorded);
	checks.ok("every module the page delivers has a size budget", coverage.agrees, coverage.why);

	for (const subject of [`island:${name}`, "runtime"]) {
		if (!recorded.subjects[subject]) {
			checks.ok(`budgets.json has a subject for ${subject}`, false, `add it, or the fixture's chunk size is unwatched`);
			continue;
		}
		const verdict = judge(served, recorded.subjects[subject], recorded.policy);
		if (verdict.state === "pending") {
			checks.note(`BUDGET ${subject}: pending -- ${verdict.why}`);
			continue;
		}
		checks.ok(`${subject} is inside its gzip budget, and measures the islands its baseline was taken on`, verdict.state === "ok", verdict.why);
		if (verdict.state === "ok") checks.note(`BUDGET ${subject}: ${verdict.why}`);
	}
}

function main() {
	const update = process.argv.includes("--update");
	const served = delivery();
	const recorded = budgets();

	if (recorded.gzip.level !== GZIP_LEVEL) {
		console.log(`FAIL   budgets.json records gzip level ${recorded.gzip.level} but lib/budget.mjs compresses at ${GZIP_LEVEL}: every baseline here was taken at a level this code no longer uses`);
		process.exit(1);
	}
	console.log(`gzip level ${GZIP_LEVEL}, node ${process.version}, zlib ${process.versions.zlib}`);
	if (recorded.gzip.measuredOn.node !== process.version || recorded.gzip.measuredOn.zlib !== process.versions.zlib) {
		console.log(`note   baselines were taken on node ${recorded.gzip.measuredOn.node} / zlib ${recorded.gzip.measuredOn.zlib}.`);
		console.log("       If several subjects moved by a few bytes at once, that is this, not a regression.");
	}
	console.log("");

	const chunking = chunkingState(served, recorded);
	if (!chunking.agrees) {
		console.log(`FAIL   chunking: ${chunking.why}`);
		console.log("");
	} else {
		console.log(`ok     chunking: ${chunking.why}`);
		console.log("");
	}

	const verdicts = judgeAll(served, recorded);
	for (const { name, verdict } of verdicts) {
		console.log(`${MARK[verdict.state]} ${name.padEnd(16)} ${verdict.why}`);
		if (verdict.measured && verdict.measured.parts.length > 1) {
			for (const part of verdict.measured.parts) console.log(`                        ${part.url} ${part.gzip} gzip / ${part.raw} raw`);
		}
	}

	// Information, never enforced. See budgets.json `$notBudgeted`.
	console.log("");
	for (const url of Object.keys(recorded.$notBudgeted)) {
		const source = served.routes.get(url);
		if (source === undefined) continue;
		const size = sizes(source);
		console.log(`note   ${url} ${size.gzip} gzip / ${size.raw} raw (not budgeted)`);
	}

	if (update) {
		const next = JSON.parse(readFileSync(BUDGETS, "utf8"));
		const today = new Date().toISOString().slice(0, 10);
		const moved = [];
		for (const { name, verdict } of verdicts) {
			if (verdict.state === "ok" || verdict.state === "pending") continue;
			const subject = next.subjects[name];
			// A subject that had a baseline keeps its history in the open: the note
			// says what it was and when, because the useful question about a moved
			// budget is always "moved from what".
			const from = typeof subject.gzip === "number" ? `${subject.gzip} gzip / ${subject.raw} raw, ${subject.measured}` : null;
			subject.gzip = verdict.measured.gzip;
			subject.raw = verdict.measured.raw;
			subject.measured = today;
			// The composition is re-recorded with the size, never separately: the whole
			// point is that the two are one fact.
			if (subject.containsIslands) subject.containsIslands = islandsIn(served, subject);
			delete subject.pending;
			subject.$rebaselined = from ? `was ${from}` : `first measurement, ${today}`;
			moved.push(`${name} -> ${verdict.measured.gzip} gzip / ${verdict.measured.raw} raw`);
		}
		if (!moved.length) {
			console.log("");
			console.log("nothing to update: every subject is inside its band");
			return;
		}
		next.gzip.measuredOn.node = process.version;
		next.gzip.measuredOn.zlib = process.versions.zlib;
		writeFileSync(BUDGETS, `${JSON.stringify(next, null, 2)}\n`);
		console.log("");
		console.log(`re-baselined ${moved.length} subject(s) as of ${today}:`);
		for (const line of moved) console.log(`  ${line}`);
		console.log("Read the diff before keeping it: a budget that moves without a reason you can name is the regression.");
		return;
	}

	const bad = verdicts.filter(({ verdict }) => verdict.state !== "ok" && verdict.state !== "pending");
	if (bad.length || !chunking.agrees) {
		console.log("");
		if (bad.length) console.log(`${bad.length} subject(s) out of budget or unmeasured: ${bad.map(entry => entry.name).join(", ")}`);
		console.log("re-run with --update once you can name the cause");
		process.exit(1);
	}
	console.log("");
	console.log("every subject is inside its band");
}

if (import.meta.filename === process.argv[1]) {
	main();
}
