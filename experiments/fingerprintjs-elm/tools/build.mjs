#!/usr/bin/env node

import { copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'

import { wrapElm } from './wrap-elm.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const compareRoot = resolve(root, '../fingerprintjs-compare')
const dist = resolve(root, 'dist')
const buildDir = resolve(root, 'build')
const rawDir = resolve(dist, 'raw')
const elmBin = process.env.ELM_BIN || resolve(root, '../../node_modules/.bin/elm')
const oracleTypes = resolve(oracleRoot, 'dist/fp.d.ts')
const rustTypes = resolve(root, '../fingerprintjs-rust/dist-types/fp.d.ts')
const defaultElmModule = resolve(buildDir, 'elm.mjs')
const productionElmModule = resolve(buildDir, 'elm.production.mjs')

for (const path of [elmBin, resolve(oracleModules, 'rollup/dist/es/rollup.js'), resolve(oracleModules, 'terser/main.js')]) {
  if (!existsSync(path)) throw new Error(`required build input is missing: ${path}`)
}

function command(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { encoding: 'utf8', ...options })
  if (result.status !== 0) throw new Error(`${cmd} ${args.join(' ')}\n${result.stderr || result.stdout}`)
  return result.stdout
}

function elmAlias(modulePath) {
  return {
    name: `elm-module-${modulePath === productionElmModule ? 'production' : 'standard'}`,
    resolveId(source, importer) {
      if (!importer) return null
      if (resolve(dirname(importer), source) === defaultElmModule) return modulePath
      return null
    },
  }
}

