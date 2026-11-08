#!/usr/bin/env node

import { copyFile, mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { homedir } from 'node:os'
import { spawnSync } from 'node:child_process'
import { gzipSync, brotliCompressSync, constants as zlibConstants } from 'node:zlib'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const dist = resolve(root, 'dist')
const rawDir = resolve(dist, 'raw')
const targetDir = resolve(root, 'target')
const fastOut = resolve(targetDir, 'fp.fast.js')
const releaseOut = resolve(targetDir, 'fp.release.js')
const oracleTypes = resolve(oracleRoot, 'dist/fp.d.ts')
const rustTypes = resolve(root, '../fingerprintjs-rust/dist-types/fp.d.ts')

const coursierBin = resolve(homedir(), 'Library/Application Support/Coursier/bin')
const env = { ...process.env, PATH: `${coursierBin}:${process.env.PATH || ''}` }

function command(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { encoding: 'utf8', env, ...options })
  if (result.status !== 0) {
    if (result.stdout) process.stdout.write(result.stdout)
    if (result.stderr) process.stderr.write(result.stderr)
    throw new Error(`${cmd} ${args.join(' ')} failed (${result.status})`)
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

async function minify(code, module) {
  const { minify } = await import(pathToFileURL(resolve(oracleModules, 'terser/main.js')))
  const result = await minify(code, {
    format: { comments: false },
    safari10: true,
    module,
  })
  if (!result.code) throw new Error('Terser produced empty output')
  return result.code
}

function compressed(bytes) {
  return {
    bytes: bytes.length,
    gzip9: gzipSync(bytes, { level: 9 }).length,
    brotli11: brotliCompressSync(bytes, {
      params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
    }).length,
  }
}

async function packageVariant({ id, profile, input, code, compilerMode, makeDefault = false }) {
  const out = resolve(dist, id)
  const raw = resolve(out, 'raw')
  await rm(out, { recursive: true, force: true })
  await mkdir(raw, { recursive: true })

  const esm = `${banner}\n${code}`
  const esmMin = await minify(code, true)

  const { rollup } = await import(pathToFileURL(resolve(oracleModules, 'rollup/dist/es/rollup.js')))
  const iifeBundle = await rollup({
    input,
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
  await iifeBundle.close()
  const iifeMin = await minify(iife.output[0].code, false)

  await Promise.all([
    writeFile(resolve(out, 'fp.esm.js'), esm),
    writeFile(resolve(out, 'fp.esm.min.js'), `${banner}\n${esmMin}\n`),
    writeFile(resolve(out, 'fp.min.js'), `${banner}\n${iifeMin}\n`),
    writeFile(resolve(raw, 'program.js'), code),
    writeFile(resolve(out, 'variant.json'), `${JSON.stringify({
      schema: 1,
      family: 'scalajs',
      id,
      profile,
      compiler: 'Scala.js 1.22.0',
      compilerMode,
      packaging: 'native ESM exports plus common Terser pass',
      runtimeSeparable: false,
    }, null, 2)}\n`),
    writeFile(resolve(out, 'attribution.json'), `${JSON.stringify({
      program: Buffer.byteLength(code),
      runtime: null,
      runtimeSeparable: false,
      facade: 0,
      source: 'Scala.js linked whole-program output',
    }, null, 2)}\n`),
  ])
  await copyFile(dts, resolve(out, 'fp.d.ts'))

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

  return {
    program: compressed(Buffer.from(code)),
    esm: compressed(Buffer.from(esm)),
    esmMin: compressed(Buffer.from(`${banner}\n${esmMin}\n`)),
    iifeMin: compressed(Buffer.from(`${banner}\n${iifeMin}\n`)),
  }
}

if (!existsSync(resolve(oracleModules, 'terser/main.js'))) {
  throw new Error(`oracle Terser missing: ${resolve(oracleModules, 'terser/main.js')}`)
}

await mkdir(targetDir, { recursive: true })
await mkdir(rawDir, { recursive: true })
await mkdir(dist, { recursive: true })

console.log('scala-cli package (fastLink / readable)')
command(
  'scala-cli',
  ['--power', 'package', root, '--js', '--js-module-kind', 'es', '--js-mode', 'fast', '-f', '-o', fastOut],
  { cwd: root, stdio: 'inherit' },
)
console.log('scala-cli package (fullLink / production)')
command(
  'scala-cli',
  ['--power', 'package', root, '--js', '--js-module-kind', 'es', '--js-mode', 'release', '-f', '-o', releaseOut],
  { cwd: root, stdio: 'inherit' },
)

if (!existsSync(fastOut)) throw new Error(`fast emit missing: ${fastOut}`)
if (!existsSync(releaseOut)) throw new Error(`release emit missing: ${releaseOut}`)

const license = await readFile(resolve(root, 'LICENSE'), 'utf8')
const banner = makeBanner(license)
const fastCode = await readFile(fastOut, 'utf8')
const releaseCode = await readFile(releaseOut, 'utf8')
const dts = existsSync(oracleTypes) ? oracleTypes : rustTypes
if (!existsSync(dts)) throw new Error('oracle fp.d.ts not found')

await Promise.all([
  writeFile(resolve(rawDir, 'fp.fast.js'), fastCode),
  writeFile(resolve(rawDir, 'fp.release.js'), releaseCode),
])

const variants = {
  fast: await packageVariant({
    id: 'fast',
    profile: 'readable',
    input: fastOut,
    code: fastCode,
    compilerMode: 'fastLinkJS',
    makeDefault: true,
  }),
  full: await packageVariant({
    id: 'full',
    profile: 'production',
    input: releaseOut,
    code: releaseCode,
    compilerMode: 'fullLinkJS/release',
  }),
}

await writeFile(resolve(rawDir, 'sizes.json'), `${JSON.stringify(variants, null, 2)}\n`)
console.log(`wrote ${dist}/fast and ${dist}/full; root dist remains the fastLink alias`)
console.log(`scala-cli fastLink ${(await stat(fastOut)).size}  fullLink/release ${(await stat(releaseOut)).size}`)
