#!/usr/bin/env node

import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const dist = resolve(root, 'dist')
const rawDir = resolve(dist, 'raw')

const NAMED = [
  'componentsToDebugString',
  'getFullscreenElement',
  'getUnstableAudioFingerprint',
  'getUnstableCanvasFingerprint',
  'getUnstableHardwareConcurrency',
  'getUnstableScreenFrame',
  'getUnstableScreenResolution',
  'getWebGLContext',
  'hashComponents',
  'isAndroid',
  'isChromium',
  'isDesktopWebKit',
  'isEdgeHTML',
  'isGecko',
  'isSamsungInternet',
  'isTrident',
  'isWebKit',
  'load',
  'loadSources',
  'murmurX64Hash128',
  'prepareForSources',
  'sources',
  'transformSource',
  'withIframe',
]

function command(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, {
    encoding: 'utf8',
    ...options,
  })
  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(' ')} failed:\n${result.stderr || result.stdout}`)
  }
  return result.stdout
}

function makeBanner(licenseText) {
  const body = licenseText.trim().split('\n').map((line) => ` * ${line}`).join('\n')
  return [
    '/**',
    ' * FingerprintJS v5.2.0 - Copyright (c) FingerprintJS, Inc, 2026 (https://fingerprint.com)',
    ' *',
    ' * Licensed under MIT License',
    ' *',
    body,
    ' */',
  ].join('\n')
}

function wrapEsm(program) {
  const named = NAMED.map((name) => `export const ${name} = __exp.${name};`).join('\n')
  return `${program}
const __exp = globalThis.__fp_jsoo_exports;
delete globalThis.__fp_jsoo_exports;
${named}
export default { load, hashComponents, componentsToDebugString };
`
}

function wrapIife(program) {
  return `(function (globalThis) {
${program}
const __exp = globalThis.__fp_jsoo_exports;
delete globalThis.__fp_jsoo_exports;
const FingerprintJS = {
${NAMED.map((name) => `  ${name}: __exp.${name},`).join('\n')}
};
FingerprintJS.default = { load: FingerprintJS.load, hashComponents: FingerprintJS.hashComponents, componentsToDebugString: FingerprintJS.componentsToDebugString };
globalThis.FingerprintJS = FingerprintJS;
})(typeof globalThis !== "undefined" ? globalThis : this);
`
}

async function minify(code, format) {
  const terserPath = resolve(oracleModules, 'terser/main.js')
  const { minify } = await import(terserPath)
  const result = await minify(code, {
    format: { comments: false },
    safari10: true,
    module: format === 'esm',
  })
  if (result.error) throw result.error
  return result.code
}

const license = await readFile(resolve(root, 'LICENSE'), 'utf8')
const banner = makeBanner(license)

opamCommand(['dune', 'build', '--root', '.'], {
  cwd: root,
  stdio: 'inherit',
})

const built = resolve(root, '_build/default/src/fingerprintjs.bc.js')
const bytecode = resolve(root, '_build/default/src/fingerprintjs.bc-for-jsoo')
if (!existsSync(built)) {
  throw new Error(`js_of_ocaml emit missing: ${built}`)
}

await mkdir(rawDir, { recursive: true })
const stripMap = (source) => source.replace(/\n\/\/# sourceMappingURL=[\s\S]*$/, '\n')
const program = stripMap(await readFile(built, 'utf8'))
await writeFile(resolve(rawDir, 'program.js'), program)

function jsooCompile(args, output) {
  opamCommand(['js_of_ocaml', 'compile', ...args.split(/\s+/), '-o', output, bytecode])
}

const prettyPath = resolve(rawDir, 'program.pretty.js')
const programOnlyPath = resolve(rawDir, 'program-only.js')
jsooCompile('--pretty --no-source-map --effects=disabled --target-env=browser --no-extern-fs --opt=3', prettyPath)
jsooCompile('--no-runtime --no-source-map --effects=disabled --target-env=browser --no-extern-fs --opt=3', programOnlyPath)
const pretty = stripMap(await readFile(prettyPath, 'utf8'))
const programOnly = stripMap(await readFile(programOnlyPath, 'utf8'))
await writeFile(resolve(rawDir, 'runtime-included-bytes.txt'), `${program.length - programOnly.length}\n`)

let runtime = ''
try {
  runtime = opamCommand(['js_of_ocaml', 'print-standard-runtime'])
  await writeFile(resolve(rawDir, 'runtime-full.js'), runtime)
} catch (error) {
  console.warn('could not emit full JSOO runtime for size notes:', error.message)
}

const esm = `${banner}\n${wrapEsm(pretty)}\n`
await mkdir(dist, { recursive: true })
await writeFile(resolve(dist, 'fp.esm.js'), esm)

const esmMin = await minify(wrapEsm(program), 'esm')
await writeFile(resolve(dist, 'fp.esm.min.js'), `${banner}\n${esmMin}\n`)

const iifeMin = await minify(wrapIife(program), 'iife')
await writeFile(resolve(dist, 'fp.min.js'), `${banner}\n${iifeMin}\n`)

const dtsCandidates = [
  resolve(oracleRoot, 'dist/fp.d.ts'),
  resolve(root, '../fingerprintjs-rust/dist-types/fp.d.ts'),
]
const dts = dtsCandidates.find((path) => existsSync(path))
if (!dts) throw new Error('oracle fp.d.ts not found')
await copyFile(dts, resolve(dist, 'fp.d.ts'))

console.log(`wrote ${dist}`)

function opamCommand(args, options = {}) {
  return command('opam', ['exec', '--root', process.env.MIRES_OPAM_ROOT || resolve(root, '../../.toolchains/opam'),
    '--switch', process.env.MIRES_OPAM_SWITCH || 'mires-comparators', '--', ...args], { ...options, env: { ...process.env, DUNE_JOBS: process.env.DUNE_JOBS || '4', ...options.env } })
}
