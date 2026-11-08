#!/usr/bin/env node

import { readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { args, assertEqual, command, fileRecord, importEsm, sha256, stableJson } from './common.mjs'

const EXPECTED_COMMIT = 'e196578ba35362fdf15647e013d66ac28b3c9fb5'
const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const oracleRoot = resolve(options.oracle || resolve(artifactRoot, '../fingerprintjs'))
const output = resolve(options.output || resolve(artifactRoot, 'oracle/frozen.json'))

const tagCommit = command('git', ['-C', oracleRoot, 'rev-parse', 'v5.2.0^{commit}']).trim()
assertEqual(tagCommit, EXPECTED_COMMIT, 'FingerprintJS v5.2.0 commit')

const gitShow = (path) => command('git', ['-C', oracleRoot, 'show', `v5.2.0:${path}`])
const packageJsonSource = gitShow('package.json')
const packageJson = JSON.parse(packageJsonSource)
assertEqual(packageJson.version, '5.2.0', 'oracle package version')

const distRoot = resolve(oracleRoot, 'dist')
const esm = await importEsm(resolve(distRoot, 'fp.esm.js'))
const files = {}
for (const name of [
  'fp.cjs.js',
  'fp.d.ts',
  'fp.esm.js',
  'fp.js',
  'fp.min.js',
  'fp.umd.js',
  'fp.umd.min.js',
]) {
  files[name] = await fileRecord(resolve(distRoot, name))
}

const frozen = {
  schema: 1,
  oracle: {
    package: '@fingerprintjs/fingerprintjs',
    version: packageJson.version,
    tag: 'v5.2.0',
    commit: tagCommit,
    checkoutCommit: command('git', ['-C', oracleRoot, 'rev-parse', 'HEAD']).trim(),
  },
  sourceChecksums: {
    packageJson: sha256(packageJsonSource),
    publicIndex: sha256(gitShow('src/index.ts')),
    sourceIndex: sha256(gitShow('src/sources/index.ts')),
    sourceTreeListing: sha256(command('git', ['-C', oracleRoot, 'ls-tree', '-r', 'v5.2.0', 'src'])),
  },
  exports: Object.keys(esm),
  defaultExports: Object.keys(esm.default),
  sources: Object.keys(esm.sources),
  files,
  gates: {
    iife: { file: 'fp.min.js', bytes: 36746, gzip9: 16178, brotli11: 14239 },
    umd: { file: 'fp.umd.min.js', bytes: 36937, gzip9: 16253, brotli11: 14371 },
  },
}

const serialized = stableJson(frozen)
if (options.stdout) process.stdout.write(serialized)
if (options.check) {
  assertEqual(JSON.parse(await readFile(output, 'utf8')), frozen, 'frozen oracle manifest')
} else if (!options.stdout) {
  await writeFile(output, serialized)
}
