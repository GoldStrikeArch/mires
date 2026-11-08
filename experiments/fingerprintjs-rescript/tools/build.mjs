#!/usr/bin/env node

import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'
import { existsSync } from 'node:fs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const dist = resolve(root, 'dist')
const compiledEntry = resolve(root, 'lib/es6/src/Index.res.mjs')
const oracleTypes = resolve(oracleRoot, 'dist/fp.d.ts')

for (const path of [
  resolve(oracleModules, 'rollup/dist/es/rollup.js'),
  resolve(oracleModules, 'terser/main.js'),
  oracleTypes,
]) {
  if (!existsSync(path)) {
    throw new Error(`required build input is missing: ${path}`)
  }
}

const compiled = spawnSync(resolve(root, '../../node_modules/.bin/rescript'), [], {
  cwd: root,
  stdio: 'inherit',
})
if (compiled.status !== 0) {
  process.exit(compiled.status ?? 1)
}

if (!existsSync(compiledEntry)) {
  throw new Error(`ReScript emit missing: ${compiledEntry}`)
}

const license = await readFile(resolve(root, 'LICENSE'), 'utf8')
const banner = makeBanner(license)

const { rollup } = await import(pathToFileURL(resolve(oracleModules, 'rollup/dist/es/rollup.js')))
const { minify } = await import(pathToFileURL(resolve(oracleModules, 'terser/main.js')))
const require = createRequire(resolve(oracleRoot, 'package.json'))
const { nodeResolve } = require('@rollup/plugin-node-resolve')

await rm(dist, { recursive: true, force: true })
await mkdir(dist, { recursive: true })

const plugins = [
  nodeResolve({
    browser: true,
    exportConditions: ['browser', 'import', 'default'],
  }),
]

const esmBundle = await rollup({
  input: compiledEntry,
  plugins,
  onwarn(warning, warn) {
    if (warning.code === 'THIS_IS_UNDEFINED') return
    warn(warning)
  },
})
const esm = await esmBundle.generate({
  format: 'esm',
  exports: 'named',
  sourcemap: false,
})
const esmCode = esm.output[0].code
await writeFile(resolve(dist, 'fp.esm.js'), `${banner}\n${esmCode}`)

const esmMin = await minify(esmCode, {
  format: { comments: false },
  safari10: true,
  module: true,
})
if (!esmMin.code) throw new Error('Terser produced empty ESM output')
await writeFile(resolve(dist, 'fp.esm.min.js'), `${banner}\n${esmMin.code}\n`)

const iifeBundle = await rollup({
  input: compiledEntry,
  plugins,
  onwarn(warning, warn) {
    if (warning.code === 'THIS_IS_UNDEFINED') return
    warn(warning)
  },
})
const iife = await iifeBundle.generate({
  format: 'iife',
  name: 'FingerprintJS',
  exports: 'named',
  sourcemap: false,
})
const iifeMin = await minify(iife.output[0].code, {
  format: { comments: false },
  safari10: true,
})
if (!iifeMin.code) throw new Error('Terser produced empty IIFE output')
await writeFile(resolve(dist, 'fp.min.js'), `${banner}\n${iifeMin.code}\n`)

// The oracle declaration is the contract; its SHA-256 must stay unchanged.
await copyFile(oracleTypes, resolve(dist, 'fp.d.ts'))

console.log('wrote dist/fp.esm.js, dist/fp.esm.min.js, dist/fp.min.js, dist/fp.d.ts')

function makeBanner(licenseText) {
  const body = licenseText
    .trim()
    .split('\n')
    .map(line => (line.length ? ` * ${line}` : ' *'))
    .join('\n')
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
