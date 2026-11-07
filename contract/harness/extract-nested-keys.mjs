// Pin solid's hydration-key allocation ACROSS COMPONENT BOUNDARIES by driving
// both real runtimes over the same component trees.
//
//   node --import ./register-loader.mjs extract-nested-keys.mjs
//
// Writes fixtures/keys-nested.json.
//
// WHY A SECOND KEY FIXTURE
// ------------------------
// fixtures/keys.json pins the ENCODING: (context id, count) -> key, driven
// through sharedConfig with hand-set contexts. It says nothing about where a new
// context comes from. This file pins the NESTING: which slot a component
// consumes, what the child context's id is, and how a sibling after the
// component continues counting. That is the oracle for Topcoat's SSR
// `Formatter::enter_component()` and for the client-side `_$createComponent`
// emission, which must agree with each other exactly or hydration cannot find
// a single node underneath a component.
//
// THE SCHEME (read off the pins, then driven to confirm)
// -----------------------------------------------------
//   solid-js/dist/solid.js:121-132   sharedConfig + the two getters
//   solid-js/dist/solid.js:133-137   getContextId(count) = context.id + letter + count
//   solid-js/dist/solid.js:141-147   nextHydrateContext()
//   solid-js/dist/solid.js:1274-1285 createComponent (client, gated on enableHydration)
//   solid-js/dist/server.js:369-407  the same four things in the SSR build
//   dom-expressions/src/client.js:615-617  getHydrationKey = getNextContextId
//   dom-expressions/src/client.js:264-280  getNextElement consumes one key per template root
//   dom-expressions/src/server.js:522-525  getHydrationKey (server), :421-424 ssrHydrationKey
//
//   context      = { id, count }
//   key(ctx)     = ctx.id + letter(digits(ctx.count) - 1) + ctx.count , then ctx.count += 1
//   letter(0)    = ""   letter(n) = String.fromCharCode(96 + n)
//   enter_component(parent) -> child = { id: key(parent), count: 0 }
//                              // NOTE: key(parent) SPENDS the parent's slot, and the
//                              // spend survives the restore -- createComponent restores
//                              // the same mutated context object, not a copy.
//   exit_component  -> parent (already advanced by one)
//
// HOW IT IS DRIVEN
// ----------------
// Each case is a declarative tree of two node kinds: "e" (a hydratable template
// root) and { c: [...] } (a component boundary). The tree is run through:
//
//   the CLIENT path  dom-expressions/src/client.js `getNextElement` (with a
//                    recording registry so it takes the real "found the node"
//                    branch) + solid's client `createComponent`, after
//                    enableHydration(). Both come from the SAME solid instance
//                    client.js's "rxcore" seam resolves to (harness/rxcore.mjs),
//                    so the sharedConfig they mutate is one object.
//
//   the SERVER path  solid-js/web/dist/server.js `renderToString` over
//                    `ssr([...], ssrHydrationKey())` elements and its own
//                    `createComponent`. The data-hk attributes in the emitted
//                    HTML, in document order, are the server's answer.
//
// The two key sequences must be identical. That agreement is the whole point:
// Topcoat writes keys on the server and matches them on the client, so an
// oracle that only pinned one side could be satisfied by a scheme that cannot
// hydrate.

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  requireUpstream,
  writeJson,
  provenance,
  assertPinnedVersion,
  DOM_SRC,
  SOLID,
  VENDOR
} from "./paths.mjs";
import { sharedConfig, createComponent } from "./rxcore.mjs";
import { enableHydration } from "solid-js/dist/solid.js";

requireUpstream();

// Both copies of each pin: the hash-verified tarball in upstream/ (what
// CONTRACT-DOM.md cites by file:line) and the yarn install in
// vendor/node_modules/ (what the bare specifiers above resolve to). The client
// driver runs the node_modules copy, because that is the instance
// harness/rxcore.mjs hands to client.js; asserting both against upstream.lock
// keeps the cited lines and the executed bytes from drifting apart.
assertPinnedVersion(
  "dom-expressions",
  JSON.parse(readFileSync(path.join(DOM_SRC, "..", "package.json"), "utf8")).version
);
assertPinnedVersion(
  "solid-js",
  JSON.parse(readFileSync(path.join(SOLID, "package.json"), "utf8")).version
);
assertPinnedVersion(
  "solid-js",
  JSON.parse(readFileSync(path.join(VENDOR, "node_modules", "solid-js", "package.json"), "utf8"))
    .version
);

