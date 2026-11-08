// What the island page delivers to the browser, read out of the server that
// serves it rather than restated here.
//
// demo-app/src/dom.rs holds the delivery constants -- the import map, the `_$HY`
// bootstrap, the loader module -- and one `#[route]` per URL the page
// references. Every one of them is load-bearing for hydration, and a harness
// that kept its own copy would keep passing while the served page broke. So the
// constants and the route table are both parsed out of the Rust source.
//
// This is not fussiness. Between this harness being started and being finished,
// dom.rs went from serving the runtime as three modules stitched by an import
// map (`topcoat-dom.js` re-exporting `solid-web.js` plus six names from
// `solid.js`) to serving one self-contained bundle. Nothing here needed
// changing, because nothing here names those files.
//
// The one thing NOT taken from dom.rs is the compiled island module, which is a
// build artifact. Its path contract is `locateIslandModule` below.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { basename, join } from "node:path";

/** contract/parity/ */
export const PARITY = join(import.meta.dirname, "..");
/** the spike root, `spike/rustc-codegen-js/` */
export const SPIKE = join(PARITY, "..", "..");
/** demo-app/src/, the directory `include_str!` paths in dom.rs are relative to */
export const DOM_RS_DIR = join(SPIKE, "demo-app", "src");
/** where the delivery constants and the routes live */
export const DOM_RS = join(DOM_RS_DIR, "dom.rs");

/**
 * A `const NAME: &str = ..;` from a Rust source, evaluated.
 *
 * Handles every form the delivery constants use:
 *
 * - `r#".."#`, a raw string literal;
 * - `concat!(..)` over string literals, which is a join;
 * - `include_str!(concat!(env!("OUT_DIR"), "/x"))`, a build artifact;
 * - `include_str!("../rel/path")`, a committed file.
 *
 * The last two are the same two forms a `#[route]` body is read by, and they
 * are here because a delivery constant can move between the four without the
 * page changing at all: the import map became a build artifact the moment the
 * islands were chunked, since the build is what knows how many chunks there
 * are. Anything else fails loudly rather than being silently half-read.
 */
export function stringConst(source, name) {
	const raw = source.match(new RegExp(`const ${name}: &str = r#"([\\s\\S]*?)"#;`));
	if (raw) return raw[1];

	const joined = source.match(new RegExp(`const ${name}: &str = concat!\\(([\\s\\S]*?)\\n\\);`));
	if (joined) {
		const pieces = [...joined[1].matchAll(/r#"([\s\S]*?)"#|"((?:[^"\\]|\\.)*)"/g)];
		if (!pieces.length) throw new Error(`${name}'s concat!(..) in dom.rs has no string literals`);
		return pieces.map(piece => piece[1] ?? JSON.parse(`"${piece[2]}"`)).join("");
	}

	const declaration = source.match(new RegExp(`const ${name}: &str = ([\\s\\S]*?);\\n`));
	if (declaration) {
		const body = declaration[1];
		const artifact = body.match(/include_str!\(concat!\(\s*env!\("OUT_DIR"\),\s*"([^"]+)"\s*\)\)/);
		// Naming the constant matters more here than anywhere else in this file.
		// The artifact is missing exactly when demo-app has not been rebuilt
		// since dom.rs started reading this constant from the build, and
		// "no <hash>/out/x" on its own does not say which constant went looking.
		if (artifact) {
			try {
				return locateOutDirArtifact(artifact[1]).source;
			} catch (error) {
				throw new Error(`${name} is a build artifact and it is not there. ${error.message}`);
			}
		}

		const file = body.match(/include_str!\(\s*"([^"]+)"\s*\)/);
		if (file) return readFileSync(join(DOM_RS_DIR, file[1]), "utf8");
	}

	throw new Error(`${name} in dom.rs is not a form this harness reads (raw string, concat!, or include_str!): the harness cannot read the served bytes`);
}

/** A path parameter in a route's URL, e.g. the `{chunk}` in `/demo/chunks/{chunk}`. */
const PARAMETER = /\{[^}]+\}/;

/**
 * One arm of a parameterized route: the name it answers to, and the file it
 * answers with. Both `include_str!` forms, since a directory of build artifacts
 * and a directory of committed files are served the same way, and an optional
 * brace, since rustfmt wraps an arm whose body does not fit on the line.
 */
const ARM = /"([^"]+)"\s*=>\s*\{?\s*Module::\w+\(\s*include_str!\(\s*(?:concat!\(\s*env!\("OUT_DIR"\),\s*"([^"]+)"\s*\)|"([^"]+)")\s*\)\s*\)/g;

