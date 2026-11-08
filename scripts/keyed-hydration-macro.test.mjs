import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { open, watch } from "../contract/parity/lib/dom.mjs";

const report = console.log.bind(console);
const programPath = path.resolve(process.argv[2] ?? "");
const mode = process.argv[3] ?? "matching";
const modes = new Set(["matching", "extra", "missing", "wrong-key"]);
if (!process.argv[2] || !modes.has(mode)) {
  console.error(
    "usage: keyed-hydration-macro.test.mjs <compiled-program.mjs> " +
      "<matching|extra|missing|wrong-key>",
  );
  process.exit(2);
}

const row = (key, label) =>
  `<li data-hk="${key}"><input value="seed"><span>${label}</span></li>`;
let serverRows = [row("k.1", 1), row("k.2", 2), row("k.3", 3)];
if (mode === "extra") serverRows.push(row("k.4", 4));
if (mode === "missing") serverRows.pop();
if (mode === "wrong-key") serverRows[1] = row("k.x", 2);

const page = open(`<!doctype html><body>
  <div id="mount"><section data-hk="k.0"><button>update</button><ul>${serverRows.join("")}</ul><aside>outside</aside></section></div>
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
  const list = section.querySelector("ul");
  const outside = section.querySelector("aside");
  const beforeRows = [...list.querySelectorAll("li")];
  const typed = beforeRows[1]?.querySelector("input");
  if (mode === "matching") {
    typed.value = "typed";
    typed.setSelectionRange(1, 3);
    typed.focus();
  }

  const mutations = watch(page.window, mount);
  const dispose = dom.hydrate(() => compiled.rust_entry(), mount, { renderId: "k." });
  const hydrationMutations = mutations.drain();
  const [state] = rt._keyed_states;
  const hydratedRows = [...list.querySelectorAll("li")];

  assert.ok(state);
  assert.equal(mount.querySelector("section"), section);
  assert.equal(section.querySelector("button"), control);
  assert.equal(section.querySelector("ul"), list);
  assert.equal(section.querySelector("aside"), outside);
  assert.equal(state.dom.parent, list);
  assert.equal(state.dom.end, null);
  assert.equal(state.dom.ownsEnd, false);
  assert.deepEqual(
    hydratedRows.map(node => node.querySelector("span").textContent),
    ["1", "2", "3"],
  );
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);

  if (mode === "matching") {
    assert.deepEqual(hydrationMutations, []);
    assert.deepEqual(hydratedRows, beforeRows);
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
      hydrationMutations.every(record => record.where.endsWith("ul")),
      true,
      `fallback escaped the list: ${JSON.stringify(hydrationMutations)}`,
    );
    assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 0);
    assert.equal(rt.keyed_metrics_snapshot().hydrationFallbacks, 1);
    assert.equal(rt.keyed_metrics_snapshot().rangeReplacements, 1);
  }

  const rowInputs = new Map([...state.rows].map(([key, value]) => [key, value.nodes[0].querySelector("input")]));
  control.click();
  const updatedRows = [...list.querySelectorAll("li")];
  assert.deepEqual(updatedRows, [state.rows.get(3).nodes[0], state.rows.get(1).nodes[0], state.rows.get(2).nodes[0]]);
  assert.deepEqual(
    updatedRows.map(node => node.querySelector("span").textContent),
    ["30", "10", "20"],
  );
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);
  for (const [key, input] of rowInputs) {
    assert.equal(state.rows.get(key).nodes[0].querySelector("input"), input);
  }
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
  report(`compiled keyed-list macro hydration: pass (${mode})`);
} finally {
  page.restore();
}