const client = await import(pathToFileURL(path.join(DOM_SRC, "client.js")).href);
const server = await import("solid-js/web/dist/server.js");

enableHydration();

// --------------------------------------------------------------- the tree DSL
// "e"          a hydratable template root: consumes one key in the current context
// { c: [...] } a component boundary: consumes one key, then opens a child context
const e = "e";
const c = (...children) => ({ c: children });
const els = (n) => Array.from({ length: n }, () => e);

/**
 * Compact readable notation for a tree, e.g. `e C(e e C(e)) e`. Runs of
 * identical siblings collapse to `e*10`, so even a 10000-element case has a
 * one-line shape.
 */
function shapeOf(nodes) {
  const tokens = nodes.map((n) => (n === e ? "e" : `C(${n.c.length ? shapeOf(n.c) : ""})`));
  const out = [];
  for (const token of tokens) {
    const last = out[out.length - 1];
    if (last && last.token === token) last.n++;
    else out.push({ token, n: 1 });
  }
  return out.map(({ token, n }) => (n === 1 ? token : `${token}*${n}`)).join(" ");
}

function countNodes(nodes) {
  let total = 0;
  for (const n of nodes) total += n === e ? 1 : 1 + countNodes(n.c);
  return total;
}

// -------------------------------------------------------------- client driver
/**
 * Run one tree through dom-expressions' client hydration path.
 *
 * Returns the ordered element keys (what `getNextElement` looked up in the
 * registry) and one allocation record per node, including the component
 * boundaries -- which never appear in HTML, so this instrumentation is the only
 * place their ids are observable.
 */
function runClient(nodes, renderId) {
  const elementKeys = [];
  const allocations = [];

  // A registry that answers every lookup, so getNextElement takes the branch a
  // real successful hydration takes (client.js:268-279) rather than falling
  // through to template(). `completed` is left undefined, which skips :277.
  sharedConfig.registry = {
    get(key) {
      elementKeys.push(key);
      return { isConnected: true };
    },
    delete() {},
    has() {
      return false;
    }
  };
  sharedConfig.completed = undefined;
  sharedConfig.effects = undefined;
  sharedConfig.done = false;
  // client.js:252-255: hydrate() seeds the root context exactly like this.
  sharedConfig.context = { id: renderId || "", count: 0 };

  const walk = (list, depth, prefix) => {
    list.forEach((node, index) => {
      const ctx = sharedConfig.context;
      const parentContextId = ctx.id;
      const parentCountBefore = ctx.count;
      const nodePath = prefix ? `${prefix}.${index}` : String(index);

      if (node === e) {
        const before = elementKeys.length;
        client.getNextElement(() => ({ outerHTML: "<div></div>" }));
        allocations.push({
          kind: "element",
          path: nodePath,
          depth,
          slotIndex: index,
          parentContextId,
          parentCountBefore,
          key: elementKeys[before],
          parentCountAfter: ctx.count
        });
        return;
      }

      let childContextId = null;
      let childCountAfter = null;
      createComponent(() => {
        childContextId = sharedConfig.context.id;
        walk(node.c, depth + 1, nodePath);
        childCountAfter = sharedConfig.context.count;
      }, {});
      allocations.push({
        kind: "component",
        path: nodePath,
        depth,
        slotIndex: index,
        parentContextId,
        parentCountBefore,
        // the key the component SPENT in the parent, which IS the child context id
        key: childContextId,
        childContextId,
        childCountAfter,
        parentCountAfter: ctx.count
      });
    });
  };

  walk(nodes, 0, "");
  sharedConfig.context = null;
  sharedConfig.registry = undefined;
  return { elementKeys, allocations };
}

