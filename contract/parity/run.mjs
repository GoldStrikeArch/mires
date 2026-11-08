// The SSR-to-client hydration parity run.
//
//   node run.mjs                  # every fixture, then every negative control
//   node run.mjs counter          # one fixture
//   node run.mjs --no-negative    # skip the negative controls
//   node run.mjs --verbose        # print the mutation records
//
// WHAT IT ESTABLISHES
// -------------------
// demo-app/smoke/*.mjs establish that the compiled island module works: driven
// against a hand-written stub, its signal, its handlers and its interpolation all
// behave. What they cannot establish is that hydration HAPPENS -- the stub has no
// registry, no `data-hk`, and no hydrating branch, so it answers a different
// question than the browser does.
//
// This run puts the real pieces together: the server's own response bytes, the
// server's own `_$HY` bootstrap executed as a script in the page, the pinned
// runtime bundle the page serves, the compiled island module from demo-app's
// build, and the server's own loader module running the hydrate bracket. Then it
// measures. The measurement that matters is the MutationObserver: hydration is
// supposed to ADOPT the server's DOM, and the difference between adopting it and
// rebuilding an identical copy of it is invisible in serialized HTML and obvious
// in mutation records.

import { readFileSync } from "node:fs";
import { basename, join } from "node:path";

import { buildHydrationScript } from "../fixtures/hydration-script.js";
import { budgetChecks, budgets } from "./budgets.mjs";
import { Checks } from "./lib/check.mjs";
import { SPIKE, captureFreshness, delivery, islandModuleUrl, islandTemplates, moduleTemplates } from "./lib/delivery.mjs";
import { open, watch } from "./lib/dom.mjs";
import { allocationOrder, contextTree, hydrationKey, keyChain, nestedKeyParity } from "./lib/keys.mjs";
import { stage, unresolvedSpecifiers } from "./lib/stage.mjs";
import { templateParity } from "./lib/tree.mjs";
import { wireParity } from "./lib/wire.mjs";
import { fixture, fixtureNames } from "./capture-ssr.mjs";

const FIXTURES = join(import.meta.dirname, "fixtures");

/**
 * The check the negative control has to trip.
 *
 * Named, because "some check failed" is not a pass condition for a negative
 * control -- a harness that failed for an unrelated reason would look like it
 * was working. The control perturbs a hydration key, so THIS is the check that
 * has to notice.
 */
const CLAIMED = "every key the server wrote was claimed off the server's own node";

/** The check that decides whether the client's template matches the server's tree. */
const TREE = "the client template is the tree the server sent, node for node";

/**
 * The check that decides whether the keys are keys at all.
 *
 * Distinct from CLAIMED: that one says the client asked for the keys the server
 * wrote, which is about agreement between two sides. This one says the keys are
 * ones the allocation scheme can produce and that they imply the component
 * boundaries the source has, which is about each side being right on its own.
 */
const SCHEME = "every key the server wrote is one the allocation scheme can produce, and they nest as the source does";

/**
 * The check that decides whether hydration ADOPTED the server's nodes or rebuilt
 * them, PER KEY.
 *
 * Here because the wave-2 backend report flagged clone identity as a limit of the
 * dom-test stub and delegated it to this harness: `harness/trace.mjs` labels a
 * node by the template it was cloned from, so three reused nodes and three fresh
 * clones read identically there. This harness holds real node objects, so it can
 * just ask.
 *
 * WHAT IT ADDS OVER THE CHECKS AROUND IT, stated precisely because it is
 * attribution rather than detection and overclaiming it would be the kind of thing
 * this harness is supposed to catch. A clone swapped in for a server node also
 * fails CLAIMED (the original is not in `_$HY.completed`) and also fails "every
 * node the server sent is still the node that is there" (the clone is a node that
 * was not there before). Neither of those says WHICH node. For a single-template
 * island that hardly matters -- there is one candidate. For a nesting island
 * "some node changed" is not a usable diagnosis and "the node at key i7.110, the
 * badge inside the card, is not the object the server sent" is, because it names
 * the boundary. That is the whole difference between this fixture finding a
 * component-context bug and reporting a symptom of one.
 */
const IDENTITY = "every keyed node is still the same node object, on both sides of every component boundary";

/**
 * The negative controls, each an inversion of one thing parity claims.
 *
 * Two of them, because the two claims fail differently and a control that only
 * covered one would leave the other free to be vacuous:
 *
 * - `key` breaks the hydration key. The runtime notices and silently walks a
 *   clone, so nothing changes in the DOM and only `_$HY.completed` shows it.
 * - `tree` adds a node to the server's markup. The runtime does NOT notice --
 *   this particular extra node is past every walk the counter performs, so
 *   hydration still reports zero mutations and the island still works. Only the
 *   tree comparison catches it. That is the whole reason the tree comparison is
 *   in the harness: it is the check that sees drift BEFORE it becomes a bug.
 */
