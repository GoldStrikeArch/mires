import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { runInNewContext } from 'node:vm';
import { artifactGraphPlugin, entryResources, readArtifactManifest, sha256 } from './artifact-manifest.mjs';

const work = path.resolve(process.argv[2]);
const compiler = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const rollupPath = process.env.ROLLUP_MODULE
  ?? path.resolve(compiler, '../../experiments/fingerprintjs/node_modules/rollup/dist/es/rollup.js');
const { rollup } = await import(pathToFileURL(rollupPath));
const workerFile = path.join(work, 'worker/worker.js.artifacts.json');
const chunkFile = path.join(work, 'chunks/inspection.js.artifacts.json');
const worker = readArtifactManifest(workerFile).manifest;
assert.equal(worker.entries.program.role, 'worker');
assert.ok(worker.entries.program.roots.worker_entry.startsWith('rust:'));
assert.ok(!Object.hasOwn(worker.entries.program.roots, 'counter'));
assert.ok(!Object.hasOwn(worker.entries.program.roots, 'search'));
assert.equal(worker.artifacts['worker.js'].runtime.module, './worker-shim.js');
assert.equal(worker.artifacts['worker.js'].runtime.artifact, 'worker-shim.js');
assert.equal(worker.artifacts['worker-shim.js'].kind, 'runtime');
assert.ok(worker.artifacts['worker.js'].runtime.closure.includes('f2i'));
assert.ok(!worker.artifacts['worker.js'].runtime.closure.includes('js_log_bool'));
const chunks = readArtifactManifest(chunkFile).manifest;
assert.equal(chunks.entries.counter.role, 'island');
assert.equal(chunks.entries.search.role, 'procedure');
assert.equal(chunks.entries.shared.role, 'shared');
const resources = entryResources(chunks);
assert.ok(!resources.has('inspection.js'), 'the inspection program must not enter deployment closure');
assert.ok(resources.has('shared.js'), 'common items must be shared');
for (const name of ['counter', 'search', 'shared']) {
  assert.ok(resources.has(`${name}.js.map`));
  assert.ok(resources.has(`${name}.d.ts`));
}

// A valid byte receipt must fail after a program, source map, or declaration changes.
for (const name of ['worker.js', 'worker.js.map', 'worker.d.ts', 'worker-shim.js']) {
  const file = path.join(work, 'worker', name);
  const original = await readFile(file);
  await writeFile(file, Buffer.concat([original, Buffer.from('changed')]));
  assert.throws(() => readArtifactManifest(workerFile), /bytes disagree/);
  const verified = spawnSync(path.join(compiler, 'target/release/jsc-link'), ['--verify-artifacts', workerFile]);
  assert.notEqual(verified.status, 0);
  await writeFile(file, original);
}

// Both consumers reject incompatible schemas and internally inconsistent graph claims.
const originalManifest = await readFile(workerFile);
for (const mutate of [
  manifest => { manifest.schema++; },
  manifest => { manifest.unknown_contract = true; },
  manifest => { manifest.entries = []; },
  manifest => { manifest.artifacts['worker.js'].imports = []; },
  manifest => { manifest.entries.program.roots.worker_entry = 'rust:missing'; },
  manifest => { manifest.artifacts['../outside.js'] = manifest.artifacts['worker.js']; },
  manifest => { manifest.artifacts['worker.js'].runtime.artifact = 'worker.js'; },
  manifest => { manifest.artifacts['worker.js'].runtime.closure = []; },
  manifest => { delete manifest.artifacts['worker.js'].runtime.external; },
  manifest => { manifest.artifacts['worker.js'].runtime.external = ['f2i']; },
]) {
  const changed = structuredClone(worker);
  mutate(changed);
  try {
    await writeFile(workerFile, JSON.stringify(changed));
    assert.throws(() => readArtifactManifest(workerFile));
    const verified = spawnSync(path.join(compiler, 'target/release/jsc-link'), ['--verify-artifacts', workerFile]);
    assert.notEqual(verified.status, 0);
  } finally {
    await writeFile(workerFile, originalManifest);
  }
}

