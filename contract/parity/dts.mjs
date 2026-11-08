// The `.d.ts` goldens: what the backend says an island's compiled module looks
// like to a TypeScript consumer, recorded and then held still.
//
//   node dts.mjs            RECORD: copy the emitted .d.ts in, rewrite the
//                           structural summary, scaffold a consumer per island,
//                           then check
//   node dts.mjs --check    CHECK:  fail if any of that would change
//
// Same two-mode shape as capture-ssr.mjs, and for the same reason: the recorded
// input comes from demo-app's own build, so "it changed" is the report.
//
// WHAT THIS CHECKS, IN TWO LAYERS
// -------------------------------
// 1. STRUCTURAL. `dts/structure.json` is the declared API reduced to names and
//    kinds and parameter types. A diff there says what changed about the surface,
//    which a diff of the .d.ts text does not: a reordered declaration or a
//    renamed type parameter moves every byte after it and means nothing.
//    The verbatim copies sit next to it so the bytes are diffable too, but the
//    summary is the thing a reader reads first.
//
// 2. TYPE. `tsc --noEmit` over the .d.ts files together with one tiny consuming
//    .ts file per island. This is the layer that catches what a golden cannot:
//    a .d.ts can be byte-stable and still be nonsense. It can name a type it
//    never declares, export a function whose parameter type does not exist, or
//    declare a module twice. None of that shows up as drift, because none of it
//    changes once it is wrong. Only a type checker sees it.
//
//    The consumer files exist because a .d.ts that type-checks alone proves less
//    than it looks. Declarations are not used by checking them; they are used by
//    importing them. A consumer is the smallest program that does that.
//
// WHY tsc AND NOT A HAND-ROLLED CHECK
// -----------------------------------
// The alternative was a structural diff alone, which is what this does when the
// pinned typescript is not fetched. It is strictly weaker and it is not close:
// every failure in layer 2 above passes a structural diff. typescript 5.9.3 is
// pinned in contract/upstream.lock like every other upstream artifact -- one
// tarball, one sha512, zero dependencies -- and it is spawned as a program, never
// imported into the harness's own module graph. See the lock's notes for why 5
// and not 7, and for why it is deliberately not in vendor/package.json.
//
// WHY THIS IS SELF-ARMING
// -----------------------
// The backend's .d.ts emission is landing while this is being written, so the
// step has to be wired before there is anything to wire it to. It handles that
// without the failure mode of a step that passes forever:
//
//   no emission and no goldens  -> PENDING, exit 0. Nothing has been recorded, so
//                                  there is nothing to be wrong about.
//   emission, no goldens        -> records them. From here on the goldens exist.
//   goldens, no emission        -> FAILS. Something that was emitted stopped being
//                                  emitted, which is the report.
//
// So the day the backend emits its first .d.ts, a record run arms the check, and
// no further edit here is needed to make absence a failure.

import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";

import { SPIKE, outDirFilesBySuffix } from "./lib/delivery.mjs";
import { Checks } from "./lib/check.mjs";

const check = process.argv.includes("--check");

const DTS = join(import.meta.dirname, "dts");
const EMITTED = join(DTS, "emitted");
const CONSUMERS = join(DTS, "consumers");
const STRUCTURE = join(DTS, "structure.json");

/** The pinned type checker, or null when vendor/upstream has not been fetched. */
const TYPESCRIPT = join(SPIKE, "contract", "vendor", "upstream", "typescript");
const TSC = join(TYPESCRIPT, "bin", "tsc");
const TS_API = join(TYPESCRIPT, "lib", "typescript.js");

/** Is the pinned type checker fetched? */
export function typescriptAvailable() {
	return existsSync(TSC);
}

/** Where `scripts/dts-test.sh` writes what the backend emitted. */
const SUITE = join(SPIKE, "build", "dtstest");

/**
 * Declaration files from the backend's own `.d.ts` suite.
 *
 * A build directory rather than a committed one, so it may be absent; that is
 * the PENDING case and not an error. Read as a peer of demo-app's OUT_DIR
 * because the two are the same kind of thing, an emission, and the goldens
 * should not care which suite produced one.
 *
 * @returns {{dir: string | null, files: {name: string, path: string, source: string}[]}}
 */
function suiteFiles(suffix) {
	let names;
	try {
		names = readdirSync(SUITE).sort();
	} catch {
		return { dir: null, files: [] };
	}
	const files = [];
	for (const name of names) {
		if (!name.endsWith(suffix)) continue;
		const path = join(SUITE, name);
		try {
			files.push({ name, path, source: readFileSync(path, "utf8") });
		} catch {
			continue;
		}
	}
	return { dir: files.length ? SUITE : null, files };
}