const CONTROLS = {
	key: { mustFail: CLAIMED, probe: "dead", describe: "a perturbed data-hk on the island root" },
	// The third control, and it exists because running the first one against the
	// nested fixture showed the fixture could not test what it was built to test.
	// `perturb` takes the FIRST `data-hk`, which is the island's own root, so the
	// island dies whole -- the loudest possible failure. The nested fixture's `dead`
	// probe was written for the QUIET one: perturb a key BELOW a component boundary
	// and the island above it may keep working while one subtree is inert. That is
	// the failure a person would ship, and no control reached it.
	//
	// Skipped for a fixture with no components, where the deepest key IS the root and
	// this would be the `key` control under another name -- which is the kind of
	// duplicate control that makes a suite look stronger than it is.
	deepKey: { mustFail: CLAIMED, probe: "dead", describe: "a perturbed data-hk on the DEEPEST nested key", needsComponents: true },
	tree: { mustFail: TREE, probe: null, describe: "an extra node in the server's markup" },
};

/**
 * The hydration key for an ordinal under a render id.
 *
 * From `contract/fixtures/keys.json` `$algorithm`: the numeric suffix is
 * prefixed by a letter encoding its digit length minus one, which is what makes
 * concatenated segments unambiguously separable. `test.mjs` checks this against
 * the 264 recorded triples, so it cannot drift from the runtime's own numbering.
 *
 * Re-exported from `lib/keys.mjs`, which owns the encoding because it also owns
 * the inverse: once components nest, a key has to be read back into the chain of
 * contexts it came through, and one implementation of the letter rule has to
 * serve both directions.
 */
export { hydrationKey };

/**
 * A fixture whose island does not exist yet.
 *
 * A pending fixture is not a skipped one. Everything about it that does not need
 * the server's bytes is checked NOW, so that the day the island lands the
 * expectations it is measured against have already been validated against the
 * oracle rather than written from memory in a hurry. What is checked is that the
 * declared key sequence is a sequence the SCHEME can produce, that it implies the
 * number of component boundaries the declared source shape has, and that it is
 * the sequence `keys-nested.json` recorded for the corpus case the fixture says
 * it mirrors.
 *
 * The fixture's `pending` block says what makes it live. Nothing here fails; the
 * run reports it as pending and moves on.
 */
function pendingFixture(name, declared, checks) {
	const pending = declared.pending;
	checks.note(`PENDING: ${pending.why}`);
	checks.note(`flips live when ALL of these hold:\n${pending.flipsWhen.map((step, at) => `        ${at + 1}. ${step}`).join("\n")}`);
	checks.note(`then: ${pending.thenRun}`);

	// A pending fixture has no chunk of its own to measure -- before per-island
	// chunking its island is not in `islands.js` at all, and after it its chunk does
	// not exist. So the budget subject is pending too, and the two pendings are
	// asserted to agree: a fixture that went live while its budget stayed pending
	// would ship an unmeasured chunk, which is the one way this could rot quietly.
	const budget = budgets().subjects[`island:${name}`];
	checks.ok(
		`budgets.json has an island:${name} subject and it is pending too, as this fixture is`,
		!!budget?.pending,
		budget ? "the subject exists but carries a baseline: it was measured against a delivery that does not contain this island" : `no island:${name} subject at all`,
	);

	// A procedure-calling fixture's whole contract with the server is checkable
	// without the island, because the wire is specified independently of it. This is
	// the same argument as the key oracle below, applied to a different oracle: the
	// day the island lands, the stub it is measured against has already been agreed
	// with `procedure-wire.json` rather than written from memory. It also runs on the
	// LIVE path, so the agreement is asserted every run and not only while pending.
	if (declared.wire) wireChecks(declared.wire, checks);

	// `mirrors: null` is a fixture with no key oracle to mirror, which is the honest
	// state for an island with no components: `keys-nested.json` records nesting, and
	// there is nothing to nest. Its keys are still put through the scheme below --
	// what it skips is only the byte-for-byte comparison against a recorded case.
	const oracle = JSON.parse(readFileSync(join(SPIKE, "contract", "fixtures", "keys-nested.json"), "utf8"));
	const mirrored = pending.mirrors === null ? null : oracle.cases.find(record => record.name === pending.mirrors);
	if (pending.mirrors === null) {
		checks.note(`no key oracle: ${declared.expected.components} component(s), so there is no recorded nesting case to mirror. The scheme checks below still apply.`);
	} else if (!checks.ok(`the corpus case it mirrors, ${pending.mirrors}, is in keys-nested.json`, !!mirrored)) {
		return checks;
	}

	// Declared relative to the render id, since the render id is the server's to
	// choose. Checked under a stand-in that is nothing like a key, so a harness
	// that ignored the render id could not accidentally pass.
	const renderId = "i7.";
	const keys = declared.expected.keys.map(key => `${renderId}${key}`);

	const tree = contextTree(keys, renderId);
	checks.is("the declared key sequence is one the scheme can produce", tree.problems, []);
	checks.is(
		"and it is in allocation order, so the fixture will compare against the order the emitter spends slots in",
		allocationOrder(keys, renderId),
		keys,
	);
	checks.is(
		"the boundaries it implies are the components the declared source has",
		tree.boundaries.length + tree.contexts.flatMap(context => context.gaps.map(count => hydrationKey(context.id, count))).filter(key => !tree.boundaries.includes(key)).length,
		declared.expected.components,
	);
	if (mirrored) {
		checks.is(
			`it is the sequence the oracle recorded for ${pending.mirrors}, render id apart`,
			declared.expected.keys,
			mirrored.elementKeys,
		);
	}
	checks.is(
		"the render id is a prefix of every key, so the loader's gather finds them all",
		keys.filter(key => !key.startsWith(renderId)),
		[],
	);
	checks.note(
		`the declared keys under a render id of ${JSON.stringify(renderId)} are ${JSON.stringify(keys)},\n`
		+ `        through ${tree.boundaries.length} component boundary/boundaries opening contexts ${JSON.stringify(tree.nested)}`,
	);

	return checks;
}

