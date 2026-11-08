import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { open, watch } from "../contract/parity/lib/dom.mjs";

const report = console.log.bind(console);
const programPath = path.resolve(process.argv[2] ?? "");
if (!process.argv[2]) {
  console.error("usage: keyed-hydration-cardinality.test.mjs <compiled-program.mjs>");
  process.exit(2);
}

const page = open(`<!doctype html><body>
  <div id="mount"><section data-hk="k.0"><button class="add">add</button><ul class="empty"></ul><button class="update">update</button><ul class="one"><li data-hk="k.1">1</li></ul><aside>outside</aside></section></div>
  <script>globalThis._$HY = { completed: new WeakSet(), done: false, events: [], r: {} };</script>
</body>`);

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
  const emptyList = section.querySelector("ul.empty");
  const oneList = section.querySelector("ul.one");
  const oneRow = oneList.querySelector("li");
  const outside = section.querySelector("aside");
  const mutations = watch(page.window, mount);
  const dispose = dom.hydrate(() => compiled.rust_entry(), mount, { renderId: "k." });

  assert.deepEqual(mutations.drain(), []);
  assert.equal(mount.querySelector("section"), section);
  assert.equal(section.querySelector("ul.empty"), emptyList);
  assert.equal(section.querySelector("ul.one"), oneList);
  assert.equal(section.querySelector("aside"), outside);
  assert.equal(oneList.querySelector("li"), oneRow);

  const states = [...rt._keyed_states];
  assert.equal(states.length, 2);
  const emptyState = states.find(state => state.order.length === 0);
  const oneState = states.find(state => state.order.length === 1);
  assert.ok(emptyState);
  assert.ok(oneState);
  assert.equal(emptyState.dom.parent, emptyList);
  assert.equal(oneState.dom.parent, oneList);
  assert.equal(emptyState.dom.end, null);
  assert.equal(oneState.dom.end, null);
  assert.equal(emptyState.dom.hydrated, true);
  assert.equal(oneState.dom.hydrated, true);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 1);
  assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 1);
  assert.equal(rt.keyed_metrics_snapshot().hydrationFallbacks, 0);

  section.querySelector("button.add").click();
  assert.deepEqual(
    [...emptyList.querySelectorAll("li")].map(node => node.textContent),
    ["2", "3"],
  );
  assert.deepEqual([...emptyState.rows.keys()], [2, 3]);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);

  section.querySelector("button.update").click();
  assert.equal(oneList.querySelector("li"), oneRow);
  assert.equal(oneRow.textContent, "10");
  assert.equal(oneState.rows.get(1).nodes[0], oneRow);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);

  mutations.stop();
  dispose();
  assert.equal(emptyState.disposed, true);
  assert.equal(oneState.disposed, true);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
  assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
  assert.equal(mount.childNodes.length, 0);
  assert.deepEqual(page.console, []);
  report("compiled keyed-list cardinality hydration: pass");
} finally {
  page.restore();
}
