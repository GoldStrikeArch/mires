// Unit tests for the harness's own moving parts.
//
//   node test.mjs
//
// The parity run is an integration test, and an integration test that fails tells
// you something is wrong without telling you whether the something is the emitter
// or the harness. These cover the pieces where the harness could be wrong on its
// own: the key algorithm it derives expectations from, the specifier rewriting the
// module graph depends on, and the tree comparison that decides what "the same
// tree" means.
//
// The key test is wired to the contract: it runs against all 264 triples in
// fixtures/keys.json, which were recorded from the runtime's own numbering. So if
// upstream changes how keys are encoded, this fails rather than the parity run
// mysteriously stopping at ordinal 10.

import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { JSDOM } from "jsdom";

import { chunkingState, unbudgetedRoutes } from "./budgets.mjs";
import { band, islandsIn, judge, measure, sizes } from "./lib/budget.mjs";
import { fetchStub, wireParity, wireSpec } from "./lib/wire.mjs";
import { Checks } from "./lib/check.mjs";
import { SPIKE, delivery, expandParameterized, stringConst } from "./lib/delivery.mjs";
import { allocationCount, allocationOrder, contextTree, documentKeys, keyChain, nestedKeyParity, rootsByKey } from "./lib/keys.mjs";
import { rewriteSpecifiers, unresolvedSpecifiers } from "./lib/stage.mjs";
import { templateParity } from "./lib/tree.mjs";
import { hydrationKey, movedKeys, perturb, perturbTree } from "./run.mjs";
import { SCAFFOLD_MARKER, scaffold, structureOf, typeCheckFiles, typescriptAvailable } from "./dts.mjs";

const checks = new Checks("harness unit tests");
const { window } = new JSDOM("<!doctype html><body>");
const document = window.document;

// ---------- the hydration key algorithm ----------

const keys = JSON.parse(readFileSync(join(SPIKE, "contract", "fixtures", "keys.json"), "utf8"));
const wrong = keys.triples.filter(triple => hydrationKey(triple.contextId, triple.count) !== triple.key);
checks.is(`hydrationKey reproduces all ${keys.triples.length} recorded triples`, wrong.slice(0, 3), []);
checks.is("ordinal 9 gets no letter", hydrationKey("i0.", 9), "i0.9");
checks.is("ordinal 10 gets 'a', which is the whole point of the letter", hydrationKey("i0.", 10), "i0.a10");
checks.is("ordinal 100 gets 'b'", hydrationKey("i0.", 100), "i0.b100");
checks.ok(
	"'i0.' + 1 is not a prefix of 'i0.' + 10, so an island's keys stay separable",
	!hydrationKey("i0.", 10).startsWith(`${hydrationKey("i0.", 1)}0`),
);

// ---------- nested keys, against the two-sided oracle ----------
//
// Same wiring as above, one level up: `keys.json` pins the ENCODING with
// hand-set contexts, `keys-nested.json` pins the NESTING by driving real
// component trees through both of solid's runtimes. Every assertion below is
// answerable today, with no demo island and no SSR bytes, because the oracle
// carries the key sequences the runtimes produced.

const nested = JSON.parse(readFileSync(join(SPIKE, "contract", "fixtures", "keys-nested.json"), "utf8"));

// A key parses into its chain of slots, and every link re-encodes to itself.
// Run over every key in every case, including the cases whose recorded list is
// truncated to its ends, since a single key is checkable on its own.
const chainFailures = [];
let keysWalked = 0;
for (const record of nested.cases) {
	// A case above 24 nodes records only the ends of its list, with a literal
	// "... n elided ..." marker between them. The marker is not a key.
	for (const key of (record.elementKeys ?? []).filter(entry => !entry.includes(" elided "))) {
		keysWalked++;
		const chain = keyChain(key, record.renderId);
		if (!chain) {
			chainFailures.push({ case: record.name, key, why: "does not parse" });
			continue;
		}
		if (chain.at(-1).key !== key) chainFailures.push({ case: record.name, key, why: "the chain does not end at the key" });
		for (const link of chain) {
			if (hydrationKey(link.contextId, link.count) !== link.key) {
				chainFailures.push({ case: record.name, key, why: `${JSON.stringify(link.contextId)} + ${link.count} is not ${link.key}` });
			}
		}
	}
}
checks.is(`every one of the ${keysWalked} recorded nested keys parses into a slot chain that re-encodes to itself`, chainFailures.slice(0, 3), []);

// The oracle records `elementKeys` in the order the runtimes allocated them, so
// recovering that order from the key strings alone must reproduce it. Only the
// cases whose list is complete can check a whole sequence.
const complete = nested.cases.filter(record => record.elementKeys?.length === record.elementKeyCount);
const orderFailures = complete
	.map(record => ({ case: record.name, got: allocationOrder(record.elementKeys, record.renderId), want: record.elementKeys }))
	.filter(entry => JSON.stringify(entry.got) !== JSON.stringify(entry.want));
checks.is(`allocation order is recovered from the keys alone in all ${complete.length} complete cases`, orderFailures.slice(0, 3), []);
checks.ok(`and those cases are most of the ${nested.cases.length} recorded`, complete.length >= 70, `only ${complete.length} carry a complete elementKeys list`);

