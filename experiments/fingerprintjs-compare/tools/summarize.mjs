#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'

import { parseArgs, compareRoot } from './ports.mjs'

const options = parseArgs()
const input = resolve(compareRoot, options.input || 'matrix.json')
const matrix = JSON.parse(await readFile(input, 'utf8'))
const selected = options.profile
  ? matrix.implementations.filter((impl) => impl.profile === String(options.profile) || impl.id === 'oracle')
  : matrix.implementations

const lines = [
  '# FingerprintJS compiler comparison matrix',
  '',
  `Generated ${matrix.generatedAt}.`,
  '',
  '| Implementation | Profile | Complete | ESM raw | ESM min | IIFE min | IIFE gzip-9 | IIFE brotli-11 | Runtime bytes |',
  '| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |',
]

for (const impl of selected) {
  const esm = impl.artifacts.esm
  const esmMin = impl.artifacts.esmMin
  const iife = impl.artifacts.iifeMin
  lines.push([
    `| ${impl.label}`,
    impl.profile || '—',
    impl.complete ? 'yes' : 'no',
    cell(esm, 'bytes'),
    cell(esmMin, 'bytes'),
    cell(iife, 'bytes'),
    cell(iife, 'gzip9'),
    cell(iife, 'brotli11'),
    impl.attribution?.runtime ?? '—',
    '',
  ].join(' | '))
}

lines.push('')
const markdown = `${lines.join('\n')}\n`
if (options.output) {
  const destination = resolve(compareRoot, String(options.output))
  await mkdir(dirname(destination), { recursive: true })
  await writeFile(destination, markdown)
}
process.stdout.write(markdown)

function cell(record, key) {
  if (!record || record[key] == null) return '—'
  return String(record[key])
}
