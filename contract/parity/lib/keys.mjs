// Hydration keys, including the nested ones a component boundary opens.
//
// `tree.mjs` answers "does the client's template describe the server's tree".
// This file answers the question one level up, which only exists once components
// are in play: are the keys the server wrote the keys the SCHEME says it should
// have written, and do they describe the context tree the source implies.
//
// THE SCHEME, from CONTRACT-DOM.md 9.1-9.9, driven in
// `contract/fixtures/keys-nested.json`
// ------------------------------------------------------------------------
// A hydration context is a pair `(id, count)`. Every hydratable template root
// spends one slot of the current context:
//
//     key   = id + letter(digits(count) - 1) + String(count)
//     count = count + 1
//
// where `letter(0) = ""` and `letter(n) = String.fromCharCode(96 + n)`, so one
// digit gets no letter, two get "a", three get "b". A component boundary spends
// one slot too, and the key that slot would have had becomes the id of the child
// context the component's own roots number from zero in. The parent's counter
// stays advanced after the component returns.
//
// WHY A KEY IS SELF-DESCRIBING, WHICH IS WHAT MAKES THIS CHECKABLE
// ---------------------------------------------------------------
// The letter is not decoration. Read a key left to right from the render id and
// the FIRST CHARACTER of what is left decides how many characters the next slot
// takes: a digit means that one digit is the whole count, and a letter means
// exactly `letter - 'a' + 2` digits follow. So a key parses into its chain of
// slots deterministically, with no search and no ambiguity, and the chain is the
// path of nested contexts it was allocated through. `keyChain` does that walk.
//
// HOW STRONG THAT IS, EXACTLY -- measured, not assumed
// ----------------------------------------------------
// Parsing alone is a weaker check than it first looks, and it is worth being
// precise about, because overstating it would make a green run mean less than a
// reader thinks. A key of nothing but digits ALWAYS parses: "0123" reads as slot
// 0, then slot 0 of context "0", then slot 2 of "012"... i.e. as a key three
// component boundaries deep. So `keyChain` rejects only a key that breaks the
// letter rule ("a1" promises two digits and brings one), carries a leading zero
// after a letter, contains something that is neither, or falls outside the render
// id. It does NOT reject a well-formed key from an implausibly deep nesting.
//
// What makes the check tight is the two things layered on top, both in
// `nestedKeyParity`:
//
//   * `components`, the number of component boundaries the fixture's SOURCE
//     contains. The keys imply a boundary count; a mismatch is a failure. This is
//     what turns "0123" from parseable into wrong.
//   * the slot conflict: a slot spent by an element that carries it cannot also
//     be the context some deeper key nests under, because a slot is spent once.
//     A key that gained or lost a level trips this as soon as anything else in
//     the island sits at the level it moved to.
//
// Both are worth more than they sound, because the failure they catch is the one
// this whole harness exists for: a key that is a plausible string no node in the
// document carries, whose only symptom is that hydration quietly does nothing.
//
// WHAT IS NOT CHECKABLE FROM BYTES, AND MUST NOT BE ASSUMED
// ---------------------------------------------------------
// 1. DOCUMENT ORDER IS NOT ALLOCATION ORDER. Under Topcoat's eager child content
//    a component's child is built BEFORE the component's own root, so it holds a
//    LOWER key while sitting INSIDE the element with the higher one.
//    `keys-nested.json` `getterChildren` drives exactly this shape:
//    `<div data-hk="10"><span data-hk="0"></span></div>`. A checker that walked
//    the document and demanded a rising sequence would fail the one layout
//    Topcoat actually emits. So the sequence comparison here is against a
//    DECLARED allocation order, recovered by sorting on the slot chain, and
//    document order is reported alongside as information.
// 2. CONTEXT NESTING IS NOT DOM NESTING, for the same reason. In that example the
//    span's context is the ROOT context while the element containing it took its
//    key from that same context. Neither containment direction may be assumed.
// 3. A slot spent by a component that rendered no root of its own leaves a gap
//    nothing in the bytes explains (CONTRACT-DOM 9.8: an empty component still
//    spends a slot). Gaps are reported, not failed on.

/**
 * The hydration key a context allocates at `count`.
 *
 * `contextId` is the render id for a top-level root and the parent's spent slot
 * key for anything inside a component. From `keys.json` `$algorithm`.
 */
export function hydrationKey(contextId, count) {
	const digits = String(count);
	const letter = digits.length - 1;
	return contextId + (letter ? String.fromCharCode(96 + letter) : "") + digits;
}

/**
 * Read ONE slot off the front of `rest`.
 *
 * The first character decides the width, which is the property the letter exists
 * to give. Returns null if the front of `rest` is not a slot.
 *
 * @param {string} rest what follows a context id
 * @returns {{count: number, width: number}|null}
 */
