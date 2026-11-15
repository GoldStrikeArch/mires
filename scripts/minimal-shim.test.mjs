import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { minimalShim, referencedMembers, shimMembers } from "./minimal-shim.mjs";
import { readArtifactManifest, sha256 } from "./artifact-manifest.mjs";

const shim = `globalThis.__rt = {
  log: (value) => console.log(value),
};
globalThis.__rt.inner = (value) => value;
globalThis.__rt.outer = (value) => globalThis.__rt.inner(value);
globalThis.__rt.unused = () => 0;
`;

test("the extractor follows dependencies and drops unused members", () => {
  const result = minimalShim(["export const run = () => __rt.outer(1);"], shim);
  assert.deepEqual(result.names, ["inner", "outer"]);
  assert.match(result.source, /__rt\.inner/);
  assert.doesNotMatch(result.source, /__rt\.unused/);
});

test("module output exports exactly the retained members", () => {
  const result = minimalShim(["__rt.log(1);"], shim, { module: true });
  assert.match(result.source, /let \$rt_log =/);
  assert.match(result.source, /export \{\n  \$rt_log as log,\n\};/);
  assert.doesNotMatch(result.source, /globalThis\.__rt/);
  assert.doesNotMatch(result.source, /outer/);
});

test("a missing runtime member is a build error", () => {
  assert.throws(
    () => minimalShim(["__rt.missing();"], shim),
    /missing runtime members: missing/,
  );
});

test("the scanners ignore longer identifiers", () => {
  assert.deepEqual([...referencedMembers("a__rt.fake; __rt.real;")], ["real"]);
  assert.deepEqual([...shimMembers(shim).keys()], ["log", "inner", "outer", "unused"]);
});

test("lexical helpers retain their owning source group transitively", () => {
  const source = `${shim}
globalThis.__rt.owner = () => 0;
const localHelper = value => globalThis.__rt.outer(value);
globalThis.__rt.caller = () => localHelper(7);
`;
  const result = minimalShim(["__rt.caller();"], source);
  assert.deepEqual(result.names, ["inner", "outer", "owner", "caller"]);
  const context = vm.createContext({});
  vm.runInContext(result.source, context);
  assert.equal(vm.runInContext("__rt.caller()", context), 7);
});

