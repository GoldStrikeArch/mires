#!/usr/bin/env node

import { copyFile, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { args, command, fileRecord, sha256, stableJson } from '../../fingerprintjs-rust-compact/tools/common.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = resolve(root, '../..')
const compilerRoot = repositoryRoot
const compilerPath = relative(repositoryRoot, compilerRoot) || '.'
const compilerCheckpointPathspec = `:(exclude)${compilerPath}/checkpoints`
const modulesRoot = resolve(repositoryRoot, 'node_modules')
const { artifactGraphPlugin, entryResources, readArtifactManifest } = await import(
  pathToFileURL(resolve(compilerRoot, 'scripts/artifact-manifest.mjs')))
const options = args()
const buildRoot = resolve(options.build || resolve(root, '_build'))
const distRoot = resolve(options.output || resolve(root, 'dist'))
const resultPath = resolve(options.receipt || resolve(root, 'results/build.json'))
const incremental = options.incremental === true
const selected = new Set(String(options.only || '').split(',').filter(Boolean))
const availableWorkloads = ['minisearch', 'marked', 'fflate', 'idb', 'idb_lines', 'ky', 'uplot', 'explorer', 'workbox', 'workbox_direct']
for (const name of selected) {
  if (!availableWorkloads.includes(name)) throw new Error(`unknown corpus workload: ${name}`)
}
const workloads = availableWorkloads.filter((name) => selected.size === 0 || selected.has(name))
if (workloads.length === 0) throw new Error('corpus workload selection is empty')
const cleanup = String(options.cleanup || 'on')
if (!['on', 'off'].includes(cleanup)) throw new Error('--cleanup must be on or off')
const compilerArgs = `js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=${cleanup}`
const compilerEnvironment = {
  ...process.env,
  JSC_REMAP_ROOT: root,
  JSC_RUST_MIR_INLINE: 'on',
  JSC_RUST_OPT_LEVEL: 'z',
  JSC_SYSROOT_ARGS: compilerArgs,
  JS_EXTRA_ARGS: `${compilerArgs} js-opt-warnings=off js-workspace-root=${repositoryRoot}`,
}

for (const path of [
  resolve(compilerRoot, 'scripts/build.sh'),
  resolve(compilerRoot, 'scripts/build_sysroot.sh'),
  resolve(compilerRoot, 'scripts/compile_core.sh'),
  resolve(compilerRoot, 'scripts/minimal-shim.mjs'),
  resolve(modulesRoot, 'rollup/dist/es/rollup.js'),
  resolve(modulesRoot, 'terser/main.js'),
]) {
  if (!existsSync(path)) throw new Error(`required build input is missing: ${path}`)
}

const compilerTreeBefore = await compilerTreeIdentity()
command(resolve(compilerRoot, 'scripts/build.sh'), [], { cwd: compilerRoot, env: compilerEnvironment, stdio: 'inherit' })
await ensureSysroot()
command('cargo', ['build', '--manifest-path', resolve(compilerRoot, 'Cargo.toml'), '-p', 'js-extern-macro'], {
  cwd: compilerRoot,
  stdio: 'inherit',
})

if (!incremental) {
  await rm(buildRoot, { recursive: true, force: true })
  await rm(distRoot, { recursive: true, force: true })
}
await mkdir(buildRoot, { recursive: true })
await mkdir(distRoot, { recursive: true })

const macroExtension = process.platform === 'darwin' ? '.dylib' : '.so'
const procMacroRoot = resolve(buildRoot, 'proc-macro')
await mkdir(procMacroRoot, { recursive: true })
const procMacro = resolve(procMacroRoot, `libjs_extern_macro${macroExtension}`)
await copyFile(resolve(compilerRoot, `target/debug/libjs_extern_macro${macroExtension}`), procMacro)

const topcoatRlib = resolve(buildRoot, 'libmires_js.rlib')
compile([
  resolve(compilerRoot, 'mires-js/src/lib.rs'),
  topcoatRlib,
  '--crate-type', 'rlib',
  '--crate-name', 'mires_js',
  '--extern', `js_extern_macro=${procMacro}`,
])

const libraryRlib = resolve(buildRoot, 'libbrowser_workloads.rlib')
compile([
  resolve(root, 'src/lib.rs'),
  libraryRlib,
  '--crate-type', 'rlib',
  '--crate-name', 'browser_workloads',
  '--extern', `mires_js=${topcoatRlib}`,
  '--search', procMacroRoot,
])

const { rollup } = await import(pathToFileURL(resolve(modulesRoot, 'rollup/dist/es/rollup.js')))
const { minify } = await import(pathToFileURL(resolve(modulesRoot, 'terser/main.js')))
const receipt = {
  schema: 3,
  generatedAt: new Date().toISOString(),
  configuration: { compilerRoot, cleanup, ecmaTarget: 2018, lane: 'jir', optimizer: 'on', profile: 'release-size' },
  provenance: {
    revision: command('git', ['rev-parse', 'HEAD'], { cwd: repositoryRoot }).trim(),
    compilerRevision: command('git', [
      'log', '-1', '--format=%H', '--', compilerPath, compilerCheckpointPathspec,
    ], { cwd: repositoryRoot }).trim(),
    compilerDirty: command('git', [
      'status', '--porcelain', '--', compilerPath, `:(exclude)${compilerPath}/checkpoints`,
    ], { cwd: repositoryRoot }).trim() !== '',
    compilerTreeBefore,
    compilerTreeScope: 'tracked and untracked non-ignored compiler files, excluding checkpoints',
    node: process.version,
    rollup: JSON.parse(await readFile(resolve(modulesRoot, 'rollup/package.json'), 'utf8')).version,
    rustc: command('rustc', ['--version']).trim(),
    sources: await sourceHashes(),
    terser: JSON.parse(await readFile(resolve(modulesRoot, 'terser/package.json'), 'utf8')).version,
  },
  workloads: {},
}

receipt.admission = {
  btreeMap: compileExpectedFailure(
    resolve(root, 'fixtures/btree-map.rs'),
    resolve(buildRoot, 'admission/btree-map.js'),
    /cannot cast a pointer to .*LeafNode.* into a pointer to .*InternalNode|cannot cast a pointer to .*InternalNode.* into a pointer to .*LeafNode/s,
  ),
}

for (const name of workloads) {
  const itemRoot = resolve(buildRoot, name)
  const outputRoot = resolve(distRoot, name)
  await mkdir(itemRoot, { recursive: true })
  await mkdir(resolve(outputRoot, 'raw'), { recursive: true })
  const program = resolve(itemRoot, 'program.js')
  const optimizerReport = resolve(itemRoot, 'optimizer-report.json')
  const laneReport = resolve(itemRoot, 'lane-report.json')
  compile([
    resolve(root, `entries/${name}.rs`),
    program,
    '--crate-name', `browser_workload_${name}`,
    '--extern', `browser_workloads=${libraryRlib}`,
    '--extern', `mires_js=${topcoatRlib}`,
    '--search', procMacroRoot,
    '--search', buildRoot,
  ], [
    'js-modules=esm',
    'js-comments=off',
    'js-lane=jir',
    `js-entry-role=${name.startsWith('workbox') ? 'worker' : 'main'}`,
    `js-lane-report=${laneReport}`,
    `js-opt-report=${optimizerReport}`,
  ])
  const shim = resolve(itemRoot, 'shim.js')
  command(process.execPath, [
    resolve(compilerRoot, 'scripts/minimal-shim.mjs'),
    '--module', '--manifest', `${program}.artifacts.json`,
    resolve(compilerRoot, 'runtime/shim.js'),
    shim,
  ], { cwd: root, stdio: 'inherit' })
  const facade = resolve(itemRoot, 'facade.js')
  await writeFile(facade, name.startsWith('workbox')
    ? "import { workload_register } from './program.js'\nworkload_register()\n"
    : "import { workload_run } from './program.js'\nfunction run(options) { return workload_run(options) }\nexport { run }\n")
  await Promise.all([
    copyFile(program, resolve(outputRoot, 'raw/program.js')),
    copyFile(shim, resolve(outputRoot, 'raw/shim.js')),
    copyFile(facade, resolve(outputRoot, 'raw/facade.js')),
    copyFile(optimizerReport, resolve(outputRoot, 'optimizer-report.json')),
    copyFile(laneReport, resolve(outputRoot, 'lane-report.json')),
  ])
  const { manifest } = readArtifactManifest(`${program}.artifacts.json`)
  for (const file of entryResources(manifest)) {
    await mkdir(dirname(resolve(outputRoot, 'raw', file)), { recursive: true })
    await copyFile(resolve(itemRoot, file), resolve(outputRoot, 'raw', file))
  }
  const manifestPath = resolve(outputRoot, 'raw/program.js.artifacts.json')
  const graphPath = resolve(outputRoot, 'input-graph.json')
  await copyFile(`${program}.artifacts.json`, manifestPath)
  const bundle = await rollup({ input: facade, treeshake: true,
    plugins: [artifactGraphPlugin([`${program}.artifacts.json`], {
      onReceipt: graph => writeFile(graphPath, stableJson(graph)),
    })],
  })
  const generated = await bundle.generate({ format: 'es', exports: name.startsWith('workbox') ? 'auto' : 'named' })
  const source = generated.output.find((output) => output.type === 'chunk')?.code
  await bundle.close()
  if (!source) throw new Error(`Rollup produced no ${name} chunk`)
  await writeFile(resolve(outputRoot, 'index.js'), source)
  const minified = await minify(source, { ecma: 2018, module: true, safari10: true, format: { comments: false } })
  if (!minified.code) throw new Error(`Terser produced no ${name} output`)
  await writeFile(resolve(outputRoot, 'index.min.js'), `${minified.code}\n`)
  command(process.execPath, [
    resolve(compilerRoot, 'scripts/es2018-gate.mjs'),
    '--parser', resolve(modulesRoot, 'acorn/dist/acorn.mjs'),
    '--root', outputRoot,
    '--output', resolve(outputRoot, 'es2018.json'),
    '--module', resolve(outputRoot, 'index.js'),
    '--module', resolve(outputRoot, 'index.min.js'),
    '--module', resolve(outputRoot, 'raw/program.js'),
    '--module', resolve(outputRoot, 'raw/shim.js'),
    '--module', resolve(outputRoot, 'raw/facade.js'),
  ], { cwd: root })
  const report = JSON.parse(await readFile(optimizerReport, 'utf8'))
  receipt.workloads[name] = {
    artifact: await fileRecord(resolve(outputRoot, 'index.min.js')),
    contract: { manifest: await fileRecord(manifestPath), inputGraph: await fileRecord(graphPath) },
    lane: laneSummary(JSON.parse(await readFile(laneReport, 'utf8'))),
    optimizer: {
      programBytes: report.program?.bytes ?? report.program_bytes ?? null,
      topItems: (report.items || [])
        .map((item) => ({ name: item.name || item.symbol, bytes: item.bytes_after ?? item.bytes ?? 0 }))
        .toSorted((left, right) => right.bytes - left.bytes)
        .slice(0, 10),
    },
  }
  process.stdout.write(`${name}: ${receipt.workloads[name].artifact.bytes} bytes minified\n`)
}

receipt.status = {
  admissionClassified: Object.values(receipt.admission).every((result) => result.supported || result.expectedDiagnostic),
  built: workloads.length > 0 && workloads.every((name) => receipt.workloads[name]?.artifact.bytes > 0),
  es2018: workloads.every((name) => existsSync(resolve(distRoot, name, 'es2018.json'))),
  ordinaryFallbackFree: workloads.every((name) => receipt.workloads[name]?.lane.ordinaryFallback === 0),
}
receipt.provenance.compilerTreeAfter = await compilerTreeIdentity()
receipt.status.compilerStable = receipt.provenance.compilerTreeBefore.digest
  === receipt.provenance.compilerTreeAfter.digest
receipt.status.passed = Object.values(receipt.status).every(Boolean)
await mkdir(dirname(resultPath), { recursive: true })
await writeFile(resultPath, stableJson(receipt))
process.stdout.write(`${resultPath}\n`)
if (!receipt.status.passed) process.exitCode = 1

function compile(arguments_, leafArgs = []) {
  command(resolve(compilerRoot, 'scripts/compile_core.sh'), arguments_, {
    cwd: compilerRoot,
    env: {
      ...compilerEnvironment,
      JS_EXTRA_ARGS: `${compilerEnvironment.JS_EXTRA_ARGS} ${leafArgs.join(' ')}`.trim(),
    },
    stdio: 'inherit',
  })
}

function compileExpectedFailure(source, output, expected) {
  const result = spawnSync(resolve(compilerRoot, 'scripts/compile_core.sh'), [
    source,
    output,
    '--crate-name', 'browser_workload_btree_map',
    '--extern', `mires_js=${topcoatRlib}`,
    '--search', procMacroRoot,
    '--search', buildRoot,
  ], {
    cwd: compilerRoot,
    env: compilerEnvironment,
    encoding: 'utf8',
  })
  const diagnostic = `${result.stderr || ''}\n${result.stdout || ''}`.trim()
  return {
    supported: result.status === 0,
    expectedDiagnostic: result.status !== 0 && expected.test(diagnostic),
    exitStatus: result.status,
    diagnostic: [...new Set(diagnostic.split('\n').filter((line) =>
      line.includes('rustc_codegen_js') || line.includes('cannot cast a pointer'),
    ))].slice(0, 12),
  }
}

async function ensureSysroot() {
  const cacheScript = resolve(compilerRoot, 'scripts/sysroot-cache.sh')
  const expectedArgs = command('bash', [cacheScript, 'args'], { cwd: compilerRoot, env: compilerEnvironment }).trim()
  const sysroot = command('bash', [cacheScript, 'path'], { cwd: compilerRoot, env: compilerEnvironment }).trim()
  try {
    const [currentArgs, currentOpt] = await Promise.all([
      readFile(resolve(sysroot, '.js-args'), 'utf8'),
      readFile(resolve(sysroot, '.rust-opt-level'), 'utf8'),
    ])
    if (currentArgs.trim() === expectedArgs && currentOpt.trim() === 'z') return
  } catch {
    // The sysroot builder reports missing prerequisites.
  }
  command(resolve(compilerRoot, 'scripts/build_sysroot.sh'), [], {
    cwd: compilerRoot,
    env: { ...compilerEnvironment, SKIP_BUILD: '1' },
    stdio: 'inherit',
  })
}

async function sourceHashes() {
  const files = []
  for (const directory of ['src', 'entries', 'fixtures', 'tools']) {
    await walk(resolve(root, directory), files)
  }
  const result = {}
  for (const path of files.toSorted()) {
    const value = await readFile(path)
    result[path.slice(root.length + 1)] = sha256(value)
  }
  return result
}

async function compilerTreeIdentity() {
  // Only compiler inputs. Experiment outputs and recovery archives are not build inputs.
  const files = [];
  const roots = ['backend','rustc-cleanup','rcgjs-object','jsc-link','mires-build','mires-js','structurizer','view-abi','view-async','js-macro','js-extern-macro','runtime','scripts','patches'];
  for (const name of roots) await walk(resolve(compilerRoot, name), files);
  files.push(...['Cargo.toml','Cargo.lock','rust-toolchain.toml'].map(name=>resolve(compilerRoot,name)));
  const entries=[];
  for(const path of files.sort()) entries.push({path:relative(compilerRoot,path),sha256:sha256(await readFile(path))});
  return {digest:sha256(stableJson(entries)),files:entries.length};
}

async function walk(rootPath, files) {
  for (const entry of await readdir(rootPath, { withFileTypes: true })) {
    const path = resolve(rootPath, entry.name)
    if (entry.isDirectory()) await walk(path, files)
    else if (entry.isFile()) files.push(path)
  }
}

function laneSummary(report) {
  const items = report.items || []
  const fallbackItems = items
    .filter((item) => (item.selected_lane || item.lane) === 'baseline')
    .map((item) => ({
      name: item.name,
      emittedBytes: item.bytes ?? null,
      availability: item.availability ?? null,
      reason: item.decline ?? null,
      classification: item.decline === 'encode.synthesized' ? 'compatibility' : 'ordinary',
    }))
  const count = (classification) => fallbackItems.filter((item) => item.classification === classification).length
  return {
    total: items.length,
    jir: items.filter((item) => item.selected_lane === 'jir' || item.lane === 'jir').length,
    fallback: fallbackItems.length,
    fallbackEmittedBytes: fallbackItems.reduce((total, item) => total + (item.emittedBytes || 0), 0),
    compatibilityFallback: count('compatibility'),
    ordinaryFallback: count('ordinary'),
    fallbackItems,
  }
}
