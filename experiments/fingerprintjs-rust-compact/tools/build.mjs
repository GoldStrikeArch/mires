#!/usr/bin/env node

import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { args, command } from './common.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = resolve(artifactRoot, '../..')
const oracleRoot = resolve(repositoryRoot, 'experiments/fingerprintjs')
const oracleModules = resolve(repositoryRoot, 'node_modules')
const options = args()
const sourceRoot = resolve(options['source-root'] || artifactRoot)
// Which compiler checkout builds this port. The frozen v1 spike is the default;
// `--compiler-root <dir>` or `JSC_SPIKE_ROOT` selects another one (v2 lives at
// `spike/rustc-codegen-js-v2`). A non-default root suffixes the build and dist
// directories so two compilers never share an output tree.
const defaultCompilerRoot = repositoryRoot
const compilerRoot = resolve(
  process.env.JSC_SPIKE_ROOT || (options['compiler-root'] ? String(options['compiler-root']) : defaultCompilerRoot),
)
const artifactContractPath = resolve(compilerRoot, 'scripts/artifact-manifest.mjs')
const artifactContract = existsSync(artifactContractPath)
  ? await import(pathToFileURL(artifactContractPath).href) : null
const compilerSuffix = compilerRoot === defaultCompilerRoot ? '' : `-${compilerRoot.split('/').pop()}`
const optimizer = String(options.optimizer || 'on')
if (!['off', 'report', 'on'].includes(optimizer)) {
  throw new Error('--optimizer must be off, report, or on')
}
const compilerProfile = options.profile === undefined ? '' : String(options.profile)
if (compilerProfile && !['dev', 'report', 'release-size', 'release-balanced', 'release-speed'].includes(compilerProfile)) {
  throw new Error('--profile must be dev, report, release-size, release-balanced, or release-speed')
}
// `JSC_PANIC` selects the build mode (scripts/sysroot-cache.sh); unset or `unwind` builds for
// targets/mires32-unwind.json with -Cpanic=unwind: rustc's own unwind edges carry host-exception
// cleanup, the scripts turn the backend's re-derived cleanup (`js-cleanup`) off, and `--cleanup on`
// would name a lowering the build does not use. `abort` selects wasm32-unknown-unknown instead.
const panic = process.env.JSC_PANIC || 'unwind'
if (!['abort', 'unwind'].includes(panic)) throw new Error('JSC_PANIC must be unset, abort, or unwind')
const cleanup = String(options.cleanup || (
  panic === 'unwind' ? 'off' : compilerProfile.startsWith('release-') ? 'on' : 'off'))
if (!['on', 'off'].includes(cleanup)) throw new Error('--cleanup must be on or off')
if (panic === 'unwind' && cleanup === 'on') {
  throw new Error('JSC_PANIC=unwind keeps cleanup through rustc unwind edges; --cleanup on is the abort-mode lowering')
}
const buildKey = options['build-key'] === undefined ? '' : String(options['build-key'])
if (buildKey && !/^[A-Za-z0-9._-]+$/.test(buildKey)) throw new Error('--build-key is not path-safe')
const ecmaTargetExplicit = options['ecma-target'] !== undefined
const ecmaTarget = String(options['ecma-target'] || (compilerProfile.startsWith('release-') ? '2018' : 'latest'))
if (!['2018', 'latest'].includes(ecmaTarget)) {
  throw new Error('--ecma-target must be 2018 or latest')
}
const rustOptLevel = options['rust-opt-level'] === undefined
  ? 'z'
  : String(options['rust-opt-level'])
if (rustOptLevel && !['0', '1', '2', '3', 's', 'z'].includes(rustOptLevel)) {
  throw new Error('--rust-opt-level must be 0, 1, 2, 3, s, or z')
}
const mirInline = String(options['mir-inline'] ?? (process.env.JSC_RUST_MIR_INLINE || 'off'))
if (!['off', 'on'].includes(mirInline)) {
  throw new Error('--mir-inline must be off or on')
}

