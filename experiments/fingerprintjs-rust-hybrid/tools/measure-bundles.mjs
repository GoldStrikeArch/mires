#!/usr/bin/env node

import { mkdir, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { args, fileRecord, stableJson } from './common.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const output = {}

for (const variant of ['boundary', 'fiber']) {
  const dist = resolve(options[variant] || resolve(root, `dist/${variant}`))
  output[variant] = {
    esm: await fileRecord(resolve(dist, 'fp.esm.min.js')),
    iife: await fileRecord(resolve(dist, 'fp.min.js')),
    umd: await fileRecord(resolve(dist, 'fp.umd.min.js')),
  }
}

const destination = resolve(options.output || resolve(root, 'bench/results/bundle-sizes.json'))
await mkdir(dirname(destination), { recursive: true })
await writeFile(destination, stableJson(output))
process.stdout.write(stableJson(output))
