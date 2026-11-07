// Tree-construction oracle.
//
//   node --import ./register-loader.mjs gen-tree-oracle.mjs
//
// Writes fixtures/templates.jsonl (one JSON object per line).
//
// WHY THIS EXISTS
// ---------------
// The compiled code reaches a dynamic position by walking .firstChild /
// .nextSibling from a cloned template root. Those walks are computed at COMPILE
// TIME against the compiler's idea of the tree, but executed at RUNTIME against
// whatever the browser's HTML parser actually built. If the two disagree, the
// walk lands on the wrong node -- silently.
//
// dom-expressions closes this gap with a parse5 round-trip check
// (plugin src/shared/validate.js). We do the same thing here, but record the
// RESULT as an oracle: for each template string, the tree parse5 builds and the
// exact firstChild/nextSibling path to every marked position.
//
// The emitter must agree with this file. Where it can't, the template is in the
// known-ambiguous set and must be rejected at compile time rather than mis-walked.
//
// parse5 is pinned to the same major the plugin itself depends on, so the
// oracle and the reference compiler cannot disagree about HTML parsing.

import { writeText, provenance, VENDOR, lock } from "./paths.mjs";
import { isInvalidMarkup, VALIDATE_SOURCE_PATH } from "./load-plugin-validate.mjs";
import { corpusTemplates } from "./extract-templates.mjs";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import path from "node:path";

const require = createRequire(path.join(VENDOR, "package.json"));
const parse5 = require("parse5");

// parse5's "exports" map does not expose ./package.json, so read the installed
// version off disk and cross-check it against the pin.
const parse5Version = JSON.parse(
  readFileSync(path.join(VENDOR, "node_modules", "parse5", "package.json"), "utf8")
).version;
if (parse5Version !== lock.npm.parse5.version) {
  throw new Error(
    `parse5 drift: upstream.lock pins ${lock.npm.parse5.version}, installed ${parse5Version}`
  );
}

// Parse in <body> context, exactly as `template.innerHTML = html` does in the
// browser and as the plugin's validate.js does at compile time.
const BODY_CONTEXT = parse5.parse(
  "<!DOCTYPE html><html><head></head><body></body></html>"
).childNodes[1].childNodes[1];

function parseFragment(html) {
  return parse5.parseFragment(BODY_CONTEXT, html);
}

const isElement = (n) => typeof n.tagName === "string";
const isText = (n) => n.nodeName === "#text";
const isComment = (n) => n.nodeName === "#comment";

/** Children as the DOM would expose them (parse5 keeps the same order). */
const kids = (n) => n.childNodes ?? [];

/**
 * Describe a node the way a walker cares about.
 */
function describe(n) {
  if (isText(n)) return { type: "text", value: n.value };
  if (isComment(n)) return { type: "comment", data: n.data };
  if (isElement(n)) {
    return {
      type: "element",
      tag: n.tagName,
      ns: n.namespaceURI,
      attrs: (n.attrs ?? []).map((a) => ({ name: a.name, value: a.value }))
    };
  }
  return { type: n.nodeName };
}

/** Recursive tree shape, for eyeballing and for diffing across versions. */
function shape(n) {
  const d = describe(n);
  const c = kids(n).map(shape);
  return c.length ? { ...d, children: c } : d;
}

/**
 * Enumerate every node with the firstChild/nextSibling path that reaches it
 * from the fragment root, in the same form the compiled code would emit.
 */
function paths(root) {
  const out = [];
  const walk = (node, expr, depth) => {
    const children = kids(node);
    for (let i = 0; i < children.length; i++) {
      const child = children[i];
      // The emitted walk: first child via .firstChild, thereafter chained
      // .nextSibling from the previous sibling.
      const childExpr = i === 0 ? `${expr}.firstChild` : `${out.at(-1).path}.nextSibling`;
      out.push({ path: childExpr, depth, node: describe(child) });
      walk(child, childExpr, depth + 1);
    }
  };
  walk(root, "root", 0);
  return out;
}

/**
 * Positions a compiler cares about: the comment markers dom-expressions emits
 * (<!>, <!$>, <!/>) which stand for dynamic insertion points.
 */
function markerPositions(all) {
  return all
    .filter((e) => e.node.type === "comment")
    .map((e) => ({ path: e.path, data: e.node.data, kind: markerKind(e.node.data) }));
}

function markerKind(data) {
  if (data === "") return "anchor (<!> -- single dynamic insertion point)";
  if (data === "$") return "open (<!$> -- start of a hydration marker pair)";
  if (data === "/") return "close (<!/> -- end of a hydration marker pair)";
  if (data.startsWith("!$")) return "ssr array separator (<!--!$-->)";
  return "other";
}