// Which lane the link step prints each item from: `baseline` is v1's printed text (the frozen
// oracle), `jir` lowers every item the JIR lowerer can and keeps the baseline for the rest, and
// `shadow` prints both — the baseline to `program.js` and the JIR lane to `program.js.jir.js`.
// It is a LINK step decision, not an emit-affecting one, so it does not enter the sysroot cache
// key and does not key the build directory: one set of objects serves every lane.
const lane = String(options.lane || 'baseline')
if (!['baseline', 'shadow', 'jir'].includes(lane)) {
  throw new Error('--lane must be baseline, shadow, or jir')
}
// `--lane-report` asks the link step for the per item lane report and publishes it beside the
// bundles, which is what scripts/jir-coverage.sh reads instead of inferring coverage from the
// optimizer report.
const laneReport = options['lane-report'] !== undefined && options['lane-report'] !== false
const scheduler = options.scheduler || 'boundary'
if (scheduler !== 'boundary') {
  throw new Error('--scheduler must be boundary; the compact port has no fiber scheduler')
}
const modeName = [
  buildKey ? `build-${buildKey}` : '',
  compilerProfile ? `profile-${compilerProfile}` : '',
  `cleanup-${cleanup}`,
  optimizer,
  ecmaTargetExplicit ? `ecma-${ecmaTarget}` : '',
  rustOptLevel ? `rust-${rustOptLevel}` : '',
  mirInline === 'on' ? 'mir-inline' : '',
  panic === 'unwind' ? 'panic-unwind' : '',
].filter(Boolean).join('-') + compilerSuffix
const buildRoot = resolve(artifactRoot, 'target/js-build', modeName)
const distRoot = resolve(options.output || resolve(artifactRoot, 'dist'))
const optimizerEmitArgs = [
  compilerProfile ? `js-profile=${compilerProfile}` : '',
  `js-cleanup=${cleanup}`,
  `js-opt=${optimizer}`,
  ecmaTargetExplicit ? `js-ecma-target=${ecmaTarget}` : '',
].filter(Boolean).join(' ')
const optimizerLeafArgs = [
  'js-opt-warnings=off',
  `js-workspace-root=${repositoryRoot}`,
].join(' ')
const compilerEnvironment = {
  JS_EXTRA_ARGS: optimizerEmitArgs,
  JSC_REMAP_ROOT: sourceRoot,
  JSC_SYSROOT_ARGS: optimizerEmitArgs,
  JSC_RUST_OPT_LEVEL: rustOptLevel,
  JSC_RUST_MIR_INLINE: mirInline,
  JSC_PANIC: panic,
}
const entry = resolve(sourceRoot, 'src/entry.rs')
const exportManifest = parseExportManifest(await readFile(entry, 'utf8'))
const crateName = 'fingerprintjs_rust_compact_experiment'

for (const path of [
  resolve(compilerRoot, 'scripts/compile_core.sh'),
  resolve(compilerRoot, 'scripts/sysroot-cache.sh'),
  resolve(compilerRoot, 'scripts/minimal-shim.mjs'),
  resolve(compilerRoot, 'runtime/shim.js'),
  resolve(oracleModules, 'rollup/dist/es/rollup.js'),
  resolve(oracleModules, 'terser/main.js'),
]) {
  if (!existsSync(path)) throw new Error(`required build input is missing: ${path}`)
}
const es2018Gate = resolve(compilerRoot, 'scripts/es2018-gate.mjs')
const es2018Parser = resolve(oracleModules, 'acorn/dist/acorn.mjs')
if (ecmaTarget === '2018') {
  for (const path of [es2018Gate, es2018Parser]) {
    if (!existsSync(path)) throw new Error(`required ES2018 gate input is missing: ${path}`)
  }
}

