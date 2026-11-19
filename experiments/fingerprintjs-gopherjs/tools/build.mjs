#!/usr/bin/env node
// Builds the GopherJS port of FingerprintJS 5.2.0.
//
// GopherJS emits a classic script, not a module. The program publishes its API on
// js.Module.exports; this script wraps the emit in a generated ESM facade and an IIFE
// facade that bind the prelude's environment probes (`window`, `module`) and re-export the
// 25 names, then runs the oracle Terser options. The facades are build output, not
// product code.

import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'
import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const rootModules = resolve(root, '../../node_modules')
const dist = resolve(root, 'dist')
const rawDir = resolve(dist, 'raw')
const work = resolve(root, 'build/gopherjs')

const GO_VERSION = 'go1.21.13'
const GOPHERJS_VERSION = '1.21.0'
const goBase = process.env.MIRES_GO_ROOT || resolve(root, '../../.toolchains/go')
const goRoot = resolve(goBase, GO_VERSION)
const gopherjs = resolve(goBase, 'bin/gopherjs')
const env = {
  ...process.env,
  GOROOT: goRoot,
  GOPHERJS_GOROOT: goRoot,
  GOPATH: resolve(goBase, 'gopath'),
  // The Go build cache stays with the toolchain instead of the user's global cache.
  GOCACHE: resolve(goBase, 'cache'),
  GOTOOLCHAIN: 'local',
  GOFLAGS: '-modcacherw',
  PATH: `${resolve(goRoot, 'bin')}:${process.env.PATH || ''}`,
}

// Named exports in the contract's order, without `default`.
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

function run(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { encoding: 'utf8', env, cwd: root, ...options })
  if (result.status !== 0) {
    if (result.stdout) process.stdout.write(result.stdout)
    if (result.stderr) process.stderr.write(result.stderr)
    throw new Error(`${cmd} ${args.join(' ')} failed (${result.status ?? result.error})`)
  }
  return result.stdout
}

function makeBanner(licenseText) {
  const body = licenseText.trim().split('\n').map((line) => (line.length ? ` * ${line}` : ' *')).join('\n')
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

// The GopherJS prelude picks its global object from `window`, `self`, `global` or `this`, and
// js.Module from `module`. Binding `window` to globalThis (the same object in a browser page)
// keeps the import from reading browser globals or taking the Node.js `require` branch, and
// `module` receives the exports.
function evaluateProgram(program, moduleName) {
  return `(function (window, module) {
${program}
}).call(globalThis, globalThis, ${moduleName});`
}

function wrapEsm(program) {
  return `const __fpModule = { exports: {} };
${evaluateProgram(program, '__fpModule')}
const __fp = __fpModule.exports;
${NAMED.map((name) => `export const ${name} = __fp.${name};`).join('\n')}
export default __fp.default;
`
}

function wrapIife(program) {
  const members = ['componentsToDebugString', 'default', ...NAMED.slice(1)]
  return `var FingerprintJS = (function () {
var __fpModule = { exports: {} };
${evaluateProgram(program, '__fpModule')}
var __fp = __fpModule.exports;
var namespace = {
${members.map((name) => `  ${name}: __fp.${name},`).join('\n')}
};
Object.defineProperty(namespace, '__esModule', { value: true });
return namespace;
})();
`
}

async function minify(code, module) {
  const { minify: terser } = await import(pathToFileURL(resolve(rootModules, 'terser/main.js')))
  const result = await terser(code, { format: { comments: false }, safari10: true, module })
  if (!result.code) throw new Error('Terser produced empty output')
  return result.code
}

function sizes(text) {
  const bytes = Buffer.from(text)
  return {
    bytes: bytes.length,
    gzip9: gzipSync(bytes, { level: 9, mtime: 0 }).length,
    brotli11: brotliCompressSync(bytes, { params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 } }).length,
  }
}

// Splits a GopherJS emit into the prelude and its `$packages["path"] = (function() {...})();`
// package blocks, attributing bytes to each package. The tail after the last package is the
// program start-up code.
function attributePackages(code) {
  const marker = /\$packages\["([^"]+)"\]\s*=\s*\(function\s*\(\)/g
  const starts = [...code.matchAll(marker)].map((match) => ({ name: match[1], index: match.index }))
  if (starts.length === 0) throw new Error('no GopherJS package blocks found')
  const end = code.indexOf('$callForAllPackages(', starts[starts.length - 1].index)
  const packages = {}
  starts.forEach((start, i) => {
    const stop = i + 1 < starts.length ? starts[i + 1].index : end
    packages[start.name] = stop - start.index
  })
  return { prelude: starts[0].index, packages, startup: code.length - end }
}

