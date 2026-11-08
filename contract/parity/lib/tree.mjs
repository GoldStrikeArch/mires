// Does the client's template describe the same tree the server sent?
//
// This is the question underneath hydration. The compiled module walks from the
// node it claims by fixed steps -- `_3.firstChild`, `$t0.nextSibling` -- computed
// against the template it was compiled from. If the server's tree differs from
// that template by even one node, every walk past the difference lands somewhere
// else, and the symptom is not an error but a handler bound to the wrong button.
//
// Comparing the two as strings cannot answer it. The template writes its holes
// with the short marker syntax (`<!$>`, a bogus comment the parser turns into a
// comment node with data `$`) while the server writes the long one (`<!--$-->`),
// and the server's markers have the rendered value between them where the
// template has nothing. Both differences are correct. So the comparison is done
// on parsed trees, where the two marker syntaxes are simply the same node, and
// the value between a marker pair is skipped as the hole it is.
//
// Two other differences are expected and allowed:
//
// - `data-hk`, which only the server writes. It IS the hydration key, so it is
//   checked separately and by name, not by tree comparison.
// - attribute order, since a DOM has none.

/** Attributes the server adds that the template is not expected to carry. */
const SERVER_ONLY_ATTRIBUTES = new Set(["data-hk"]);

/**
 * Compare the tree a template describes against the tree the server sent.
 *
 * @param {Document} document a document to parse the template with
 * @param {string} templateHtml the template string the compiled module declares
 * @param {Element} serverRoot the server's node the template corresponds to
 * @returns {{ok: boolean, differences: string[], holes: {where: string, content: string}[]}}
 */
export function templateParity(document, templateHtml, serverRoot) {
	const host = document.createElement("template");
	host.innerHTML = templateHtml;
	const templateRoot = host.content.firstChild;

	const differences = [];
	const holes = [];
	if (!templateRoot) {
		differences.push("the template string parses to nothing");
		return { ok: false, differences, holes };
	}
	compareNode(templateRoot, serverRoot, tag(serverRoot), { differences, holes });
	return { ok: differences.length === 0, differences, holes };
}

function compareNode(expected, actual, where, out) {
	if (!actual) {
		out.differences.push(`${where}: the server tree ends where the template has ${describe(expected)}`);
		return;
	}
	if (expected.nodeType !== actual.nodeType) {
		out.differences.push(`${where}: template has ${describe(expected)}, server has ${describe(actual)}`);
		return;
	}
	if (expected.nodeType === 3 || expected.nodeType === 8) {
		if (expected.data !== actual.data) {
			out.differences.push(`${where}: template has ${describe(expected)}, server has ${describe(actual)}`);
		}
		return;
	}
	if (expected.nodeName !== actual.nodeName) {
		out.differences.push(`${where}: template has <${expected.nodeName.toLowerCase()}>, server has <${actual.nodeName.toLowerCase()}>`);
		return;
	}
	compareAttributes(expected, actual, where, out);
	compareChildren(expected, actual, where, out);
}

function compareAttributes(expected, actual, where, out) {
	const wanted = attributeMap(expected);
	const got = attributeMap(actual);
	for (const [name, value] of wanted) {
		if (!got.has(name)) out.differences.push(`${where}: the server is missing ${name}="${value}"`);
		else if (got.get(name) !== value) out.differences.push(`${where}: ${name} is "${value}" in the template and "${got.get(name)}" on the server`);
	}
	for (const [name, value] of got) {
		if (!wanted.has(name)) out.differences.push(`${where}: the server adds ${name}="${value}", which the template does not declare`);
	}
}

function attributeMap(element) {
	const map = new Map();
	for (const attribute of element.attributes) {
		if (!SERVER_ONLY_ATTRIBUTES.has(attribute.name)) map.set(attribute.name, attribute.value);
	}
	return map;
}

/**
 * Walk two child lists together, treating a `$`../`/` marker pair in the
 * template as a hole that matches the server's marker pair and whatever is
 * between them.
 *
 * The `/` is found by the same depth counting `getNextMarker` uses, so nested
 * holes are skipped as one.
 */
function compareChildren(expected, actual, where, out) {
	let e = expected.firstChild;
	let a = actual.firstChild;

	while (e || a) {
		if (e && isMarker(e, "$")) {
			if (!a || !isMarker(a, "$")) {
				out.differences.push(`${where}: the template opens a hole where the server has ${a ? describe(a) : "nothing"}`);
				return;
			}
			// The template's hole is empty; the server's carries the value.
			const templateEnd = closingMarker(e.nextSibling);
			const serverEnd = closingMarker(a.nextSibling);
			if (!templateEnd || !serverEnd) {
				out.differences.push(`${where}: an unterminated hole (template ${templateEnd ? "closed" : "open"}, server ${serverEnd ? "closed" : "open"})`);
				return;
			}
			out.holes.push({ where, content: between(a, serverEnd) });
			e = templateEnd.nextSibling;
			a = serverEnd.nextSibling;
			continue;
		}
		if (!e) {
			// A SERVER-RENDERED LOOP. The dashboard is the first island whose list
			// arrives filled: its `for` body is hoisted into its own template, so the
			// parent template declares an EMPTY `<ul>` while the server sends five
			// `<li>`s. There are no `$`/`/` markers to align against, because element
			// content is hydrated by key rather than by marker, so the hole machinery
			// above never sees it and the rows read as undeclared children.
			//
			// They are declared, just not here: each row is an instance of another
			// template the same module declares, and each carries the `data-hk` that
			// says the runtime intends to claim it. So a keyed element is recorded as
			// hole content rather than as a difference.
			//
			// THE `data-hk` IS WHAT KEEPS THIS HONEST, and it is the reason this is not
			// simply "trailing children are fine". The `tree` negative control appends a
			// plain `<span>` with no key, which still reads as a difference and still
			// trips the control. An emitter that stopped declaring a real child would
			// also produce an unkeyed extra, so the check it was written for survives.
			if (a.nodeType === 1 && a.hasAttribute?.("data-hk")) {
				out.holes.push({ where, content: a.outerHTML });
				a = a.nextSibling;
				continue;
			}
			out.differences.push(`${where}: the server has an extra ${describe(a)} the template does not declare`);
			return;
		}
		compareNode(e, a, `${where} > ${tag(e)}`, out);
		if (!a) return;
		e = e.nextSibling;
		a = a.nextSibling;
	}
}

/** The `/` that closes a hole, by the depth counting `getNextMarker` uses. */
function closingMarker(start) {
	let depth = 0;
	for (let at = start; at; at = at.nextSibling) {
		if (isMarker(at, "$")) depth++;
		else if (isMarker(at, "/")) {
			if (depth === 0) return at;
			depth--;
		}
	}
	return null;
}

/** The serialized content between an opening marker and its closing one. */
function between(open, close) {
	let html = "";
	for (let at = open.nextSibling; at && at !== close; at = at.nextSibling) {
		html += at.nodeType === 1 ? at.outerHTML : at.nodeType === 8 ? `<!--${at.data}-->` : at.data;
	}
	return html;
}

function isMarker(node, data) {
	return node.nodeType === 8 && node.data === data;
}

function describe(node) {
	if (node.nodeType === 3) return `text ${JSON.stringify(node.data)}`;
	if (node.nodeType === 8) return `comment ${JSON.stringify(node.data)}`;
	return `<${node.nodeName.toLowerCase()}>`;
}

function tag(node) {
	return node.nodeType === 1 ? node.nodeName.toLowerCase() : describe(node);
}
