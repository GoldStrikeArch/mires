import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { open, watch } from "../contract/parity/lib/dom.mjs";

const report = console.log.bind(console);
const programPath = path.resolve(process.argv[2] ?? "");
const mode = process.argv[3] ?? "matching";
if (!process.argv[2] || !new Set(["matching", "inner-extra"]).has(mode)) {
  console.error(
    "usage: keyed-hydration-nested.test.mjs <compiled-program.mjs> " +
      "<matching|inner-extra>",
  );
  process.exit(2);
}

const extra = mode === "inner-extra" ? '<li data-hk="k.x">stale</li>' : "";
const page = open(`<!doctype html><body>
  <div id="mount"><section data-hk="k.0"><button>update</button><main><article data-hk="k.1"><h2>1</h2><ul><li data-hk="k.2">1</li><li data-hk="k.3">2</li>${extra}</ul></article><article data-hk="k.4"><h2>2</h2><ul><li data-hk="k.5">3</li></ul></article></main><aside>outside</aside></section></div>
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
  const control = section.querySelector("button");
  const main = section.querySelector("main");
  const outside = section.querySelector("aside");
  const beforeArticles = [...main.querySelectorAll(":scope > article")];
  const beforeOneRows = [...beforeArticles[0].querySelectorAll("li")].slice(0, 2);
  const beforeTwoRow = beforeArticles[1].querySelector("li");
  const mutations = watch(page.window, mount);
  const dispose = dom.hydrate(() => compiled.rust_entry(), mount, { renderId: "k." });
  const hydrationMutations = mutations.drain();

  assert.equal(mount.querySelector("section"), section);
  assert.equal(section.querySelector("button"), control);
  assert.equal(section.querySelector("main"), main);
  assert.equal(section.querySelector("aside"), outside);

  const states = [...rt._keyed_states];
  assert.equal(states.length, 3);
  const outer = states.find(state => [...state.rows.keys()].join(",") === "1,2");
  const one = states.find(state => [...state.rows.keys()].join(",") === "11,12");
  const two = states.find(state => [...state.rows.keys()].join(",") === "21");
  assert.ok(outer);
  assert.ok(one);
  assert.ok(two);
  const articleOne = outer.rows.get(1).nodes[0];
  const articleTwo = outer.rows.get(2).nodes[0];
  assert.equal(articleOne, beforeArticles[0]);
  assert.equal(articleTwo, beforeArticles[1]);
  assert.equal(one.rows.get(11).nodes[0], beforeOneRows[0]);
  assert.equal(one.rows.get(12).nodes[0], beforeOneRows[1]);
  assert.equal(two.rows.get(21).nodes[0], beforeTwoRow);
  assert.equal(outer.dom.parent, main);
  assert.equal(one.dom.parent, articleOne.querySelector("ul"));
  assert.equal(two.dom.parent, articleTwo.querySelector("ul"));
  assert.equal(outer.dom.end, null);
  assert.equal(one.dom.end, null);
  assert.equal(two.dom.end, null);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 5);

  if (mode === "matching") {
    assert.deepEqual(hydrationMutations, []);
    assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 5);
    assert.equal(rt.keyed_metrics_snapshot().hydrationFallbacks, 0);
  } else {
    assert.notDeepEqual(hydrationMutations, []);
    assert.equal(
      hydrationMutations.every(record => record.where.endsWith("ul")),
      true,
      `nested fallback escaped the child list: ${JSON.stringify(hydrationMutations)}`,
    );
    assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 3);
    assert.equal(rt.keyed_metrics_snapshot().hydrationFallbacks, 1);
    assert.equal(rt.keyed_metrics_snapshot().rangeReplacements, 1);
  }

  const oneRows = new Map(one.rows);
  const twoRows = new Map(two.rows);
  control.click();
  assert.deepEqual([...main.querySelectorAll(":scope > article")], [articleTwo, articleOne]);
  assert.equal(outer.rows.get(1).nodes[0], articleOne);
  assert.equal(outer.rows.get(2).nodes[0], articleTwo);
  assert.equal(one.rows.get(11), oneRows.get(11));
  assert.equal(one.rows.get(12), oneRows.get(12));
  assert.equal(two.rows.get(21), twoRows.get(21));
  assert.deepEqual([...one.rows.keys()], [12, 11, 13]);
  assert.deepEqual(
    [...articleOne.querySelectorAll("li")].map(node => node.textContent),
    ["20", "10", "30"],
  );
  assert.deepEqual(
    [...articleTwo.querySelectorAll("li")].map(node => node.textContent),
    ["3"],
  );
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 6);

  mutations.stop();
  dispose();
  for (const state of states) assert.equal(state.disposed, true);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
  assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
  assert.equal(mount.childNodes.length, 0);
  assert.deepEqual(page.console, []);
  report(`compiled keyed-list nested hydration: pass (${mode})`);
} finally {
  page.restore();
}