/**
 * The invariant the plugin enforces, run through UPSTREAM'S OWN validator
 * (src/shared/validate.js). It is given `templateWithClosingTags` -- the
 * closed, attribute-free form the compiler maintains alongside the emitted
 * template string -- because that is what upstream validates.
 *
 * Returns null when the parser leaves the markup alone, or the divergence.
 */
function validate(closed) {
  return isInvalidMarkup(closed) ?? null;
}


// ---------------------------------------------------------------- the corpus
//
// Each case carries TWO strings:
//
//   html    the template string as _$template() would receive it: tags left
//           unclosed where the compiler omits them, attributes present.
//   closed  `templateWithClosingTags` -- every tag explicitly closed and ALL
//           ATTRIBUTES REMOVED. This is the form upstream validates; see the
//           note at plugin src/shared/validate.js:45-49 referencing
//           solidjs/solid#2338.
//
// `ambiguous: true` records the EXPECTATION that upstream's validator rejects
// the case. Those are templates whose walk cannot be trusted and which an
// emitter must refuse rather than guess at.
const TEMPLATES = [
  // --- simple nesting -------------------------------------------------------
  { id: "single-element", html: "<div>", closed: "<div></div>", note: "unclosed -- the normal emitted form" },
  { id: "single-closed", html: "<div></div>", closed: "<div></div>" },
  { id: "nested-2", html: "<div><span>", closed: "<div><span></span></div>" },
  { id: "nested-3", html: "<div><span><a>", closed: "<div><span><a></a></span></div>" },
  { id: "siblings-2", html: "<div><span></span><span>", closed: "<div><span></span><span></span></div>" },
  { id: "siblings-3", html: "<div><a></a><b></b><i>", closed: "<div><a></a><b></b><i></i></div>" },
  {
    id: "deep-mixed",
    html: "<div><p>x</p><ul><li>a</li><li>b</li></ul><footer>",
    closed: "<div><p>x</p><ul><li>a</li><li>b</li></ul><footer></footer></div>"
  },
  { id: "text-only", html: "<span>hello", closed: "<span>hello</span>" },
  { id: "text-then-element", html: "<div>hi<span>", closed: "<div>hi<span></span></div>" },
  { id: "element-then-text", html: "<div><span></span>tail", closed: "<div><span></span>tail</div>" },

  // --- <!> anchors and hydration markers ------------------------------------
  { id: "anchor-only", html: "<div><!>", closed: "<div><!></div>" },
  { id: "anchor-between-text", html: "<span> <!> ", closed: "<span> <!> </span>" },
  { id: "anchor-after-element", html: "<div><div></div><!>", closed: "<div><div></div><!></div>" },
  { id: "hydration-pair", html: "<div><!$><!/>", closed: "<div><!$><!/></div>" },
  { id: "hydration-pair-with-text", html: "<module>Hi <!$><!/>", closed: "<module>Hi <!$><!/></module>" },
  { id: "two-anchors", html: "<div><!><!>", closed: "<div><!><!></div>" },
  {
    id: "nested-hydration-pairs",
    html: "<div><!$><span><!$><!/></span><!/>",
    closed: "<div><!$><span><!$><!/></span><!/></div>"
  },

  // --- void elements --------------------------------------------------------
  { id: "void-input", html: "<input>", closed: "<input>" },
  { id: "void-br-siblings", html: "<div>a<br>b<br>c", closed: "<div>a<br>b<br>c</div>" },
  { id: "void-img-attrs", html: '<img src="x" alt="y">', closed: "<img>" },
  { id: "void-hr-then", html: "<div><hr><span>", closed: "<div><hr><span></span></div>" },
  {
    id: "void-self-closed",
    html: "<div><br/><span>",
    closed: "<div><br><span></span></div>",
    note: "XHTML-style slash on a void element; serializes back without the slash"
  },

  // --- whitespace variants --------------------------------------------------
  { id: "ws-leading", html: "<div> <span>", closed: "<div> <span></span></div>" },
  { id: "ws-trailing", html: "<div><span></span> ", closed: "<div><span></span> </div>" },
  { id: "ws-between", html: "<div><a></a> <b></b>", closed: "<div><a></a> <b></b></div>" },
  { id: "ws-newline", html: "<div>\n  <span>\n", closed: "<div>\n  <span>\n</span></div>" },
  { id: "ws-only", html: "<div>   ", closed: "<div>   </div>" },
  { id: "ws-tab-nbsp", html: "<div>\t&nbsp;<span>", closed: "<div>\t&nbsp;<span></span></div>" },

  // --- attributes (unquoted form the plugin emits by default) ---------------
  // closed forms carry NO attributes -- that is what upstream validates.
  { id: "attr-unquoted", html: "<div id=main>", closed: "<div></div>" },
  { id: "attr-multiple", html: "<input id=entry type=text>", closed: "<input>" },
  { id: "attr-entity", html: "<div title=Search&amp;hellip;>", closed: "<div></div>" },
  { id: "attr-boolean", html: "<input disabled>", closed: "<input>" },

  // --- raw-text / escapable-raw-text containers -----------------------------
  {
    id: "style-braces",
    html: "<div><style>div { color: red; }</style><h1>x",
    closed: "<div><style>div { color: red; }</style><h1>x</h1></div>"
  },
  {
    id: "script-content",
    html: "<div><script>a<b</script><span>",
    closed: "<div><script>a<b</script><span></span></div>",
    note: "script is raw text: the < inside is NOT a tag"
  },
  {
    id: "textarea",
    html: "<div><textarea>a<b</textarea><span>",
    closed: "<div><textarea>a<b</textarea><span></span></div>",
    ambiguous: true,
    note:
      "textarea is ESCAPABLE raw text, unlike script which is raw text. A bare '<' " +
      "in its content round-trips as '&lt;', so upstream's validator rejects it " +
      "while the otherwise-identical <script> case passes. Any '<' inside a " +
      "textarea template must be pre-escaped by the emitter."
  },
  { id: "noscript", html: "<div><noscript>", closed: "<div><noscript></noscript></div>" },

  // --- KNOWN-AMBIGUOUS: table fragments -------------------------------------
  {
    id: "table-tbody-implied",
    html: "<table><tr><td>x",
    closed: "<table><tr><td>x</td></tr></table>",
    ambiguous: true,
    note: "the parser injects <tbody>; a walk that skips it is wrong"
  },
  {
    id: "table-explicit-tbody",
    html: "<table><tbody><tr><td>x",
    closed: "<table><tbody><tr><td>x</td></tr></tbody></table>"
  },
  {
    id: "table-fragment-orphan-tr",
    html: "<tr><td>x",
    closed: "<tr><td>x</td></tr>",
    note: "upstream's validator wraps a leading <tr> in <table><tbody> before checking"
  },
  {
    id: "table-with-div-inside",
    html: "<table><div></div><tr><td>x",
    closed: "<table><div></div><tr><td>x</td></tr></table>",
    ambiguous: true,
    note: "foster parenting moves the <div> OUT of the table, before it"
  },
  {
    id: "table-nested-ok",
    html: "<div><div><table><tbody></tbody></table></div><div>",
    closed: "<div><div><table><tbody></tbody></table></div><div></div></div>"
  },

  // --- KNOWN-AMBIGUOUS: select / option -------------------------------------
  {
    id: "select-option",
    html: "<select><option>a<option>b",
    closed: "<select><option>a<option>b</option></select>",
    ambiguous: true,
    note: "<option> auto-closes the previous <option>"
  },
  {
    id: "select-nested-div",
    html: "<select><div></div><option>a",
    closed: "<select><div></div><option>a</option></select>",
    ambiguous: true,
    note: "<div> is not permitted in <select> and is dropped"
  },
  {
    id: "optgroup",
    html: "<select><optgroup><option>a",
    closed: "<select><optgroup><option>a</option></optgroup></select>"
  },

  // --- KNOWN-AMBIGUOUS: <p> auto-close --------------------------------------
  {
    id: "p-auto-close-div",
    html: "<p>a<div>b",
    closed: "<p>a<div>b</div></p>",
    ambiguous: true,
    note: "<div> implicitly closes <p>, so <div> becomes a SIBLING not a child"
  },
  {
    id: "p-auto-close-p",
    html: "<p>a<p>b",
    closed: "<p>a<p>b</p></p>",
    ambiguous: true,
    note: "<p> implicitly closes <p>"
  },
  {
    id: "p-with-span",
    html: "<p>a<span>b",
    closed: "<p>a<span>b</span></p>",
    note: "phrasing content does NOT close <p>"
  },

  // --- other implicit-close hazards -----------------------------------------
  {
    id: "li-auto-close",
    html: "<ul><li>a<li>b",
    closed: "<ul><li>a<li>b</li></ul>",
    ambiguous: true
  },
  {
    id: "dd-dt-auto-close",
    html: "<dl><dt>a<dd>b",
    closed: "<dl><dt>a<dd>b</dd></dl>",
    ambiguous: true
  },
  {
    id: "form-nested",
    html: "<form><form><input>",
    closed: "<form><form><input></form></form>",
    ambiguous: true,
    note: "a nested <form> is ignored by the parser"
  },
  {
    id: "unclosed-tag-tolerance",
    html: "<div><span><a>text",
    closed: "<div><span><a>text</a></span></div>",
    note: "the emitted form: every tag left open, closed implicitly at fragment end"
  }
];