function readSlot(rest) {
	if (rest.length === 0) return null;
	const first = rest[0];

	let digits;
	let width;
	if (first >= "0" && first <= "9") {
		digits = first;
		width = 1;
	} else if (first >= "a" && first <= "z") {
		const length = first.charCodeAt(0) - 96 + 1;
		digits = rest.slice(1, 1 + length);
		width = 1 + length;
		if (digits.length !== length) return null;
		// A letter says the count has more than one digit, so it cannot start
		// with a zero: `String(count)` never writes one.
		if (digits[0] === "0") return null;
	} else {
		return null;
	}

	for (const digit of digits) if (digit < "0" || digit > "9") return null;
	return { count: Number(digits), width };
}

/**
 * The count `key` was allocated at in `contextId`, or null if that context
 * cannot have produced exactly this key.
 *
 * The inverse of `hydrationKey` for one slot. Use `keyChain` for a key that
 * passed through component boundaries.
 */
export function allocationCount(contextId, key) {
	if (!key.startsWith(contextId)) return null;
	const slot = readSlot(key.slice(contextId.length));
	if (!slot || slot.width !== key.length - contextId.length) return null;
	return slot.count;
}

/**
 * The chain of slots `key` was allocated through, from the render id down.
 *
 * Every element of the chain but the last is a slot a COMPONENT BOUNDARY spent,
 * and its key is the id of the context the next slot came from. The last element
 * is the key itself.
 *
 * @param {string} key a `data-hk` value
 * @param {string} renderId the root context's id
 * @returns {{contextId: string, count: number, key: string}[]|null} null if the
 *   scheme cannot produce this key under this render id
 */
export function keyChain(key, renderId = "") {
	if (!key.startsWith(renderId)) return null;
	const chain = [];
	let contextId = renderId;
	let at = renderId.length;
	while (at < key.length) {
		const slot = readSlot(key.slice(at));
		if (!slot) return null;
		at += slot.width;
		const spent = key.slice(0, at);
		chain.push({ contextId, count: slot.count, key: spent });
		contextId = spent;
	}
	return chain.length ? chain : null;
}

/**
 * Reconstruct the context tree a set of keys implies.
 *
 * Contexts are discovered from the keys themselves: a component boundary writes
 * no `data-hk`, so the only evidence its context exists is that some key was
 * allocated through it, which `keyChain` recovers.
 *
 * @param {string[]} keys every `data-hk` value, in any order
 * @param {string} renderId the root context's id
 */