// -------------------------------------------------------------- server driver
/** Build the SSR value for one node. Children evaluate left to right, in order. */
function serverNode(node) {
  if (node === e) {
    return server.ssr(["<div", "></div>"], server.ssrHydrationKey());
  }
  return server.createComponent(() => node.c.map(serverNode), {});
}

function runServer(nodes, renderId) {
  const html = server.renderToString(
    () => nodes.map(serverNode),
    renderId ? { renderId } : {}
  );
  const keys = [...html.matchAll(/data-hk="([^"]*)"/g)].map((m) => m[1]);
  return { html, elementKeys: keys };
}

// ------------------------------------------------------------------- cases
const cases = [];
const addCase = (group, name, nodes, renderId = "") =>
  cases.push({ group, name, renderId, nodes });

// A. no components at all -- the flat baseline keys.json already covers,
//    repeated here so a nested case can be read against its unnested twin.
for (const n of [1, 2, 3, 10, 11]) addCase("flat", `flat-${n}`, els(n));

// B. one component at root, `pad` elements before it, `inner` elements inside.
//    pad crosses the letter boundary of the PARENT counter (the component's own
//    id); inner crosses the letter boundary of the CHILD counter.
for (const pad of [0, 1, 2, 9, 10, 11, 99, 100]) {
  for (const inner of [0, 1, 2, 10, 11]) {
    addCase(
      "one-component",
      `pad${pad}-inner${inner}`,
      [...els(pad), c(...els(inner)), e]
    );
  }
}

// C. siblings: several components in one context, with and without elements
//    between them. This is where trap #1 lives -- a component's spent slot must
//    not be replayed by the next sibling.
addCase("siblings", "two-components", [c(e), c(e)]);
addCase("siblings", "component-element-component", [c(e), e, c(e)]);
addCase("siblings", "element-around", [e, c(e), c(e), e]);
addCase("siblings", "three-components", [c(e), c(e), c(e)]);
addCase("siblings", "empty-then-component", [c(), c(e)]);
addCase("siblings", "eleven-components", els(0).concat(Array.from({ length: 11 }, () => c(e))));
addCase(
  "siblings",
  "eleven-components-then-element",
  Array.from({ length: 11 }, () => c(e)).concat([e])
);
addCase("siblings", "component-pairs-interleaved", [c(e, e), e, c(e, e), e, c(e, e)]);

// D. depth: a chain of components, one element at the deepest level and one at
//    each level, so every level's counter is exercised.
for (const depth of [2, 3, 4, 5]) {
  let node = c(e);
  for (let i = 1; i < depth; i++) node = c(e, node);
  addCase("depth", `chain-${depth}`, [node, e]);
}
// D2. depth where EVERY level crosses its own letter boundary before descending.
for (const depth of [2, 3]) {
  for (const pad of [10, 11]) {
    let node = c(...els(pad), e);
    for (let i = 1; i < depth; i++) node = c(...els(pad), node, e);
    addCase("depth", `chain-${depth}-pad${pad}`, [...els(pad), node, e]);
  }
}
addCase("depth", "chain-3-inner-100", [c(...els(100), c(...els(100), c(...els(100), e)))]);

// E. irregular shapes -- the ones a real page produces.
addCase("shape", "layout-page-island", [c(e, c(e), e, c(e, e), e)]);
addCase("shape", "component-only-tree", [c(c(c()))]);
addCase("shape", "empty-components-then-element", [c(), c(), c(), e]);
addCase("shape", "element-then-empty-component", [e, c(), e]);
addCase("shape", "wide-fragment-component", [c(...els(3)), c(...els(3)), c(...els(3))]);
addCase("shape", "deep-left-heavy", [c(c(c(e), e), e), e]);
addCase("shape", "deep-right-heavy", [e, c(e, c(e, c(e)))]);
addCase("shape", "component-in-second-slot", [e, c(e, e), e]);
addCase("shape", "nested-sibling-pairs", [c(c(e), c(e)), c(c(e), c(e))]);
addCase("shape", "single-component-single-element", [c(e)]);