/**
 * The wire a procedure-calling fixture declares, checked against the vectors it
 * names.
 *
 * Run on BOTH paths -- pending and live -- because the failure it prevents is not
 * "the fixture was written wrong once", it is "the spec moved and the fixture kept
 * stubbing the old wire". A stub is a second statement of the wire, and the wave-2
 * notes record what a second statement costs: the rendered argument path was
 * written `1` here and observed `[1]` by the server, and nothing but a real HTTP
 * test could tell. So the fixture names vectors and this reads the values off them.
 */
function wireChecks(wire, checks) {
	const parity = wireParity(wire);
	checks.is(
		`the fixture's wire block is the ${wire.wire} wire procedure-wire.json specifies, vector for vector`,
		parity.differences,
		[],
	);
	checks.is(
		"every vector it names exists",
		Object.entries(wire.vectors).filter(([role]) => !parity.vectors[role]).map(([role, id]) => `${role} -> ${id}`),
		[],
	);
	for (const note of parity.notes) checks.note(note);
}

/**
 * Change one `data-hk` in the server HTML to a key the client will never ask
 * for.
 *
 * The new key still starts with the render id, so `gatherHydratable` still puts
 * it in the registry -- which is what makes this a sharp control. The failure is
 * not "nothing was registered", it is "the client asked for a key that is not
 * there while a key nobody asked for sat in the registry", and a harness that
 * cannot tell those apart is not measuring hydration.
 */
export function perturb(html, renderId) {
	const match = html.match(/ data-hk="([^"]+)"/);
	if (!match) throw new Error("the server HTML carries no data-hk to perturb");
	const from = match[1];
	// A different ordinal under the same render id: gathered, never requested.
	const to = hydrationKey(renderId, 7777);
	if (from === to) throw new Error("the perturbed key equals the real one");
	return { html: html.replace(match[0], ` data-hk="${to}"`), what: `data-hk="${from}" became data-hk="${to}"`, from, to, which: "root" };
}

/**
 * Break the key of the node furthest DOWN the context chain.
 *
 * Depth is measured with `keyChain`, not by string length. Those usually agree and
 * are not the same thing: a key's length also grows with the ordinal it encodes
 * (`a10` is longer than `0` at the same depth), so sorting on length would
 * sometimes pick a shallow node with a big ordinal and quietly turn this control
 * back into the `key` control. Reading the chain asks the question that is actually
 * meant.
 *
 * The replacement stays inside the SAME parent context -- ordinal 7777 of the
 * deepest context, not a fresh top-level key -- so the failure is as narrow as
 * possible: everything about the context chain is right and only the last slot is
 * wrong. A control that also moved the node out of its context would prove less,
 * because two things would be broken and either could be the cause.
 */
export function perturbDeepest(html, renderId) {
	const keys = [...html.matchAll(/ data-hk="([^"]+)"/g)]
		.map(match => ({ match: match[0], key: match[1], chain: keyChain(match[1], renderId) }))
		.filter(entry => entry.chain);
	if (!keys.length) throw new Error(`the server HTML carries no data-hk parseable under render id ${JSON.stringify(renderId)}`);

	const deepest = keys.reduce((best, entry) => (entry.chain.length > best.chain.length ? entry : best), keys[0]);
	if (deepest.chain.length < 2) {
		throw new Error(`the deepest key ${JSON.stringify(deepest.key)} is only ${deepest.chain.length} slot(s) deep, so there is no nested key to perturb; this control needs a fixture with components`);
	}
	const context = deepest.chain.at(-1).contextId;
	const to = hydrationKey(context, 7777);
	if (deepest.key === to) throw new Error("the perturbed key equals the real one");
	return {
		html: html.replace(deepest.match, ` data-hk="${to}"`),
		what: `data-hk="${deepest.key}" became data-hk="${to}" -- ${deepest.chain.length} contexts deep, still inside context ${JSON.stringify(context)}`,
		from: deepest.key,
		to,
		which: "deepest",
		depth: deepest.chain.length,
	};
}

/**
 * Add one element to the server's markup that the client's template does not
 * declare.
 *
 * Done through the DOM rather than by string surgery so it works for any fixture:
 * whatever the island's root is, this appends a `<span>` to it. Appended rather
 * than inserted on purpose -- an extra node at the END is past every walk the
 * counter performs, so hydration cannot notice it and the tree comparison is the
 * only thing standing between this and a template that has quietly diverged.
 */
export function perturbTree(html, declared) {
	const page = open(html);
	try {
		const mount = page.document.querySelector(declared.mount);
		const root = mount.firstElementChild;
		root.appendChild(page.document.createElement("span"));
		return { html: page.serialize(), what: `appended a <span> to <${root.nodeName.toLowerCase()}>, the island root` };
	} finally {
		page.restore();
	}
}

/**
 * Run one fixture.
 *
 * @param {string} name a directory under fixtures/
 * @param {{control?: "key" | "tree", verbose?: boolean}} options
 * @returns {Promise<Checks>}
 */
/** How long the loader gets to hydrate an island before the run calls it lazy. */
export const SETTLE_MS = 2000;