export function contextTree(keys, renderId = "") {
	const problems = [];

	const duplicates = [...new Set(keys.filter((key, at) => keys.indexOf(key) !== at))];
	if (duplicates.length) problems.push(`the same key was written more than once: ${duplicates.join(", ")}`);
	const unique = [...new Set(keys)];

	/** @type {Map<string, {contextId: string, count: number, key: string}[]>} */
	const chains = new Map();
	const unreachable = [];
	for (const key of unique) {
		const chain = keyChain(key, renderId);
		if (chain) chains.set(key, chain);
		else unreachable.push(key);
	}
	if (unreachable.length) {
		problems.push(
			`the scheme cannot produce these keys under the render id ${JSON.stringify(renderId)}: ${unreachable.join(", ")}`,
		);
	}

	// Every slot anyone spent, and what spent it. A slot spent by an element is
	// the last link of that element's chain; every earlier link is a boundary.
	/** @type {Map<string, {contextId: string, count: number, by: "element"|"component"}>} */
	const slots = new Map();
	const conflicts = [];
	for (const [key, chain] of chains) {
		chain.forEach((link, index) => {
			const by = index === chain.length - 1 ? "element" : "component";
			const already = slots.get(link.key);
			if (already && already.by !== by) {
				conflicts.push(link.key);
				return;
			}
			slots.set(link.key, { contextId: link.contextId, count: link.count, by });
		});
		void key;
	}
	for (const key of [...new Set(conflicts)]) {
		problems.push(`slot ${key} is spent both by an element that carries it and by a component boundary a deeper key nests under, and a slot is spent once`);
	}

	// One entry per context that allocated something.
	const grouped = new Map([[renderId, []]]);
	for (const [key, slot] of slots) {
		if (!grouped.has(slot.contextId)) grouped.set(slot.contextId, []);
		grouped.get(slot.contextId).push({ key, count: slot.count, by: slot.by });
	}
	const contexts = [...grouped]
		.map(([id, allocations]) => {
			allocations.sort((a, b) => a.count - b.count);
			const held = new Set(allocations.map(allocation => allocation.count));
			const gaps = [];
			for (let count = 0; count < (allocations.length ? allocations.at(-1).count + 1 : 0); count++) {
				if (!held.has(count)) gaps.push(count);
			}
			return { id, allocations, gaps, opensAt: id === renderId ? null : slots.get(id) ?? null };
		})
		.sort((a, b) => (a.id === renderId ? -1 : b.id === renderId ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

	return {
		ok: problems.length === 0,
		problems,
		contexts,
		chains,
		slots,
		duplicates,
		unreachable,
		/** Contexts a component boundary opened, in discovery-independent order. */
		nested: contexts.filter(context => context.id !== renderId).map(context => context.id),
		/** Slots spent by a component boundary, i.e. the boundaries themselves. */
		boundaries: [...slots].filter(([, slot]) => slot.by === "component").map(([key]) => key),
	};
}

/**
 * Every `data-hk` under `root` in document order, including `root`'s own.
 *
 * `renderId` filters to one island's keys, the way the loader's
 * `gatherHydratable` does. Pass "" to take every key.
 */
export function documentKeys(root, renderId = "") {
	const found = [];
	if (root.hasAttribute?.("data-hk")) found.push(root.getAttribute("data-hk"));
	for (const element of root.querySelectorAll("[data-hk]")) found.push(element.getAttribute("data-hk"));
	return renderId ? found.filter(key => key.startsWith(renderId)) : found;
}

/**
 * The order the scheme spent these keys' slots in, recovered from the keys.
 *
 * A context's slots run in count order, and everything a context allocated falls
 * between the slot that opened it and its parent's next slot -- which is exactly
 * lexicographic order on the chain of counts. This is the order
 * `keys-nested.json` records `elementKeys` in and the order a Rust `Formatter`
 * spends slots in. It is NOT document order; see the top of this file.
 */
export function allocationOrder(keys, renderId = "") {
	return [...new Set(keys)]
		.map(key => ({ key, path: keyChain(key, renderId)?.map(link => link.count) ?? null }))
		.filter(entry => entry.path)
		.sort((a, b) => comparePath(a.path, b.path))
		.map(entry => entry.key);
}

function comparePath(a, b) {
	for (let at = 0; at < Math.min(a.length, b.length); at++) {
		if (a[at] !== b[at]) return a[at] - b[at];
	}
	return a.length - b.length;
}

/**
 * The keys a nested island wrote, checked against the scheme and, if the fixture
 * declares one, against an expected allocation order.
 *
 * @param {Element} root the island mount, or any element containing the keys
 * @param {{renderId?: string, expected?: string[], components?: number}} options
 *   `expected` is in ALLOCATION order. `components` is how many component
 *   boundaries the fixture's source contains, which pins the boundaries the keys
 *   imply against the ones the source declares.
 */
export function nestedKeyParity(root, options = {}) {
	const renderId = options.renderId ?? "";
	const inDocument = documentKeys(root, renderId);
	const tree = contextTree(inDocument, renderId);
	const ordered = allocationOrder(inDocument, renderId);

	const differences = [...tree.problems];
	if (options.expected && JSON.stringify(ordered) !== JSON.stringify(options.expected)) {
		differences.push(`the keys in allocation order are ${JSON.stringify(ordered)}, expected ${JSON.stringify(options.expected)}`);
	}

	// A gap is a slot nothing carries. It is accounted for if a component boundary
	// spent it, and unaccounted for otherwise -- which is either a component that
	// rendered no root of its own (allowed, CONTRACT-DOM 9.8) or a hydratable root
	// the server never wrote, which is the defect the `components` count catches.
	// Nothing else can leave a gap: a slot is only spent when a root RENDERS, and
	// a `NoHydration` subtree does not spend one at all, because the server's
	// `getHydrationKey` short-circuits on `noHydrate` before calling
	// `getNextContextId` (dom-expressions/src/server.js:522-525).
	const unaccounted = tree.contexts
		.flatMap(context => context.gaps.map(count => hydrationKey(context.id, count)))
		.filter(key => !tree.boundaries.includes(key));

	if (typeof options.components === "number") {
		const implied = tree.boundaries.length + unaccounted.length;
		if (implied !== options.components) {
			differences.push(
				`the keys imply ${implied} component boundary/boundaries: ${tree.boundaries.length} that rendered a root of its own, and ${unaccounted.length} slot(s) nothing carries. The fixture declares ${options.components}.`,
			);
		}
	}

	return {
		ok: differences.length === 0,
		differences,
		tree,
		allocationOrder: ordered,
		documentOrder: inDocument,
		nestedContexts: tree.nested,
		boundaries: tree.boundaries,
		unaccountedSlots: unaccounted,
	};
}

/**
 * The element carrying each key, so a caller can compare a template against the
 * root that key belongs to rather than against whatever came first.
 *
 * @returns {Map<string, Element>}
 */
export function rootsByKey(root, renderId = "") {
	const map = new Map();
	const consider = element => {
		const key = element.getAttribute("data-hk");
		if (key && (!renderId || key.startsWith(renderId)) && !map.has(key)) map.set(key, element);
	};
	if (root.hasAttribute?.("data-hk")) consider(root);
	for (const element of root.querySelectorAll("[data-hk]")) consider(element);
	return map;
}