// The context tree the keys imply must be the one the oracle recorded, i.e. the
// boundaries derived from the key strings are exactly the component slots whose
// child context contains at least one element key.
const treeFailures = [];
for (const record of complete) {
	const tree = contextTree(record.elementKeys, record.renderId);
	if (!tree.ok) {
		treeFailures.push({ case: record.name, problems: tree.problems });
		continue;
	}
	const recorded = new Set((record.allocations ?? []).filter(entry => entry.kind === "component").map(entry => entry.childContextId));
	if (record.allocations && !tree.boundaries.every(key => recorded.has(key))) {
		treeFailures.push({ case: record.name, derived: tree.boundaries, recorded: [...recorded] });
	}
}
checks.is("every derived component boundary is one the oracle recorded", treeFailures.slice(0, 3), []);

const inBody = nested.cases.find(record => record.name === "07b-component-in-body");
const bodyTree = contextTree(inBody.elementKeys, "");
checks.is("corpus 07b nested_in_body implies two boundaries, the card and the badge inside it", bodyTree.boundaries, ["0", "01"]);
checks.is("and three contexts: the root, the card's, and the badge's", bodyTree.contexts.map(context => context.id), ["", "0", "01"]);
checks.is("the component's own root is slot 0 of its own context", keyChain("00", "").at(-1), { contextId: "0", count: 0, key: "00" });
checks.is("and the badge inside it nests one context deeper", keyChain("010", "").map(link => link.key), ["0", "01", "010"]);

const depth = nested.cases.find(record => record.name === "07b-depth-three");
checks.is("depth three implies three boundaries", contextTree(depth.elementKeys, "").boundaries, ["0", "01", "011"]);

// The letter-boundary case, which is where an implementation that dropped the
// letter would still pass every small case.
const crossing = nested.cases.find(record => record.elementKeys?.includes("a10a10"));
checks.ok("the oracle has a case where both counters cross the letter boundary at once", !!crossing);
checks.is("a10a10 is count 10 of the context count 10 opened", keyChain("a10a10", "").map(link => [link.contextId, link.count]), [["", 10], ["a10", 10]]);

// Malformed keys are rejected rather than shrugged at, which is the whole point
// of the letter being in the key.
checks.is(
	"a two-digit run is not one count of the root context -- it is two slots, so it implies a boundary",
	keyChain("00", "").map(link => [link.contextId, link.count]),
	[["", 0], ["0", 0]],
);
checks.is("a letter promising two digits followed by one is rejected", keyChain("a1", ""), null);
checks.is("a letter promising three digits followed by two is rejected", keyChain("b12", ""), null);
checks.is("a character that is neither digit nor lowercase letter is rejected", keyChain("0A", ""), null);
checks.is("a leading zero after a letter is rejected", keyChain("a01", ""), null);
checks.is("a key outside the render id is rejected", keyChain("i1.0", "i0."), null);
checks.is("the empty key is rejected", keyChain("", ""), null);
checks.is("allocationCount is exact, not a prefix match", allocationCount("", "01"), null);
checks.is("and it reads a whole-key allocation", allocationCount("0", "01"), 1);
checks.ok("an unreachable key is a reported problem, not a silent pass", !contextTree(["i0.0", "i0.zz"], "i0.").ok);

// DOCUMENT ORDER IS NOT ALLOCATION ORDER. The getterChildren rows are real SSR
// HTML from solid's server runtime, so this runs the parity entry point over
// bytes -- the same call the pending nested fixture will make.
function elementFrom(html) {
	const host = document.createElement("div");
	host.innerHTML = html;
	return host;
}

const eager = nested.getterChildren.rows.find(row => row.topcoatShape && !row.case.includes("sibling"));
const eagerHost = elementFrom(eager.html);
checks.is("the eager row's keys in document order put the child's lower key second", documentKeys(eagerHost), ["10", "0"]);
const eagerParity = nestedKeyParity(eagerHost, { renderId: "", expected: ["0", "10"], components: 1 });
checks.is("but the allocation order recovered from the keys is the source order", eagerParity.allocationOrder, ["0", "10"]);
checks.is("so the eager layout passes with its declared expectation", eagerParity.differences, []);
checks.is("the component's key nests the caller's other root under no context of its own", eagerParity.nestedContexts, ["1"]);
checks.ok(
	"the element holding the LOWER key is a DOM descendant of the one holding the higher, so DOM nesting is not context nesting",
	eagerHost.querySelector('[data-hk="10"]').contains(eagerHost.querySelector('[data-hk="0"]')),
);

const lazy = nested.getterChildren.rows.find(row => row.solidIdiomatic && !row.case.includes("sibling"));
const lazyParity = nestedKeyParity(elementFrom(lazy.html), { renderId: "", expected: ["00", "01"], components: 1 });
checks.is("solid's lazy layout is a different sequence from the same source", lazyParity.allocationOrder, ["00", "01"]);
checks.is("and it also passes, against its own expectation", lazyParity.differences, []);
checks.ok(
	"the two layouts are genuinely different, which is the permanent delta 07b records",
	JSON.stringify(eagerParity.allocationOrder) !== JSON.stringify(lazyParity.allocationOrder),
);

const eagerSibling = nested.getterChildren.rows.find(row => row.topcoatShape && row.case.includes("sibling"));
checks.is(
	"a sibling after an eager component takes slot 2, because the child took 0 and the boundary took 1",
	nestedKeyParity(elementFrom(eagerSibling.html), { renderId: "", expected: ["0", "10", "2"] }).differences,
	[],
);