/** The name every arm of a match answers to, whether or not its body is readable. */
const ARM_LABEL = /"([^"]+)"\s*=>/g;

/**
 * The URLs a parameterized route answers, paired with the bytes each returns.
 *
 * The parameter is substituted with the arm's own literal, so the keys are the
 * URLs a browser requests rather than the pattern the router matched.
 *
 * `missed` names the arms whose body this could not read. It exists because
 * reading SOME of a route is the worst of the three outcomes: a route that
 * reads is checked, a route that fails is reported, and a route that half
 * reads leaves a served module invisible to every consumer including the
 * coverage check whose whole job is to notice a new module. One arm braced by
 * rustfmt did exactly that here, and nothing said so.
 *
 * @param {string} url the route's URL, containing one parameter
 * @param {string} body the route function's source
 * @returns {{served: [string, string][], missed: string[]}}
 */
export function expandParameterized(url, body) {
	const served = [];
	const read = new Set();
	const unbuilt = [];
	for (const [, name, artifact, file] of body.matchAll(ARM)) {
		let contents;
		if (artifact) {
			// An arm naming a build artifact that is not on disk is a DIFFERENT
			// problem from an arm this cannot parse, and it needs a different
			// answer: rebuild, rather than teach the harness a new shape. It also
			// happens routinely and through nobody's mistake, because dom.rs and
			// the build that produces the chunks are edited by different people at
			// different times -- an island's route lands before its chunk is built.
			// Letting `locateOutDirArtifact` throw here aborted the entire run with
			// a stack trace, taking every OTHER fixture down with it; recording it
			// lets `delivery` report it as the one-line diagnosis it is.
			try {
				contents = locateOutDirArtifact(artifact).source;
			} catch {
				unbuilt.push(`${name} (${artifact})`);
				continue;
			}
		} else {
			contents = readFileSync(join(DOM_RS_DIR, file), "utf8");
		}
		served.push([url.replace(PARAMETER, name), contents]);
		read.add(name);
	}
	const missed = [...body.matchAll(ARM_LABEL)]
		.map(match => match[1])
		.filter(name => !read.has(name) && !unbuilt.some(entry => entry.startsWith(`${name} (`)));
	return { served, missed, unbuilt };
}

/**
 * Everything the island page delivers, as the server has it.
 *
 * `routes` maps each URL dom.rs answers to its source. A route body is read one
 * of four ways, which is every way dom.rs currently answers one:
 *
 * - `include_str!(concat!(env!("OUT_DIR"), "/x"))` -- a build artifact, found
 *   under demo-app's OUT_DIR by `locateOutDirArtifact`.
 * - `include_str!("../rel/path")` -- a committed file, relative to demo-app/src.
 * - `Module(SOME_CONST)` -- one of the string constants above.
 * - a route whose URL carries a `{parameter}`, matching a name to one of the
 *   above per arm. It contributes one entry per arm, keyed by the URL that arm
 *   answers, so a directory of chunks reads as the several modules it is.
 *
 * A route whose body is none of these is reported, not skipped: an unreadable
 * route is a module the page serves and this harness would not be running.
 */
