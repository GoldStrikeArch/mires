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
  assert.match(result.source, /export const \{\n  log,\n\}/);
  assert.doesNotMatch(result.source, /  outer,/);
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
      schema: 2, object_format: 27, jir_schema: 15,
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
