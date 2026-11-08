#!/usr/bin/env node

import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

import { artifactPaths, compareRoot, parseArgs, resolvePort, rustRoot } from './ports.mjs'

const options = parseArgs()
const port = await resolvePort(options)
const files = artifactPaths(port.dist)
const output = resolve(options.output || resolve(compareRoot, 'results', port.id, 'bench'))
await mkdir(output, { recursive: true })

const candidate = options.minified === true || options.minified === 'true'
  ? files.esmMin
  : files.esm
const script = resolve(rustRoot, 'bench/run.mjs')
const args = [
  script,
  '--oracle',
  resolve(compareRoot, '../fingerprintjs'),
  '--boundary',
  candidate,
  '--fiber',
  candidate,
  '--output',
  output,
  '--scenarios',
  String(options.scenarios || 'idle'),
]
if (options.minified === true || options.minified === 'true') args.push('--minified')
if (options.browsers) args.push('--browsers', String(options.browsers))
if (options.browser) args.push('--browser', String(options.browser))
if (options.runs) args.push('--runs', String(options.runs))
if (options.warmups) args.push('--warmups', String(options.warmups))

const result = spawnSync(process.execPath, args, { stdio: 'inherit' })
if (result.status !== 0) {
  process.exit(result.status ?? 1)
}
console.log(`bench written to ${output} for ${port.id} (candidate in the boundary slot)`)