async function minifyElmCompilerOutput(source) {
  const pureFuncs = ['F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'A2', 'A3', 'A4', 'A5', 'A6', 'A7', 'A8', 'A9']
  const compressed = await minify(source, {
    module: true,
    mangle: false,
    compress: {
      pure_funcs: pureFuncs,
      pure_getters: true,
      keep_fargs: false,
      unsafe_comps: true,
      unsafe: true,
    },
    format: { comments: false },
    safari10: true,
  })
  if (!compressed.code) throw new Error('Terser produced no compiler-aware Elm compression output')
  const mangled = await minify(compressed.code, {
    module: true,
    compress: false,
    mangle: true,
    format: { comments: false },
    safari10: true,
  })
  if (!mangled.code) throw new Error('Terser produced no compiler-aware Elm mangling output')
  return mangled.code
}

async function commonMinify(code, module) {
  const result = await minify(code, {
    format: { comments: false },
    safari10: true,
    module,
  })
  if (!result.code) throw new Error('Terser produced empty output')
  return result.code
}

async function bundleVariant({ id, profile, elmModule, compilerAware, makeDefault = false }) {
  const out = resolve(dist, id)
  const raw = resolve(out, 'raw')
  await rm(out, { recursive: true, force: true })
  await mkdir(raw, { recursive: true })

  const plugins = [elmAlias(elmModule)]
  const onwarn = (warning, warn) => {
    if (warning.code === 'THIS_IS_UNDEFINED') return
    if (warning.code === 'CIRCULAR_DEPENDENCY') return
    warn(warning)
  }

  const esmBundle = await rollup({ input: entry, plugins, onwarn })
  const esm = await esmBundle.generate({ format: 'esm', exports: 'named', sourcemap: false })
  await esmBundle.close()
  const esmCode = esm.output.find((output) => output.type === 'chunk')?.code
  if (!esmCode) throw new Error(`Rollup produced no ESM chunk for Elm ${id}`)
  const esmMin = await commonMinify(esmCode, true)

  const iifeBundle = await rollup({ input: entry, plugins, onwarn })
  const iife = await iifeBundle.generate({ format: 'iife', name: 'FingerprintJS', exports: 'named', sourcemap: false })
  await iifeBundle.close()
  const iifeCode = iife.output.find((output) => output.type === 'chunk')?.code
  if (!iifeCode) throw new Error(`Rollup produced no IIFE chunk for Elm ${id}`)
  const iifeMin = await commonMinify(iifeCode, false)

  await Promise.all([
    writeFile(resolve(out, 'fp.esm.js'), `${banner}\n${esmCode}`),
    writeFile(resolve(out, 'fp.esm.min.js'), `${banner}\n${esmMin}\n`),
    writeFile(resolve(out, 'fp.min.js'), `${banner}\n${iifeMin}\n`),
    writeFile(resolve(raw, 'program.js'), await readFile(elmModule, 'utf8')),
    writeFile(resolve(out, 'variant.json'), `${JSON.stringify({
      schema: 1,
      family: 'elm',
      id,
      profile,
      compiler: 'Elm 0.19.2 --optimize',
      compilerAwareMinifier: compilerAware,
      compilerAwareScope: compilerAware ? 'generated Elm module only; JS host uses ordinary Terser' : null,
      packaging: 'Rollup plus common Terser pass',
    }, null, 2)}\n`),
  ])
  await copyFile(dts, resolve(out, 'fp.d.ts'))

  const iifeBytes = Buffer.byteLength(`${banner}\n${iifeMin}\n`)
  const attribution = {
    elmProgram: elmProgramBytes,
    elmRuntime: elmRuntimeBytes,
    jsHost: jsHostBytes,
    facade: facadeBytes,
    iifeMin: iifeBytes,
    mostlyJs: iifeBytes > 0 && jsHostBytes / iifeBytes > 0.5,
    compilerAwareMinifier: compilerAware,
  }
  await writeFile(resolve(out, 'attribution.json'), `${JSON.stringify(attribution, null, 2)}\n`)

  if (makeDefault) {
    await Promise.all([
      copyFile(resolve(out, 'fp.esm.js'), resolve(dist, 'fp.esm.js')),
      copyFile(resolve(out, 'fp.esm.min.js'), resolve(dist, 'fp.esm.min.js')),
      copyFile(resolve(out, 'fp.min.js'), resolve(dist, 'fp.min.js')),
      copyFile(resolve(out, 'fp.d.ts'), resolve(dist, 'fp.d.ts')),
      copyFile(resolve(out, 'variant.json'), resolve(dist, 'variant.json')),
      copyFile(resolve(out, 'attribution.json'), resolve(dist, 'attribution.json')),
    ])
  }
  return attribution
}

await mkdir(buildDir, { recursive: true })
command(elmBin, ['make', 'src/Fingerprint.elm', '--optimize', '--output=build/elm.js'], { cwd: root, stdio: 'inherit' })
command(elmBin, ['make', 'src/Runtime.elm', '--optimize', '--output=build/elm-runtime.js'], { cwd: root, stdio: 'inherit' })

const elmSource = await readFile(resolve(buildDir, 'elm.js'), 'utf8')
const elmRuntimeSource = await readFile(resolve(buildDir, 'elm-runtime.js'), 'utf8')
const wrappedElm = wrapElm(elmSource)
await writeFile(defaultElmModule, wrappedElm)

const { rollup } = await import(pathToFileURL(resolve(oracleModules, 'rollup/dist/es/rollup.js')))
const { minify } = await import(pathToFileURL(resolve(oracleModules, 'terser/main.js')))
const productionElm = await minifyElmCompilerOutput(wrappedElm)
await writeFile(productionElmModule, `${productionElm}\n`)

const banner = (await readFile(resolve(compareRoot, 'banner.txt'), 'utf8')).trimEnd()
const entry = resolve(root, 'js/facade.js')
const dts = existsSync(oracleTypes) ? oracleTypes : rustTypes
if (!existsSync(dts)) throw new Error('oracle fp.d.ts not found')

await rm(dist, { recursive: true, force: true })
await mkdir(rawDir, { recursive: true })
const jsHostBytes = await directoryBytes(resolve(root, 'js'), (name) => name !== 'facade.js')
const facadeBytes = (await stat(resolve(root, 'js/facade.js'))).size
const elmProgramBytes = Math.max(0, Buffer.byteLength(elmSource) - Buffer.byteLength(elmRuntimeSource))
const elmRuntimeBytes = Buffer.byteLength(elmRuntimeSource)

const variants = {
  standard: await bundleVariant({
    id: 'standard',
    profile: 'readable',
    elmModule: defaultElmModule,
    compilerAware: false,
    makeDefault: true,
  }),
  production: await bundleVariant({
    id: 'production',
    profile: 'production',
    elmModule: productionElmModule,
    compilerAware: true,
  }),
}

await Promise.all([
  writeFile(resolve(rawDir, 'elm.js'), elmSource),
  writeFile(resolve(rawDir, 'elm-runtime.js'), elmRuntimeSource),
  writeFile(resolve(rawDir, 'elm.mjs'), wrappedElm),
  writeFile(resolve(rawDir, 'elm.production.mjs'), `${productionElm}\n`),
  writeFile(resolve(rawDir, 'variants.json'), `${JSON.stringify(variants, null, 2)}\n`),
])

console.log(`wrote ${dist}/standard and ${dist}/production; root dist remains the standard-minifier alias`)
console.log(JSON.stringify(variants))

async function directoryBytes(dir, keep) {
  let total = 0
  for (const item of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, item.name)
    if (item.isDirectory()) total += await directoryBytes(path, keep)
    else if (item.isFile() && (!keep || keep(item.name))) total += (await stat(path)).size
  }
  return total
}