command(resolve(compilerRoot, 'scripts/build.sh'), [], { cwd: compilerRoot, stdio: 'inherit' })
await ensureSysroot()
command(
  'cargo',
  ['build', '--manifest-path', resolve(compilerRoot, 'Cargo.toml'), '-p', 'js-extern-macro'],
  { cwd: compilerRoot, stdio: 'inherit' },
)

await mkdir(buildRoot, { recursive: true })
const procMacroRoot = resolve(buildRoot, 'proc-macro')
await rm(procMacroRoot, { recursive: true, force: true })
await mkdir(procMacroRoot, { recursive: true })
const macroExtension = process.platform === 'darwin' ? '.dylib' : '.so'
const sourceMacro = resolve(compilerRoot, `target/debug/libjs_extern_macro${macroExtension}`)
const procMacro = resolve(procMacroRoot, `libjs_extern_macro${macroExtension}`)
await copyFile(sourceMacro, procMacro)

const compileCore = resolve(compilerRoot, 'scripts/compile_core.sh')
const topcoatRlib = resolve(buildRoot, 'libmires_js.rlib')
compile([
  resolve(compilerRoot, 'mires-js/src/lib.rs'),
  topcoatRlib,
  '--crate-type',
  'rlib',
  '--crate-name',
  'mires_js',
  '--extern',
  `js_extern_macro=${procMacro}`,
])

const license = await readFile(resolve(artifactRoot, 'LICENSE'), 'utf8')
const banner = makeBanner(license)
const { rollup } = await import(pathToFileURL(resolve(oracleModules, 'rollup/dist/es/rollup.js')))
const { minify } = await import(pathToFileURL(resolve(oracleModules, 'terser/main.js')))

await buildBoundary()

