#!/usr/bin/env node

import { createRequire } from 'node:module'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

import { args, assertEqual, fileRecord, importEsm, sha256, stableJson } from './common.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const distRoot = resolve(options.dist || resolve(artifactRoot, 'dist'))
const frozen = JSON.parse(await readFile(resolve(artifactRoot, 'oracle/frozen.json'), 'utf8'))
const files = {
  esm: resolve(distRoot, options.esm || 'fp.esm.js'),
  cjs: resolve(distRoot, options.cjs || 'fp.cjs.js'),
  iife: resolve(distRoot, options.iife || 'fp.js'),
  iifeMin: resolve(distRoot, options['iife-min'] || 'fp.min.js'),
  umd: resolve(distRoot, options.umd || 'fp.umd.js'),
  umdMin: resolve(distRoot, options['umd-min'] || 'fp.umd.min.js'),
  types: resolve(distRoot, options.types || 'fp.d.ts'),
}

function checkNamespace(namespace, label) {
  assertEqual(Object.keys(namespace), frozen.exports, `${label} exports`)
  assertEqual(Object.keys(namespace.default), frozen.defaultExports, `${label} default exports`)
  assertEqual(Object.keys(namespace.sources), frozen.sources, `${label} source order`)
  for (const name of frozen.defaultExports) {
    if (namespace.default[name] !== namespace[name]) throw new Error(`${label} default.${name} identity differs`)
  }
}

const esm = await importEsm(files.esm)
checkNamespace(esm, 'ESM')

checkNamespace(await loadCommonJs(files.cjs), 'CJS')

for (const [label, path] of [['IIFE', files.iife], ['UMD', files.umd]]) {
  // A fresh VM context has ECMAScript intrinsics but not the browser's Encoding API. The bundles
  // legitimately initialize their UTF-8 shim at module load, so model the web globals that both
  // supported browsers and the surrounding Node process provide.
  const sandbox = {
    TextDecoder: globalThis.TextDecoder,
    TextEncoder: globalThis.TextEncoder,
  }
  sandbox.globalThis = sandbox
  sandbox.self = sandbox
  sandbox.window = sandbox
  vm.runInNewContext(await readFile(path, 'utf8'), sandbox, { filename: path })
  checkNamespace(sandbox.FingerprintJS, label)
}

const expectedTypes = frozen.files['fp.d.ts'].sha256
const actualTypes = sha256(await readFile(files.types))
assertEqual(actualTypes, expectedTypes, 'TypeScript declaration checksum')

for (const path of [files.esm, files.cjs, files.iife, files.iifeMin, files.umd, files.umdMin, files.types]) {
  const prefix = (await readFile(path, 'utf8')).slice(0, 3000)
  if (!/FingerprintJS/i.test(prefix) || !/MIT License/i.test(prefix)) {
    throw new Error(`${path} is missing the FingerprintJS MIT attribution banner`)
  }
}

const sizes = {
  iife: await fileRecord(files.iifeMin),
  umd: await fileRecord(files.umdMin),
}
const failures = []
for (const format of ['iife', 'umd']) {
  for (const metric of ['bytes', 'gzip9', 'brotli11']) {
    const gate = frozen.gates[format][metric]
    if (sizes[format][metric] > gate) {
      failures.push({ actual: sizes[format][metric], format, gate, metric })
    }
  }
}

const report = stableJson({ distRoot, exports: frozen.exports, failures, sources: frozen.sources, sizes })
if (options.output) {
  const destination = resolve(options.output)
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, report)
}
process.stdout.write(report)
if (failures.length && options['allow-size-failure'] !== true) {
  throw new Error(`${failures.length} bundle-size gate(s) failed`)
}

async function loadCommonJs(path) {
  const source = await readFile(path, 'utf8')
  const module = { exports: {} }
  const wrapper = `(function (exports, require, module, __filename, __dirname) {\n${source}\n})`
  const factory = vm.runInThisContext(wrapper, { filename: path })
  factory(module.exports, createRequire(path), module, path, dirname(path))
  return module.exports
}