// ------------------------------------------------------- corpus templates
//
// Everything above is hand-chosen: cases picked because someone thought the
// parser might restructure them. That biases the oracle toward hazards and away
// from the ordinary. So the second source of cases is mechanical -- every
// distinct template string the REFERENCE COMPILER emitted anywhere in
// fixtures/corpus/, pulled out of the compiled output by
// extract-templates.mjs.
//
// Their `ambiguous` expectation is false, uniformly, and that is a real
// assertion rather than a default: the plugin runs this same validator before
// it commits to a template, and falls back to non-template codegen when it
// fails. A corpus template that upstream's validator rejects would mean the
// plugin emitted markup it had already judged unsafe, and it shows up below as
// a surprise.
const CORPUS = corpusTemplates().map((t) => ({ ...t, ambiguous: false }));

const ALL = [...TEMPLATES, ...CORPUS];

// ------------------------------------------------------------------- emit
const lines = [];
lines.push(
  JSON.stringify({
    $record: "header",
    ...provenance("gen-tree-oracle.mjs"),
    $format: "JSON Lines. This first line is the header; each later line is one template case.",
    $parse5Version: parse5Version,
    $validatorSource: VALIDATE_SOURCE_PATH.split("/upstream/")[1] ?? VALIDATE_SOURCE_PATH,
    $sources: {
      handWritten: `${TEMPLATES.length} cases chosen for the parser hazard each one probes`,
      corpus: `${CORPUS.length} distinct template strings the reference compiler emitted across fixtures/corpus/, extracted mechanically by extract-templates.mjs`
    },
    $fields: {
      id: "stable case name; corpus cases are prefixed `corpus:`",
      html: "the template string as passed to _$template() (tags may be unclosed, attributes present)",
      closed: "templateWithClosingTags: every tag closed, ALL attributes removed -- what upstream validates",
      ambiguous: "true when the HTML parser is EXPECTED to restructure the markup",
      invalidMarkup: "null when upstream's isInvalidMarkup() accepts it, else {html, browser}",
      tree: "the tree parse5 builds from `html`, in body context",
      walk: "every node with the firstChild/nextSibling expression that reaches it",
      markers: "the walk entries that are dom-expressions comment markers",
      fromCorpus: "true when the string was extracted from compiled corpus output rather than hand-written",
      sources: "corpus cases only: every <family>/<preset>#<n> the identical string came from",
      flags: "corpus cases only: the isImportNode/isSVG/isMathML arguments, null when the plugin omitted them"
    },
    $howToUse:
      "The emitter must produce the same firstChild/nextSibling path for a given " +
      "template as the `walk` recorded here. Where invalidMarkup is non-null the " +
      "template must be REJECTED at compile time, not walked."
  })
);