/**
 * Wait until the loader says it has hydrated `mount`.
 *
 * The loader sets `data-tl-hydrated` after its bracket returns, so this polls
 * that rather than sleeping a fixed time: a delay long enough to be safe on a
 * slow machine makes every run pay for it, and one short enough not to is a
 * flake waiting to happen.
 *
 * @param {Element} mount
 * @param {number} deadline
 * @returns {Promise<boolean>} whether it happened in time
 */
export async function settled(mount, deadline = SETTLE_MS) {
	const until = Date.now() + deadline;
	while (Date.now() < until) {
		if (mount.dataset.tlHydrated !== undefined) return true;
		await new Promise(resolve => setTimeout(resolve, 5));
	}
	return mount.dataset.tlHydrated !== undefined;
}

export async function runFixture(name, options = {}) {
	const declared = fixture(name);
	const expected = declared.expected;
	const control = options.control ? CONTROLS[options.control] : null;
	if (options.control && !control) throw new Error(`no such negative control: ${options.control}`);
	const checks = new Checks(control ? `${name} -- negative control: ${options.control}` : name);

	if (declared.pending) return pendingFixture(name, declared, checks);

	// ---- what the server delivers, read out of the server ----
	const served = delivery();
	// The island under test has its own chunk now, resolved through the page's
	// import map. Reading the whole delivery instead would re-introduce exactly the
	// mistake `islandTemplates` was written for: counting another island's templates
	// as this one's.
	const islandUrl = islandModuleUrl(served, declared.island);
	const islandSource = served.routes.get(islandUrl);

	// The contract's bootstrap ends in `<!--xs-->`, the marker that terminates the
	// script section for solid's STREAMING parser. dom.rs omits it, which is
	// sound -- this emitter does no streaming, and the marker is meaningless to a
	// client that never sees a stream -- but it is a deliberate delta from the
	// extracted artifact, so it is subtracted by name rather than tolerated by a
	// loose comparison.
	const contractBootstrap = buildHydrationScript(declared.events);
	const STREAMING_TERMINATOR = "<!--xs-->";
	checks.ok(
		"the contract's bootstrap ends in the streaming terminator",
		contractBootstrap.endsWith(STREAMING_TERMINATOR),
		"if this fails the delta below is no longer the delta it documents",
	);
	// Compared against the CAPTURED page and not against dom.rs's `BOOTSTRAP`
	// constant. The delegated event list is per page now -- every name costs a
	// document listener for the whole pre-hydration window, so a page installs the
	// ones its own islands answer -- and that constant is only the default list.
	// Checking every page against it would hold for the two pages that take the
	// default and be wrong about the one that does not, which is precisely the
	// page this matters on: `search` is the first fixture delegating anything but
	// `click`, and getting it from the page is what makes the claim per page.
	const wanted = contractBootstrap.slice(0, -STREAMING_TERMINATOR.length);

	let html = readFileSync(join(FIXTURES, name, "ssr.html"), "utf8");
	checks.ok(
		"the page serves the contract's bootstrap for ITS OWN event list, less the streaming terminator",
		html.includes(wanted),
		`the page does not carry ${JSON.stringify(declared.events)}'s bootstrap.\n`
			+ `        wanted: ${wanted}\n`
			+ `        dom.rs's default-page BOOTSTRAP constant is ${served.bootstrap === wanted ? "this same list" : "a different list, which is fine unless this fixture takes the default"}`,
	);

	// Anything the page says about its own delivery is only evidence if the binary
	// that sent it was built from the sources dom.rs now has.
	const freshness = captureFreshness();
	if (freshness.fresh) {
		checks.ok(
			"the captured response carries the import map dom.rs now serves",
			html.includes(JSON.stringify(served.importMap)),
			`page must contain ${JSON.stringify(served.importMap)}`,
		);
	} else {
		checks.note(
			`NOT CHECKED: the page's import map, because the capture is stale. ${freshness.binary === null ? "demo-app is not built" : `${freshness.stale.length} input(s) are newer than the binary, newest ${freshness.stale[0].file}`}.\n`
			+ "        Rebuild demo-app and re-run capture-ssr.mjs to check it. Everything below reads the runtime,\n"
			+ "        the loader and the compiled island from disk, so it is unaffected.",
		);
	}

	// ---- what this fixture's delivery costs a browser ----
	budgetChecks(name, served, checks);

	// ---- the wire, if this island talks to the server ----
	// Before the DOM work, deliberately: if the fixture's stub no longer matches the
	// spec then everything the interaction phase concludes about the island's calls
	// is a conclusion about the wrong wire, and it is better to say so at the top of
	// the report than to have it explain a confusing failure at the bottom.
	if (declared.wire) wireChecks(declared.wire, checks);

	// ---- the module graph, linked the way the import map links it ----
	const staged = stage(served);
	const leaked = unresolvedSpecifiers(readFileSync(join(staged.dir, staged.locations.get(islandUrl)), "utf8"));
	checks.is("every specifier the compiled module imports is resolved by the import map", leaked, []);

	// ---- the render id, and what the client will ask for ----
	// Read verbatim from data-tk rather than recomputed. The server owns the
	// instance id; a harness that recomputed it would agree with itself while
	// disagreeing with the page.
	const probe = open(html);
	const probeMount = probe.document.querySelector(declared.mount);
	if (!probeMount) {
		probe.restore();
		checks.ok(`the captured response contains ${declared.mount}`, false);
		return checks;
	}
	const renderId = `${probeMount.dataset.tk}.`;
	probe.restore();

	let perturbed = null;
	if (control) {
		const perturbations = {
			key: () => perturb(html, renderId),
			deepKey: () => perturbDeepest(html, renderId),
			tree: () => perturbTree(html, declared),
		};
		perturbed = perturbations[options.control]();
		html = perturbed.html;
		checks.note(`PERTURBED: ${perturbed.what}`);
		checks.note(`this run passes only if ${JSON.stringify(control.mustFail)} fails`);
	}

	const page = open(html);
	const { document, window } = page;
	try {
		// ---- the bootstrap ran, and installed what the runtime reads ----
		const hy = window._$HY;
		checks.ok("the page's bootstrap installed _$HY", !!hy);
		checks.is("_$HY.events starts empty", hy?.events?.length, 0);
		checks.ok("_$HY.completed is a WeakSet", hy?.completed instanceof window.WeakSet);
		checks.ok("_$HY.r exists, so the client's unconditional read of it is safe", !!hy && typeof hy.r === "object");
		checks.ok("_$HY.fe is callable", typeof hy?.fe === "function");
		checks.ok("_$HY has no `done` yet -- the client assigns it", !!hy && !("done" in hy));

		const mount = document.querySelector(declared.mount);
		const seeds = JSON.parse(mount.dataset.ts || "[]");
		checks.is("the seeds the server wrote are the island's arguments", seeds, expected.seeds);
		checks.is("the render id is the instance id plus a separator", renderId, `${mount.dataset.tk}.`);

		// What `gatherHydratable` will collect: every data-hk under the mount whose
		// key starts with the render id.
		const hydratable = [...mount.querySelectorAll("[data-hk]")].filter(element => element.getAttribute("data-hk").startsWith(renderId));
		checks.ok("the server wrote at least one hydration key under the render id", hydratable.length > 0);
		if (options.control !== "key") {
			checks.is(
				"the first key is the render id's ordinal zero",
				hydratable[0]?.getAttribute("data-hk"),
				hydrationKey(renderId, 0),
			);
		}

		// ---- the keys are keys the SCHEME can produce, nesting included ----
		// The counter has one context, so this is nearly the check above. It is
		// here anyway because it is the check that has to hold for a fixture with
		// components, and a claim that only runs on the fixture it was written for
		// is a claim nobody has run.
		const nested = nestedKeyParity(mount, {
			renderId,
			expected: options.control === "key" || !expected.keys ? undefined : expected.keys.map(key => `${renderId}${key}`),
			components: expected.components,
		});
		checks.is(SCHEME, nested.differences, []);
		if (nested.nestedContexts.length || nested.unaccountedSlots.length) {
			checks.note(
				`the keys imply ${nested.boundaries.length} component boundary/boundaries opening ${JSON.stringify(nested.nestedContexts)}`
				+ `${nested.unaccountedSlots.length ? `, plus ${JSON.stringify(nested.unaccountedSlots)} spent by something that rendered no root` : ""}`,
			);
		}
		if (JSON.stringify(nested.allocationOrder) !== JSON.stringify(nested.documentOrder)) {
			checks.note(
				`allocation order ${JSON.stringify(nested.allocationOrder)} differs from document order ${JSON.stringify(nested.documentOrder)},\n`
				+ "        which is the eager-child-content layout: a component's child is built before the boundary is spent.",
			);
		}

		// ---- the client's template describes the server's tree ----
		// The counter has exactly one template because it has no components. A
		// component body is its own template root, so a nesting island declares more
		// than one, and `expected.templates` is how a fixture says how many.
		//
		// ATTRIBUTED TO THIS ISLAND, not counted across the module. Templates are
		// module-scoped bindings shared by every island in the bundle, so counting
		// them answers a different question -- and the difference is not academic: the
		// moment a second island landed in demo-app's bundle the counter fixture's
		// true claim of one template started reading as four. `islandTemplates`
		// follows reachability from the island's entry point instead.
		const attributed = islandTemplates(islandSource, declared.island);
		const templates = attributed.templates;
		const wanted = expected.templates ?? 1;
		checks.is(
			wanted === 1
				? "this island reaches exactly one template, its own root"
				: "this island reaches the template roots its source has: the island's own plus one per component body",
			templates.length,
			wanted,
		);
		checks.note(`this island reaches ${templates.length} of the module's ${moduleTemplates(islandSource).length} template(s): ${JSON.stringify(attributed.vars)}`);

		// WHICH template is the island's own is a fact about the emitter's declaration
		// order, and with components in the module it is no longer trivially the first.
		// A fixture may name it; the default stays 0. When the named one does not match
		// and another one does, that is reported by index -- because "the template does
		// not describe the server's tree" and "the templates are declared in a different
		// order than the fixture assumed" are completely different findings, and telling
		// them apart by hand from a tree diff is miserable.
		const islandTemplate = expected.islandTemplate ?? 0;
		const parity = templateParity(document, templates[islandTemplate], hydratable[0] ?? mount.firstElementChild);
		checks.is(TREE, parity.differences, []);
		if (parity.differences.length && templates.length > 1) {
			const matching = templates
				.map((template, at) => ({ at, differences: templateParity(document, template, hydratable[0] ?? mount.firstElementChild).differences }))
				.filter(candidate => candidate.differences.length === 0)
				.map(candidate => candidate.at);
			checks.note(
				matching.length
					? `TEMPLATE ORDER, not a tree defect: template ${islandTemplate} was compared, but template ${matching.join(" or ")} describes the server's tree exactly.\n`
						+ `        Set "islandTemplate": ${matching[0]} in the fixture's expected block. The emitter declares component bodies before the island's own.`
					: `no template in the module describes the server's tree, so this is not a declaration-order problem: all ${templates.length} were tried.`,
			);
		}
		if (parity.holes.length) checks.note(`the server filled ${parity.holes.length} hole(s): ${parity.holes.map(hole => JSON.stringify(hole.content)).join(", ")}`);

		// ---- load the graph BEFORE watching, so module init is not counted ----
		// Declaring a template creates a detached <template> element and
		// delegateEvents adds a document listener; neither touches the observed
		// tree, but importing under the observer would still make the run's numbers
		// depend on module-evaluation order. The loader's own imports hit node's
		// module cache, so the bracket below runs against these instances.
		await import(staged.href("/demo/topcoat-dom.js"));

		// A fixture's chance to put something in the realm BEFORE hydration.
		//
		// `search` stubs `fetch` from inside `interact`, which works because its
		// call happens after a debounce -- long after hydration has returned. The
		// dashboard cannot: its island constructs an `EventSource` and registers a
		// listener during its first effect run, which IS hydration, and jsdom
		// provides no `EventSource` at all. Without a stub in place first, the
		// island throws inside the hydrate bracket and every assertion after it
		// measures the wreckage instead of the island.
		//
		// Optional and awaited. A fixture that does not export it costs nothing,
		// which is why this is a hook rather than a parameter every fixture has to
		// acknowledge.
		const behaviour = await import(join(FIXTURES, name, "interact.mjs"));
		if (typeof behaviour.setup === "function") {
			await behaviour.setup({ window, document, checks, declared, expected });
		}

		const islands = await import(staged.href(islandUrl));
		const entry = islands[`__island_${declared.island}`];
		checks.ok(`the compiled module exports __island_${declared.island}`, typeof entry === "function");
		checks.is("its arity is the island's argument count", entry?.length, expected.seeds.length);

		// ---- the hydrate bracket ----
		// `keyed` is what makes CLONE IDENTITY observable PER KEY rather than in the
		// aggregate. The wave-2 backend report flagged this as a limit of the dom-test
		// stub -- `harness/trace.mjs`'s `nodeName` labels a node by the template it was
		// cloned from, so every clone of one template is indistinguishable from every
		// other -- and delegated it here. The aggregate checks below (`CLAIMED`, the
		// root's identity, "every node the server sent is still there") each see part
		// of it; none of them says "the node carrying key K is still the SAME OBJECT",
		// which is the claim that has to hold separately on each side of a component
		// boundary. Captured as objects before hydration so the comparison after it is
		// by identity and not by shape.
		const before = {
			html: mount.innerHTML,
			root: mount.firstElementChild,
			nodes: descendants(mount),
			keyed: new Map(hydratable.map(element => [element.getAttribute("data-hk"), element])),
		};
		const observer = watch(window, document.documentElement);

		// The headline number below is expected to be ZERO, which a broken observer
		// would also report. So the observer proves itself first, on a scratch node
		// outside the mount, and the tree is put back before anything real happens.
		// Without this the most important assertion in the harness could pass by
		// being deaf, which is the same failure the negative control exists to rule
		// out one level up.
		const scratch = document.createComment("parity-observer-probe");
		document.body.appendChild(scratch);
		scratch.remove();
		const probed = observer.drain();
		checks.is("the observer reports a mutation it is meant to see", probed.length, 2);
		checks.is("and the probe left the document as it found it", mount.innerHTML, before.html);

		let threw = null;
		let hydratedInTime = false;
		try {
			await import(staged.href("/demo/island-loader.js"));
			// Importing the loader is no longer the same as hydrating. It queues
			// each island on a promise chain, because hydration is a single global
			// cursor and two islands hydrating at once would share it, so the
			// module finishes evaluating with the work still pending. Draining the
			// observer here would have measured an untouched document and called it
			// zero mutations -- which is the number this fixture expects, so it
			// would have passed by being early.
			//
			// Waited on the loader's OWN signal rather than on a timer: it sets
			// `data-tl-hydrated` on the mount after the bracket returns, so this
			// asks the thing that knows instead of guessing how long it takes.
			hydratedInTime = await settled(mount);
		} catch (error) {
			threw = error;
		}
		const drained = observer.drain();
		observer.stop();

		// The loader tags the mount `data-tl-hydrated` when it is done, which is a
		// real mutation and is not hydration. Kept out of the headline number rather
		// than folded into it: `hydrateMutations` is the claim that hydration ADOPTED
		// the server's DOM instead of rebuilding it, and a fixture declaring 1 to
		// absorb the loader's bookkeeping could no longer tell those apart. Narrow on
		// purpose -- an attribute record, for that one name, on the mount itself --
		// and the attribute's presence is separately asserted above, so nothing is
		// being hidden.
		const LOADER_MARK = "data-tl-hydrated";
		const mountTag = declared.mount.split("[")[0];
		const marks = drained.filter(record => record.type === "attributes" && record.name === LOADER_MARK && record.where.endsWith(mountTag));
		const mutations = drained.filter(record => !marks.includes(record));

		checks.ok("the loader's hydrate bracket completed without throwing", threw === null, threw && `${threw.name}: ${threw.message}`);
		checks.ok(
			"the loader hydrated this island",
			hydratedInTime,
			`the loader never set data-tl-hydrated on ${declared.mount} within ${SETTLE_MS}ms. `
				+ "It hydrates eagerly only for islands the page names in `data-tl-eager`; everything else waits to be "
				+ "scrolled into view, and without an IntersectionObserver it falls back to hydrating all of them.",
		);
		checks.is("the loader logged nothing", page.console.filter(message => message.level === "error" || message.level === "jsdomError").map(message => message.text), []);

		// (f) THE MEASUREMENTS
		checks.is("the hydrate bracket's mutation count, less the loader's own marker", mutations.length, expected.hydrateMutations);
		checks.is(`the loader marked the mount ${LOADER_MARK} exactly once`, marks.length, 1);

		// The registry is not exported, so consumption is proved from the other end:
		// `getNextElement` adds every node it claims to `_$HY.completed` and deletes
		// that key from the registry in the same breath, so "in completed" and "gone
		// from the registry" are the same fact. Asserting it against the node object
		// the capture was parsed into also proves the claim was the SERVER's node
		// and not an identical clone, which no serialized-HTML comparison can.
		const claimed = hydratable.filter(element => hy?.completed?.has(element));
		checks.is(CLAIMED, claimed.length, hydratable.length);
		checks.ok(
			"the island root is the same node object it was before hydration",
			before.root && before.root === mount.firstElementChild,
			"a different object here means the template was cloned, not adopted",
		);
		// Per key, so a boundary that adopted its context's nodes and one that
		// silently cloned them are told apart. The root check above cannot: an island
		// whose own root was adopted and whose component subtree was rebuilt passes it.
		checks.is(IDENTITY, movedKeys(mount, before.keyed), []);
		checks.is(
			"every node the server sent is still the node that is there",
			[...descendants(mount)].filter(node => !before.nodes.has(node)).map(short),
			[],
		);
		checks.is(
			"the mount serializes as it did before hydration",
			mount.innerHTML,
			expected.htmlAfterHydrate === null ? before.html : expected.htmlAfterHydrate,
		);

		if (mutations.length || options.verbose) {
			checks.note(`hydrate mutations:\n${mutations.map(record => `        ${JSON.stringify(record)}`).join("\n") || "        none"}`);
		}

		// ---- the island is alive, and its updates are surgical ----
		const live = watch(window, document.documentElement);
		// AWAITED, because an island that calls out to the server cannot be measured
		// synchronously: a debounce is a claim about an interval, and the only way to
		// assert that nothing happened yet is to let real time pass and look. The two
		// synchronous fixtures return undefined, which awaits to undefined, so this
		// costs them nothing. `console` and `wire` are handed in for the same reason
		// `nestedKeys` is: so a fixture reads the run's own values rather than
		// reimplementing access to them.
		const context = {
			document,
			window,
			mount,
			checks,
			drain: () => live.drain(),
			expected,
			renderId,
			nestedKeys: nestedKeyParity,
			wire: declared.wire,
			// The whole declaration, for a fixture whose inputs do not fit the blocks
			// named above. `wire` stays hoisted because three fixtures reach for it and
			// because run.mjs validates it against the spec before handing it over; this
			// is the general case, added when the dashboard arrived with a `stream` and a
			// `chart` block that only it has. A fixture reading `declared.foo` is reading
			// its own file, which is the point -- the alternative was a context parameter
			// per fixture-specific block.
			declared,
			// The staged file URL a served URL was written to, so a fixture can import
			// a module the PAGE imported and get the SAME instance -- node caches by
			// resolved URL, and stage.mjs writes each served module exactly once.
			//
			// The dashboard needs it and nothing else does: its chart library is a
			// served module that RECORDS what was done to it, so the only way to read
			// the island's foreign calls is to hold the same module object the island
			// called into. Importing the file twice would give two recorders, one of
			// them empty, which is the failure this exists to make impossible.
			moduleFor: url => staged.href(url),
			console: page.console,
			// Which key was broken and how deep it was, so a `dead` probe can assert
			// the failure mode that PARTICULAR perturbation should produce rather than
			// accepting either. The nested fixture's probe needs it: a broken root key
			// and a broken key two contexts down are supposed to look different, and a
			// probe that could not tell them apart was the reason `deepKey` exists.
			perturbed,
		};
		if (control?.probe === "dead") {
			// Not "it broke somehow" but "it broke in exactly this way". The symptom
			// is worth pinning down because it is so quiet: see the note below.
			await behaviour.dead(context);
			checks.note(
				"HOW A KEY MISS DEGRADES, MEASURED on this run and not carried over -- the previous description here\n"
				+ "        was stale and said the opposite, which is worth knowing about before trusting the rest of it.\n"
				+ "        It said: the clone is never inserted, ZERO mutations, a byte-identical DOM, a completely inert\n"
				+ "        island. That was true of a loader that handed the runtime `[...mount.childNodes]` as a\n"
				+ "        workaround for the zero-sized return type. dom.rs's LOADER no longer does -- it now calls\n"
				+ "        `hydrate(() => entry(...seeds), mount, { renderId })` -- so upstream's own insert semantics\n"
				+ "        apply and the clone IS inserted.\n"
				+ "\n"
				+ "        What actually happens: `getNextElement` misses the registry and falls back to `template()`, and\n"
				+ "        the fresh clone REPLACES the server's node in the document. The scope of the replacement is\n"
				+ "        exactly the context that missed, which is why there are two key controls:\n"
				+ "          - a miss on the island ROOT replaces the whole island (1 childList record at the mount, plus\n"
				+ "            one per component subtree the rebuild drops). The island then WORKS -- it is a normal\n"
				+ "            client render -- so nothing a person would look at reports a problem.\n"
				+ "          - a miss on a key BELOW a component boundary replaces ONLY that subtree. The island root and\n"
				+ "            every boundary above it stay adopted, same node objects, and the island keeps working.\n"
				+ "\n"
				+ "        So the failure mode is not death, it is SILENT LOSS OF HYDRATION: the server's HTML is thrown\n"
				+ "        away and rebuilt, the user sees a re-render, and no throw or console message says so. That is a\n"
				+ "        better functional outcome than an inert island and a worse observability one, and CLAIMED plus\n"
				+ "        the per-key identity check are the only things in reach that see it.",
			);
		} else if (!control) {
			await behaviour.interact(context);
		}
		live.stop();

		return checks;
	} finally {
		page.restore();
	}
}