// The negative direction: a wrong expectation must fail, and a key that breaks
// the scheme must fail even with no expectation to compare against.
checks.ok(
	"a wrong expected sequence is caught",
	!nestedKeyParity(eagerHost, { renderId: "", expected: ["10", "0"] }).ok,
);
checks.ok(
	"a key the scheme cannot produce is caught with no expectation at all",
	!nestedKeyParity(elementFrom('<div data-hk="0a1"></div>'), { renderId: "" }).ok,
);
// The honest limit, asserted so nobody reads more into a green run than is
// there: a well-formed key from an implausible depth parses, and it takes the
// declared component count to reject it.
checks.ok(
	"a well-formed but too-deep key passes the parse on its own",
	nestedKeyParity(elementFrom('<div data-hk="0123"></div>'), { renderId: "" }).ok,
);
checks.ok(
	"and is rejected once the fixture says how many components its source has",
	!nestedKeyParity(elementFrom('<div data-hk="0123"></div>'), { renderId: "", components: 0 }).ok,
);
checks.ok(
	"a duplicated key is caught",
	!nestedKeyParity(elementFrom('<div data-hk="0"></div><div data-hk="0"></div>'), { renderId: "" }).ok,
);
checks.ok(
	"a slot spent by both an element and a boundary is caught",
	!nestedKeyParity(elementFrom('<div data-hk="0"><span data-hk="00"></span></div>'), { renderId: "" }).ok,
);
checks.ok(
	"a declared component count the keys do not account for is caught",
	!nestedKeyParity(eagerHost, { renderId: "", components: 2 }).ok,
);
const empty = nestedKeyParity(elementFrom('<div data-hk="0"></div><div data-hk="2"></div>'), { renderId: "", components: 1 });
checks.is("an empty component still spends a slot, and that slot is accounted for by the declared count", empty.differences, []);
checks.is("and it is reported as a slot nothing carries", empty.unaccountedSlots, ["1"]);

// The roots map, which is how a multi-template fixture pairs a template with
// the root it describes rather than with whatever came first in the document.
const roots = rootsByKey(elementFrom('<div data-hk="10"><span data-hk="0"></span></div>'), "");
checks.is("rootsByKey finds every keyed root", [...roots.keys()], ["10", "0"]);
checks.is("and maps a key to the element that carries it", roots.get("0")?.nodeName, "SPAN");

// ---------- what the server delivers ----------

// How a delivery constant is spelled, checked against synthetic sources rather
// than against dom.rs. These are the forms the harness has to be able to read,
// and pinning them here means they stay proved when demo-app is not built --
// which is exactly when someone is most likely to be changing one.
checks.is(
	"a raw string constant is read whole",
	stringConst('const LOADER: &str = r#"import x from "y";"#;\n', "LOADER"),
	'import x from "y";',
);
checks.is(
	"a concat! constant is joined in order",
	stringConst('const B: &str = concat!(\n    r#"<a>"#,\n    "click",\n    r#"</a>"#,\n);\n', "B"),
	"<a>click</a>",
);
checks.is(
	"a committed file constant is read off disk",
	stringConst('const C: &str = include_str!("dom.rs");\n', "C").includes("const LOADER"),
	true,
);
checks.ok(
	"a constant the harness cannot read fails, and says so",
	(() => {
		try {
			stringConst("const D: &str = SOMETHING_ELSE;\n", "D");
			return false;
		} catch (error) {
			return error.message.includes("not a form this harness reads");
		}
	})(),
	"silently half-reading a delivery constant is how a harness passes while the page is broken",
);
checks.ok(
	"a missing build artifact names the constant that wanted it",
	(() => {
		try {
			stringConst('const E: &str = include_str!(concat!(env!("OUT_DIR"), "/nope.json"));\n', "E");
			return false;
		} catch (error) {
			return error.message.startsWith("E is a build artifact") && /build demo-app first/.test(error.message);
		}
	})(),
);

// A parameterized route is one route serving many files, so it has to read as
// the several modules it is. Checked on a synthetic body using the committed
// file arm, which needs no build; the artifact arm differs only in where the
// bytes come from.
const chunkRoute = `
async fn chunk_js(cx: &Cx) -> Result<Module> {
	let module = match path_param::<Chunk>(cx) {
		"counter.js" => Module::script(include_str!("dom.rs")),
		"counter.js.map" => Module::source_map(include_str!(
			"dom.rs"
		)),
		_ => return None::<Module>.ok_or_not_found()?,
	};
	Ok(module)
}
`;
const chunks = expandParameterized("/demo/chunks/{chunk}", chunkRoute);
checks.is(
	"a parameterized route contributes one entry per arm, keyed by the URL a browser asks for",
	chunks.served.map(([url]) => url),
	["/demo/chunks/counter.js", "/demo/chunks/counter.js.map"],
);
checks.ok("and each entry carries that arm's bytes", chunks.served.every(([, body]) => body.includes("const LOADER")));
checks.is("and no arm was left unread", chunks.missed, []);
checks.is(
	"an arm split across lines is still one arm",
	expandParameterized("/x/{n}", '"a" => Module::source_map(include_str!(\n\t"dom.rs"\n)),').served.map(([url]) => url),
	["/x/a"],
);
// rustfmt wraps an arm whose body does not fit on the line, which is not a
// change to what the route serves and must not be a change to what is read.
checks.is(
	"a braced arm is read like any other",
	expandParameterized("/x/{n}", '"a" => {\n\tModule::script(include_str!("dom.rs"))\n}').served.map(([url]) => url),
	["/x/a"],
);
// The one that matters. Half-reading a route is worse than failing to read it:
// the modules that did parse look like the whole delivery, and a served module
// nothing knows about is invisible to the coverage check built to notice one.
checks.is(
	"an arm whose body cannot be read is NAMED rather than silently dropped",
	expandParameterized("/x/{n}", '"a" => Module::script(include_str!("dom.rs")),\n"b" => something_else(),').missed,
	["b"],
);
checks.is(
	"a parameterized route with no arms expands to nothing, so delivery() reports it unreadable",
	expandParameterized("/demo/chunks/{chunk}", "async fn chunk_js() { todo!() }").served,
	[],
);

