// Chunk-size budgets for the modules the island page delivers.
//
// WHY THIS IS A BAND AND NOT A CEILING
// ------------------------------------
// The spike's own budgets (`examples/*/NN.maxbytes`, enforced by
// `scripts/test.sh:270`) are one-sided ceilings, and for what they watch that is
// right: they exist to catch a lost dead-code elimination, which only ever makes
// the output bigger.
//
// What ships to a browser needs the other side too. An island chunk that
// SHRANK unexpectedly is not good news -- it means something that was in it is
// no longer in it, and the two ways that happens are a lowering that silently
// stopped emitting and a dead-code pass that reached something it should not
// have. Both produce a smaller file and a broken island, and a ceiling reports
// them as a pass. So a subject is in budget when its size is inside a band
// around a recorded baseline, and a move in EITHER direction is a result you
// have to look at and then re-record.
//
// WHY GZIP
// --------
// Raw bytes are what the ceilings in `scripts/test.sh` watch because they are
// measuring the emitter's output as an artifact. These budgets are measuring
// what a user downloads, and no server sends this uncompressed. The two numbers
// also move differently: the emitter's repetitive shim boilerplate compresses
// far better than a template string does, so a change that adds 400 raw bytes of
// generated preamble and one that adds 400 raw bytes of markup are the same
// regression to a raw ceiling and very different to a user. Raw is recorded
// alongside, because when a subject moves it is the first thing worth reading.
//
// THE COMPRESSION LEVEL IS PINNED, and at 9 rather than at a realistic 6. A
// realistic level is the wrong target: the point of the number is that a change
// in it means a change in the CONTENT, not a change in how hard the compressor
// tried or in which zlib the runner shipped. 9 is the reproducible extreme.
// (Measured on this environment: at level 6 `topcoat-dom.js` is 9105 and at 9 it
// is 9106 -- deflate is not monotone in effort, which is exactly why the level
// has to be written down rather than left to a default.)

import { gzipSync } from "node:zlib";

/** The pinned compression level. See the header: a pinned extreme, not a realistic one. */
export const GZIP_LEVEL = 9;

/**
 * The two sizes of a module source.
 *
 * @param {string} source
 * @returns {{raw: number, gzip: number}}
 */
export function sizes(source) {
	const bytes = Buffer.from(source, "utf8");
	return { raw: bytes.length, gzip: gzipSync(bytes, { level: GZIP_LEVEL }).length };
}

/**
 * The tolerance band around a baseline.
 *
 * The slack is proportional with an absolute floor, because a proportional band
 * alone is unusable at both ends of the range this harness covers: 5% of the
 * 307-byte loader is 15 bytes, which one reworded comment in dom.rs would break,
 * while 5% of the 9 KB runtime is 456, which is the right order. So the floor
 * carries the small subjects and the proportion carries the large ones.
 *
 * @param {number} baseline
 * @param {{tolerance: number, floorBytes: number}} policy
 * @returns {{low: number, high: number, slack: number}}
 */
export function band(baseline, policy) {
	const slack = Math.max(Math.ceil(policy.tolerance * baseline), policy.floorBytes);
	return { low: baseline - slack, high: baseline + slack, slack };
}

/**
 * Measure a subject: the sum of the artifacts it is made of.
 *
 * A subject is a sum rather than a single file because that is what a chunked
 * delivery looks like from the browser's side. Once the backend's per-island
 * chunking lands, an island's subject is its own chunk PLUS the shared chunk it
 * imports, and the thing worth budgeting is what the page actually pulls down.
 * Written that way now so the shape does not have to change then.
 *
 * @param {{routes: Map<string,string>}} served
 * @param {{artifacts: string[]}} subject
 * @returns {{raw: number, gzip: number, parts: {url: string, raw: number, gzip: number}[]}}
 */
export function measure(served, subject) {
	const parts = subject.artifacts.map(url => {
		const source = served.routes.get(url);
		if (source === undefined) {
			throw new Error(`budgets.json names ${url} as an artifact, but dom.rs does not serve it. Either the route moved or the subject is stale.`);
		}
		return { url, ...sizes(source) };
	});
	// Summed per artifact, NOT gzipped as one concatenated stream: the browser
	// fetches and decompresses each module separately, so its cost is the sum of
	// the parts. Concatenating first would credit the delivery with cross-file
	// redundancy no transport ever exploits.
	return {
		raw: parts.reduce((total, part) => total + part.raw, 0),
		gzip: parts.reduce((total, part) => total + part.gzip, 0),
		parts,
	};
}

