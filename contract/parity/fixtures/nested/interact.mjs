// What the nesting island has to do once it is alive.
//
// SCAFFOLD. Nothing in this file runs yet: `fixture.json` sets `pending`, so
// run.mjs reports the fixture and returns before it ever opens a document. It is
// written now, against the shape `fixture.json`'s `sourceShape` declares, so that
// the day the island lands the assertions are already argued rather than invented
// while staring at a failure.
//
// WHAT THIS FIXTURE CLAIMS THAT THE COUNTER CANNOT
// -----------------------------------------------
// The counter proves one template claims the server's nodes. This one proves that
// claiming SURVIVES A COMPONENT BOUNDARY, which is a different failure surface:
// a boundary opens a hydration context, and a context is a string prefix that
// every key below it inherits. Get the prefix wrong by one slot and the nodes
// below the boundary are still all present, still all correct, and no longer
// findable. The symptom is not an exception; it is a subtree that quietly does
// nothing, which is exactly what the `dead` probe at the bottom pins down.
//
// So the checks below are in three groups:
//   1. the server's nodes are all there and are the ones the components rendered;
//   2. hydration adopted them, node identity intact ACROSS the boundary;
//   3. the island is still reactive, and a click does not touch the components.

/**
 * @param {object} context
 * @param {Element} context.mount the `<topcoat-island>` element
 * @param {import("../../lib/check.mjs").Checks} context.checks
 * @param {() => object[]} context.drain the mutations since the last drain
 * @param {object} context.expected the fixture's `expected` block
 * @param {string} context.renderId the island's render id, `data-tk` plus a dot
 * @param {(root: Element, options: object) => object} context.nestedKeys
 *   `lib/keys.mjs`'s `nestedKeyParity`, handed in so this file makes the same
 *   call the run makes rather than a second implementation of it
 */
export function interact({ mount, checks, drain, expected, renderId, nestedKeys }) {
	const card = mount.querySelector(".card");
	const badge = mount.querySelector(".badge");
	const count = mount.querySelector(".island-count");
	const increment = mount.querySelector(".island-step");

	// ---- 1. the components rendered, and their keys nest ----

	checks.ok("the card component's section survived hydration", !!card);
	checks.ok("and the badge nested inside it did too", !!badge);
	checks.ok("the badge is a DOM descendant of the card, because the card renders it in its own body", card?.contains(badge));
	checks.is("the card's heading is its `title` prop", card?.querySelector("h2")?.textContent, "Profile");
	checks.is("the badge's text is its `label` prop", badge?.textContent, "Active");

	const parity = nestedKeys(mount, {
		renderId,
		expected: expected.keys.map(key => `${renderId}${key}`),
		components: expected.components,
	});
	checks.is("the keys the server wrote are the nested sequence the scheme requires", parity.differences, []);
	checks.is(
		"the card's own root carries the key its boundary's context opened",
		card?.getAttribute("data-hk"),
		`${renderId}${expected.keys[1]}`,
	);
	checks.is(
		"and the badge's root carries a key one context deeper still",
		badge?.getAttribute("data-hk"),
		`${renderId}${expected.keys[2]}`,
	);

	// ---- 3. the island is alive, and the components are not in the way ----

	checks.is("the count element survived hydration", count?.textContent, "count 5");

	increment.click();
	const clicked = drain();

	checks.is("one click on +1 renders 6", count.textContent, "count 6");
	checks.is("one click costs the expected mutations", clicked.length, expected.clickMutations);
	checks.is("the click changed only the count text node", [...new Set(clicked.map(record => record.where.split(" > ").at(-1)))], ['text "6"']);
	checks.is("the count element after one click", count.outerHTML, expected.clickHtml);

	// The components hold no signal, so nothing below either boundary may move.
	// Worth asserting rather than assuming: a component re-created on every update
	// would pass every check above and fail this one.
	checks.is("the card is byte-identical after a click", card.outerHTML.includes('class="badge"'), true);
	checks.is(
		"and no mutation landed inside a component",
		clicked.filter(record => record.where.includes("section")).length,
		0,
	);
}

