// The smallest ES module shim a compiled program can be run against.
//
//   node min-shim.mjs <program.js> <out/shim.js>
//
// WHY NOT scripts/make-esm-shim.mjs
// ---------------------------------
// That script writes the WHOLE of runtime/shim.js plus an export clause -- 61 KB
// of it -- because it exists to run the test suites, where any member may be
// reached. For a size comparison that number is meaningless: the store below
// touches 23 members, and shipping the other sixty would be measuring the
// spike's test infrastructure rather than the program.
//
// So this walks the other way. It reads the `__rt.<name>` references out of the
// compiled program, closes them over the references those members make to each
// other, and writes only the statements that install them. What comes out is
// what a build tool would have to publish beside the program, and it is what
// the size table counts as our runtime.
//
// The idea is the one scripts/make-esm-shim.mjs and mires-build's shimcheck
// already use -- the member set is READ, never maintained by hand -- with the
// direction reversed: shimcheck asks "does the shim have everything the program
// wants" and this asks "what is the least of the shim that answers that". Both
// FAIL on a member that is referenced and absent, which is the property that
// matters: a silently missing member is a `TypeError` at run time.
//
// HOW THE SHIM IS SLICED
// ----------------------
// runtime/shim.js is a plain script with a rigid top level shape: one object
// literal assigned to `globalThis.__rt`, then one `globalThis.__rt.<name> = ...;`
// statement per member, each preceded by its documentation as column-zero `//`
// comments. So a statement is "from its first line to the last line before the
// next one that is neither blank nor a column-zero comment", and the six members
// of the opening literal are lifted out of it into the same assignment form.
//
// The documentation is NOT carried over. That is deliberate and it is stated in
// the report: the comments are for a reader of the spike, not for a browser, and
// counting them would inflate our side of the table with prose. Nothing else is
// changed -- the member bodies are the file's own text, byte for byte.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const shimPath = path.resolve(here, "..", "..", "runtime", "shim.js");

/**
 * Every member the shim installs, as the statement that installs it.
 *
 * @param {string} source
 * @returns {Map<string, string>}
 */
export function shimMembers(source) {
	const lines = source.split("\n");
	const members = new Map();

	// The opening object literal: `globalThis.__rt = {` ... `};`, whose entries
	// are at two spaces of indent. Each becomes the assignment form the rest of
	// the file already uses, so that the output is one uniform kind of statement
	// and a member can be picked without dragging its five neighbours along.
	const open = lines.findIndex(line => line.startsWith("globalThis.__rt = {"));
	if (open === -1) throw new Error("runtime/shim.js does not open with `globalThis.__rt = {`");
	let close = open;
	while (close < lines.length && lines[close] !== "};") close++;
	if (close === lines.length) throw new Error("runtime/shim.js: the opening literal is not closed by `};`");

	let entry = null;
	const flush = () => {
		if (entry === null) return;
		// Drop the trailing comma the literal needs and the assignment does not.
		const body = entry.body.join("\n").replace(/,\s*$/, "");
		members.set(entry.name, `globalThis.__rt.${entry.name} = ${body};`);
		entry = null;
	};
	for (let i = open + 1; i < close; i++) {
		const line = lines[i];
		const start = /^ {2}([A-Za-z_$][A-Za-z0-9_$]*):\s*(.*)$/.exec(line);
		if (start) {
			flush();
			entry = { name: start[1], body: [start[2]] };
		} else if (entry !== null && !/^ *\/\//.test(line)) {
			entry.body.push(line);
		}
	}
	flush();

	// Then one statement per member, each running to the line before the next
	// statement's documentation begins.
	const starts = [];
	for (let i = close + 1; i < lines.length; i++) {
		const match = /^globalThis\.__rt\.([A-Za-z_$][A-Za-z0-9_$]*)\s*=/.exec(lines[i]);
		if (match) starts.push({ line: i, name: match[1] });
	}
	for (let s = 0; s < starts.length; s++) {
		const from = starts[s].line;
		let to = (s + 1 < starts.length ? starts[s + 1].line : lines.length) - 1;
		while (to > from && (lines[to].trim() === "" || lines[to].startsWith("//"))) to--;
		members.set(starts[s].name, lines.slice(from, to + 1).join("\n"));
	}

	return members;
}

/**
 * Every `__rt.<name>` the text mentions.
 *
 * `globalThis.__rt.x` matches too, which is how a member's references to other
 * members are found: the shim spells them that way throughout.
 *
 * @param {string} source
 * @returns {Set<string>}
 */
export function referencedMembers(source) {
	const found = new Set();
	for (const match of source.matchAll(/__rt\.([A-Za-z_$][A-Za-z0-9_$]*)/g)) found.add(match[1]);
	return found;
}

/**
 * The transitive closure of what `program` needs, and the module text for it.
 *
 * @param {string} program the compiled JavaScript
 * @param {string} shimSource runtime/shim.js
 * @returns {{names: string[], source: string}}
 */
export function minimalShim(program, shimSource) {
	const members = shimMembers(shimSource);

	const needed = new Set();
	const queue = [...referencedMembers(program)];
	const missing = [];
	while (queue.length > 0) {
		const name = queue.pop();
		if (needed.has(name)) continue;
		const statement = members.get(name);
		if (statement === undefined) {
			missing.push(name);
			continue;
		}
		needed.add(name);
		for (const next of referencedMembers(statement)) {
			if (!needed.has(next)) queue.push(next);
		}
	}

	if (missing.length > 0) {
		// Loudly, and before anything is written: a member the program calls and
		// the shim does not have is a `TypeError` on the first call, at which
		// point the reason is a long way from the cause.
		throw new Error(
			`the program references ${missing.sort().join(", ")}, which runtime/shim.js does not install`,
		);
	}

	// In the file's own order, so the output reads like the source it came from
	// and a member that another one uses at load time is installed first.
	const ordered = [...members.keys()].filter(name => needed.has(name));

	const source = [
		"// The subset of runtime/shim.js that this program actually reaches, as an ES",
		"// module. Generated by bench/melange/min-shim.mjs; every member body below is",
		"// runtime/shim.js's own text. See that script for why the documentation is not",
		"// carried over and why this is the number the size table counts.",
		"globalThis.__rt = globalThis.__rt || {};",
		...ordered.map(name => members.get(name)),
		`export const {\n${ordered.map(name => `  ${name},`).join("\n")}\n} = globalThis.__rt;`,
		"",
	].join("\n");

	return { names: ordered, source };
}

// ----------------------------------------------------------------------- cli

if (import.meta.url === `file://${process.argv[1]}`) {
	const [program, out] = process.argv.slice(2);
	if (!program || !out) {
		console.error("usage: min-shim.mjs <program.js> <out/shim.js>");
		process.exit(2);
	}
	const { names, source } = minimalShim(readFileSync(program, "utf8"), readFileSync(shimPath, "utf8"));
	mkdirSync(path.dirname(out), { recursive: true });
	writeFileSync(out, source);
	console.log(`${names.length} members, ${Buffer.byteLength(source, "utf8")} bytes -> ${out}`);
	console.log(names.join(" "));
}