function gopherjsBuild(pkg, output, minified) {
  const args = ['build', ...(minified ? ['-m'] : []), '--source_map=false', '-o', output, pkg]
  run(gopherjs, args)
  return readFile(output, 'utf8')
}

if (!existsSync(gopherjs)) {
  throw new Error(`GopherJS not found at ${gopherjs}; run experiments/tools/setup-go-comparators.sh`)
}
const gopherjsVersion = run(gopherjs, ['version']).trim()
if (gopherjsVersion !== `GopherJS ${GOPHERJS_VERSION}+${GO_VERSION}`) {
  throw new Error(`expected GopherJS ${GOPHERJS_VERSION}+${GO_VERSION}, got ${gopherjsVersion}`)
}

await rm(work, { recursive: true, force: true })
await mkdir(work, { recursive: true })

const readable = await gopherjsBuild('./src', resolve(work, 'fp.js'), false)
const compact = await gopherjsBuild('./src', resolve(work, 'fp.min.js'), true)
const floor = await gopherjsBuild('./tools/runtime-floor', resolve(work, 'runtime-floor.min.js'), true)

const banner = makeBanner(await readFile(resolve(root, 'LICENSE'), 'utf8'))
const esm = `${banner}\n${wrapEsm(readable)}`
const esmMin = `${banner}\n${await minify(wrapEsm(compact), true)}\n`
const iifeMin = `${banner}\n${await minify(wrapIife(compact), false)}\n`
const floorEsmMin = await minify(wrapEsm(floor), true)
const floorIifeMin = await minify(wrapIife(floor), false)

await rm(dist, { recursive: true, force: true })
await mkdir(rawDir, { recursive: true })

const attribution = {
  schema: 1,
  compiler: gopherjsVersion,
  method:
    'bytes of the `gopherjs build -m` emit before the facade and Terser, split at its $packages[...] blocks; ' +
    'the runtime floor is a one-export empty program built the same way',
  program: { emit: compact.length, ...attributePackages(compact) },
  runtimeFloor: {
    emit: floor.length,
    ...attributePackages(floor),
    esmMin: sizes(floorEsmMin),
    iifeMin: sizes(floorIifeMin),
  },
  shipped: {
    'fp.esm.js': sizes(esm),
    'fp.esm.min.js': sizes(esmMin),
    'fp.min.js': sizes(iifeMin),
  },
}

await Promise.all([
  writeFile(resolve(dist, 'fp.esm.js'), esm),
  writeFile(resolve(dist, 'fp.esm.min.js'), esmMin),
  writeFile(resolve(dist, 'fp.min.js'), iifeMin),
  copyFile(resolve(oracleRoot, 'dist/fp.d.ts'), resolve(dist, 'fp.d.ts')),
  writeFile(resolve(rawDir, 'program.js'), compact),
  writeFile(resolve(rawDir, 'runtime-floor.min.js'), floorIifeMin),
  writeFile(resolve(rawDir, 'attribution.json'), `${JSON.stringify(attribution, null, 2)}\n`),
  writeFile(resolve(dist, 'variant.json'), `${JSON.stringify({
    schema: 1,
    family: 'gopherjs',
    id: 'default',
    profile: 'production',
    compiler: gopherjsVersion,
    compilerMode: 'gopherjs build -m (fp.esm.js: gopherjs build, unminified)',
    packaging: 'generated ESM / IIFE facade over js.Module.exports, then the oracle Terser options',
    runtimeSeparable: false,
  }, null, 2)}\n`),
])

console.log(`wrote ${dist}`)
for (const [file, size] of Object.entries(attribution.shipped)) {
  console.log(`${file.padEnd(14)} ${String(size.bytes).padStart(8)} raw ${String(size.gzip9).padStart(7)} gzip-9 ${String(size.brotli11).padStart(7)} brotli-11`)
}
console.log(`runtime floor  ${String(attribution.runtimeFloor.iifeMin.bytes).padStart(8)} raw ${String(attribution.runtimeFloor.iifeMin.gzip9).padStart(7)} gzip-9 ${String(attribution.runtimeFloor.iifeMin.brotli11).padStart(7)} brotli-11 (IIFE)`)
