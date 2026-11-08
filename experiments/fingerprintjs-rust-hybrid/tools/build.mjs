#!/usr/bin/env node

import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { args, command } from './common.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = resolve(artifactRoot, '../..')
const oracleRoot = resolve(repositoryRoot, 'experiments/fingerprintjs')
const oracleModules = resolve(repositoryRoot, 'node_modules')
const options = args()
// Which compiler checkout builds this port. The frozen v1 spike is the default;
// `--compiler-root <dir>` or `JSC_SPIKE_ROOT` selects another one (v2 lives at
// `spike/rustc-codegen-js-v2`). A non-default root suffixes the build and dist
// directories so two compilers never share an output tree.
const defaultCompilerRoot = repositoryRoot
const compilerRoot = resolve(
  process.env.JSC_SPIKE_ROOT || (options['compiler-root'] ? String(options['compiler-root']) : defaultCompilerRoot),
)
const compilerSuffix = compilerRoot === defaultCompilerRoot ? '' : `-${compilerRoot.split('/').pop()}`
const optimizer = String(options.optimizer || 'on')
if (!['off', 'report', 'on'].includes(optimizer)) {
  throw new Error('--optimizer must be off, report, or on')
}
const rustOptLevel = options['rust-opt-level'] === undefined
  ? ''
  : String(options['rust-opt-level'])
if (rustOptLevel && !['0', '1', '2', '3', 's', 'z'].includes(rustOptLevel)) {
  throw new Error('--rust-opt-level must be 0, 1, 2, 3, s, or z')
}
const mirInline = options['mir-inline'] === undefined ? 'off' : String(options['mir-inline'])
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
const modeName = [
  optimizer,
  rustOptLevel ? `rust-${rustOptLevel}` : '',
  mirInline === 'on' ? 'mir-inline' : '',
].filter(Boolean).join('-') + compilerSuffix
const buildRoot = resolve(artifactRoot, 'target/js-build', modeName)
const isDefaultBuild = modeName === 'on'
const distRoot = resolve(options.output || (
  isDefaultBuild ? resolve(artifactRoot, 'dist') : resolve(artifactRoot, 'dist', modeName)
))
const optimizerEmitArgs = `js-opt=${optimizer}`
const optimizerLeafArgs = [
  'js-opt-warnings=off',
  `js-workspace-root=${repositoryRoot}`,
].join(' ')
const compilerEnvironment = {
  JS_EXTRA_ARGS: optimizerEmitArgs,
  JSC_SYSROOT_ARGS: optimizerEmitArgs,
  JSC_RUST_OPT_LEVEL: rustOptLevel,
  JSC_RUST_MIR_INLINE: mirInline,
}
const entry = resolve(artifactRoot, 'src/entry.rs')
const exportManifest = parseExportManifest(await readFile(entry, 'utf8'))

const scheduler = options.scheduler || 'boundary'
if (scheduler !== 'boundary') {
  throw new Error('--scheduler must be boundary (fiber is not part of the hybrid port)')
}
const variants = ['boundary']

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

command(resolve(compilerRoot, 'scripts/build.sh'), [], { cwd: compilerRoot, stdio: 'inherit' })
await ensureSysroot()
command(
  'cargo',
  ['build', '--manifest-path', resolve(compilerRoot, 'Cargo.toml'), '-p', 'js-extern-macro', '-p', 'js-macro'],
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
const sourceJsMacro = resolve(compilerRoot, `target/debug/libjs_macro${macroExtension}`)
const jsMacro = resolve(procMacroRoot, `libjs_macro${macroExtension}`)
await copyFile(sourceJsMacro, jsMacro)

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

for (const variant of variants) {
  await buildVariant(variant)
}
if (variants.includes('boundary')) await publishDefault('boundary')

async function buildVariant(variant) {
  const variantRoot = resolve(buildRoot, variant)
  const outputRoot = resolve(distRoot, variant)
  await rm(variantRoot, { recursive: true, force: true })
  await rm(outputRoot, { recursive: true, force: true })
  await mkdir(variantRoot, { recursive: true })
  await mkdir(outputRoot, { recursive: true })

  const libraryRlib = resolve(variantRoot, 'libfingerprintjs_rust_hybrid_experiment.rlib')
  compile([
    resolve(artifactRoot, 'src/lib.rs'),
    libraryRlib,
    '--crate-type',
    'rlib',
    '--crate-name',
    'fingerprintjs_rust_hybrid_experiment',
    '--extern',
    `mires_js=${topcoatRlib}`,
    '--extern',
    `js_macro=${jsMacro}`,
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
      `fingerprintjs_${variant}_entry`,
      '--extern',
      `fingerprintjs_rust_hybrid_experiment=${libraryRlib}`,
      '--extern',
      `mires_js=${topcoatRlib}`,
      '--extern',
      `js_macro=${jsMacro}`,
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
      ].filter(Boolean).join(' '),
    },
  )

  const evidenceRoot = resolve(artifactRoot, 'bench/results/optimizer-reports')
  await mkdir(evidenceRoot, { recursive: true })
  await copyFile(optimizerReport, resolve(evidenceRoot, `${modeName}-${variant}.json`))

  // The link step's own answer to "what could the JIR lane own", published beside the bundles so
  // a coverage gate reads the lane report rather than inferring it from the optimizer report.
  if (laneReport) {
    await copyFile(laneReportPath, resolve(outputRoot, 'lane-report.json'))
  }

  const shim = resolve(variantRoot, 'shim.js')
  command(
    'node',
    [
      resolve(compilerRoot, 'scripts/minimal-shim.mjs'),
      '--module',
      resolve(compilerRoot, 'runtime/shim.js'),
      shim,
      program,
    ],
    { cwd: artifactRoot, stdio: 'inherit' },
  )

  const jsRoot = resolve(artifactRoot, 'js')
  const kernelFiles = (await readdir(jsRoot)).filter((name) => name.endsWith('.js'))
  for (const name of kernelFiles) {
    await copyFile(resolve(jsRoot, name), resolve(variantRoot, name))
  }

  const facade = resolve(variantRoot, 'facade.js')
  await writeFile(facade, makeFacade())
  const rawRoot = resolve(outputRoot, 'raw')
  await mkdir(rawRoot, { recursive: true })
  await Promise.all([
    copyFile(program, resolve(rawRoot, 'program.js')),
    copyFile(shim, resolve(rawRoot, 'shim.js')),
    copyFile(facade, resolve(rawRoot, 'facade.js')),
    ...kernelFiles.map((name) => copyFile(resolve(jsRoot, name), resolve(rawRoot, name))),
  ])
  const bundle = await rollup({ input: facade, treeshake: true })
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
    if (!code) throw new Error(`Rollup produced no ${format} chunk for ${variant}`)
    raw.set(format, code)
    await writeFile(resolve(outputRoot, file), code)
  }
  await bundle.close()

  for (const [format, file] of [
    ['es', 'fp.esm.min.js'],
    ['iife', 'fp.min.js'],
    ['umd', 'fp.umd.min.js'],
  ]) {
    const result = await minify(raw.get(format), {
      module: format === 'es',
      format: { comments: false },
      safari10: true,
    })
    if (!result.code) throw new Error(`Terser produced no ${format} output for ${variant}`)
    await writeFile(resolve(outputRoot, file), `${banner}\n${result.code}\n`)
  }

  await copyFile(resolve(artifactRoot, 'dist-types/fp.d.ts'), resolve(outputRoot, 'fp.d.ts'))
  const attribution = await writeAttribution({
    outputRoot,
    program,
    shim,
    facade,
    iifeMin: resolve(outputRoot, 'fp.min.js'),
    kernelFiles: kernelFiles.map((name) => resolve(jsRoot, name)),
  })
  process.stdout.write(
    `fingerprintjs-rust-hybrid: built ${variant} -> ${outputRoot} (${JSON.stringify(attribution)})\n`,
  )
}

