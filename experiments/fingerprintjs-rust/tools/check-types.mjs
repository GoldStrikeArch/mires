#!/usr/bin/env node

import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { args, assertEqual, command, sha256 } from './common.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const oracleRoot = resolve(options.oracle || resolve(artifactRoot, '../fingerprintjs'))
const types = resolve(options.types || resolve(artifactRoot, 'dist/boundary/fp.d.ts'))
const tsc = resolve(options.tsc || resolve(oracleRoot, 'node_modules/typescript/bin/tsc'))
const frozen = JSON.parse(await readFile(resolve(artifactRoot, 'oracle/frozen.json'), 'utf8'))

assertEqual(sha256(await readFile(types)), frozen.files['fp.d.ts'].sha256, 'TypeScript declaration checksum')
command(process.execPath, [tsc, '--isolatedModules', '--noEmit', types], { cwd: oracleRoot })
command(process.execPath, [tsc, '--project', resolve(artifactRoot, 'fixtures/types/tsconfig.json')], { cwd: oracleRoot })
console.log(`Type declarations match the v5.2.0 contract: ${types}`)