/**
 * The islands a subject's artifacts actually contain.
 *
 * Read off the compiled module's entry points, because a size baseline is only
 * meaningful alongside what it was measured on -- and before per-island chunking
 * those two things come apart badly. `/demo/islands.js` holds EVERY island, so a
 * subject named `island:counter` pointed at it measures the counter plus whatever
 * else has landed since, and a bare number cannot say which.
 *
 * This was not a hypothetical worry. It happened during this wave: a second island
 * landed in demo-app's bundle and `island:counter` jumped 969 -> 1422 gzip with
 * nothing about the counter having changed. Re-baselining to 1422 would have
 * recorded "the counter's chunk is 1422 bytes", which is false, and the budget would
 * have gone on looking green while measuring something else entirely.
 *
 * So a baseline records its COMPOSITION, and a change in composition is its own
 * failure with its own message. That turns the shared bundle's non-independence from
 * an unexplained size jump into a self-reporting fact.
 *
 * @param {{routes: Map<string,string>}} served
 * @param {{artifacts: string[]}} subject
 * @returns {string[]} island names, sorted
 */
export function islandsIn(served, subject) {
	const found = new Set();
	for (const url of subject.artifacts) {
		const source = served.routes.get(url) ?? "";
		for (const match of source.matchAll(/^function __island_([\w$]+)\(/gm)) found.add(match[1]);
	}
	return [...found].sort();
}

/**
 * Judge one subject against its recorded baseline.
 *
 * Three outcomes rather than two, because "no baseline yet" is a real state and
 * not a pass: a subject whose island does not exist yet, or one added since the
 * last baselining run, has nothing to compare against. It reports the
 * measurement and what to run, and it is NOT counted as in-budget.
 *
 * @returns {{state: "ok" | "over" | "under" | "unmeasured" | "pending", measured: {raw: number, gzip: number, parts: object[]} | null, band: {low: number, high: number, slack: number} | null, why: string}}
 */
export function judge(served, subject, policy) {
	if (subject.pending) {
		return { state: "pending", measured: null, band: null, why: subject.pending };
	}
	const measured = measure(served, subject);

	// Composition BEFORE size, because a size comparison across a changed
	// composition is not a comparison at all -- it would report a number moving and
	// invite someone to look for a regression in the wrong island.
	if (subject.containsIslands) {
		const actual = islandsIn(served, subject);
		if (JSON.stringify(actual) !== JSON.stringify(subject.containsIslands)) {
			const appeared = actual.filter(island => !subject.containsIslands.includes(island));
			const gone = subject.containsIslands.filter(island => !actual.includes(island));
			return {
				state: "composition",
				measured,
				band: null,
				why: `this subject's artifacts now contain ${JSON.stringify(actual)}, not the ${JSON.stringify(subject.containsIslands)} its ${subject.measured} baseline was measured on`
					+ `${appeared.length ? `. Appeared: ${appeared.join(", ")}` : ""}${gone.length ? `. Gone: ${gone.join(", ")}` : ""}.`
					+ ` The size moved ${subject.gzip} -> ${measured.gzip} gzip, but that is not a regression in ${subject.artifacts.join(", ")} -- it is a different set of islands.`
					+ " Before per-island chunking every island subject IS the whole bundle, so this fires for every one of them at once; that non-independence is the argument for chunking.",
			};
		}
	}

	if (typeof subject.gzip !== "number") {
		return {
			state: "unmeasured",
			measured,
			band: null,
			why: `no baseline recorded. Measured ${measured.gzip} gzip / ${measured.raw} raw; run \`node budgets.mjs --update\` to record it.`,
		};
	}
	const window = band(subject.gzip, policy);
	if (measured.gzip > window.high) {
		return {
			state: "over",
			measured,
			band: window,
			why: `${measured.gzip} gzip is ${measured.gzip - subject.gzip} over the ${subject.gzip} baseline of ${subject.measured}, past the +${window.slack} band. Raw went ${subject.raw} -> ${measured.raw}.`,
		};
	}
	if (measured.gzip < window.low) {
		return {
			state: "under",
			measured,
			band: window,
			why: `${measured.gzip} gzip is ${subject.gzip - measured.gzip} UNDER the ${subject.gzip} baseline of ${subject.measured}, past the -${window.slack} band. A chunk that shrank is not automatically good news: something that was in it is not in it any more. Raw went ${subject.raw} -> ${measured.raw}.`,
		};
	}
	return {
		state: "ok",
		measured,
		band: window,
		why: `${measured.gzip} gzip within [${window.low}, ${window.high}] of the ${subject.gzip} baseline of ${subject.measured}`,
	};
}
