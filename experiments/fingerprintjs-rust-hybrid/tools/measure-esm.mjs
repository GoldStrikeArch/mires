#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { pathToFileURL, fileURLToPath } from 'node:url'

import { args, brotli11, gzip9, sha256, stableJson } from './common.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const oracleRoot = resolve(options.oracle || resolve(artifactRoot, '../fingerprintjs'))
const candidate = resolve(options.candidate || resolve(artifactRoot, 'dist/boundary/fp.esm.js'))
const upstream = resolve(options.upstream || resolve(oracleRoot, 'dist/fp.esm.js'))
const terserPath = resolve(options.terser || resolve(oracleRoot, 'node_modules/terser/main.js'))
const { minify } = await import(pathToFileURL(terserPath))

async function measure(path) {
  const source = await readFile(path, 'utf8')
  const result = await minify(source, {
    module: true,
    safari10: true,
    format: { comments: false },
  })
  if (!result.code) throw new Error(`Terser produced no code for ${path}`)
  const value = Buffer.from(result.code)
  return {
    sourceBytes: Buffer.byteLength(source),
    minifiedBytes: value.length,
    gzip9: gzip9(value),
    brotli11: brotli11(value),
    sha256: sha256(value),
  }
}

const packageJson = JSON.parse(await readFile(resolve(dirname(terserPath), 'package.json'), 'utf8').catch(async () => {
  return readFile(resolve(dirname(terserPath), '../package.json'), 'utf8')
}))
const report = stableJson({
  terserVersion: packageJson.version,
  upstream: await measure(upstream),
  candidate: await measure(candidate),
})
if (options.output) {
  const destination = resolve(options.output)
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, report)
}
process.stdout.write(report)
