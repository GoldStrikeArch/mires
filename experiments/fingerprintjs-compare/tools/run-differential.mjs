#!/usr/bin/env node

import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

import { artifactPaths, compareRoot, parseArgs, resolvePort, rustRoot } from './ports.mjs'

const options = parseArgs()
const port = await resolvePort(options)
const files = artifactPaths(port.dist)
const output = resolve(
  options.output || resolve(compareRoot, 'results', port.id, 'differential'),
)
await mkdir(output, { recursive: true })

const script = resolve(rustRoot, 'tests/differential.mjs')
const args = [
  script,
  '--candidate',
  files.esm,
  '--oracle',
  resolve(compareRoot, '../fingerprintjs'),
  '--output',
  output,
]
if (options.browsers) args.push('--browsers', String(options.browsers))
if (options.browser) args.push('--browser', String(options.browser))
if (options.repetitions) args.push('--repetitions', String(options.repetitions))
if (options.headed === true) args.push('--headed')

const result = spawnSync(process.execPath, args, { stdio: 'inherit' })
if (result.status !== 0) {
  process.exit(result.status ?? 1)
}
console.log(`differential written to ${output} for ${port.id}`)