let receipt;
const workerBundle = await rollup({ input: path.join(work, 'worker/worker.js'),
  plugins: [artifactGraphPlugin([workerFile], { onReceipt: value => { receipt = value; } })] });
const output = await workerBundle.generate({ format: 'es' });
await workerBundle.close();
assert.equal(receipt.runtimeProviders.length, 1);
assert.ok(receipt.runtimeProviders[0].path.endsWith('/worker-shim.js'));
assert.ok(receipt.modules.every(module => !module.path.endsWith('/shim.js')));
await writeFile(path.join(work, 'worker/input-graph.json'), JSON.stringify(receipt, null, 2));
await writeFile(path.join(work, 'worker/bundle.mjs'), output.output.find(item => item.type === 'chunk').code);
const program = await import(pathToFileURL(path.join(work, 'worker/bundle.mjs')));
assert.equal(program.worker_entry(3.75), 3);
assert.equal(program.worker_entry(Infinity), 2147483647);

// The manifest is bound when the plugin is created. A later file edit must not
// become trusted merely because Rollup loaded the same changed bytes from disk.
for (const name of ['worker.js', 'worker-shim.js', 'worker.js.map', 'worker.d.ts']) {
  const file = path.join(work, 'worker', name);
  const original = await readFile(file);
  const plugin = artifactGraphPlugin([workerFile]);
  try {
    await writeFile(file, Buffer.concat([original, Buffer.from('\n// changed after manifest verification\n')]));
    await assert.rejects(rollup({ input: path.join(work, 'worker/worker.js'), plugins: [plugin] }),
      /bytes disagree with linker manifest|bundler input differs from compiler artifact/, name);
  } finally {
    await writeFile(file, original);
  }
}

// Graph verification resolves imports again. Metadata changed by an asynchronous
// resolver must be checked after resolution, not only when buildEnd starts.
{
  const file = path.join(work, 'worker/worker.js.map');
  const original = await readFile(file);
  let checking = false, changed = false;
  const resolver = {
    name: 'late-metadata-change',
    buildEnd() { checking = true; },
    async resolveId(source) {
      if (checking && source === './worker-shim.js' && !changed) {
        changed = true;
        await writeFile(file, Buffer.concat([original, Buffer.from('\nchanged during resolution')]));
      }
      return null;
    },
  };
  try {
    await assert.rejects(rollup({ input: path.join(work, 'worker/worker.js'),
      plugins: [resolver, artifactGraphPlugin([workerFile])] }), /bytes disagree with linker manifest/);
    assert.ok(changed, 'the control must reach the graph verifier resolution');
  } finally {
    await writeFile(file, original);
  }
}

// Even an identically populated provider at the wrong deployment identity must be rejected.
// Missing exports must not be the reason this negative control fails.
const wrongProvider = path.join(work, 'chunks/wrong-provider.js');
await writeFile(wrongProvider, await readFile(path.join(work, 'worker/worker-shim.js')));
await assert.rejects(rollup({ input: path.join(work, 'worker/worker.js'), plugins: [
  { name: 'wrong-provider', resolveId(source) { return source === './worker-shim.js' ? wrongProvider : null; } },
  artifactGraphPlugin([workerFile]),
] }), /resolved outside its declared artifact identity/);

const chunkBundle = await rollup({ input: [path.join(work, 'chunks/counter.js'), path.join(work, 'chunks/search.js')],
  plugins: [artifactGraphPlugin([chunkFile], { onReceipt: value => { receipt = value; } })] });
await chunkBundle.generate({ format: 'es' });
await chunkBundle.close();
assert.equal(receipt.runtimeProviders.length, 1);
const programsWithRuntime = [...resources].filter(name => chunks.artifacts[name].runtime);
assert.equal(receipt.runtimeProviders[0].requiredBy.length, programsWithRuntime.length);
assert.deepEqual(receipt.runtimeProviders[0].required,
  [...new Set(programsWithRuntime.flatMap(name => chunks.artifacts[name].runtime.closure))].sort());
