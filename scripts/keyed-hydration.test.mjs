import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { open, watch } from "../contract/parity/lib/dom.mjs";

const report = console.log.bind(console);

const programPath = path.resolve(process.argv[2] ?? "");
if (!process.argv[2]) {
  console.error("usage: keyed-hydration.test.mjs <compiled-program.mjs>");
  process.exit(2);
}

const page = open(`<!doctype html><body>
  <div id="mount"><button data-hk="k.0">update</button><!--$--><button data-hk="k.1">1</button><button data-hk="k.2">2</button><button data-hk="k.3">3</button><!--/--></div>
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
  const control = mount.querySelector('[data-hk="k.0"]');
  const rows = [1, 2, 3].map(index => mount.querySelector(`[data-hk="k.${index}"]`));
  const closing = mount.lastChild;
  const mutations = watch(page.window, mount);
  const dispose = dom.hydrate(() => compiled.rust_entry(), mount, { renderId: "k." });

  assert.deepEqual(mutations.drain(), [], "matching keyed hydration must not mutate the DOM");
  assert.equal(mount.querySelector('[data-hk="k.0"]'), control);
  assert.deepEqual([1, 2, 3].map(index => mount.querySelector(`[data-hk="k.${index}"]`)), rows);
  const [state] = rt._keyed_states;
  assert.equal(state.dom.end, closing);
  assert.equal(state.dom.ownsEnd, false);
  assert.equal(rt.keyed_metrics_snapshot().hydratedRows, 3);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);

  control.click();
  assert.deepEqual(
    [...mount.querySelectorAll("button")].slice(1),
    [rows[2], rows[0], rows[1]],
    "the first update reorders the adopted nodes",
  );
  assert.deepEqual(rows.map(row => row.textContent), ["10", "20", "30"]);
  assert.equal(rt.keyed_metrics_snapshot().rowFactories, 3);
  assert.notDeepEqual(mutations.drain(), [], "the first update must exercise the adopted range");

  dispose();
  assert.equal(state.disposed, true);
  assert.equal(rt.keyed_metrics_snapshot().liveRowRecords, 0);
  assert.equal(rt.keyed_metrics_snapshot().transactionStates, 0);
  mutations.stop();
  report(`compiled keyed-list hydration: pass (${path.basename(programPath)})`);
} finally {
  page.restore();
}