// F. renderId: the root context id is the renderId, so every key gains it as a
//    prefix (client.js:253, server.js:37-42). "0" is included on purpose: a
//    renderId that looks like a count must still work, because the island
//    filter is a startsWith (CONTRACT-DOM.md point 11.2).
const RENDER_ID_SHAPE = [e, c(e, c(e), e), e];
for (const renderId of ["", "z", "r0", "0", "a10"]) {
  addCase("renderId", `renderId-${renderId || "empty"}`, RENDER_ID_SHAPE, renderId);
}

// G. the letter-boundary matrix, driven for real (no seeded contexts): the
//    component opens at a parent count that just crossed a digit length, and
//    its body crosses one too.
for (const pad of [9, 10, 99, 100, 999, 1000]) {
  for (const inner of [10, 100]) {
    addCase("boundary", `boundary-pad${pad}-inner${inner}`, [
      ...els(pad),
      c(...els(inner)),
      e
    ]);
  }
}
// The 5-digit letter ('d') at a component boundary. One case: it costs 10k
// elements per side.
addCase("boundary", "boundary-pad10000-inner1", [...els(10000), c(e), e]);

// H. the letter boundary of a CHILD context at a nested component boundary --
//    i.e. the inner component's own id crosses a digit length. This is the case
//    that distinguishes "the letter comes from the counter of the context the
//    component is opened in" from "the letter comes from the depth" or "from the
//    child index of the element list".
for (const pad of [0, 1, 9, 10, 11, 99, 100]) {
  addCase("inner-boundary", `inner-pad${pad}`, [c(...els(pad), c(e), e), e]);
}
addCase("inner-boundary", "inner-pad10-depth3", [c(...els(10), c(...els(10), c(e), e), e), e]);
addCase("inner-boundary", "inner-pad100-depth3", [c(...els(100), c(...els(100), c(e), e), e), e]);
for (const renderId of ["z", "0"]) {
  addCase("inner-boundary", `inner-pad10-renderId-${renderId}`, [c(...els(10), c(e), e), e], renderId);
}
addCase("inner-boundary", "component-first-in-child-context", [c(c(c(e), e), e)]);

// I. the exact trees of corpus family 07b, so its NOTES.md can cite a driven key
//    layout per case instead of asserting one. Names match the fn names in
//    fixtures/corpus/07b-nested-components/view.rs.
//
//    The pair that matters is `children-component-eager` against
//    `children-component-getter`. They are the SAME source shape -- a component
//    passed as another component's child content -- under the two prop models.
//    With Topcoat's eager children the inner component is built as an argument,
//    so its boundary is a SIBLING of the outer one in the parent context. With
//    solid's `get children()` the inner component is built when the outer body
//    reads the prop, so its boundary is INSIDE the outer context, which is
//    indistinguishable from having called it in the body.
addCase("corpus-07b", "07b-component-in-body", [c(e, c(e))]);
addCase("corpus-07b", "07b-children-component-eager", [c(e), c(e)]);
addCase("corpus-07b", "07b-children-component-getter", [c(e, c(e))]);
addCase("corpus-07b", "07b-depth-three", [c(e, c(e, c(e)))]);
addCase("corpus-07b", "07b-siblings-in-body", [c(e, c(e), c(e))]);
addCase("corpus-07b", "07b-nested-in-element", [e, c(e, c(e))]);

// -------------------------------------------------------------------- run
const VERBATIM_LIMIT = 24; // nodes; above this, store an elided view + hashes

const sha = (parts) => createHash("sha256").update(parts.join(" ")).digest("hex");

/** first N + last N of a long array, with the count of what was dropped. */
function elide(arr, n = 6) {
  if (arr.length <= n * 2) return arr;
  return [...arr.slice(0, n), `... ${arr.length - n * 2} elided ...`, ...arr.slice(-n)];
}

const results = [];
const mismatches = [];
let allocationTotal = 0;
let elementTotal = 0;