/**
 * The keys whose node is no longer the object it was.
 *
 * Looked up by key rather than by walking, because the question is per key: the
 * server wrote key K on a node, and after hydration the node carrying K has to be
 * that same object. A key that vanished from the document counts as moved too --
 * `querySelector` returns null, which is not the node.
 *
 * Exported so `test.mjs` can prove it catches a swapped clone. A check whose
 * expected value is the empty list is exactly the kind that can pass by being
 * unable to see anything, which is the failure the negative controls exist to rule
 * out one level up and which a unit test rules out here.
 *
 * @param {Element} mount
 * @param {Map<string, Element>} keyed the keys and their nodes before hydration
 * @returns {string[]}
 */
export function movedKeys(mount, keyed) {
	return [...keyed].filter(([key, node]) => mount.querySelector(`[data-hk="${key}"]`) !== node).map(([key]) => key);
}

/** Every node under an element, as a Set for identity comparison. */
function descendants(root) {
	const seen = new Set();
	const walk = node => {
		for (let at = node.firstChild; at; at = at.nextSibling) {
			seen.add(at);
			walk(at);
		}
	};
	walk(root);
	return seen;
}

function short(node) {
	if (node.nodeType === 3) return `text ${JSON.stringify(node.data)}`;
	if (node.nodeType === 8) return `comment ${JSON.stringify(node.data)}`;
	return `<${node.nodeName.toLowerCase()}>`;
}