const single = await rollup({ input: path.join(work, 'chunks/counter.js'),
  plugins: [artifactGraphPlugin([{ path: chunkFile, entries: ['counter'] }], { onReceipt: value => { receipt = value; } })] });
await single.generate({ format: 'es' });
await single.close();
assert.equal(receipt.programs.length, 2);
assert.ok(receipt.programs.some(program => program.path.endsWith('/shared.js')));
assert.ok(receipt.modules.every(module => !module.path.endsWith('/search.js')));

// Input hashes alone cannot preserve semantics: default Rollup deletes an empty
// infinite loop, including when called from an exported function. Preserve only
// the declared compiler module and still remove unused runtime exports.
{
  const directory = path.join(work, 'nontermination');
  await mkdir(directory, { recursive: true });
  const sources = {
    'program.js': "import { touch } from './runtime.js';\nfunction spin(){for(;;){}}\nexport function run(stall){touch();if(stall)spin();return 42;}\n",
    'runtime.js': "export function touch(){globalThis.touches++;}\nexport function unused(){return 'unused_runtime_sentinel';}\n",
    'facade.js': "export { run } from './program.js';\n",
  };
  for (const [name, code] of Object.entries(sources)) await writeFile(path.join(directory, name), code);
  const manifest = structuredClone(worker);
  manifest.entries = { program: { role: 'main', program: 'program.js', roots: { run: 'rust:run' } } };
  manifest.artifacts = Object.fromEntries(['program.js', 'runtime.js'].map(name => [name, {
    kind: name === 'program.js' ? 'program' : 'runtime', bytes: Buffer.byteLength(sources[name]),
    sha256: sha256(sources[name]), imports: name === 'program.js' ? { './runtime.js': 'runtime.js' } : {},
    items: name === 'program.js' ? ['rust:run'] : [], exports: name === 'program.js' ? ['run'] : ['touch', 'unused'],
  }]));
  const manifestFile = path.join(directory, 'program.js.artifacts.json');
  await writeFile(manifestFile, JSON.stringify(manifest, null, 2));
  const input = path.join(directory, 'facade.js');
  const invoke = (code, stall) => {
    const context = { exports: {}, touches: 0 };
    runInNewContext(code, context, { timeout: 100 });
    try {
      runInNewContext(`result = exports.run(${stall});`, context, { timeout: 100 });
      return context.result;
    } finally {
      assert.equal(context.touches, 1, 'preserve host effects before the loop');
    }
  };
  const { minify } = await import(pathToFileURL(path.resolve(path.dirname(rollupPath), '../../../terser/main.js')));
  for (const preserve of [false, true]) {
    const bundle = await rollup({ input, treeshake: { moduleSideEffects: false },
      plugins: preserve ? [artifactGraphPlugin([manifestFile])] : [] });
    let code;
    try { code = (await bundle.generate({ format: 'cjs' })).output[0].code; }
    finally { await bundle.close(); }
    const minified = await minify(code, { ecma: 2018, compress: true, mangle: true });
    for (const [stage, source] of [['rollup', code], ['terser', minified.code]]) {
      assert.equal(typeof source, 'string');
      await writeFile(path.join(directory, `${preserve ? 'preserved' : 'default'}-${stage}.cjs`), source);
      assert.ok(!source.includes('unused_runtime_sentinel'), 'runtime tree shaking must remain enabled');
      assert.equal(invoke(source, false), 42);
      if (preserve) assert.throws(() => invoke(source, true), { code: 'ERR_SCRIPT_EXECUTION_TIMEOUT' });
      else assert.equal(invoke(source, true), 42, 'negative control must expose the removed loop');
    }
  }
  await assert.rejects(rollup({ input, plugins: [artifactGraphPlugin([manifestFile]), {
    name: 'override-program-preservation', transform() { return { moduleSideEffects: true }; },
  }] }), /compiler program preservation was overridden/);
}