for (const kase of cases) {
  const { elementKeys, allocations } = runClient(kase.nodes, kase.renderId);
  const srv = runServer(kase.nodes, kase.renderId);

  const clientHash = sha(elementKeys);
  const serverHash = sha(srv.elementKeys);
  const agrees = clientHash === serverHash;
  if (!agrees) {
    mismatches.push({
      case: kase.name,
      client: elide(elementKeys),
      server: elide(srv.elementKeys)
    });
  }

  allocationTotal += allocations.length;
  elementTotal += elementKeys.length;

  const nodeCount = countNodes(kase.nodes);
  const verbatim = nodeCount <= VERBATIM_LIMIT;

  const row = {
    group: kase.group,
    name: kase.name,
    renderId: kase.renderId,
    shape: shapeOf(kase.nodes),
    nodeCount,
    elementKeyCount: elementKeys.length,
    clientServerAgree: agrees,
    elementKeysSha256: clientHash,
    elementKeys: verbatim ? elementKeys : elide(elementKeys),
    // The triples the emitter must reproduce. For a component the key IS the
    // child context's id; for an element it is the data-hk written by SSR.
    allocations: verbatim ? allocations : elide(allocations),
    // The server's own bytes, small cases only -- this is what data-hk looks
    // like in real SSR output.
    ssr: verbatim ? srv.html : undefined
  };
  if (!verbatim) row.$elided = `${nodeCount} nodes: only the ends are stored; the sha256 covers all keys`;
  results.push(row);
}

// ------------------------------------------------------- nesting triples table
// The (parent context id, parent count) -> child context id relation on its
// own, deduplicated across every case, which is the table a Rust
// `enter_component` can be unit-tested against directly.
const tripleMap = new Map();
for (const row of results) {
  const list = Array.isArray(row.allocations) ? row.allocations : [];
  for (const a of list) {
    if (typeof a !== "object" || a.kind !== "component") continue;
    const key = `${a.parentContextId} ${a.parentCountBefore}`;
    if (!tripleMap.has(key)) {
      tripleMap.set(key, {
        parentContextId: a.parentContextId,
        parentCount: a.parentCountBefore,
        childContextId: a.childContextId,
        parentCountAfter: a.parentCountAfter
      });
    }
  }
}
const nestedTriples = [...tripleMap.values()].sort(
  (a, b) =>
    a.parentContextId.localeCompare(b.parentContextId) || a.parentCount - b.parentCount
);

// --------------------------------------------------- getter child content
// The tree DSL above models Topcoat's EAGER child content: a child is built as
// an argument, so its keys come from the caller's context. Solid's compiled
// output instead passes `get children()`, so the child is built when the callee
// reads the prop, from the CALLEE's context. That is family 07b's one real
// divergence, so it is driven here rather than asserted, with real getter props
// through solid's SSR runtime.
//
// The three lazy rows differ only in WHERE the body reads props.children, which
// is the point: under the getter model the child's key depends on how far
// through its own allocations the body is at the moment it reads the prop, and a
// compiler cannot see that. Under the eager model the order is fixed by the
// source.
const leafSSR = () => server.ssr(["<span", "></span>"], server.ssrHydrationKey());
const wrap = (fn) => server.renderToString(fn, {});
const getterChildren = [
  {
    case: "lazy: body allocates its own root, then reads props.children",
    solidIdiomatic: true,
    html: wrap(() =>
      server.createComponent(
        (props) => server.ssr(["<div", ">", "</div>"], server.ssrHydrationKey(), props.children),
        {
          get children() {
            return leafSSR();
          }
        }
      )
    )
  },
  {
    case: "lazy: body destructures at entry, so the child is read first",
    solidIdiomatic: false,
    html: wrap(() =>
      server.createComponent(
        ({ children }) => server.ssr(["<div", ">", "</div>"], server.ssrHydrationKey(), children),
        {
          get children() {
            return leafSSR();
          }
        }
      )
    )
  },
  {
    case: "eager: the child is a plain prop value, built before the boundary",
    solidIdiomatic: false,
    topcoatShape: true,
    html: wrap(() =>
      server.createComponent(
        (props) => server.ssr(["<div", ">", "</div>"], server.ssrHydrationKey(), props.children),
        { children: leafSSR() }
      )
    )
  },
  {
    case: "eager, with a sibling element after the component",
    topcoatShape: true,
    html: wrap(() => [
      server.createComponent(
        (props) => server.ssr(["<div", ">", "</div>"], server.ssrHydrationKey(), props.children),
        { children: leafSSR() }
      ),
      leafSSR()
    ])
  },
  {
    case: "lazy, with a sibling element after the component",
    solidIdiomatic: true,
    html: wrap(() => [
      server.createComponent(
        (props) => server.ssr(["<div", ">", "</div>"], server.ssrHydrationKey(), props.children),
        {
          get children() {
            return leafSSR();
          }
        }
      ),
      leafSSR()
    ])
  }
];

