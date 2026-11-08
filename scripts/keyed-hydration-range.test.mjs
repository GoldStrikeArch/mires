import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { open, watch } from "../contract/parity/lib/dom.mjs";

const report = console.log.bind(console);
const programPath = path.resolve(process.argv[2] ?? "");
const mode = process.argv[3] ?? "matching";
const modes = new Set(["matching", "extra", "missing", "stray-node", "bad-boundary"]);
if (!process.argv[2] || !modes.has(mode)) {
  console.error(
    "usage: keyed-hydration-range.test.mjs <compiled-program.mjs> " +
      "<matching|extra|missing|stray-node|bad-boundary>",
  );
  process.exit(2);
}

const row = (offset, label) =>
  `<span class="row-label" data-hk="k.${offset}">${label}</span>` +
  `<input class="row-input" data-hk="k.${offset + 1}" value="seed">`;
let serverRows = [row(1, 1), row(3, 2), row(5, 3)];
if (mode === "extra") serverRows.push(row(7, 4));
if (mode === "missing") serverRows[2] = `<span class="row-label" data-hk="k.5">3</span>`;
if (mode === "stray-node") serverRows[0] = serverRows[0].replace(
  "<input",
  "<em>stale</em><input",
);

const page = open(`<!doctype html><body>
  <div id="mount"><section data-hk="k.0"><button>update</button><div class="range"><b>before</b><!--$-->${serverRows.join("")}<!--${mode === "bad-boundary" ? "?" : "/"}--><i>after</i></div><aside>outside</aside></section></div>
  <script>globalThis._$HY = { completed: new WeakSet(), done: false, events: [], r: {} };</script>
</body>`);

const between = (parent, opening, closing) => {
  const nodes = [];
  for (let node = opening.nextSibling; node !== closing; node = node.nextSibling) {
    if (node === null) throw new Error("the keyed closing marker is unreachable");
    nodes.push(node);
  }
  return nodes;
};

try {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const dom = await import(pathToFileURL(path.resolve(
    here,
    "..",
    "runtime",
    "dom",
    "dist",
    "topcoat-dom.js",
  )).href);
  const compiled = await import(pathToFileURL(programPath).href);
  const rt = globalThis.__rt;
  rt._keyed_states.clear();
  rt.keyed_metrics_reset();

  const mount = document.querySelector("#mount");
  const section = mount.querySelector("section");
  const control = section.querySelector("button");
  const range = section.querySelector("div.range");
  const outside = section.querySelector("aside");
  const before = range.querySelector("b");
  const after = range.querySelector("i");
  const opening = [...range.childNodes].find(node => node.nodeType === 8 && node.data === "$");
  const closing = [...range.childNodes].find(node => node.nodeType === 8 && node !== opening);
  const beforeRange = between(range, opening, closing);
  const typed = range.querySelector('[data-hk="k.4"]');
  if (mode === "matching") {
    typed.value = "typed";
    typed.setSelectionRange(1, 3);
    typed.focus();
  }

  const mutations = watch(page.window, mount);
  const dispose = dom.hydrate(() => compiled.rust_entry(), mount, { renderId: "k." });
  const hydrationMutations = mutations.drain();
  const [state] = rt._keyed_states;
  const hydratedRange = between(range, opening, closing);

  assert.ok(state);
  assert.equal(mount.querySelector("section"), section);
  assert.equal(section.querySelector("button"), control);
  assert.equal(section.querySelector("div.range"), range);
  assert.equal(section.querySelector("aside"), outside);
  assert.equal(range.querySelector("b"), before);
  assert.equal(range.querySelector("i"), after);
  assert.equal(state.dom.parent, range);
  assert.equal(state.dom.end, closing);
  assert.equal(state.dom.ownsEnd, false);
  assert.deepEqual(state.order.map(record => record.nodes.length), [2, 2, 2]);
  assert.deepEqual(
    hydratedRange.filter(node => node.matches?.("span")).map(node => node.textContent),
    ["1", "2", "3"],
  );
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);

  if (mode === "matching") {
    assert.deepEqual(hydrationMutations, []);
    assert.deepEqual(hydratedRange, beforeRange);
    assert.equal(document.activeElement, typed);
    assert.equal(typed.value, "typed");
    assert.equal(typed.selectionStart, 1);
    assert.equal(typed.selectionEnd, 3);
    assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 3);
    assert.equal(rt.keyed_metrics_snapshot().hydrationFallbacks, 0);
    assert.equal(rt.keyed_metrics_snapshot().rangeReplacements, 0);
  } else {
    assert.notDeepEqual(hydrationMutations, []);
    assert.equal(
      hydrationMutations.every(record => record.where.includes("div")),
      true,
      `fallback escaped the marker range: ${JSON.stringify(hydrationMutations)}`,
    );
    assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 0);
    assert.equal(rt.keyed_metrics_snapshot().hydrationFallbacks, 1);
    assert.equal(rt.keyed_metrics_snapshot().rangeReplacements, 1);
  }

  const rowNodes = new Map([...state.rows].map(([key, record]) => [key, record.nodes.slice()]));
  control.click();
  assert.deepEqual(
    between(range, opening, closing),
    [
      ...rowNodes.get(3),
      ...rowNodes.get(1),
      ...rowNodes.get(2),
    ],
  );
  assert.deepEqual(
    [...range.querySelectorAll("span.row-label")].map(node => node.textContent),
    ["30", "10", "20"],
  );
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);
  if (mode === "matching") {
    assert.equal(document.activeElement, typed);
    assert.equal(typed.value, "typed");
    assert.equal(typed.selectionStart, 1);
    assert.equal(typed.selectionEnd, 3);
  }

  mutations.stop();
  dispose();
  assert.equal(state.disposed, true);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
  assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
  assert.equal(mount.childNodes.length, 0);
  assert.deepEqual(page.console, []);
  report(`compiled keyed-list range hydration: pass (${mode})`);
} finally {
  page.restore();
}