async function main() {
	const args = process.argv.slice(2);
	const verbose = args.includes("--verbose");
	const negatives = !args.includes("--no-negative");
	const only = args.filter(arg => !arg.startsWith("--"));
	const names = only.length ? only : fixtureNames();

	let failed = 0;
	for (const name of names) {
		const checks = await runFixture(name, { verbose });
		console.log(checks.report());
		console.log("");
		if (checks.failures.length) failed++;
	}

	if (negatives) {
		for (const name of names) {
			// A pending fixture has no bytes to perturb, so a control over it would
			// pass vacuously, which is the one thing a negative control may not do.
			if (fixture(name).pending) {
				console.log(`skip  negative controls for ${name}: the fixture is pending, so there is nothing to perturb`);
				console.log("");
				continue;
			}
			for (const [id, control] of Object.entries(CONTROLS)) {
				// A control that cannot apply is SKIPPED WITH A REASON, never silently
				// dropped: the deepest key of a component-free island is its root, so
				// running `deepKey` there would be the `key` control wearing a second
				// name and would make the suite look broader than it is.
				if (control.needsComponents && !fixture(name).expected.components) {
					console.log(`skip  ${id} for ${name}: it declares no components, so its deepest key IS its root and this would duplicate the \`key\` control`);
					console.log("");
					continue;
				}
				const checks = await runFixture(name, { control: id, verbose });
				console.log(checks.report());
				console.log("");

				// Inverted on purpose: a parity harness that cannot fail is not a parity
				// harness. And inverted on ONE NAMED CHECK, not on "something failed" --
				// a control that passed because an unrelated check broke would look
				// exactly like a control that was working.
				const broke = checks.failures.map(failure => failure.what);
				const noticed = broke.includes(control.mustFail);
				console.log(`${noticed ? "ok  " : "FAIL"}  the harness detects ${control.describe}`);
				if (noticed) {
					const other = broke.filter(what => what !== control.mustFail);
					console.log(`      caught by ${JSON.stringify(control.mustFail)}`);
					if (other.length) console.log(`      also failed, which is allowed but worth reading: ${other.join("; ")}`);
				} else {
					failed++;
					if (broke.length) {
						console.log(`      ${broke.length} check(s) failed but NOT ${JSON.stringify(control.mustFail)}, so this control proved something else: ${broke.join("; ")}`);
					} else {
						console.log(`      NOTHING failed. The harness accepted a document with ${control.describe}, so that claim is vacuous.`);
					}
				}
				console.log("");
			}
		}
	}

	if (failed) {
		console.log(`${failed} fixture run(s) failed`);
		process.exit(1);
	}
	console.log("parity holds for every fixture, and the negative controls fail as they must");
}

if (import.meta.filename === process.argv[1]) {
	await main();
}