/**
 * Run `tsc --noEmit` over an explicit file list, rooted at `dir`.
 *
 * Split out from the step so `test.mjs` can point it at a deliberately broken
 * declaration file and assert that it fails. A type check nobody has ever seen
 * fail is not evidence that it can.
 *
 * `skipLibCheck` is OFF on purpose: the declaration files ARE the subject, and
 * skipping their check is skipping the check. `types: []` keeps @types packages
 * that happen to be installed nearby out of the program, so the result depends
 * on the emission and the pin and nothing else.
 *
 * @returns {{ok: boolean, output: string}}
 */
export function typeCheckFiles(dir, files) {
	const config = {
		compilerOptions: {
			noEmit: true,
			strict: true,
			skipLibCheck: false,
			target: "ES2022",
			module: "ESNext",
			moduleResolution: "Bundler",
			lib: ["ES2022", "DOM"],
			types: [],
			forceConsistentCasingInFileNames: true
		},
		files: files.map(path => relative(dir, path))
	};
	const configPath = join(dir, "tsconfig.check.json");
	writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
	try {
		execFileSync(process.execPath, [TSC, "-p", configPath], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
		return { ok: true, output: "" };
	} catch (error) {
		return { ok: false, output: `${error.stdout ?? ""}${error.stderr ?? ""}`.trim() || `tsc exited ${error.status}` };
	} finally {
		rmSync(configPath, { force: true });
	}
}

/** The island a .d.ts belongs to, which is its name without the extension. */
function islandOf(name) {
	return name.replace(/\.d\.ts$/, "");
}

/**
 * The declared surface of one .d.ts, reduced to what a consumer can see.
 *
 * Uses typescript's own parser where it is available, because a regex over
 * declaration text is a parser that is wrong about generics and overloads. The
 * degraded path below exists so a missing vendor tree costs coverage rather than
 * the whole step, and it says so in the summary rather than pretending.
 *
 * @returns {{parser: "typescript" | "degraded", exports: object[]}}
 */
export function structureOf(name, source) {
	if (!existsSync(TS_API)) return { parser: "degraded", exports: degradedExports(source) };

	const ts = createRequire(import.meta.url)(TS_API);
	const file = ts.createSourceFile(name, source, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
	const exports = [];

	const text = node => source.slice(node.pos, node.end).trim();
	const modifiers = node => (node.modifiers ?? []).map(m => ts.tokenToString(m.kind)).sort();

	const visit = node => {
		const kind = ts.SyntaxKind[node.kind];
		if (ts.isFunctionDeclaration(node) || ts.isMethodSignature(node)) {
			exports.push({
				kind: "function",
				name: node.name?.getText(file) ?? "(anonymous)",
				typeParameters: (node.typeParameters ?? []).map(p => p.getText(file)),
				parameters: (node.parameters ?? []).map(p => ({
					name: p.name.getText(file),
					optional: !!p.questionToken,
					type: p.type ? p.type.getText(file) : "any"
				})),
				returns: node.type ? node.type.getText(file) : "any",
				modifiers: modifiers(node)
			});
		} else if (ts.isInterfaceDeclaration(node) || ts.isClassDeclaration(node)) {
			exports.push({
				kind: ts.isInterfaceDeclaration(node) ? "interface" : "class",
				name: node.name?.getText(file) ?? "(anonymous)",
				typeParameters: (node.typeParameters ?? []).map(p => p.getText(file)),
				members: (node.members ?? []).map(m => ({
					name: m.name ? m.name.getText(file) : `(${ts.SyntaxKind[m.kind]})`,
					type: m.type ? m.type.getText(file) : null,
					optional: !!m.questionToken
				})),
				modifiers: modifiers(node)
			});
		} else if (ts.isTypeAliasDeclaration(node)) {
			exports.push({
				kind: "type",
				name: node.name.getText(file),
				typeParameters: (node.typeParameters ?? []).map(p => p.getText(file)),
				aliases: node.type.getText(file),
				modifiers: modifiers(node)
			});
		} else if (ts.isVariableStatement(node)) {
			for (const declaration of node.declarationList.declarations) {
				exports.push({
					kind: "variable",
					name: declaration.name.getText(file),
					type: declaration.type ? declaration.type.getText(file) : "any",
					modifiers: modifiers(node)
				});
			}
		} else if (ts.isModuleDeclaration(node)) {
			// A nested `declare module "chart.js" { .. }` declares somebody ELSE's
			// module. Its members are recorded, because a change to them is a change
			// to the emission, but they are marked with the module they belong to so
			// the scaffold does not try to import them from THIS file. Getting that
			// wrong is what the type-check layer caught the first time it ran over a
			// real emission: the scaffold bound `ext$Chart$..` as an export of
			// 01_shapes and tsc correctly said no such export exists.
			const inner = node.name.getText(file);
			exports.push({ kind: "module", name: inner });
			if (node.body && ts.isModuleBlock(node.body)) {
				const before = exports.length;
				node.body.statements.forEach(visit);
				for (let at = before; at < exports.length; at++) exports[at].module = inner;
			}
			return;
		} else if (ts.isExportDeclaration(node) || ts.isExportAssignment(node)) {
			exports.push({ kind: "export", text: text(node) });
		} else if (ts.isImportDeclaration(node)) {
			exports.push({ kind: "import", from: node.moduleSpecifier.getText(file), text: text(node) });
		} else if (kind !== "EndOfFileToken") {
			exports.push({ kind: `unhandled:${kind}`, text: text(node).slice(0, 200) });
		}
	};

	file.statements.forEach(visit);
	// Sorted, so a reordered emission is not a diff. Order in a .d.ts carries no
	// meaning to a consumer; the set of declarations does.
	exports.sort((a, b) => `${a.kind} ${a.name ?? a.text ?? ""}`.localeCompare(`${b.kind} ${b.name ?? b.text ?? ""}`));
	return { parser: "typescript", exports };
}

/** Names a `declare`/`export` line gives away, when there is no parser to ask. */
function degradedExports(source) {
	const found = [];
	const pattern = /^\s*(?:export\s+)?declare\s+(function|const|let|var|class|interface|type|namespace|module)\s+([A-Za-z_$][\w$]*)/gm;
	for (const [, kind, name] of source.matchAll(pattern)) found.push({ kind, name });
	found.sort((a, b) => `${a.kind} ${a.name}`.localeCompare(`${b.kind} ${b.name}`));
	return found;
}

/**
 * The smallest program that USES a declaration file.
 *
 * Scaffolded rather than hand written, because the islands are not known until
 * they are emitted. A scaffold is deliberately weak: it imports the module and
 * pins each export to a local of its own declared type, which proves the file
 * resolves, parses, and names types that exist. It proves nothing about whether
 * those types are the RIGHT ones.
 *
 * The header says so. A scaffold that has been reviewed and sharpened gets its
 * marker line removed by hand, and `dts.mjs` never rewrites a consumer whose
 * marker is gone -- so the generated ones stay generated and the reviewed ones
 * stay reviewed.
 */
export const SCAFFOLD_MARKER = "// SCAFFOLD: generated by dts.mjs and not yet reviewed.";

export function scaffold(island, structure) {
	// `module` marks a member of a nested `declare module ".."`, which belongs to
	// somebody else's module and is not importable from this one. `exported` keeps
	// out declarations the file makes without exporting: a `type` alias with no
	// `export` is part of the file's vocabulary, not its interface.
	const own = structure.exports.filter(entry => !entry.module && entry.modifiers?.includes("export"));
	const named = own
		.filter(entry => (entry.kind === "function" || entry.kind === "variable" || entry.kind === "class") && entry.name)
		.map(entry => entry.name);
	const types = own
		.filter(entry => (entry.kind === "interface" || entry.kind === "type") && entry.name)
		.map(entry => entry.name);

	const lines = [
		SCAFFOLD_MARKER,
		`// Delete the line above once this file asserts something about ${island}'s API`,
		"// beyond the fact that it resolves. See the header of dts.mjs.",
		"",
		`import type * as Island from "../emitted/${island}";`,
		""
	];
	if (!named.length && !types.length) {
		lines.push(`// ${island}.d.ts declares no value or type export this scaffold can bind.`);
		lines.push("declare const _unused: Island;");
		lines.push("void _unused;");
	}
	for (const name of named) {
		lines.push(`declare const _value_${name}: typeof Island.${name};`);
		lines.push(`void _value_${name};`);
	}
	for (const name of types) {
		lines.push(`declare const _type_${name}: Island.${name};`);
		lines.push(`void _type_${name};`);
	}
	lines.push("");
	return lines.join("\n");
}

/** `tsc --noEmit` over the recorded .d.ts files and every consumer. */
function typeCheck(checks) {
	if (!typescriptAvailable()) {
		checks.note("typescript is not fetched (contract/vendor/upstream/typescript); ran the structural diff only, which every layer-2 failure passes. Run contract/vendor/fetch.sh.");
		return;
	}

	const sources = [
		...readdirSync(EMITTED).filter(name => name.endsWith(".d.ts")).map(name => join(EMITTED, name)),
		...(existsSync(CONSUMERS) ? readdirSync(CONSUMERS).filter(name => name.endsWith(".ts")).map(name => join(CONSUMERS, name)) : [])
	];
	if (!sources.length) return;

	const { ok, output } = typeCheckFiles(DTS, sources);
	checks.ok(`tsc --noEmit over ${sources.length} file(s)`, ok, output);
}

function main() {
	const checks = new Checks(`.d.ts goldens (${check ? "check" : "record"})`);

	mkdirSync(EMITTED, { recursive: true });
	mkdirSync(CONSUMERS, { recursive: true });

	// TWO SOURCES, because the backend emits `.d.ts` from a suite of its own rather
	// than from demo-app's build. `build/dtstest/` is where `scripts/dts-test.sh`
	// writes what `-Cllvm-args=js-dts=on` produced; demo-app's OUT_DIR is where an
	// island's would appear if the island build ever turned the flag on. Both are
	// read so this step covers the emission that exists today without needing an
	// edit the day the other one starts.
	const fromOutDir = outDirFilesBySuffix(".d.ts");
	const fromSuite = suiteFiles(".d.ts");
	const files = [...fromSuite.files, ...fromOutDir.files]
		.filter((file, at, all) => all.findIndex(other => other.name === file.name) === at)
		.sort((left, right) => left.name.localeCompare(right.name));
	const dir = [fromSuite.dir, fromOutDir.dir].filter(Boolean).join(" + ") || null;
	const recorded = readdirSync(EMITTED).filter(name => name.endsWith(".d.ts")).sort();

	if (!files.length && !recorded.length) {
		process.stdout.write("PENDING: no .d.ts emitted and none recorded.\n");
		process.stdout.write(`  looked in ${dir ?? "no OUT_DIR at all (build demo-app first)"}\n`);
		process.stdout.write("  This is not a pass. The step arms itself on the first recording; see dts.mjs.\n");
		return 0;
	}
	if (!files.length) {
		process.stdout.write(`FAIL: ${recorded.length} .d.ts golden(s) recorded and the build emits none.\n`);
		process.stdout.write(`  looked in ${dir ?? "no OUT_DIR at all"}\n`);
		process.stdout.write(`  recorded: ${recorded.join(", ")}\n`);
		return 1;
	}

	checks.note(`emitted from ${dir}`);

	/** @type {Record<string, object>} */
	const structure = {};
	for (const file of files) {
		const island = islandOf(file.name);
		const golden = join(EMITTED, file.name);
		const previous = existsSync(golden) ? readFileSync(golden, "utf8") : null;

		if (check) {
			checks.ok(`${file.name} matches its golden`, previous === file.source,
				previous === null ? "no golden recorded; run `node dts.mjs` to record it" : "the emitted bytes differ from the recorded ones");
		} else {
			writeFileSync(golden, file.source);
		}

		structure[file.name] = structureOf(file.name, file.source);

		const consumer = join(CONSUMERS, `${island}.ts`);
		const existing = existsSync(consumer) ? readFileSync(consumer, "utf8") : null;
		const generated = existing === null || existing.startsWith(SCAFFOLD_MARKER);
		if (!check && generated) writeFileSync(consumer, scaffold(island, structure[file.name]));
		if (check) {
			checks.ok(`${island} has a consumer`, existing !== null, "run `node dts.mjs` to scaffold one");
		} else if (!generated) {
			checks.note(`${island}.ts is hand written and was left alone`);
		}
	}

	// A golden with no emission behind it any more.
	for (const name of recorded) {
		if (!files.some(file => file.name === name)) {
			checks.ok(`${name} is still emitted`, false, "a recorded .d.ts is no longer produced by the build");
			if (!check) rmSync(join(EMITTED, name), { force: true });
		}
	}

	const summary = { $generatedBy: "contract/parity/dts.mjs", $doNotEditByHand: "Re-run the step. A diff here IS the API change.", files: structure };
	const rendered = `${JSON.stringify(summary, null, 2)}\n`;
	if (check) {
		const previous = existsSync(STRUCTURE) ? readFileSync(STRUCTURE, "utf8") : null;
		checks.ok("structure.json matches", previous === rendered,
			previous === null ? "no structural summary recorded" : "the declared surface changed; diff dts/structure.json");
	} else {
		writeFileSync(STRUCTURE, rendered);
	}

	const parsers = new Set(Object.values(structure).map(entry => entry.parser));
	if (parsers.has("degraded")) checks.note("structural summary came from the degraded parser; it is weaker than the real one");

	typeCheck(checks);

	process.stdout.write(`${checks.report()}\n`);
	return checks.failures.length ? 1 : 0;
}

// Importable so test.mjs can exercise the pieces; only the direct run does work.
if (import.meta.main) process.exit(main());
