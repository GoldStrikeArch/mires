import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { setImmediate } from "node:timers/promises";
import { pathToFileURL } from "node:url";

import { open } from "../contract/parity/lib/dom.mjs";
import * as dom from "../runtime/dom/dist/topcoat-dom.js";
import { minimalShim } from "./minimal-shim.mjs";
import { readArtifactManifest, sha256 } from "./artifact-manifest.mjs";

if (typeof globalThis.gc !== "function") throw new Error("run with node --expose-gc");
if (!process.argv[2]) throw new Error("provide the compiled lifecycle fixture");
let program = path.resolve(process.argv[2]);
if (process.argv.includes("--minify")) {
  const { transform } = await import("../runtime/dom/node_modules/esbuild/lib/main.js");
  const code = await readFile(program, "utf8");
  const source = await readFile(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const { manifest } = readArtifactManifest(`${program}.artifacts.json`);
  const runtime = manifest.artifacts[path.basename(program)].runtime;
  assert.equal(runtime.source_sha256, sha256(source));
  assert.deepEqual(minimalShim([code], source).names.sort(), runtime.closure,
    "the linker and packager must agree on lexical helper dependencies");
  assert.equal(runtime.module, "./shim.mjs");
  assert.ok(code.includes('"./shim.mjs"'));
  const runtimeName = `${path.basename(program)}.runtime.min.mjs`;
  const minimal = minimalShim([code, "__rt.keyed_metrics_snapshot;"], source, { module: true });
  const outputs = [
    [`${program}.min.mjs`, code.replaceAll('"./shim.mjs"', JSON.stringify(`./${runtimeName}`))],
    [path.join(path.dirname(program), runtimeName), minimal.source],
  ];
  for (const [file, input] of outputs) {
    const result = await transform(input, {
      format: "esm", minify: true, target: "es2018", sourcefile: path.basename(file), sourcemap: "external",
    });
    assert.deepEqual(result.warnings, []);
    await writeFile(file, `${result.code}//# sourceMappingURL=${path.basename(file)}.map\n`);
    await writeFile(`${file}.map`, result.map);
  }
  program = outputs[0][0];
}
const page = open("<!doctype html><body></body>");
const compiled = await import(pathToFileURL(program));
const rt = globalThis.__rt;
let dispose;

// Dereferencing a WeakRef keeps its target alive for the current job. Cross a
// job boundary before each collection so the observation cannot retain it.
async function collected(references) {
  for (let attempt = 0; attempt < 20; attempt++) {
    await setImmediate();
    globalThis.gc();
    await setImmediate();
    if (references.every(reference => reference.deref() === undefined)) return true;
  }
  return false;
}

try {
  dom.createRoot(cleanup => {
    dispose = cleanup;
    dom.insert(document.body, compiled.rust_entry());
  });
  const toggle = document.querySelector("button");
  const references = () => [...document.querySelectorAll("li")].map(node => new WeakRef(node));
  const states = [...rt._keyed_states];
  assert.equal(states.length, 1);
  const baseline = rt.keyed_metrics_snapshot();

  // A deliberately retained DOM node must fail the same collector used below.
  let retained = [...document.querySelectorAll("li")];
  const control = retained.map(node => new WeakRef(node));
  toggle.click();
  assert.equal(document.querySelectorAll("li").length, 0);
  assert.equal(await collected(control), false, "the injected leak must be detected");
  retained = null;
  assert.equal(await collected(control), true, "cleared initial rows must be collectible");

  for (let cycle = 0; cycle < 12; cycle++) {
    toggle.click();
    const rows = references();
    assert.equal(rows.length, 2);
    assert.deepEqual([...document.querySelectorAll("li")].map(node => node.textContent), ["10", "20"]);
    toggle.click();
    assert.equal(states[0].rows.size, 0);
    assert.equal(document.querySelectorAll("li").length, 0);
    assert.equal(await collected(rows), true, `removed DOM retention at cycle ${cycle}`);
    const metrics = rt.keyed_metrics_snapshot();
    assert.equal(metrics.liveRowRecords, 0);
    assert.equal(metrics.transactionStates, baseline.transactionStates);
    assert.equal(metrics.rowFactories - baseline.rowFactories, (cycle + 1) * 2);
    assert.equal(metrics.disposals, baseline.disposals + (cycle + 2) * 2);
  }

  toggle.click();
  const lastRows = references();
  dispose();
  assert.equal(states[0].rows.size, 0);
  assert.equal(rt._keyed_states.size, 0);
  assert.equal(document.querySelectorAll("li").length, 0);
  assert.equal(await collected(lastRows), true, "island teardown must release its rows");
} finally {
  dispose?.();
  page.restore();
}
process.stdout.write(`compiled keyed lifecycle: pass (${path.basename(program)}; 12 cycles, GC retention and injected leak control)\n`);
