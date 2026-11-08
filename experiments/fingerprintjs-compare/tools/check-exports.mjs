#!/usr/bin/env node

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'

import { importEsm } from '../../fingerprintjs-rust/tools/common.mjs'
import { artifactPaths, compareRoot, parseArgs, resolvePort } from './ports.mjs'

const options = parseArgs()
const port = await resolvePort(options)
const files = artifactPaths(port.dist)
const frozen = JSON.parse(
  await readFile(resolve(compareRoot, '../fingerprintjs-rust/oracle/frozen.json'), 'utf8'),
)
const namespace = await importEsm(files.esm)
const problems = []

function same(actual, expected, label) {
  const left = JSON.stringify(actual)
  const right = JSON.stringify(expected)
  if (left !== right) problems.push(`${label}: expected ${right}, got ${left}`)
}

same(Object.keys(namespace), frozen.exports, 'named exports')
same(Object.keys(namespace.default || {}), frozen.defaultExports, 'default exports')
same(Object.keys(namespace.sources || {}), frozen.sources, 'source order')
for (const name of frozen.defaultExports) {
  if (namespace.default?.[name] !== namespace[name]) {
    problems.push(`default.${name} is not the same function as the named export`)
  }
}

if (problems.length) {
  for (const problem of problems) console.error(problem)
  process.exit(1)
}
console.log(`ESM namespace matches the v5.2.0 contract: ${files.esm}`)