// The rest of this group reads the real dom.rs, so it needs a built demo-app.
// A missing build is an environment fact and is skipped LOUDLY; a dom.rs this
// harness cannot parse is a harness bug and still fails, which is why the two
// are told apart rather than both caught.
let served = null;
try {
	served = delivery();
} catch (error) {
	const notBuilt = /build demo-app first/.test(error.message);
	checks.ok(
		"the delivery constants are readable, or unreadable only because demo-app is not built",
		notBuilt,
		error.message,
	);
	if (notBuilt) checks.note(`SKIPPED the served-delivery group: ${error.message}`);
}

if (served) {
	const domRs = readFileSync(join(SPIKE, "demo-app", "src", "dom.rs"), "utf8");
	const declaredRoutes = [...domRs.matchAll(/#\[route\(GET "([^"]+)"\)\]/g)].map(match => match[1]).sort();
	// A parameterized route is not a key: it contributes one key per name it
	// answers to. So the comparison is per declared route, and the reverse
	// direction is checked separately -- a served key belonging to no declared
	// route would mean this harness invented a module.
	const accounts = url => {
		const parameter = url.indexOf("{");
		if (parameter < 0) return served.routes.has(url) ? [url] : [];
		const prefix = url.slice(0, parameter);
		return [...served.routes.keys()].filter(key => key.startsWith(prefix));
	};
	for (const url of declaredRoutes) {
		checks.ok(`the route ${url} was read`, accounts(url).length > 0, "dom.rs declares it and delivery() has nothing for it");
	}
	checks.is(
		"and every module read belongs to a route dom.rs declares",
		[...served.routes.keys()].filter(key => !declaredRoutes.some(url => accounts(url).includes(key))).sort(),
		[],
	);
	checks.ok("the import map parsed to an imports object", !!served.importMap.imports);
	checks.ok("the bootstrap is the _$HY installer", served.bootstrap.startsWith("<script>window._$HY||"));
	checks.ok("the loader imports hydrate", served.loader.includes('import { hydrate } from "topcoat-dom"'));
	// Two claims, not one. The separator and the source of the id are separate
	// things to get wrong, and the loader is free to put the id in a local on
	// the way past -- which it now does, so pinning one spelling of the
	// expression would report a rename as a hydration bug.
	checks.ok(
		"the loader's render id carries the separator",
		/renderId: `\$\{[A-Za-z0-9_.]+\}\.`/.test(served.loader),
		"without the separator, island i1 claims island i10's keys",
	);
	checks.ok(
		"and the id it renders is the mount's instance id",
		served.loader.includes("mount.dataset.tk"),
		"a render id from anywhere else stops islands on one page from being told apart",
	);
	for (const [bare, url] of Object.entries(served.importMap.imports)) {
		checks.ok(`the import map's ${bare} points at a route dom.rs serves`, served.routes.has(url), `${url} is not served`);
	}
}

// ---------- specifier rewriting ----------

const rewrites = new Map([
	["/demo/islands.js", "./islands.js"],
	["/demo/islands.js.map", "./islands.js.map"],
	["topcoat-dom", "./topcoat-dom.js"],
]);
checks.is(
	"a bare specifier becomes a relative path",
	rewriteSpecifiers('import { hydrate } from "topcoat-dom";', rewrites),
	'import { hydrate } from "./topcoat-dom.js";',
);
checks.is(
	"a dynamic import is rewritten too",
	rewriteSpecifiers('const m = await import("/demo/islands.js");', rewrites),
	'const m = await import("./islands.js");',
);
checks.is(
	"a specifier that is a prefix of another is not corrupted",
	rewriteSpecifiers('a("/demo/islands.js.map");b("/demo/islands.js");', rewrites),
	'a("./islands.js.map");b("./islands.js");',
);
checks.is("single-quoted specifiers are rewritten too", rewriteSpecifiers("import 'topcoat-dom';", rewrites), "import './topcoat-dom.js';");
checks.is(
	"a string that merely contains a specifier is left alone",
	rewriteSpecifiers('const label = "the topcoat-dom runtime";', rewrites),
	'const label = "the topcoat-dom runtime";',
);
checks.is(
	"a bare specifier left behind is reported",
	unresolvedSpecifiers('import { a } from "solid-js";\nimport { b } from "./x.js";'),
	["solid-js"],
);
checks.is(
	"relative, absolute and URL specifiers are not reported",
	unresolvedSpecifiers('import "./a.js";import "/demo/b.js";import "file:///c.js";'),
	[],
);
checks.is(
	"a bare dynamic import is reported",
	unresolvedSpecifiers('await import("solid-js");'),
	["solid-js"],
);

// ---------- tree parity ----------

const TEMPLATE = '<div class="island"><p class="island-count">count <!$><!/></p><button type="button">+1</button></div>';

function serverTree(html) {
	const host = document.createElement("div");
	host.innerHTML = html;
	return host.firstElementChild;
}

const good = templateParity(
	document,
	TEMPLATE,
	serverTree('<div data-hk="i0.0" class="island"><p class="island-count">count <!--$-->5<!--/--></p><button type="button">+1</button></div>'),
);
checks.is("a matching tree has no differences", good.differences, []);
checks.is("the hole's content is reported", good.holes.map(hole => hole.content), ["5"]);

checks.ok(
	"the long and short marker syntaxes are the same node",
	templateParity(document, "<p><!$><!/></p>", serverTree("<p><!--$--><!--/--></p>")).ok,
);
checks.ok(
	"a missing element is caught",
	!templateParity(document, TEMPLATE, serverTree('<div class="island"><p class="island-count">count <!--$-->5<!--/--></p></div>')).ok,
);
checks.ok(
	"an extra server element is caught",
	!templateParity(document, "<div><p></p></div>", serverTree("<div><p></p><span></span></div>")).ok,
);
checks.ok(
	"a changed attribute is caught",
	!templateParity(document, '<div class="island"></div>', serverTree('<div class="isle"></div>')).ok,
);
checks.ok(
	"an unexpected server attribute is caught",
	!templateParity(document, "<div></div>", serverTree('<div title="x"></div>')).ok,
);
checks.ok("data-hk is the one attribute the server may add", templateParity(document, "<div></div>", serverTree('<div data-hk="i0.0"></div>')).ok);
checks.ok(
	"changed static text is caught",
	!templateParity(document, "<p>count </p>", serverTree("<p>total </p>")).ok,
);
checks.ok(
	"a nested hole is skipped as one",
	templateParity(document, "<p><!$><!/></p>", serverTree("<p><!--$--><!--$-->x<!--/-->y<!--/--></p>")).ok,
);

// ---------- the negative control's perturbation ----------

const perturbed = perturb('<div data-hk="i0.0" class="island"></div>', "i0.");
const replacement = perturbed.html.match(/data-hk="([^"]+)"/)[1];
checks.ok("perturb replaces the first data-hk", replacement !== "i0.0");
checks.ok("with a key under the same render id, so it is still gathered", replacement.startsWith("i0."));
checks.ok("it says what it did", perturbed.what.includes("i0.0") && perturbed.what.includes(replacement));
checks.ok("and it leaves no original key behind", !perturbed.html.includes('data-hk="i0.0"'));

const treePerturbed = perturbTree(
	'<!doctype html><body><topcoat-island data-ti="c" data-tk="i0"><div data-hk="i0.0"><p></p></div></topcoat-island>',
	{ mount: "topcoat-island[data-ti]" },
);
checks.ok("perturbTree appends an element to the island root", /<div data-hk="i0\.0"><p><\/p><span><\/span><\/div>/.test(treePerturbed.html));
checks.ok("and the template comparison then rejects it", !templateParity(document, "<div><p></p></div>", serverTree('<div data-hk="i0.0"><p></p><span></span></div>')).ok);

// ---------- clone identity, per key ----------
//
// The parity run's IDENTITY check expects an empty list, and a check that expects
// nothing can pass by being unable to see anything. So the comparison is exercised
// here against a document where a keyed node HAS been swapped for a clone -- the
// exact failure the dom-test stub cannot see (wave-2 backend report: trace.mjs
// labels a node by the template it was cloned from, so a reused node and a fresh
// clone are indistinguishable there).
//
// Two keys and a nesting shape on purpose: the claim the nested fixture makes is
// that identity survives a component boundary, so the interesting case is the
// INNER node being rebuilt while the outer one was adopted, which is what a
// mis-numbered component context produces.

const identityPage = new JSDOM('<!doctype html><body><topcoat-island data-tk="i0"><div data-hk="i0.0"><span data-hk="i0.10">a</span></div></topcoat-island>');
const identityMount = identityPage.window.document.querySelector("topcoat-island");
const keyedBefore = new Map([...identityMount.querySelectorAll("[data-hk]")].map(element => [element.getAttribute("data-hk"), element]));

checks.is("movedKeys reports nothing when every keyed node was adopted", movedKeys(identityMount, keyedBefore), []);

const inner = identityMount.querySelector('[data-hk="i0.10"]');
inner.replaceWith(inner.cloneNode(true));
checks.is(
	"and it names the INNER key when only the node below the boundary was rebuilt",
	movedKeys(identityMount, keyedBefore),
	["i0.10"],
);
checks.ok(
	"which a serialized-HTML comparison cannot see: the markup is byte-identical",
	identityMount.innerHTML === '<div data-hk="i0.0"><span data-hk="i0.10">a</span></div>',
);

const outer = identityMount.querySelector('[data-hk="i0.0"]');
outer.remove();
checks.is(
	"a key that vanished counts as moved too, so a dropped subtree is not silently fine",
	movedKeys(identityMount, keyedBefore).sort(),
	["i0.0", "i0.10"],
);

// ---------- the size budgets ----------
//
// The band arithmetic and the three-way verdict, checked without measuring
// anything: `budgets.mjs` measures the real delivery, and a unit test that also
// measured it would only be able to agree with it.

checks.is("a band is proportional once the proportion beats the floor", band(10000, { tolerance: 0.05, floorBytes: 128 }), { low: 9500, high: 10500, slack: 500 });
checks.is("and the floor carries a subject too small for the proportion to matter", band(307, { tolerance: 0.05, floorBytes: 128 }), { low: 179, high: 435, slack: 128 });
checks.is("gzip sizes are the pinned level's, and raw is the byte length", sizes("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").raw, 30);
checks.ok("gzip is smaller than raw for compressible input", sizes("a".repeat(4096)).gzip < 4096);

const budgetPolicy = { tolerance: 0.05, floorBytes: 10 };
const budgetServed = { routes: new Map([["/x.js", "a".repeat(1000)], ["/y.js", "b".repeat(1000)]]) };
const budgetBaseline = sizes("a".repeat(1000)).gzip;
checks.is("a subject at its baseline is ok", judge(budgetServed, { artifacts: ["/x.js"], gzip: budgetBaseline, raw: 1000, measured: "x" }, budgetPolicy).state, "ok");
checks.is("one over the band is OVER", judge(budgetServed, { artifacts: ["/x.js"], gzip: budgetBaseline - 11, raw: 1000, measured: "x" }, budgetPolicy).state, "over");
checks.is(
	"and one under the band is UNDER, not a pass -- a chunk that shrank lost something",
	judge(budgetServed, { artifacts: ["/x.js"], gzip: budgetBaseline + 11, raw: 1000, measured: "x" }, budgetPolicy).state,
	"under",
);
checks.is("a subject with no baseline is unmeasured, which is not ok either", judge(budgetServed, { artifacts: ["/x.js"] }, budgetPolicy).state, "unmeasured");

// COMPOSITION, which is the state that came out of a real event: a second island
// landed in demo-app's shared bundle and `island:counter` jumped 969 -> 1422 gzip
// with nothing about the counter having changed. A bare number cannot say that, and
// re-baselining to 1422 would have recorded a false claim about the counter's size.
const twoIslands = { routes: new Map([["/i.js", "function __island_counter(a) {\n}\nfunction __island_nested(b) {\n}\n"]]) };
checks.is("the islands in an artifact are read off its entry points", islandsIn(twoIslands, { artifacts: ["/i.js"] }), ["counter", "nested"]);
const composed = judge(twoIslands, { artifacts: ["/i.js"], containsIslands: ["counter"], gzip: 1, raw: 1, measured: "then" }, budgetPolicy);
checks.is("a subject whose artifact gained an island reports composition, not a size verdict", composed.state, "composition");
checks.ok("and names which island appeared", composed.why.includes("Appeared: nested"));
checks.ok("and says the size move is not a regression in that artifact", composed.why.includes("not a regression"));
checks.is(
	"composition is checked BEFORE size, so a changed set is never reported as a size regression",
	judge(twoIslands, { artifacts: ["/i.js"], containsIslands: ["counter"], gzip: 99999, raw: 1, measured: "then" }, budgetPolicy).state,
	"composition",
);
checks.is(
	"a subject whose composition still matches is judged on size as usual",
	judge(twoIslands, { artifacts: ["/i.js"], containsIslands: ["counter", "nested"], gzip: sizes(twoIslands.routes.get("/i.js")).gzip, raw: 1, measured: "then" }, budgetPolicy).state,
	"ok",
);
checks.is("a pending subject is pending", judge(budgetServed, { artifacts: ["/x.js"], pending: "no island yet" }, budgetPolicy).state, "pending");
checks.is(
	"a subject is the SUM of its artifacts, summed per artifact and not gzipped as one stream",
	judge(budgetServed, { artifacts: ["/x.js", "/y.js"], gzip: budgetBaseline * 2, raw: 2000, measured: "x" }, budgetPolicy).measured.gzip,
	sizes("a".repeat(1000)).gzip + sizes("b".repeat(1000)).gzip,
);
let budgetNamed = null;
try {
	measure(budgetServed, { artifacts: ["/gone.js"] });
} catch (error) {
	budgetNamed = error.message;
}
checks.ok("an artifact the server does not serve fails loudly and says so", budgetNamed?.includes("/gone.js") && budgetNamed.includes("dom.rs does not serve"));

// The coverage check, which is the one that makes budgets.json hard to leave
// stale, and the one whose first implementation misfired: a regex meant for
// `/demo/islands.js` matched `/demo/island-loader.js` and reported chunking as
// landed. Asked from the other side now -- "is this route one budgets.json names"
// -- so there is no guess about what a chunk will be called.
const budgetRecorded = { subjects: { a: { artifacts: ["/demo/islands.js"] } }, $notBudgeted: { "/demo/islands.js.map": "map" }, chunking: { landed: false, islandArtifact: "/demo/islands.js" } };
const budgetAsToday = { routes: new Map([["/demo/islands.js", ""], ["/demo/islands.js.map", ""]]) };
checks.is("every served module named: no unbudgeted routes", unbudgetedRoutes(budgetAsToday, budgetRecorded), []);
checks.ok("so the chunking flag agrees", chunkingState(budgetAsToday, budgetRecorded).agrees);
checks.ok(
	"the real loader route does NOT read as a chunk, which the first attempt got wrong",
	unbudgetedRoutes({ routes: new Map([["/demo/islands.js", ""], ["/demo/island-loader.js", ""]]) }, budgetRecorded).length === 1,
);
const budgetChunked = { routes: new Map([["/demo/islands.js", ""], ["/demo/chunks/counter.js", ""], ["/demo/chunks/shared.js", ""]]) };
checks.ok("a chunked delivery is caught as unbudgeted", chunkingState(budgetChunked, budgetRecorded).why.includes("/demo/chunks/shared.js"));
checks.ok("and the message says to split the subjects and add the shared chunk", chunkingState(budgetChunked, budgetRecorded).why.includes("shared chunk"));
checks.ok(
	"the island module disappearing is a different message from a new one appearing",
	chunkingState({ routes: new Map() }, budgetRecorded).why.includes("does not serve it"),
);

// ---------- the procedure wire, and the stub built from it ----------
//
// `wireParity` exists to stop a fixture's fetch stub drifting from
// procedure-wire.json. A cross-check that cannot fail would not do that, and this
// one has a specific reason to be doubted: the value it guards MOST closely --
// the rendered argument path in a decode error -- was already wrong once, written
// `1` here and observed `[1]` by the server. So every field it compares is
// perturbed and asserted to be caught by name.

const sampleWire = JSON.parse(readFileSync(join(import.meta.dirname, "lib", "wire-sample.json"), "utf8")).wire;

checks.is("the sample wire block agrees with procedure-wire.json", wireParity(sampleWire).differences, []);
checks.ok(
	"and the run is told the error vector was corrected, so nobody re-derives the old spelling",
	wireParity(sampleWire).notes.some(note => note.includes("$corrected")),
);

const caught = field => {
	const perturbedWire = structuredClone(sampleWire);
	if (field === "vectors.error") perturbedWire.vectors.error = "no-such-vector";
	else perturbedWire[field] = typeof perturbedWire[field] === "number" ? perturbedWire[field] + 1 : `${perturbedWire[field]}-wrong`;
	return wireParity(perturbedWire).differences;
};

for (const field of ["requestContentType", "responseContentType", "zeroArguments", "routePrefix", "method", "errorBodyStartsWith", "errorBodyEndsWith", "errorStatus", "errorContentType"]) {
	checks.ok(`a wrong ${field} is caught and both sides are named`, caught(field).length === 1 && caught(field)[0].includes("fixture"));
}
checks.ok("a vector id that does not exist is caught", caught("vectors.error").some(difference => difference.includes("no-such-vector")));

// THE PATH RULE, validated then applied. The fixture's procedure takes ONE argument
// so its bad-argument path is `[0]`, while the vector pins a TWO-argument signature
// and records `[1]`. Copying the vector's literal would make the fixture stub an
// error the real server never sends -- which the framework agent's live curl run
// against POST /demo/search confirmed answers `[0]`. So the rule is first checked
// against the vector's own recorded suffix, which is a closed loop, and only then
// applied at the fixture's own index.
checks.is("the fixture's error suffix is the rule at ITS argument index, not the vector's literal", sampleWire.errorBodyEndsWith, "(at `[0]`)");
checks.is("and the vector it validates the rule against records a different index", wireSpec().vectors.find(vector => vector.id === sampleWire.vectors.error).expect.bodyEndsWith, "(at `[1]`)");
checks.ok(
	"a fixture whose suffix does not match the rule at its own index is caught",
	caught("errorBodyEndsWith")[0].includes("the path rule at argument index 0"),
);
const ruleBroken = structuredClone(sampleWire);
ruleBroken.errorVectorArgumentIndex = 5;
checks.ok(
	"and a rule that stops reproducing the VECTOR's suffix is caught separately, naming the vector",
	wireParity(ruleBroken).differences.some(difference => difference.includes("does not reproduce vector serde-wrong-argument-type")),
);
checks.ok(
	"with an explicit instruction not to fix it by editing the fixture",
	wireParity(ruleBroken).differences.some(difference => difference.includes("do not fix this by editing the fixture")),
);

// The call target. Two shapes, because a compiled island cannot use the procedure
// route: the id is minted per expansion and an island's file is expanded twice.
checks.is("the sample wire targets a written-down path, not the procedure route", sampleWire.callTarget.kind, "exactPath");
const badTarget = structuredClone(sampleWire);
badTarget.callTarget = { kind: "exactPath" };
checks.ok("an exactPath target with no path is caught", wireParity(badTarget).differences.some(difference => difference.includes("no `path` is declared")));
badTarget.callTarget = { kind: "somethingElse", path: "/x" };
checks.ok("and an unknown target kind is caught", wireParity(badTarget).differences.some(difference => difference.includes("not one of")));

// The stub. Its job is to record what was sent before answering, and to refuse to
// invent a reply -- an island that made one more call than the fixture planned has
// learned something and must not be told it passed.
const stub = fetchStub(sampleWire, [{ status: 200, contentType: "application/json", body: '["rust"]' }]);
const answered = await stub.fetch(sampleWire.callTarget.path, {
	method: "POST",
	headers: { "Content-Type": sampleWire.requestContentType },
	body: '["ru"]',
});
checks.is("the stub records the decoded arguments, not the byte string", stub.calls[0].args, ["ru"]);
checks.is("and the content type it was sent", stub.calls[0].contentType, sampleWire.requestContentType);
checks.ok("a call to the fixture's declared path is on target", stub.calls[0].onTarget);
checks.ok("and that path is NOT the procedure route, so the two are told apart", !stub.calls[0].isProcedureRoute);
checks.is("the reply's body is readable as text", await answered.text(), '["rust"]');
checks.is("and its status is the queued one", answered.status, 200);
checks.is("the queue is now empty", stub.pending, 0);

let overflowed = null;
try {
	await stub.fetch(sampleWire.callTarget.path, { method: "POST", body: "[]" });
} catch (error) {
	overflowed = error.message;
}
checks.ok("a call past the end of the queue throws rather than inventing a reply", overflowed?.includes("the fixture queued"));

// The route-shape test, which is the whole reason the fixture does not assert a
// literal id: two segments is not a procedure route, and neither is a bare prefix.
const shapes = fetchStub(sampleWire, [{ status: 200, contentType: "application/json", body: "null" }, { status: 200, contentType: "application/json", body: "null" }]);
await shapes.fetch(`${sampleWire.routePrefix}/a/b`, { method: "POST", body: "[]" });
await shapes.fetch(`${sampleWire.routePrefix}/`, { method: "POST", body: "[]" });
checks.ok("two path segments is not a procedure route", !shapes.calls[0].isProcedureRoute);
checks.ok("the prefix plus exactly one segment IS one, which is all that is knowable about a uuid id", (await (async () => { const one = fetchStub(sampleWire, [{ status: 200, contentType: "application/json", body: "null" }]); await one.fetch(`${sampleWire.routePrefix}/018f-abc`, { method: "POST", body: "[]" }); return one.calls[0].isProcedureRoute; })()));
checks.ok("and neither is the prefix with an empty id", !shapes.calls[1].isProcedureRoute);

// ---------- the .d.ts goldens harness ----------
//
// The point of these is that the type-check layer has been SEEN to fail. A tsc
// invocation that has only ever run over correct input is not evidence that it
// would catch anything, and the failures it exists to catch (a declaration naming
// a type that does not exist, a consumer using an export the wrong way) are
// exactly the ones a structural golden diff passes.

const goodDts = [
	"export declare function mount(root: HTMLElement, initial: Count): void;",
	"export interface Count { value: number; label?: string }",
	"export declare const version: string;",
	""
].join("\n");

const structure = structureOf("good.d.ts", goodDts);
checks.is("structureOf finds every declaration", structure.exports.map(entry => `${entry.kind} ${entry.name}`).sort(), [
	"function mount",
	"interface Count",
	"variable version"
]);
checks.is("a parameter is recorded with its type", structure.exports.find(entry => entry.name === "mount").parameters, [
	{ name: "root", optional: false, type: "HTMLElement" },
	{ name: "initial", optional: false, type: "Count" }
]);
checks.is("an optional member is recorded as optional", structure.exports.find(entry => entry.name === "Count").members, [
	{ name: "value", type: "number", optional: false },
	{ name: "label", type: "string", optional: true }
]);
checks.ok("the summary is sorted, so a reordered emission is not a diff",
	structureOf("good.d.ts", `export declare const version: string;\nexport interface Count { value: number; label?: string }\nexport declare function mount(root: HTMLElement, initial: Count): void;\n`)
		.exports.map(entry => entry.name).join() === structure.exports.map(entry => entry.name).join());

const scaffolded = scaffold("good", structure);
checks.ok("a scaffolded consumer marks itself unreviewed", scaffolded.startsWith(SCAFFOLD_MARKER));
checks.ok("a scaffolded consumer binds every value export", scaffolded.includes("typeof Island.mount") && scaffolded.includes("typeof Island.version"));
checks.ok("a scaffolded consumer binds every type export", scaffolded.includes("_type_Count: Island.Count"));

if (!typescriptAvailable()) {
	checks.note("typescript is not fetched, so the type-check layer is untested here. Run contract/vendor/fetch.sh.");
} else {
	const tmp = mkdtempSync(join(tmpdir(), "topcoat-dts-"));
	try {
		// 1. A good declaration file plus a consumer of it: passes.
		writeFileSync(join(tmp, "good.d.ts"), goodDts);
		writeFileSync(join(tmp, "good-consumer.ts"), `import type * as Island from "./good";\ndeclare const m: typeof Island.mount;\nvoid m;\n`);
		const good = typeCheckFiles(tmp, [join(tmp, "good.d.ts"), join(tmp, "good-consumer.ts")]);
		checks.ok("tsc passes a well formed .d.ts and its consumer", good.ok, good.output);

		// 2. A declaration file that is byte-stable and still nonsense: it names a
		//    type it never declares. No golden diff would ever report this.
		writeFileSync(join(tmp, "dangling.d.ts"), "export declare function mount(root: NoSuchType): void;\n");
		const dangling = typeCheckFiles(tmp, [join(tmp, "dangling.d.ts")]);
		checks.ok("tsc catches a .d.ts naming a type it never declares", !dangling.ok);
		checks.ok("and says which name", dangling.output.includes("NoSuchType"), dangling.output);

		// 3. A consumer that uses a correct declaration incorrectly. This is the
		//    half a .d.ts checked on its own cannot reach.
		writeFileSync(join(tmp, "bad-consumer.ts"), `import type * as Island from "./good";\nconst c: Island.Count = { value: "not a number" };\nvoid c;\n`);
		const bad = typeCheckFiles(tmp, [join(tmp, "good.d.ts"), join(tmp, "bad-consumer.ts")]);
		checks.ok("tsc catches a consumer that misuses a correct declaration", !bad.ok);
		checks.ok("and it is the consumer that is blamed", bad.output.includes("bad-consumer.ts"), bad.output);
	} finally {
		rmSync(tmp, { recursive: true, force: true });
	}
}

// ---------- the check list itself ----------

const sample = new Checks("sample");
sample.is("a", 1, 1);
sample.is("b", 1, 2);
checks.is("Checks counts failures", sample.failures.length, 1);
checks.is("Checks counts passes", sample.passed, 1);
checks.ok("a failure report carries both sides", sample.report().includes("expected 2") && sample.report().includes("actual   1"));
checks.ok("Checks compares by shape, not identity", new Checks("x").is("deep", { a: [1] }, { a: [1] }));

console.log(checks.report());
if (checks.failures.length) process.exit(1);