// ------------------------------------------------------- boundary assertions
// Every property Topcoat's enter_component must have, asserted here so a
// version bump that changes any of them fails this extractor rather than
// silently rebaselining a fixture.
const checks = [];
const expect = (name, actual, wanted) => {
  const ok = JSON.stringify(actual) === JSON.stringify(wanted);
  checks.push({ name, actual, expected: wanted, ok });
};
const keysOf = (nodes, renderId = "") => runClient(nodes, renderId).elementKeys;
const ssrOf = (nodes, renderId = "") => runServer(nodes, renderId).html;

expect("a component's first child key is the spent parent slot plus 0", keysOf([c(e)]), ["00"]);
expect("depth 3, all counters 0, gives runs of zeros", keysOf([c(c(c(e)))]), ["0000"]);
expect(
  "a component spends exactly one parent slot; the next sibling skips it",
  keysOf([c(e), e]),
  ["00", "1"]
);
expect(
  "an EMPTY component still spends a parent slot",
  keysOf([c(), e]),
  ["1"]
);
expect(
  "the spend survives the restore for every sibling",
  keysOf([c(e), c(e), c(e)]),
  ["00", "10", "20"]
);
expect(
  "the parent counter, not the child index, feeds the letter",
  keysOf([...els(10), c(e)]),
  [...Array.from({ length: 10 }, (_, i) => String(i)), "a100"]
);
expect(
  "the child counter gets its own letter, independent of the parent's",
  keysOf([c(...els(11))]).slice(9),
  ["09", "0a10"]
);
expect(
  "a component opened at parent count 10 is prefixed a10",
  keysOf([...els(10), c(e), e]).slice(-2),
  ["a100", "a11"]
);
expect(
  "renderId prefixes every key, including nested ones",
  keysOf([c(e, c(e))], "z"),
  ["z00", "z010"]
);
expect(
  "an inner component's id crosses the CHILD context's letter boundary",
  keysOf([c(...els(10), c(e))]).slice(-1),
  ["0a100"]
);
expect(
  "server data-hk order equals client lookup order",
  ssrOf([e, c(e, c(e)), e]),
  '<div data-hk="0"></div><div data-hk="10"></div><div data-hk="110"></div><div data-hk="2"></div>'
);

const failed = checks.filter((k) => !k.ok);
if (failed.length || mismatches.length) {
  if (failed.length) {
    console.error("Nested key-scheme drift:");
    for (const f of failed) {
      console.error(`  ${f.name}\n    got      ${JSON.stringify(f.actual)}`);
      console.error(`    expected ${JSON.stringify(f.expected)}`);
    }
  }
  if (mismatches.length) {
    console.error("Client/server key disagreement:");
    for (const m of mismatches) console.error(`  ${m.case}`);
  }
  process.exit(1);
}

