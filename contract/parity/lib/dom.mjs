// The live DOM the parity run happens in, and the bridge that lets modules
// loaded by node operate on it.
//
// jsdom gives a document that behaves like a browser's: comment nodes survive
// parsing and serialization, `dataset` works, `click()` dispatches a real event
// through the capture and bubble phases, and MutationObserver reports what
// changed. That last one is the reason this harness exists -- "hydration did not
// touch the DOM" is not a claim any amount of reading serialized HTML can make,
// because a node can be replaced by an identical one and serialize the same.
//
// THE REALM SEAM
// --------------
// jsdom's document lives in one JavaScript realm and the modules node imports
// live in another. The runtime bundle reaches for exactly four globals --
// `document`, `window`, `queueMicrotask` and `globalThis._$HY` -- so the bridge
// is small, but `_$HY` needs care. The bootstrap is a classic inline script in
// the page, so it runs inside jsdom and installs `window._$HY` there, while the
// runtime reads `globalThis._$HY` from node's realm. They have to be the SAME
// object: the runtime writes `_$HY.done` and reads `_$HY.completed`, and the
// bootstrap's capture listener reads `_$HY.events`. So the bridge aliases it
// rather than copying it.

import { JSDOM, VirtualConsole } from "jsdom";

/** The globals the bridge installs, beyond `_$HY`. */
const BRIDGED = [
	"document",
	"window",
	"queueMicrotask",
	"Node",
	"Element",
	"HTMLElement",
	"Text",
	"Comment",
	"DocumentFragment",
	"MutationObserver",
	"Event",
	"CustomEvent",
	"MouseEvent",
	"getComputedStyle",
];

/**
 * Parse server HTML into a live document and bridge it to node's realm.
 *
 * `runScripts: "dangerously"` is deliberate and is what makes the bootstrap
 * under test the SERVED bootstrap: it is an inline classic script in the page,
 * so jsdom executes the real bytes rather than a copy this harness keeps. The
 * page's other two scripts are not executed -- the import map is data, and the
 * loader is an external module, which jsdom neither fetches (no `resources`
 * option) nor supports. Running the loader is the harness's job anyway, since it
 * has to attach an observer first.
 *
 * @param {string} html the server's response body
 * @returns {{window: Window, document: Document, console: object[], restore: () => void, serialize: () => string}}
 */
export function open(html) {
	const messages = [];
	const virtualConsole = new VirtualConsole();
	for (const level of ["error", "warn", "info", "log", "debug"]) {
		virtualConsole.on(level, (...args) => messages.push({ level, text: args.map(String).join(" ") }));
	}
	virtualConsole.on("jsdomError", error => messages.push({ level: "jsdomError", text: error.message }));

	const dom = new JSDOM(html, {
		runScripts: "dangerously",
		url: "http://127.0.0.1:3000/island",
		virtualConsole,
	});
	const window = dom.window;

	const saved = new Map();
	for (const name of BRIDGED) {
		saved.set(name, Reflect.getOwnPropertyDescriptor(globalThis, name));
		Object.defineProperty(globalThis, name, {
			configurable: true,
			writable: true,
			value: name === "window" ? window : window[name],
		});
	}

	// The same object on both sides of the realm seam, not a copy.
	saved.set("_$HY", Reflect.getOwnPropertyDescriptor(globalThis, "_$HY"));
	Object.defineProperty(globalThis, "_$HY", { configurable: true, writable: true, value: window._$HY });

	// `fetch` is saved so a fixture's stub cannot outlive its page. It is NOT
	// bridged from the window: jsdom does not implement fetch, and a fixture that
	// needs one installs it (see below on why it installs two).
	saved.set("fetch", Reflect.getOwnPropertyDescriptor(globalThis, "fetch"));

	// The page's modules are imported by node, so their `console` is node's and
	// their output never reaches the VirtualConsole above. Capturing both into one
	// list is what makes "the page logged nothing" a claim about the page rather
	// than about one of its two realms.
	//
	// This was not a refinement. The loader is a node module, so every message it
	// wrote went to a console nothing was reading, and the check asserting the
	// loader logged nothing was passing because the list it read could not be
	// filled -- a check that cannot fail. It matters now: the loader's dev build
	// warns about lost hydration, and the search island reports a failed request.
	const nodeConsole = globalThis.console;
	const captured = Object.create(nodeConsole);
	for (const level of ["error", "warn", "info", "log", "debug"]) {
		captured[level] = (...args) => messages.push({ level, realm: "node", text: args.map(String).join(" ") });
	}
	saved.set("console", Reflect.getOwnPropertyDescriptor(globalThis, "console"));
	Object.defineProperty(globalThis, "console", { configurable: true, writable: true, value: captured });

	return {
		window,
		document: window.document,
		console: messages,
		serialize: () => dom.serialize(),
		restore() {
			for (const [name, descriptor] of saved) {
				if (descriptor) Object.defineProperty(globalThis, name, descriptor);
				else delete globalThis[name];
			}
			window.close();
		},
	};
}

/**
 * Watch a subtree for every change a DOM can report.
 *
 * `attributeOldValue` and `characterDataOldValue` are on because a parity report
 * that says "one attribute changed" without saying from what is not worth
 * reading.
 *
 * @param {Window} window the realm the observer and the target belong to
 * @param {Node} target
 */
export function watch(window, target) {
	const records = [];
	const observer = new window.MutationObserver(batch => records.push(...batch));
	observer.observe(target, {
		childList: true,
		subtree: true,
		characterData: true,
		attributes: true,
		attributeOldValue: true,
		characterDataOldValue: true,
	});

	return {
		/**
		 * Every record since the last drain.
		 *
		 * `takeRecords` is what makes this synchronous. Records are normally
		 * delivered in a microtask, so a run that only read the callback's array
		 * would report zero mutations for anything that happened in the same tick
		 * -- which is all of hydration. Draining the queue by hand cannot
		 * double-count, because delivering a record to the callback removes it from
		 * the queue and vice versa.
		 */
		drain() {
			records.push(...observer.takeRecords());
			return records.splice(0, records.length).map(describe);
		},
		stop: () => observer.disconnect(),
	};
}

/**
 * A mutation record as a line a person can read.
 *
 * @param {MutationRecord} record
 */
export function describe(record) {
	const where = path(record.target);
	if (record.type === "characterData") {
		return { type: "characterData", where, from: record.oldValue, to: record.target.data };
	}
	if (record.type === "attributes") {
		return {
			type: "attributes",
			where,
			name: record.attributeName,
			from: record.oldValue,
			to: record.target.getAttribute(record.attributeName),
		};
	}
	return {
		type: "childList",
		where,
		added: [...record.addedNodes].map(node),
		removed: [...record.removedNodes].map(node),
	};
}

/** A node as a short readable token. */
function node(value) {
	if (value.nodeType === 3) return `text ${JSON.stringify(value.data)}`;
	if (value.nodeType === 8) return `comment ${JSON.stringify(value.data)}`;
	return `<${value.nodeName.toLowerCase()}>`;
}

/** Where in the tree a node is, as a chain of tag names. */
function path(target) {
	const parts = [];
	for (let at = target; at && at.nodeType !== 9; at = at.parentNode) {
		parts.unshift(at.nodeType === 1 ? at.nodeName.toLowerCase() : node(at));
	}
	return parts.slice(-3).join(" > ");
}