let invalidCount = 0;
const surprises = [];

for (const tpl of ALL) {
  const frag = parseFragment(tpl.html);
  const walk = paths(frag);
  const invalid = validate(tpl.closed);
  if (invalid) invalidCount++;

  const expectedAmbiguous = tpl.ambiguous === true;
  if (expectedAmbiguous !== Boolean(invalid)) {
    surprises.push({ id: tpl.id, expectedAmbiguous, actuallyInvalid: Boolean(invalid), invalid });
  }

  lines.push(
    JSON.stringify({
      id: tpl.id,
      html: tpl.html,
      closed: tpl.closed,
      note: tpl.note ?? null,
      ambiguous: expectedAmbiguous,
      invalidMarkup: invalid,
      tree: kids(frag).map(shape),
      walk: walk.map((e) => ({ path: e.path, node: e.node })),
      markers: markerPositions(walk),
      ...(tpl.fromCorpus ? { fromCorpus: true, sources: tpl.sources, flags: tpl.flags } : {})
    })
  );
}

writeText("templates.jsonl", `${lines.join("\n")}\n`);

console.log(
  `templates.jsonl  ${ALL.length} templates ` +
    `(${TEMPLATES.length} hand-written + ${CORPUS.length} from corpus), ` +
    `${invalidCount} rejected by upstream's validator`
);
if (surprises.length) {
  console.log(`\n  ${surprises.length} case(s) where the 'ambiguous' label disagrees with upstream:`);
  for (const s of surprises) {
    console.log(
      `    ${s.id}: labelled ambiguous=${s.expectedAmbiguous}, upstream rejects=${s.actuallyInvalid}`
    );
    if (s.invalid) {
      console.log(`        ours:    ${s.invalid.html}`);
      console.log(`        browser: ${s.invalid.browser}`);
    }
  }
}