async function buildBoundary() {
  const variantRoot = resolve(buildRoot, 'boundary')
  const outputRoot = distRoot
  await rm(variantRoot, { recursive: true, force: true })
  await mkdir(variantRoot, { recursive: true })
  await mkdir(outputRoot, { recursive: true })

  const libraryRlib = resolve(variantRoot, `lib${crateName}.rlib`)
  compile([
    resolve(sourceRoot, 'src/lib.rs'),
    libraryRlib,
    '--crate-type',
    'rlib',
    '--crate-name',
    crateName,
    '--extern',
    `mires_js=${topcoatRlib}`,
    '--search',
    procMacroRoot,
  ])

  const program = resolve(variantRoot, 'program.js')
  const optimizerReport = resolve(variantRoot, 'optimizer-report.json')
  const laneReportPath = resolve(variantRoot, 'lane-report.json')
  compile(
    [
      entry,
      program,
      '--crate-name',
      'fingerprintjs_compact_entry',
      '--extern',
      `${crateName}=${libraryRlib}`,
      '--extern',
      `mires_js=${topcoatRlib}`,
      '--search',
      procMacroRoot,
      '--search',
      buildRoot,
    ],
    {
      JS_EXTRA_ARGS: [
        'js-modules=esm',
        'js-comments=off',
        `js-opt-report=${optimizerReport}`,
        lane === 'baseline' ? '' : `js-lane=${lane}`,
        laneReport ? `js-lane-report=${laneReportPath}` : '',
        // `--str bytes`: link with `js-str=bytes` (experimental; link-only, needs `--lane jir`).
        options.str ? `js-str=${options.str}` : '',
      ].filter(Boolean).join(' '),
    },
  )

  const evidenceRoot = resolve(options['evidence-output'] || resolve(artifactRoot, 'bench/results/optimizer-reports'))
  await mkdir(evidenceRoot, { recursive: true })
  await copyFile(optimizerReport, resolve(evidenceRoot, `${modeName}-boundary.json`))

  // The link step's own answer to "what could the JIR lane own", published beside the bundles so
  // a coverage gate reads the lane report rather than inferring it from the optimizer report.
  if (laneReport) {
    await Promise.all([
      copyFile(laneReportPath, resolve(outputRoot, 'lane-report.json')),
      copyFile(`${program}.lane.json`, resolve(outputRoot, 'link-lane-report.json')),
    ])
  }

  const shim = resolve(variantRoot, 'shim.js')
  command(
    'node',
    [
      resolve(compilerRoot, 'scripts/minimal-shim.mjs'),
      ...(artifactContract ? ['--manifest', `${program}.artifacts.json`] : []),
      '--module',
      resolve(compilerRoot, 'runtime/shim.js'),
      shim,
      ...(artifactContract ? [] : [program]),
    ],
    { cwd: artifactRoot, stdio: 'inherit' },
  )

  const facade = resolve(variantRoot, 'facade.js')
  await writeFile(facade, makeFacade())
  const rawRoot = resolve(outputRoot, 'raw')
  await mkdir(rawRoot, { recursive: true })
  await Promise.all([
    copyFile(program, resolve(rawRoot, 'program.js')),
    copyFile(shim, resolve(rawRoot, 'shim.js')),
    copyFile(facade, resolve(rawRoot, 'facade.js')),
  ])
  if (artifactContract) {
    const { manifest } = artifactContract.readArtifactManifest(`${program}.artifacts.json`)
    for (const name of artifactContract.entryResources(manifest)) {
      await mkdir(dirname(resolve(rawRoot, name)), { recursive: true })
      await copyFile(resolve(variantRoot, name), resolve(rawRoot, name))
    }
    await copyFile(`${program}.artifacts.json`, resolve(rawRoot, 'program.js.artifacts.json'))
  }
  const bundle = await rollup({ input: facade, treeshake: true,
    plugins: artifactContract ? [artifactContract.artifactGraphPlugin([`${program}.artifacts.json`], {
      onReceipt: receipt => writeFile(resolve(outputRoot, 'input-graph.json'), `${JSON.stringify(receipt, null, 2)}\n`),
    })] : [],
  })
  const formats = [
    ['es', 'fp.esm.js'],
    ['cjs', 'fp.cjs.js'],
    ['iife', 'fp.js'],
    ['umd', 'fp.umd.js'],
  ]
  const raw = new Map()
  for (const [format, file] of formats) {
    const generated = await bundle.generate({
      format,
      name: 'FingerprintJS',
      exports: 'named',
      banner,
    })
    const code = generated.output.find((output) => output.type === 'chunk')?.code
    if (!code) throw new Error(`Rollup produced no ${format} chunk`)
    raw.set(format, code)
    await writeFile(resolve(outputRoot, file), code)
  }
  await bundle.close()

  for (const [format, file] of [
    ['es', 'fp.esm.min.js'],
    ['iife', 'fp.min.js'],
    ['umd', 'fp.umd.min.js'],
  ]) {
    const sourceFile = file.replace('.min.js', '.js')
    const result = await minify({ [sourceFile]: raw.get(format) }, {
      ecma: 2018,
      module: format === 'es',
      format: { comments: false, preamble: banner },
      safari10: true,
      sourceMap: { filename: file, includeSources: true, url: `${file}.map` },
    })
    if (!result.code || !result.map) throw new Error(`Terser produced incomplete ${format} output`)
    await Promise.all([
      writeFile(resolve(outputRoot, file), `${result.code}\n`),
      writeFile(resolve(outputRoot, `${file}.map`), `${result.map}\n`),
    ])
  }

  await copyFile(resolve(oracleRoot, 'dist/fp.d.ts'), resolve(outputRoot, 'fp.d.ts'))
  if (ecmaTarget === '2018') checkEs2018(outputRoot)
  process.stdout.write(`fingerprintjs-rust-compact: built boundary -> ${outputRoot}\n`)
}