// Namespace member reads can produce warnings rather than link errors. Verify
// the manifest's public runtime contract, without demanding private dependency
// exports. Follow ESM binding identity through aliases, stars and cycles.
{
  const directory = path.join(work, 'runtime-exports');
  await mkdir(directory, { recursive: true });
  const program = "import * as rt from './runtime.js'; export function run(){return rt.required()+rt.host();}\n";
  const input = path.join(directory, 'program.js');
  await writeFile(input, program);
  const manifest = structuredClone(worker);
  manifest.entries = { program: { role: 'main', program: 'program.js', roots: { run: 'rust:run' } } };
  manifest.artifacts = { 'program.js': {
    kind: 'program', bytes: Buffer.byteLength(program), sha256: sha256(program),
    imports: { './runtime.js': null }, items: ['rust:run'], exports: ['run'],
    runtime: { module: './runtime.js', source_sha256: sha256('test runtime'),
      direct: ['required'], closure: ['required', 'privateDependency'], external: ['host'] },
  } };
  const manifestFile = path.join(directory, 'program.js.artifacts.json');
  await writeFile(manifestFile, JSON.stringify(manifest, null, 2));
  const definitions = 'export function required(){return 40;} export function host(){return 2;}';
  await writeFile(path.join(directory, 'bindings.js'), definitions);
  await writeFile(path.join(directory, 'aliases.js'), "import { required as value, host } from './bindings.js'; export { value as required, host };");
  await writeFile(path.join(directory, 'other.js'), definitions);
  await writeFile(path.join(directory, 'ambiguous.js'), "export * from './bindings.js'; export * from './other.js';");
  await writeFile(path.join(directory, 'cycle.js'), "export * from './runtime.js'; export * from './bindings.js';");
  const cases = [
    ['direct', definitions, null],
    ['named', "export { required, host } from './bindings.js';", null],
    ['aliases', "export * from './bindings.js'; export * from './aliases.js';", null],
    ['cycle', "export * from './cycle.js';", null],
    ['missing-direct', 'export function host(){return 2;}', 'required'],
    ['missing-host', 'export function required(){return 40;}', 'host'],
    ['default-is-not-named', 'export default function required(){return 40;} export function host(){return 2;}', 'required'],
    ['ambiguous', "export * from './bindings.js'; export * from './other.js';", 'host'],
    ['nested-ambiguous', "export * from './ambiguous.js'; export * from './bindings.js';", 'host'],
  ];
  for (const [name, source, missing] of cases) {
    await writeFile(path.join(directory, 'runtime.js'), source);
    let published = false;
    const build = () => rollup({ input, onwarn() {}, plugins: [artifactGraphPlugin([manifestFile], {
      onReceipt(value) {
        published = true;
        assert.deepEqual(value.runtimeProviders[0].requiredExports, ['host', 'required']);
      },
    })] });
    if (missing) {
      await assert.rejects(build(), new RegExp(`has no unambiguous export ${missing}`), name);
      assert.equal(published, false, `${name}: a failed export must not publish a receipt`);
    } else {
      const bundle = await build();
      try {
        const code = (await bundle.generate({ format: 'cjs' })).output[0].code;
        const context = { exports: {} };
        runInNewContext(code + '; globalThis.result = exports.run();', context, { timeout: 100 });
        assert.equal(context.result, 42, name);
        assert.ok(published);
      } finally { await bundle.close(); }
    }
  }
}
// The demo publishes the canonical abort helper through its generated subset.
{
  const runtime = await import(pathToFileURL(path.join(compiler, 'demo-app/src/island-rt.mjs')));
  let conversions = 0;
  assert.throws(() => runtime.js_abort({ toString() { conversions++; return 'probe'; } }),
    { name: 'Error', message: 'rust abort: probe' });
  assert.equal(conversions, 1);
  const sentinel = {};
  assert.throws(() => runtime.js_abort({ toString() { throw sentinel; } }), error => error === sentinel);
}
console.log('artifact-manifest: PASS (roots, roles, shared closure, maps, declarations, runtime identity, bundler graph, negative controls)');
