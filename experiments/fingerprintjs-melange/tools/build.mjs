#!/usr/bin/env node

import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const distRoot = resolve(root, 'dist')
const emitEntry = resolve(root, '_build/default/src/fp/src/fingerprint.js')

function command(file, args, options = {}) {
  const result = spawnSync(file, args, {
    encoding: 'utf8',
    ...options,
  })
  if (result.status !== 0) {
    if (result.stdout) process.stdout.write(result.stdout)
    if (result.stderr) process.stderr.write(result.stderr)
    throw new Error(`${file} ${args.join(' ')} failed (${result.status})`)
  }
  return result.stdout
}

function opamEnv() {
  const output = command('opam', [
    'env', '--root', process.env.MIRES_OPAM_ROOT || resolve(root, '../../.toolchains/opam'),
    '--switch', process.env.MIRES_OPAM_SWITCH || 'mires-comparators', '--set-switch', '--shell=sh',
  ])
  const env = { ...process.env, DUNE_JOBS: process.env.DUNE_JOBS || '4' }
  for (const line of output.split('\n')) {
    const match = line.match(/^([A-Za-z_][A-Za-z0-9_]*)='(.*)'/)
    if (match) env[match[1]] = match[2]
  }
  return env
}

function makeBanner(licenseText) {
  const body = licenseText.trim().split('\n').map((line) => ` * ${line}`).join('\n')
  return [
    '/**',
    ' * FingerprintJS 5.2.0 compatibility experiment compiled from Melange 7.0.1',
    ' * Unpublished research artifact; not an official FingerprintJS release.',
    ' *',
    ' * Licensed under MIT License',
    ' *',
    body,
    ' */',
  ].join('\n')
}

const env = opamEnv()
for (const bin of ['dune', 'melc']) {
  const check = spawnSync('which', [bin], { encoding: 'utf8', env })
  if (check.status !== 0) {
    throw new Error(`${bin} is missing; wait for the fingerprintjs-compare switch to finish installing`)
  }
}

command('dune', ['build', '@default'], { cwd: root, env, stdio: 'inherit' })
if (!existsSync(emitEntry)) {
  throw new Error(`melange emit missing: ${emitEntry}`)
}

const { rollup } = await import(pathToFileURL(resolve(oracleModules, 'rollup/dist/es/rollup.js')))
const { minify } = await import(pathToFileURL(resolve(oracleModules, 'terser/main.js')))

const license = await readFile(resolve(root, 'LICENSE'), 'utf8')
const banner = makeBanner(license)

await mkdir(distRoot, { recursive: true })

const bundle = await rollup({
  input: emitEntry,
  treeshake: true,
})

const esm = await bundle.generate({
  format: 'es',
  exports: 'named',
  banner,
})
const esmCode = esm.output.find((output) => output.type === 'chunk')?.code
if (!esmCode) throw new Error('Rollup produced no ESM chunk')
await writeFile(resolve(distRoot, 'fp.esm.js'), esmCode)

const iife = await bundle.generate({
  format: 'iife',
  name: 'FingerprintJS',
  exports: 'named',
  banner,
})
const iifeCode = iife.output.find((output) => output.type === 'chunk')?.code
if (!iifeCode) throw new Error('Rollup produced no IIFE chunk')
await bundle.close()

const esmMin = await minify(esmCode, {
  module: true,
  format: { comments: false },
  safari10: true,
})
if (!esmMin.code) throw new Error('Terser produced no ESM output')
await writeFile(resolve(distRoot, 'fp.esm.min.js'), `${banner}\n${esmMin.code}\n`)

const iifeMin = await minify(iifeCode, {
  module: false,
  format: { comments: false },
  safari10: true,
})
if (!iifeMin.code) throw new Error('Terser produced no IIFE output')
await writeFile(resolve(distRoot, 'fp.min.js'), `${banner}\n${iifeMin.code}\n`)

await copyFile(resolve(oracleRoot, 'dist/fp.d.ts'), resolve(distRoot, 'fp.d.ts'))

const imports = [...esmCode.matchAll(/from\s+['"]([^'"]+)['"]/g)].map((match) => match[1])
if (imports.length) {
  process.stdout.write(`melange runtime imports in packed ESM: ${imports.join(', ')}\n`)
} else {
  process.stdout.write('packed ESM is import-free\n')
}
process.stdout.write(`fingerprintjs-melange: built ${distRoot}\n`)