writeJson("keys-nested.json", {
  ...provenance("extract-nested-keys.mjs"),
  $what:
    "Hydration-key allocation ACROSS COMPONENT BOUNDARIES, driven through both " +
    "real runtimes (dom-expressions client hydration path and solid's SSR " +
    "renderToString) over the same declarative component trees. The oracle for " +
    "Topcoat's Formatter::enter_component and for _$createComponent emission.",
  $source: [
    "solid-js/dist/solid.js:121-132 (sharedConfig, getContextId, getNextContextId)",
    "solid-js/dist/solid.js:133-137 (getContextId: id + letter + count)",
    "solid-js/dist/solid.js:141-147 (nextHydrateContext)",
    "solid-js/dist/solid.js:1274-1285 (createComponent, client; gated on enableHydration)",
    "solid-js/dist/server.js:369-407 (the same four, SSR build; gated on !noHydrate)",
    "dom-expressions/src/client.js:615-617 (getHydrationKey), :264-280 (getNextElement), :244-262 (hydrate seeds the root context)",
    "dom-expressions/src/server.js:522-525 (getHydrationKey), :421-424 (ssrHydrationKey), :37-42 (root context)"
  ],
  $algorithm: [
    "context = { id, count }.",
    "key(ctx) = ctx.id + letter(digits(ctx.count) - 1) + String(ctx.count), then ctx.count += 1.",
    "letter(0) = ''; letter(n) = String.fromCharCode(96 + n)  // 1 digit -> '', 2 -> 'a', 3 -> 'b'",
    "Every hydratable template root consumes one key of the current context.",
    "enter_component(): child = { id: key(parent), count: 0 }. The call to key() SPENDS the parent's slot.",
    "exit_component(): restore the parent context -- whose count has ALREADY advanced past the component's slot.",
    "Therefore a component costs exactly one slot in its parent and opens a fresh namespace beneath it."
  ],
  $traps: [
    "Restoring a COPY of the parent context instead of the mutated parent replays the component's slot and shifts every later key. In JS the mutation is in place (this.context.count++ inside nextHydrateContext) and createComponent restores the same object reference (solid.js:1277-1280).",
    "The spread in nextHydrateContext is evaluated BEFORE the id, so the child inherits the parent's other fields at their pre-increment values, and `count: 0` then overrides the copied count.",
    "An empty component -- one that renders no hydratable root -- still spends a parent slot.",
    "The client's createComponent does not consult sharedConfig.done, so contexts keep nesting after a hydration mismatch; it DOES require enableHydration() to have run.",
    "sharedConfig.context.count is the key counter; sharedConfig.count is the async-hydration gate and is unrelated (see CONTRACT-DOM.md 9.3)."
  ],
  $whatIsRecorded: [
    "One row per case. `allocations` is the (parentContextId, parentCountBefore, slotIndex) -> key relation, in allocation order, for elements AND component boundaries.",
    "For a component row, `key` and `childContextId` are the same string: the parent slot the component spent IS the child context's id.",
    "`elementKeys` is what the client looked up in the registry, in order; the server's data-hk attributes in document order are asserted equal to it, per case, by sha256.",
    `Cases above ${VERBATIM_LIMIT} nodes store only the ends of each list; the sha256 still covers every key.`,
    "The tree DSL models KEY ALLOCATION ORDER AND SCOPING, not DOM nesting: every element is an empty sibling <div>, so the `ssr` field is a flat run of divs even where the real fixture nests them. Keys are unaffected, because a key is allocated when a template ROOT is created and a template's interior nodes never get one (point 8.2)."
  ],
  caseCount: results.length,
  allocationCount: allocationTotal,
  elementKeyCount: elementTotal,
  componentBoundaryCount: allocationTotal - elementTotal,
  clientServerAgreement: {
    cases: results.length,
    mismatches: mismatches.length,
    $how:
      "Per case: sha256 of the client's registry-lookup key sequence vs sha256 of the " +
      "data-hk attributes in solid's SSR output, in document order. Any mismatch fails " +
      "this extractor."
  },
  boundaryChecks: checks,
  getterChildren: {
    $what:
      "Solid's getter child content against Topcoat's eager child content, driven " +
      "through solid's SSR runtime. The tree cases above model the eager shape only.",
    $why:
      "Under `get children()` the child is built when the callee reads the prop, so its " +
      "key comes from the callee's context and depends on how far through its own " +
      "allocations the body is at that moment -- which a compiler cannot see. Under eager " +
      "child content the child is built as an argument, so it takes a slot in the CALLER's " +
      "context and the component's own slot shifts by one. See corpus family 07b NOTES.md.",
    rows: getterChildren
  },
  nestedTriples,
  cases: results
});

console.log(
  `keys-nested.json  ${results.length} cases, ${allocationTotal} allocations ` +
    `(${allocationTotal - elementTotal} component boundaries), ${nestedTriples.length} distinct ` +
    `nesting triples, ${checks.length} boundary checks green, client/server agree on all cases`
);