function checkEs2018(outputRoot) {
  const rawRoot = resolve(outputRoot, 'raw')
  command(process.execPath, [
    es2018Gate,
    '--parser', es2018Parser,
    '--root', outputRoot,
    '--output', resolve(outputRoot, 'es2018.json'),
    '--module', resolve(outputRoot, 'fp.esm.js'),
    '--module', resolve(outputRoot, 'fp.esm.min.js'),
    '--module', resolve(rawRoot, 'program.js'),
    '--module', resolve(rawRoot, 'shim.js'),
    '--module', resolve(rawRoot, 'facade.js'),
    '--script', resolve(outputRoot, 'fp.cjs.js'),
    '--script', resolve(outputRoot, 'fp.js'),
    '--script', resolve(outputRoot, 'fp.min.js'),
    '--script', resolve(outputRoot, 'fp.umd.js'),
    '--script', resolve(outputRoot, 'fp.umd.min.js'),
  ], { cwd: artifactRoot })
}

function compile(arguments_, extraEnvironment = {}) {
  const extraArgs = [
    compilerEnvironment.JS_EXTRA_ARGS,
    optimizerLeafArgs,
    extraEnvironment.JS_EXTRA_ARGS,
  ]
    .filter(Boolean)
    .join(' ')
  command(compileCore, arguments_, {
    cwd: compilerRoot,
    env: {
      ...process.env,
      ...compilerEnvironment,
      ...extraEnvironment,
      JS_EXTRA_ARGS: extraArgs,
    },
    stdio: 'inherit',
  })
}

async function ensureSysroot() {
  command(resolve(compilerRoot, 'scripts/build_sysroot.sh'), [], {
    cwd: compilerRoot,
    env: { ...process.env, ...compilerEnvironment, SKIP_BUILD: '1' },
    stdio: 'inherit',
  })
}

function makeFacade() {
  const roots = [...exportManifest.map(([, root]) => root), 'fp_sources']
  const wrappers = exportManifest.map(([name, root, arity, defaultAt, localName = name]) => {
    const arguments_ = ['a', 'b', 'c', 'd'].slice(0, arity)
    const parameters = arguments_.map((argument, index) =>
      index === defaultAt ? `${argument} = undefined` : argument,
    )
    return `function ${localName}(${parameters.join(', ')}) { return ${root}(${arguments_.join(', ')}) }`
  }).join('\n')
  const aliases = exportManifest
    .filter(([name, , , , localName = name]) => name !== localName)
    .map(([name, , , , localName]) => `const ${name} = ${localName}`)
    .join('\n')
  const exports = [
    ...exportManifest.map(([name]) => name),
    'sources',
  ].sort()
  return [
    `import { ${roots.join(', ')} } from './program.js'`,
    '',
    wrappers,
    aliases,
    'const sources = fp_sources()',
    'const defaultExport = { load, hashComponents, componentsToDebugString }',
    '',
    `export { ${exports.join(', ')} }`,
    'export default defaultExport',
    '',
  ].join('\n')
}

function parseExportManifest(source) {
  const start = source.indexOf('const PACKAGE_EXPORTS:')
  const end = source.indexOf('];', start)
  if (start < 0 || end < 0) throw new Error('entry.rs has no PACKAGE_EXPORTS manifest')
  const entries = []
  const tuple = /\(\s*"([^"]+)",\s*"([^"]+)",\s*(\d+),\s*(-?\d+),\s*"([^"]+)"\s*,?\s*\)/g
  for (const match of source.slice(start, end).matchAll(tuple)) {
    const [, name, root, arity, defaultAt, localName] = match
    entries.push([name, root, Number(arity), Number(defaultAt), localName])
  }
  if (entries.length === 0) throw new Error('entry.rs PACKAGE_EXPORTS manifest is empty')
  return entries
}

function makeBanner(licenseText) {
  const body = licenseText.trim().split('\n').map((line) => ` * ${line}`).join('\n')
  return [
    '/**',
    ' * FingerprintJS 5.2.0 compatibility experiment compiled from Rust',
    ' * Unpublished compact-Rust research artifact; not an official FingerprintJS release.',
    ' *',
    ' * Licensed under MIT License',
    ' *',
    body,
    ' */',
  ].join('\n')
}
