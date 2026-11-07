// Pull every template HTML string out of the corpus's compiled reference
// output, so the tree-construction oracle can be fed the templates the
// REFERENCE COMPILER actually emits rather than only hand-picked ones.
//
//   node --import ./register-loader.mjs extract-templates.mjs          list them
//   node --import ./register-loader.mjs extract-templates.mjs --json   as JSON
//
// As a library:
//   import { corpusTemplates } from "./extract-templates.mjs";
//
// HOW THE STRINGS ARE FOUND
// -------------------------
// By parsing, not by regex. The compiled module is parsed with the pinned
// @babel/parser; the local name bound by `import { template as X } from
// "r-dom"` is resolved, and every `X(...)` call's first argument is read. That
// survives the plugin renaming `_tmpl$` however it likes, and it cannot be
// fooled by a template string that happens to contain something call-shaped.
//
// HOW `closed` IS DERIVED  (the fussy part)
// -----------------------------------------
// upstream's validator does not check the emitted template string. It checks
// `templateWithClosingTags`: the same markup with every tag explicitly closed
// and ALL ATTRIBUTES REMOVED, which the compiler maintains alongside the
// emitted string as it walks the JSX.
//
// We do not have that string, so we rebuild it -- and we rebuild it with a flat
// TOKENIZER, never with parse5. That restriction is the whole point. Deriving
// `closed` by parsing with parse5 and re-serialising would make the validator's
// parse-and-re-serialise round trip pass by construction, turning the check into
// a tautology. A tokenizer only closes what the text left open, so if the parser
// would restructure the markup the round trip still catches it.

import { readFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { VENDOR, FIXTURES } from "./paths.mjs";

const require = createRequire(path.join(VENDOR, "package.json"));
const parser = require("@babel/parser");

const CORPUS = path.join(FIXTURES, "corpus");

/** HTML void elements: never pushed on the stack, never given a closing tag. */
const VOID = new Set([
  "area", "base", "br", "col", "embed", "hr", "img", "input", "link",
  "meta", "param", "source", "track", "wbr"
]);

/** Elements whose content is raw or escapable-raw text: skip to the close tag. */
const RAW_TEXT = new Set(["script", "style", "textarea", "title"]);

/**
 * Rebuild `templateWithClosingTags` from an emitted template string: strip
 * attributes, normalise self-closing syntax, and close every tag the text left
 * open, innermost first. Comment markers (`<!>`, `<!$>`, `<!/>`) pass through
 * untouched, matching the hand-written cases already in the oracle.
 */
export function closeTags(html) {
  let out = "";
  const stack = [];
  let i = 0;

  while (i < html.length) {
    const lt = html.indexOf("<", i);
    if (lt < 0) {
      out += html.slice(i);
      break;
    }
    out += html.slice(i, lt);

    // A comment or marker: copy verbatim through the next ">".
    if (html[lt + 1] === "!") {
      const gt = html.indexOf(">", lt);
      if (gt < 0) {
        out += html.slice(lt);
        break;
      }
      out += html.slice(lt, gt + 1);
      i = gt + 1;
      continue;
    }

    // A closing tag.
    if (html[lt + 1] === "/") {
      const gt = html.indexOf(">", lt);
      if (gt < 0) {
        out += html.slice(lt);
        break;
      }
      const name = html.slice(lt + 2, gt).trim().toLowerCase();
      const at = stack.lastIndexOf(name);
      if (at >= 0) {
        // Close anything the text left open inside it, then it.
        while (stack.length > at + 1) out += `</${stack.pop()}>`;
        stack.pop();
      }
      out += `</${name}>`;
      i = gt + 1;
      continue;
    }

    // An opening tag. Find its end, respecting quoted attribute values so a
    // ">" inside one does not terminate the tag early.
    let j = lt + 1;
    let quote = null;
    while (j < html.length) {
      const c = html[j];
      if (quote) {
        if (c === quote) quote = null;
      } else if (c === '"' || c === "'") {
        quote = c;
      } else if (c === ">") {
        break;
      }
      j++;
    }
    if (j >= html.length) {
      // Truncated tag; nothing sensible to do but keep it.
      out += html.slice(lt);
      break;
    }

    const inner = html.slice(lt + 1, j);
    const selfClosing = inner.endsWith("/");
    const name = (inner.match(/^[^\s/>]+/) ?? [""])[0].toLowerCase();
    out += `<${name}>`;
    i = j + 1;

    if (VOID.has(name) || selfClosing) {
      // `<br/>` serialises back as `<br>`: no closing tag either way.
      if (!VOID.has(name) && selfClosing) out += `</${name}>`;
      continue;
    }

    if (RAW_TEXT.has(name)) {
      // Content is text, not markup. Copy it through and close explicitly.
      const close = html.toLowerCase().indexOf(`</${name}`, i);
      if (close < 0) {
        out += html.slice(i);
        out += `</${name}>`;
        i = html.length;
      } else {
        out += html.slice(i, close);
        const gt = html.indexOf(">", close);
        out += `</${name}>`;
        i = gt < 0 ? html.length : gt + 1;
      }
      continue;
    }

    stack.push(name);
  }

  while (stack.length) out += `</${stack.pop()}>`;
  return out;
}

/**
 * Every `_$template(...)` argument in one compiled module, in source order.
 * Returns the raw strings; a call whose first argument is not a plain literal
 * is skipped and reported, since that would mean the plugin built a template
 * some way we do not model.
 */
export function templatesIn(code) {
  const ast = parser.parse(code, { sourceType: "module" });

  // Resolve the local binding for the imported `template`.
  const locals = new Set();
  for (const node of ast.program.body) {
    if (node.type !== "ImportDeclaration") continue;
    for (const spec of node.specifiers) {
      if (spec.type === "ImportSpecifier" && (spec.imported.name ?? spec.imported.value) === "template") {
        locals.add(spec.local.name);
      }
    }
  }
  if (!locals.size) return { templates: [], skipped: [] };

  const templates = [];
  const skipped = [];

  const literal = (n) => {
    if (!n) return null;
    if (n.type === "StringLiteral") return n.value;
    if (n.type === "TemplateLiteral" && n.expressions.length === 0 && n.quasis.length === 1) {
      return n.quasis[0].value.cooked;
    }
    return null;
  };

  const seen = new Set();
  const walk = (node) => {
    if (!node || typeof node !== "object" || seen.has(node)) return;
    if (Array.isArray(node)) {
      for (const c of node) walk(c);
      return;
    }
    if (typeof node.type !== "string") return;
    seen.add(node);

    if (node.type === "CallExpression" && node.callee.type === "Identifier" && locals.has(node.callee.name)) {
      const html = literal(node.arguments[0]);
      if (html === null) skipped.push(node.arguments[0]?.type ?? "missing");
      else {
        templates.push({
          html,
          // The plugin appends these three booleans only when at least one is
          // true (CONTRACT-DOM 2.4), so their absence is meaningful.
          isImportNode: node.arguments[1]?.value ?? null,
          isSVG: node.arguments[2]?.value ?? null,
          isMathML: node.arguments[3]?.value ?? null
        });
      }
    }

    for (const key of Object.keys(node)) {
      if (key === "loc" || key === "leadingComments" || key === "trailingComments") continue;
      walk(node[key]);
    }
  };
  walk(ast.program.body);

  return { templates, skipped };
}

/**
 * Every distinct template emitted anywhere in the corpus, as oracle cases.
 *
 * Both the hydratable and non-hydratable outputs are read: hydration changes the
 * template string (marker pairs appear, anchors move), so the two presets emit
 * genuinely different markup from the same source and both need walking.
 *
 * Deduplicated by html string. `sources` lists every place it came from, which
 * is how a reader tells an incidental template from a load-bearing one.
 */
export function corpusTemplates() {
  if (!existsSync(CORPUS)) return [];

  const byHtml = new Map();
  const { readdirSync } = require("node:fs");

  for (const family of readdirSync(CORPUS, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort()) {
    for (const [file, preset] of [
      ["expected.reference.js", "dom"],
      ["expected.reference.hydratable.js", "domHydratable"]
    ]) {
      const p = path.join(CORPUS, family, file);
      if (!existsSync(p)) continue;
      const { templates } = templatesIn(readFileSync(p, "utf8"));
      templates.forEach((t, n) => {
        const source = `${family}/${preset}#${n}`;
        const hit = byHtml.get(t.html);
        if (hit) {
          hit.sources.push(source);
          return;
        }
        byHtml.set(t.html, {
          id: `corpus:${family}:${preset}:${n}`,
          html: t.html,
          closed: closeTags(t.html),
          flags: { isImportNode: t.isImportNode, isSVG: t.isSVG, isMathML: t.isMathML },
          sources: [source],
          fromCorpus: true
        });
      });
    }
  }

  return [...byHtml.values()];
}

// ------------------------------------------------------------------- CLI
if (import.meta.url === `file://${process.argv[1]}`) {
  const all = corpusTemplates();
  if (process.argv.includes("--json")) {
    process.stdout.write(`${JSON.stringify(all, null, 2)}\n`);
  } else {
    for (const t of all) {
      console.log(`${t.id}`);
      console.log(`  html:   ${JSON.stringify(t.html)}`);
      console.log(`  closed: ${JSON.stringify(t.closed)}`);
      if (t.sources.length > 1) console.log(`  also:   ${t.sources.slice(1).join(", ")}`);
    }
    console.log(`\n${all.length} distinct templates across the corpus`);
  }
}