export function delivery() {
	const source = readFileSync(DOM_RS, "utf8");
	const importMap = JSON.parse(stringConst(source, "IMPORT_MAP"));
	const bootstrap = stringConst(source, "BOOTSTRAP");
	const loader = stringConst(source, "LOADER");

	const routes = new Map();
	const unreadable = [];
	const unbuilt = [];
	// Each route runs to the next attribute at column 0, which is the next
	// `#[route(..)]` or the `#[cfg(test)]` module.
	const bodies = [...source.matchAll(/#\[route\(GET "([^"]+)"\)\]([\s\S]*?)(?=\n#\[|$)/g)];
	for (const [, url, body] of bodies) {
		// A route with a path parameter is one route serving many files, and the
		// URL a browser asks for is never the one written in the attribute. Every
		// name it answers to is in the source -- each file is compiled into the
		// binary, so an arm that is not there is not a file -- and expanding them
		// keeps every consumer working in terms of the URLs actually fetched.
		// Without this, one parameterized route reads as a single module and the
		// first `include_str!` in it silently stands in for all of them.
		if (PARAMETER.test(url)) {
			const expanded = expandParameterized(url, body);
			if (expanded.unbuilt.length) unbuilt.push(...expanded.unbuilt);
			if (!expanded.served.length || expanded.missed.length) {
				unreadable.push(expanded.missed.length ? `${url} (arms not read: ${expanded.missed.join(", ")})` : url);
				continue;
			}
			for (const [name, contents] of expanded.served) routes.set(name, contents);
			continue;
		}

		const artifact = body.match(/include_str!\(concat!\(\s*env!\("OUT_DIR"\),\s*"([^"]+)"\s*\)\)/);
		if (artifact) {
			routes.set(url, locateOutDirArtifact(artifact[1]).source);
			continue;
		}
		const file = body.match(/include_str!\(\s*"([^"]+)"\s*\)/);
		if (file) {
			routes.set(url, readFileSync(join(DOM_RS_DIR, file[1]), "utf8"));
			continue;
		}
		// A route that hands one of the delivery constants to a `Module`
		// constructor. Which constructor is not read: `script` and `source_map`
		// differ in the content type served, and this map is about bytes.
		const named = body.match(/Module(?:::\w+)?\(\s*([A-Z][A-Z0-9_]*)\s*\)/);
		if (named) {
			routes.set(url, stringConst(source, named[1]));
			continue;
		}
		unreadable.push(url);
	}
	if (unreadable.length) {
		throw new Error(`dom.rs serves ${unreadable.join(", ")} in a shape lib/delivery.mjs cannot read; teach it the new shape`);
	}
	// NOT thrown. An arm whose chunk is not built yet is reported and the other
	// arms are still delivered, because the alternative is that one island's route
	// landing before its chunk is built takes down every OTHER fixture's run. The
	// caller decides what it costs: a fixture that needs the missing chunk fails on
	// its own terms, and a fixture that does not is unaffected. `unbuilt` is
	// returned rather than only logged so nothing can consume this map believing it
	// is the whole delivery -- which is the same argument `missed` makes one level
	// down, applied to a condition that fixes itself with a rebuild.
	return { importMap, bootstrap, loader, routes, unbuilt };
}

/**
 * A build artifact demo-app's build script wrote into its own OUT_DIR.
 *
 * PATH CONTRACT: `demo-app/target/debug/build/<demo-app-HASH>/out/<name>`.
 * dom.rs serves it with `include_str!(concat!(env!("OUT_DIR"), "/<name>"))`, so
 * OUT_DIR is the only place it exists -- there is no copy anywhere predictable.
 * The hash is cargo's and changes with the build script's inputs, so the
 * directory is searched rather than named, the same way demo-app/smoke/check.mjs
 * finds it.
 *
 * `out/.mires-build/` is the backend's own staging copy and is not searched:
 * `out/<name>` is the one dom.rs serves.
 *
 * @param {string} name leading-slash artifact name, e.g. "/islands.js"
 * @returns {{path: string, source: string}}
 */
export function locateOutDirArtifact(name) {
	const builds = join(SPIKE, "demo-app", "target", "debug", "build");
	let entries;
	try {
		entries = readdirSync(builds);
	} catch {
		throw new Error(`${builds} does not exist: build demo-app first (cd demo-app && cargo build)`);
	}

	const found = [];
	for (const entry of entries) {
		const candidate = join(builds, entry, "out", name.replace(/^\//, ""));
		try {
			found.push({ path: candidate, source: readFileSync(candidate, "utf8"), mtime: statSync(candidate).mtimeMs });
		} catch {
			continue;
		}
	}
	if (!found.length) {
		throw new Error(`no <hash>/out${name} under ${builds}: build demo-app first (cd demo-app && cargo build)`);
	}
	// More than one means stale build directories are lying around. Newest wins,
	// and it is reported, because running parity against a stale emitter is the
	// exact class of mistake this harness exists to catch.
	if (found.length > 1) {
		found.sort((a, b) => b.mtime - a.mtime);
		console.error(`warning: ${found.length} copies of out${name} under ${builds}; using the newest (${found[0].path}). Stale build dirs?`);
	}
	return found[0];
}

/** The compiled client half of every island on the page. */
export function locateIslandModule() {
	return locateOutDirArtifact("/islands.js");
}

/**
 * Every build artifact whose name ends in `suffix`, from the NEWEST OUT_DIR only.
 *
 * Same PATH CONTRACT and same hash-directory search as `locateOutDirArtifact`,
 * for the case where the harness does not know the artifact's name in advance.
 * The `.d.ts` emission is that case: what the backend calls each file is the
 * backend's to decide, and a harness that named the files itself would report
 * "nothing emitted" the day the naming changed, which is the least useful
 * possible failure.
 *
 * Only the newest build directory is read. Merging several would present two
 * emitters' output as one API surface.
 *
 * @param {string} suffix e.g. ".d.ts"
 * @returns {{dir: string | null, files: {name: string, path: string, source: string}[]}}
 */
export function outDirFilesBySuffix(suffix) {
	const builds = join(SPIKE, "demo-app", "target", "debug", "build");
	let entries;
	try {
		entries = readdirSync(builds);
	} catch {
		return { dir: null, files: [] };
	}

	/** @type {{path: string, mtime: number}[]} */
	const outs = [];
	for (const entry of entries) {
		const candidate = join(builds, entry, "out");
		try {
			outs.push({ path: candidate, mtime: statSync(candidate).mtimeMs });
		} catch {
			continue;
		}
	}
	if (!outs.length) return { dir: null, files: [] };
	outs.sort((a, b) => b.mtime - a.mtime);

	// Newest first, but an empty newest directory is not evidence that nothing was
	// emitted, so the first OUT_DIR that actually holds a match wins.
	for (const out of outs) {
		const files = [];
		for (const name of readdirSync(out.path).sort()) {
			if (!name.endsWith(suffix)) continue;
			const full = join(out.path, name);
			try {
				files.push({ name, path: full, source: readFileSync(full, "utf8") });
			} catch {
				continue;
			}
		}
		if (files.length) return { dir: out.path, files };
	}
	return { dir: outs[0].path, files: [] };
}

/** The bare specifier a page imports an island's chunk by. */
export const ISLAND_SPECIFIER = "topcoat-island/";

/**
 * Where an island's compiled client half is served.
 *
 * Resolved through the page's OWN import map rather than by building a path,
 * because the map is what the loader resolves `topcoat-island/<name>` through
 * and it is written by the build. A harness that assembled the URL itself would
 * agree with itself while disagreeing with the page the moment the chunk
 * directory moved, which is a build option.
 *
 * @param {{importMap: {imports: Record<string,string>}, routes: Map<string,string>}} served
 * @param {string} island
 * @returns {string} the URL, which is a key of `served.routes`
 */
export function islandModuleUrl(served, island) {
	const specifier = `${ISLAND_SPECIFIER}${island}`;
	const url = served.importMap.imports[specifier];
	if (!url) {
		const offered = Object.keys(served.importMap.imports).filter(name => name.startsWith(ISLAND_SPECIFIER));
		throw new Error(`the page's import map has no ${specifier}; it offers ${offered.join(", ") || "no islands at all"}`);
	}
	if (!served.routes.has(url)) {
		throw new Error(`the import map points ${specifier} at ${url}, which dom.rs does not serve`);
	}
	return url;
}

/** The demo-app binary the SSR captures come from. */
export const BINARY = join(SPIKE, "demo-app", "target", "debug", "demo-app");

/**
 * Is the built server newer than everything that decides what it sends?
 *
 * A captured `ssr.html` is only evidence about the current tree if the binary
 * that produced it was built from the current sources. This is not a hypothetical
 * check: it caught a real staleness the first time it ran. dom.rs had been
 * rewritten to serve the runtime as one self-contained bundle instead of three
 * import-map-stitched modules, and the captured page still advertised the old
 * import map, because the binary predated the edit by twenty minutes.
 *
 * The compiled island module is checked separately and by content -- the template
 * parity check compares it against the captured markup directly -- so this only
 * covers what the page says about its own delivery.
 *
 * @returns {{fresh: boolean, binary: number | null, stale: {file: string, mtime: number}[]}}
 */
export function captureFreshness() {
	let binary;
	try {
		binary = statSync(BINARY).mtimeMs;
	} catch {
		return { fresh: false, binary: null, stale: [] };
	}

	const inputs = [
		join(SPIKE, "demo-app", "build.rs"),
		join(SPIKE, "runtime", "dom", "dist", "topcoat-dom.js"),
		...sources(join(SPIKE, "demo-app", "src")),
		...sources(join(SPIKE, "demo-app", "island")),
	];
	const stale = [];
	for (const file of inputs) {
		try {
			const mtime = statSync(file).mtimeMs;
			if (mtime > binary) stale.push({ file, mtime });
		} catch {
			continue;
		}
	}
	stale.sort((a, b) => b.mtime - a.mtime);
	return { fresh: stale.length === 0, binary, stale };
}

function sources(dir) {
	try {
		return readdirSync(dir, { withFileTypes: true })
			.filter(entry => entry.isFile() && entry.name.endsWith(".rs"))
			.map(entry => join(dir, entry.name));
	} catch {
		return [];
	}
}

/**
 * The names the compiled module imports, the specifiers it imports them by, and
 * the island entry points it exports.
 *
 * Read out of the module rather than listed here, so this cannot drift from what
 * the emitter actually calls.
 */
export function moduleImports(source) {
	const imports = [...source.matchAll(/^import \{ (\w+) as (\w+) \} from "([^"]+)";$/gm)];
	if (!imports.length) throw new Error("the compiled island module imports nothing: is it an ES module?");
	return {
		specifiers: [...new Set(imports.map(match => match[3]))],
		names: [...new Set(imports.map(match => match[1]))].sort(),
		islands: [...source.matchAll(/^function (__island_\w+)\(/gm)].map(match => match[1]),
	};
}

/** The template strings the compiled module declares, in declaration order. */
export function moduleTemplates(source) {
	return [...source.matchAll(/_\$template\("((?:[^"\\]|\\.)*)"\)/g)].map(match => JSON.parse(`"${match[1]}"`));
}

/**
 * The template strings ONE island declares, in declaration order.
 *
 * WHY THIS EXISTS, because `moduleTemplates` above looks like it is enough and was
 * for exactly as long as demo-app had one island. Templates are module-scoped
 * `const tmpl$h<hash> = _$template("..")` bindings shared by every island in the
 * bundle, so "how many templates does this island declare" is not answerable by
 * counting them. The moment a second island landed, the counter fixture's true
 * claim -- one template -- started reading as four, and it was the fixture that
 * failed rather than anything about the counter.
 *
 * So the question is asked properly: which template bindings are reachable from
 * this island's entry point. Reachability is transitive through the module's own
 * functions, because a component body is a separate function that names its own
 * template -- the whole reason a nesting island has more than one.
 *
 * Read out of the module by identifier, never by position. The emitter's
 * declaration order is its own business and this makes no assumption about it; what
 * it establishes is the SET, which the caller may then order.
 *
 * @param {string} source the compiled island module
 * @param {string} island the island name, without the `__island_` prefix
 * @returns {{templates: string[], vars: string[], reached: string[]}}
 */
export function islandTemplates(source, island) {
	// Every module-scope template binding, in declaration order.
	// `[\w$]` and not `\w`: the emitter's mangled names carry `$` (`tmpl$h<hash>`,
	// `islands_js_expanded$counter$..`), which `\w` does not match. Getting this
	// wrong found NO templates rather than the wrong ones, which is the direction a
	// reader like this should fail in.
	const declared = [...source.matchAll(/^const ([\w$]+) = _\$template\("((?:[^"\\]|\\.)*)"\);$/gm)]
		.map(match => ({ name: match[1], template: JSON.parse(`"${match[2]}"`) }));
	if (!declared.length) throw new Error("the compiled module declares no `const .. = _$template(..)` bindings; teach lib/delivery.mjs the new shape");

	// Every top-level function and its body, so references can be followed.
	const bodies = new Map();
	for (const match of source.matchAll(/^function ([\w$]+)\([^)]*\) \{\n([\s\S]*?)\n\}$/gm)) {
		bodies.set(match[1], match[2]);
	}
	const entry = `__island_${island}`;
	if (!bodies.has(entry)) throw new Error(`the compiled module has no top-level \`function ${entry}\`, so its templates cannot be attributed`);

	// Transitive closure over identifiers. Crude on purpose: an identifier that
	// happens to appear in a string literal would be followed too, which can only
	// ever ADD templates to the set and so cannot make the count look smaller than
	// it is. The count is asserted against the fixture, so an over-count fails
	// loudly rather than passing.
	const reached = new Set([entry]);
	const queue = [entry];
	while (queue.length) {
		const body = bodies.get(queue.shift());
		// No `\b`, because a word boundary does not fall either side of a `$`.
		for (const match of body.matchAll(/[A-Za-z_$][\w$]*/g)) {
			const name = match[0];
			if (reached.has(name)) continue;
			if (bodies.has(name) || declared.some(binding => binding.name === name)) {
				reached.add(name);
				if (bodies.has(name)) queue.push(name);
			}
		}
	}

	const vars = declared.filter(binding => reached.has(binding.name));
	return { templates: vars.map(binding => binding.template), vars: vars.map(binding => binding.name), reached: [...reached] };
}

/** The basename a URL path is staged under. */
export function stagedName(url) {
	return basename(url);
}
