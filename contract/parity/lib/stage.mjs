// Links the page's module graph the way the browser's import map links it.
//
// The browser resolves four kinds of specifier on the island page: a bare one
// (`topcoat-dom`) through the import map, a bare SUBPATH (`topcoat-island/counter`)
// through the same map, an absolute URL path (`/demo/x.js`) through the origin,
// and a dynamic import in the loader whose specifier is BUILT AT RUNTIME.
//
// The first three can be rewritten, because they are written down. The fourth
// cannot: the loader holds `const CHUNK = "topcoat-island/"` and imports
// `CHUNK + name`, so the specifier does not exist as text anywhere and no
// rewrite can find it. That is not an accident of this loader either -- it is
// what an import map is FOR, resolving a name the page computes.
//
// So the specifier has to really resolve. Node's own analogue of an import map
// is a package with an `exports` map, and that is what this builds: the island
// chunks are staged inside `node_modules/topcoat-island/` with an `exports`
// entry per island, so `import("topcoat-island/" + name)` resolves the way the
// browser resolves it, at runtime, with nothing rewritten.
//
// Because the chunks then sit in a subdirectory, a specifier's rewrite depends
// on WHERE THE IMPORTER IS. Every module gets a location first, and each
// specifier is rewritten to the path relative to its own importer.
//
// The bytes are otherwise untouched. That matters: the compiled island chunks
// and the runtime bundle are the artifacts under test, and a harness that
// rewrote their bodies would be testing something else. Only specifiers move.
//
// Each call to `stage` gets a FRESH directory, which is what makes fixtures
// independent. Solid's `sharedConfig` is a module-level singleton and hydration
// state lives on it, so two fixtures sharing a module instance would share a
// hydration cursor. Fresh file URLs mean fresh module instances mean a fresh
// `sharedConfig` per run. For the same reason every module is staged EXACTLY
// ONCE: a chunk reachable at two paths would be two module instances with two
// hydration cursors, which is the bug this file exists to avoid rather than a
// detail of it.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative } from "node:path";
import { pathToFileURL } from "node:url";

/** The package an island's chunk is imported from, as a bare subpath specifier. */
const ISLAND_PACKAGE = "topcoat-island";

/** Where in the staging directory a served URL is written. */
function locationFor(url, bareFor) {
	const bare = bareFor.get(url);
	if (bare && bare.startsWith(`${ISLAND_PACKAGE}/`)) {
		return join("node_modules", ISLAND_PACKAGE, basename(url));
	}
	return basename(url);
}

/** A relative specifier from `fromLocation`'s directory to `toLocation`. */
function specifierBetween(fromLocation, toLocation) {
	const path = relative(dirname(fromLocation) || ".", toLocation);
	return path.startsWith(".") ? path : `./${path}`;
}

/**
 * Write the page's modules into a fresh staging directory with their specifiers
 * rewritten.
 *
 * @param {{importMap: {imports: Record<string,string>}, routes: Map<string,string>}} served
 * @returns {{dir: string, href: (url: string) => string, locations: Map<string,string>}}
 */
export function stage(served) {
	const dir = mkdtempSync(join(tmpdir(), "topcoat-parity-"));

	// The first bare specifier pointing at a URL decides where that URL lives.
	// An island chunk named both `topcoat-island/counter` and something else
	// would still be staged once.
	const bareFor = new Map();
	for (const [bare, url] of Object.entries(served.importMap.imports)) {
		if (!served.routes.has(url)) {
			throw new Error(`the import map points ${bare} at ${url}, which dom.rs does not serve`);
		}
		if (!bareFor.has(url)) bareFor.set(url, bare);
	}

	const locations = new Map();
	for (const url of served.routes.keys()) locations.set(url, locationFor(url, bareFor));

	// What each written-down specifier resolves to, per importer location.
	const targets = new Map();
	for (const [url, location] of locations) targets.set(url, location);
	for (const [bare, url] of Object.entries(served.importMap.imports)) targets.set(bare, locations.get(url));

	for (const [url, source] of served.routes) {
		const location = locations.get(url);
		const rewrites = new Map();
		for (const [specifier, target] of targets) rewrites.set(specifier, specifierBetween(location, target));

		const file = join(dir, location);
		mkdirSync(dirname(file), { recursive: true });
		writeFileSync(file, rewriteSpecifiers(source, rewrites));
	}

	// Node's import map: the chunks answer to the bare subpath the loader builds.
	const islands = Object.entries(served.importMap.imports).filter(([bare]) => bare.startsWith(`${ISLAND_PACKAGE}/`));
	if (islands.length) {
		const exports = {};
		for (const [bare, url] of islands) {
			exports[`.${bare.slice(ISLAND_PACKAGE.length)}`] = `./${basename(locations.get(url))}`;
		}
		const packageDir = join(dir, "node_modules", ISLAND_PACKAGE);
		mkdirSync(packageDir, { recursive: true });
		writeFileSync(
			join(packageDir, "package.json"),
			`${JSON.stringify({ name: ISLAND_PACKAGE, type: "module", exports }, null, 2)}\n`,
		);
	}

	return {
		dir,
		locations,
		href(url) {
			const location = locations.get(url);
			if (!location) throw new Error(`${url} is not served by dom.rs, so it was not staged`);
			return pathToFileURL(join(dir, location)).href;
		},
	};
}

/**
 * Rewrite every specifier in a module source.
 *
 * Specifiers are replaced as whole quoted strings, which is why the longest
 * specifier has to go first: `/demo/islands.js` is a prefix of
 * `/demo/islands.js.map`, and only the closing quote keeps them apart. Matching
 * the quotes makes that safe, and sorting makes it safe if a future specifier
 * pair is not separated by one.
 *
 * @param {string} source
 * @param {Map<string,string>} rewrites
 */
export function rewriteSpecifiers(source, rewrites) {
	const ordered = [...rewrites.entries()].sort((a, b) => b[0].length - a[0].length);
	for (const [from, to] of ordered) {
		source = source.replaceAll(`"${from}"`, `"${to}"`).replaceAll(`'${from}'`, `'${to}'`);
	}
	return source;
}

/**
 * Every bare specifier a module still imports after staging.
 *
 * A bare specifier left behind is one node will try to resolve against
 * node_modules -- and one the browser would have needed an import-map entry for.
 * Either way it is a hole in the delivery, so the run reports it rather than
 * letting node's resolver produce a confusing error.
 *
 * The island package is excluded: it is the one bare specifier that is SUPPOSED
 * to survive staging, because the loader builds it at runtime and this directory
 * really does resolve it.
 *
 * @param {string} source
 * @returns {string[]}
 */
export function unresolvedSpecifiers(source) {
	const specifiers = [
		...source.matchAll(/(?:^|[;\s])(?:import|export)[^;]*?from\s*["']([^"']+)["']/g),
		...source.matchAll(/\bimport\s*\(\s*["']([^"']+)["']\s*\)/g),
	].map(match => match[1]);
	return [...new Set(specifiers.filter(specifier => (
		!specifier.startsWith(".")
		&& !specifier.startsWith("/")
		&& !specifier.includes(":")
		&& specifier !== ISLAND_PACKAGE
		&& !specifier.startsWith(`${ISLAND_PACKAGE}/`)
	)))];
}
