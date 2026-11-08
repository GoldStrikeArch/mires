// What the counter island has to do once it is alive.
//
// Hydration parity is only half the claim. The other half is that the island the
// browser ends up with is a working one whose updates are surgical: a counter
// that re-rendered its whole subtree on every click would pass every hydration
// check in run.mjs and still be wrong.
//
// A fixture's interaction file gets the live mount and the same check list the
// run uses, and asserts through real dispatched events -- `button.click()`, not a
// direct call to the handler property. That is the part demo-app/smoke/check.mjs
// cannot reach: it calls `$$click` by hand, so it proves the compiled closure
// works but not that the runtime's delegated listener ever finds it.

/**
 * @param {object} context
 * @param {Element} context.mount the `<topcoat-island>` element
 * @param {import("../../lib/check.mjs").Checks} context.checks
 * @param {() => object[]} context.drain the mutations since the last drain
 * @param {object} context.expected the fixture's `expected` block
 */
export function interact({ mount, checks, drain, expected }) {
	const count = mount.querySelector(".island-count");
	const [decrement, increment] = mount.querySelectorAll(".island-step");

	checks.is("the count element survived hydration", count?.textContent, "count 5");
	checks.is("the -1 button is not disabled at 5", decrement.disabled, false);

	// A real dispatched click, through the runtime's own delegated listener.
	increment.click();
	const clicked = drain();

	checks.is("one click on +1 renders 6", count.textContent, "count 6");
	checks.is("one click costs the expected mutations", clicked.length, expected.clickMutations);
	// Upstream writes `.data` on the text node between the markers, so the one
	// mutation targets the text node itself -- tighter than touching the <p>.
	checks.is("the click changed only the count text node", [...new Set(clicked.map(record => record.where.split(" > ").at(-1)))], ['text "6"']);
	checks.is("the count element after one click", count.outerHTML, expected.clickHtml);

	// The bound property is the other half of the reactive graph: it proves the
	// effect the server never ran is now live and reading the same signal.
	for (let click = 0; click < 6; click++) decrement.click();
	drain();
	checks.is("six clicks on -1 reach zero", count.textContent, "count 0");
	checks.is("the -1 button disables itself at zero", decrement.disabled, true);

	increment.click();
	drain();
	checks.is("the -1 button re-enables above zero", decrement.disabled, false);
	checks.is("the count came back up", count.textContent, "count 1");
}

/**
 * What the counter looks like when its hydration key did not match.
 *
 * Run by the negative control. These checks PASS -- they assert the failure mode,
 * not the working behaviour -- and the control's own pass condition is that the
 * named parity check in run.mjs failed.
 *
 * REWRITTEN AGAINST A MEASUREMENT, and the previous version is worth recording
 * because it was confidently wrong. It asserted that the island was INERT: that
 * clicking `+1` did nothing and cost no mutations. That described a loader which
 * handed the runtime `[...mount.childNodes]`, so the fallback clone was built but
 * never inserted. dom.rs's LOADER now calls `hydrate(() => entry(...seeds), mount,
 * { renderId })`, upstream's insert semantics apply, and the clone REPLACES the
 * server's root.
 *
 * So the symptom is the opposite of dead, and quieter for it: the island is a
 * perfectly working CLIENT-RENDERED island. The server's HTML was thrown away and
 * rebuilt, the user saw a re-render, and nothing throws or logs. Every signal a
 * person would think to check -- does it work, does it look right, is the console
 * clean -- says yes. Only the registry and per-key identity checks in run.mjs see
 * it, which is the entire argument for this harness existing.
 *
 * @param {object} context same shape as `interact`
 */
export function dead({ mount, checks, drain }) {
	const count = mount.querySelector(".island-count");
	const [, increment] = mount.querySelectorAll(".island-step");

	// Byte-identical to what the server sent, because it is the same template
	// rendered from the same seed. This is what makes the failure invisible to any
	// comparison of markup, here and in a browser's devtools.
	checks.is("the rebuilt markup is byte-identical to what the server sent", count.outerHTML, '<p class="island-count">count <!--$-->5<!--/--></p>');

	increment.click();
	const clicked = drain();

	checks.is("and the island WORKS: clicking +1 renders 6, because it is a normal client render", count.textContent, "count 6");
	checks.is("costing the same one mutation a hydrated island costs", clicked.length, 1);
	checks.note(
		"THE POINT OF THIS PROBE: every observation above is indistinguishable from a correctly hydrated\n"
		+ "        island. Same markup, same behaviour, same mutation cost, no throw, no console output. What was\n"
		+ "        actually lost is the server's DOM, and the only evidence is that the node objects changed and the\n"
		+ "        registry was never consumed.",
	);
}