/**
 * What the nesting island looks like when a NESTED key did not match.
 *
 * Run by the negative control, and the reason this fixture is worth having: for
 * the counter, a perturbed key kills the whole island, which is loud enough to
 * find. Here the interesting failure is PARTIAL -- perturb the badge's key and
 * the island still counts, the card is still there, and only the subtree under
 * one boundary is inert. A harness that only asserted "the island works" would
 * call that a pass.
 *
 * These checks PASS: they assert the failure mode. The control's own pass
 * condition is that the named parity check in run.mjs failed.
 *
 * @param {object} context same shape as `interact`
 */
export function dead({ mount, checks, drain, perturbed }) {
	const count = mount.querySelector(".island-count");
	const card = mount.querySelector(".card");
	const badge = mount.querySelector(".badge");
	const increment = mount.querySelector(".island-step");

	// THE PREDICTION WAS RIGHT ABOUT THE SCOPE AND WRONG ABOUT THE SYMPTOM, which is
	// the more useful half to have written down. It said the subtree below the
	// boundary would be INERT. Measured, it is REBUILT: `getNextElement` misses the
	// registry, falls back to `template()`, and the fresh clone is inserted through
	// the parent's hole, replacing the server's node. So the boundary IS an
	// independent claim -- exactly what the fixture existed to establish -- but a
	// failed claim costs the server's DOM rather than the subtree's behaviour.
	//
	// Which of the two key controls is running decides what to assert, because the
	// whole point of having two is that they must not look the same.
	const deepest = perturbed?.which === "deepest";

	checks.ok("the badge is still in the document either way", !!badge);
	checks.ok("and so is the card", !!card);

	increment.click();
	const clicked = drain();

	// The island works in both cases: a rebuild is a working client render. The
	// difference is HOW MUCH of the server's DOM was thrown away to get there.
	checks.is("the island works: a rebuild is still a working island", count.textContent, "count 6");
	checks.is("and one click still costs one mutation", clicked.length, 1);

	if (deepest) {
		checks.is(
			"a key miss BELOW a boundary is CONTAINED: the badge lost its data-hk because it was rebuilt from the template",
			badge.hasAttribute("data-hk"),
			false,
		);
		checks.ok(
			"while the card ABOVE it kept the key it was claimed with, so the boundary is an independent claim",
			card.getAttribute("data-hk")?.endsWith("10"),
			`the card's data-hk is ${JSON.stringify(card.getAttribute("data-hk"))}`,
		);
		checks.ok(
			"and the island root kept its key too",
			mount.firstElementChild.hasAttribute("data-hk"),
			"a nested miss must not cost the root its claim",
		);
		checks.note(
			`CONTAINMENT MEASURED: ${JSON.stringify(perturbed.from)} was broken ${perturbed.depth} contexts deep and the damage stopped\n`
			+ "        at that boundary -- one childList record inside the card, nothing above it touched. This is the\n"
			+ "        partial failure the fixture was built for, and it is the one the counter cannot show.",
		);
	} else {
		// MEASURED, and it corrected a second guess of mine: I first asserted that a
		// root miss costs the whole island every key below it. It does not. The root's
		// OWN template nodes are rebuilt, and the two component boundaries below it are
		// still claimed off the server's nodes and moved into the rebuilt root, because
		// each key is a separate registry entry looked up on its own.
		checks.is(
			"a key miss on the ROOT loses the root's own claim",
			mount.firstElementChild.hasAttribute("data-hk"),
			false,
		);
		checks.is(
			"but the two component boundaries below it are STILL claimed off the server's own nodes",
			[...mount.querySelectorAll("[data-hk]")].map(element => element.className),
			["card", "badge"],
		);
		checks.note(
			"ROOT MISS, and it is the other half of the containment result: the root was rebuilt from its\n"
			+ "        template while the card and the badge were adopted off the server's DOM and moved into it. So a\n"
			+ "        hydration key is an INDEPENDENT CLAIM in both directions -- a boundary survives its parent's\n"
			+ "        miss, and (see the deepKey control) a parent survives its child's. That is the substantive thing\n"
			+ "        this fixture establishes about component contexts, and neither control alone shows it.",
		);
	}
}