async function publishDefault(variant) {
  const sourceRoot = resolve(distRoot, variant)
  for (const file of [
    'fp.esm.js',
    'fp.esm.min.js',
    'fp.cjs.js',
    'fp.js',
    'fp.min.js',
    'fp.umd.js',
    'fp.umd.min.js',
    'fp.d.ts',
    'attribution.json',
  ]) {
    await copyFile(resolve(sourceRoot, file), resolve(distRoot, file))
  }
  const rawSource = resolve(sourceRoot, 'raw')
  const rawDest = resolve(distRoot, 'raw')
  await mkdir(rawDest, { recursive: true })
  for (const name of await readdir(rawSource)) {
    await copyFile(resolve(rawSource, name), resolve(rawDest, name))
  }
}

async function writeAttribution({ outputRoot, program, shim, facade, iifeMin, kernelFiles }) {
  const [programBytes, shimBytes, facadeBytes, iifeBytes, kernelBytes, jsBlockBytes] = await Promise.all([
    fileSize(program),
    fileSize(shim),
    fileSize(facade),
    fileSize(iifeMin),
    kernelSourceBytes(kernelFiles),
    jsBlockSourceBytes(),
  ])
  const attribution = {
    rustProgram: programBytes,
    rustShim: shimBytes,
    jsKernels: kernelBytes + jsBlockBytes,
    facade: facadeBytes,
    iifeMin: iifeBytes,
    jsFiles: kernelBytes,
    jsBlocks: jsBlockBytes,
    mostlyJs: iifeBytes > 0 && (kernelBytes + jsBlockBytes) / iifeBytes > 0.5,
  }
  await writeFile(resolve(outputRoot, 'attribution.json'), `${JSON.stringify(attribution, null, 2)}\n`)
  return attribution
}

async function fileSize(path) {
  return (await readFile(path)).length
}

async function kernelSourceBytes(files) {
  let total = 0
  for (const file of files) {
    total += (await readFile(file)).length
  }
  return total
}

async function jsBlockSourceBytes() {
  const source = await readFile(resolve(artifactRoot, 'src/kernels.rs'), 'utf8')
  let total = 0
  for (const match of source.matchAll(/js!\s*\{([^}]*)\}/g)) {
    total += Buffer.byteLength(match[1].trim())
  }
  return total
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
  const sysroot = command('bash', [resolve(compilerRoot, 'scripts/sysroot-cache.sh'), 'path'], {
    cwd: compilerRoot,
    env: { ...process.env, ...compilerEnvironment },
  }).trim()
  const argsStamp = resolve(sysroot, '.js-args')
  const rustOptStamp = resolve(sysroot, '.rust-opt-level')
  let currentArgs
  let currentRustOpt
  try {
    ;[currentArgs, currentRustOpt] = await Promise.all([
      readFile(argsStamp, 'utf8'),
      readFile(rustOptStamp, 'utf8'),
    ])
  } catch {
    // The sysroot builder below gives the useful prerequisite error.
  }
  if (currentArgs?.trim() === optimizerEmitArgs && currentRustOpt?.trim() === rustOptLevel) return
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
    ' * FingerprintJS 5.2.0 hybrid Rust+JS compatibility experiment',
    ' * Unpublished research artifact; not an official FingerprintJS release.',
    ' *',
    ' * Licensed under MIT License',
    ' *',
    body,
    ' */',
  ].join('\n')
}