test("manifest publication distinguishes application hooks from canonical helpers", () => {
  const directory = mkdtempSync(path.join(tmpdir(), "rcgjs-host-runtime-"));
  const manifestPath = path.join(directory, "program.js.artifacts.json");
  const outputPath = path.join(directory, "runtime.js");
  const shimPath = path.join(directory, "canonical.js");
  const script = "__rt.host_emit(__rt.outer(7));\n";
  const runtime = {
    module: null, source_sha256: sha256(shim), direct: ["outer"],
    closure: ["inner", "outer"], external: ["host_emit"], artifact: null,
  };
  const publish = (source, requirement, flags = []) => {
    writeFileSync(path.join(directory, "program.js"), source);
    writeFileSync(manifestPath, JSON.stringify({
      schema: 2, object_format: 27, jir_schema: 16,
      semantic_policy: "faithful", syntax_ceiling: "2018",
      entries: { program: { role: "main", program: "program.js", roots: { entry: "rust:entry" } } },
      artifacts: { "program.js": {
        kind: "program", bytes: Buffer.byteLength(source), sha256: sha256(source),
        imports: requirement.module ? { [requirement.module]: null } : {},
        items: ["rust:entry"], exports: [], runtime: requirement,
      } },
    }));
    return spawnSync(process.execPath, [fileURLToPath(new URL("./minimal-shim.mjs", import.meta.url)),
      ...flags, "--manifest", manifestPath, shimPath, outputPath], { encoding: "utf8" });
  };
  try {
    writeFileSync(shimPath, shim);
    for (const flags of [[], ["--full"]]) {
      const result = publish(script, runtime, flags);
      assert.equal(result.status, 0, result.stderr);
      const declared = readArtifactManifest(manifestPath).manifest.artifacts["program.js"].runtime;
      assert.deepEqual(declared.external, ["host_emit"]);
      assert.equal(declared.artifact, "runtime.js");
      const source = readFileSync(outputPath, "utf8");
      assert.doesNotMatch(source, /host_emit/);
      const observed = [];
      const context = vm.createContext({ __rt: { host_emit: value => observed.push(value) } });
      vm.runInContext(source, context);
      vm.runInContext(script, context);
      assert.deepEqual(observed, [7]);
      const missing = vm.createContext({});
      vm.runInContext(source, missing);
      assert.throws(() => vm.runInContext(script, missing), /host_emit/);
    }
    const before = readFileSync(outputPath);
    const module = publish(`import * as __rt from './runtime.js';\n${script}`,
      { ...runtime, module: "./runtime.js" }, ["--module"]);
    assert.notEqual(module.status, 0);
    assert.match(module.stderr, /canonical runtime cannot supply external host members/);
    assert.deepEqual(readFileSync(outputPath), before, "a rejected provider must not be written");
    const omitted = publish(script, { ...runtime, external: [] });
    assert.notEqual(omitted.status, 0);
    assert.match(omitted.stderr, /members disagree/);
    const misclassified = publish(script, { ...runtime, direct: [], closure: [], external: ["host_emit", "outer"] });
    assert.notEqual(misclassified.status, 0);
    assert.match(misclassified.stderr, /canonical runtime helper declared as external/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("module mode emits lexical ESM bindings, elides unused _boxes, and deduplicates errors", async () => {
  const { parse } = await import("acorn");
  const fullShim = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const withoutBox = minimalShim(["__rt._put(b, 0, v); __rt.js_abort('x'); __rt.read_array(p, 1); __rt.array_ref(p, 1);"], fullShim, { module: true });
  const classicWithoutBox = minimalShim(["__rt._put(b, 0, v); __rt.js_abort('x'); __rt.read_array(p, 1); __rt.array_ref(p, 1);"], fullShim);
  assert.deepEqual(withoutBox.names, classicWithoutBox.names);
  parse(withoutBox.source, { ecmaVersion: 2018, sourceType: "module" });
  assert.match(withoutBox.source, /let \$rt__boxes = null;/);
  assert.doesNotMatch(withoutBox.source, /\$rt__boxes\.(?:has|set)\(/);
  assert.match(withoutBox.source, /const \$rt_err = \(m\) => new Error\("rustc_codegen_js: " \+ m\);/);
  assert.match(withoutBox.source, /new Error\("rust abort: " \+ msg\)/);
  assert.doesNotMatch(withoutBox.source, /globalThis\.__rt/);

  const withBox = minimalShim(["__rt._put(b, 0, v); __rt.box(v);"], fullShim, { module: true });
  parse(withBox.source, { ecmaVersion: 2018, sourceType: "module" });
  assert.match(withBox.source, /let \$rt__boxes = new WeakMap\(\);/);
  assert.match(withBox.source, /\$rt__boxes\.set\(value, \{ buf, off \}\);/);

  const literalShim = `${shim}globalThis.__rt.str = () => "__rt.outer";\n`;
  const literalMod = minimalShim(["__rt.str(); __rt.outer(1);"], literalShim, { module: true });
  assert.match(literalMod.source, /"__rt\.outer"/);
  assert.match(literalMod.source, /let \$rt_outer =/);

  const keyedMod = minimalShim(["__rt.keyed_metrics_snapshot;"], fullShim, { module: true });
  parse(keyedMod.source, { ecmaVersion: 2018, sourceType: "module" });
  assert.match(keyedMod.source, /globalThis\.__rt = globalThis\.__rt \|\| \{\};/);
  assert.match(keyedMod.source, /export const \{\n[\s\S]*  keyed_metrics_snapshot,\n[\s\S]*\} = globalThis\.__rt;/);
  assert.doesNotMatch(keyedMod.source, /\$rt_/);
});

test("ptr_is_null matches (addr(p, 1) >>> 0) === 0 without minting on live buffers", () => {
  const fullShim = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const context = vm.createContext({});
  vm.runInContext(fullShim, context);
  const rt = context.__rt;

  const stackArr = [1, 2, 3, 4];
  const heapPtr = rt.alloc(16, 4);
  const dangling = rt._dangling(4);
  const cases = [
    0,
    -0,
    4294967296,
    1,
    4095,
    null,
    undefined,
    { buf: stackArr, off: 0 },
    { buf: stackArr, off: 3 },
    heapPtr,
    { pr: 0, ps: 1 },
    { pr: null, ps: 1 },
    { pr: { buf: stackArr, off: 0 }, ps: 4 },
    { buf: null, off: 0 },
    { buf: 0, off: 0 },
    dangling,
    { buf: dangling.buf, off: -4 },
  ];
  for (const ptr of cases) {
    assert.equal(rt.ptr_is_null(ptr), (rt.addr(ptr, 1) >>> 0) === 0, `mismatch for ${String(ptr)}`);
  }

  // Fresh live pointer must not mint an address or flip _addrs membership.
  const freshArr = [10, 20];
  assert.equal(rt.ptr_is_null({ buf: freshArr, off: 0 }), false);
  assert.equal(rt.ptr_is_null({ buf: freshArr, off: 1 }), false);
  assert.equal(rt._addrs.has(freshArr), false);

  // Offset-wrapped pointer reaching address 0 must fall back and return true.
  const freshContext = vm.createContext({});
  vm.runInContext(fullShim, freshContext);
  const frt = freshContext.__rt;
  assert.equal(frt.ptr_is_null({ buf: [1], off: -4096 }), true);

  // Projected `.pr` must preserve `.ps` for dangling/heap negative offsets and large wraps, plus 2-arg size.
  const d4 = frt._dangling(4);
  const dMoved = frt.offset(d4, -1, 4);
  const dProj = frt.prefix(dMoved, "_0", 4);
  assert.equal(frt.ptr_is_null(dProj), true);
  assert.equal((frt.addr(dProj, 1) >>> 0) === 0, true);

  const hCtx = vm.createContext({});
  vm.runInContext(fullShim, hCtx);
  const hrt = hCtx.__rt;
  const h = hrt.alloc(16, 4); // base = 4096
  const hProj = hrt.prefix(hrt.offset(h, -1024, 4), "_0", 4);
  assert.equal(hrt.ptr_is_null(hProj), true);
  assert.equal((hrt.addr(hProj, 1) >>> 0) === 0, true);

  const wCtx = vm.createContext({});
  vm.runInContext(fullShim, wCtx);
  const wrt = wCtx.__rt;
  wrt._addrNext = 1048576;
  const largeBuf = [1];
  const wProj = { pr: { buf: largeBuf, off: 4095 }, ps: 1048576 };
  assert.equal(wrt.ptr_is_null(wProj), true);
  assert.equal((wrt.addr(wProj, 1) >>> 0) === 0, true);
  assert.equal(wrt.ptr_is_null({ buf: largeBuf, off: 4095 }, 1048576), true);

  // Poisoned provenance-less pointer must throw just like addr().
  assert.throws(() => rt.ptr_is_null({ buf: undefined, off: NaN }), /no provenance/);
});

test("dealloc skips _addrHeap WeakMap lookup when no heap address was minted", () => {
  const fullShim = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");
  const context = vm.createContext({});
  vm.runInContext(fullShim, context);
  const rt = context.__rt;

  let addrHeapGets = 0;
  const origGet = rt._addrHeap.get.bind(rt._addrHeap);
  rt._addrHeap.get = (k) => {
    addrHeapGets += 1;
    return origGet(k);
  };

  const p1 = rt.alloc(16, 4);
  assert.equal(rt.ptr_is_null(p1), false);
  rt.dealloc(p1, 16, 4);
  assert.equal(addrHeapGets, 0);

  const p2 = rt.alloc(16, 4);
  const a2 = rt.addr(p2, 1);
  rt.dealloc(p2, 16, 4);
  assert.equal(addrHeapGets, 1);
  const p3 = rt.alloc(16, 4);
  assert.equal(rt.addr(p3, 1), a2);
});

test("minimalShim isolates _ASCII_CHUNK subslice state cleanly in classic and module modes", async () => {
  const { parse } = await import("acorn");
  const fullShim = readFileSync(new URL("../runtime/shim.js", import.meta.url), "utf8");

  for (const mod of [false, true]) {
    const bytesOnly = minimalShim(["__rt.bytes_str(b, 0, n);"], fullShim, { module: mod });
    assert.ok(bytesOnly.names.includes("_ASCII_CHUNK"));
    assert.ok(!bytesOnly.names.includes("_str_memo"));
    assert.ok(!bytesOnly.names.includes("str_bytes"));
    parse(bytesOnly.source, { ecmaVersion: 2018, sourceType: mod ? "module" : "script" });

    const lenOnly = minimalShim(["__rt.str_len(s);"], fullShim, { module: mod });
    assert.ok(lenOnly.names.includes("_ASCII_CHUNK"));
    assert.ok(lenOnly.names.includes("_str_memo"));
    assert.ok(!lenOnly.names.includes("bytes_str"));
    assert.ok(!lenOnly.names.includes("str_bytes"));
    parse(lenOnly.source, { ecmaVersion: 2018, sourceType: mod ? "module" : "script" });
  }

  const combined = minimalShim(
    ["__rt.str_bytes(s); __rt.bytes_str(b, 0, n); __rt.str_len(s);"],
    fullShim,
    { module: true },
  );
  parse(combined.source, { ecmaVersion: 2018, sourceType: "module" });
  const mod = await import(
    `data:text/javascript;base64,${Buffer.from(combined.source).toString("base64")}`
  );
  const src = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
  const rec = mod.str_bytes(src);
  const sub = mod.bytes_str(rec.buf, 5, 22);
  assert.equal(sub, src.slice(5, 27));
  assert.equal(mod.str_bytes(sub).buf, rec.buf);
  assert.equal(mod.str_bytes(sub).off, 5);
  assert.equal(mod.str_len(sub), 22);
});
