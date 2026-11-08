#!/usr/bin/env node

import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

import { compareRoot, loadRegistry, parseArgs } from './ports.mjs'

const options = parseArgs()
const registry = await loadRegistry()
const requested = options.ports
  ? new Set(String(options.ports).split(',').map((value) => value.trim()).filter(Boolean))
  : null
const ports = registry.ports.filter((port) => {
  if (requested) return requested.has(port.id)
  return port.runtimeDefault
})
if (!ports.length) throw new Error('no runtime candidates selected')

const smoke = options.smoke === true || options.smoke === 'true'
const browsers = String(options.browsers || options.browser || (smoke ? 'chrome' : 'chrome,firefox,safari'))
const scenarios = String(options.scenarios || (smoke ? 'idle' : 'idle,cpu,dom'))
const runs = String(options.runs || (smoke ? 5 : 30))
const warmups = String(options.warmups || (smoke ? 1 : 5))
const runner = resolve(compareRoot, 'tools/run-bench.mjs')

for (const port of ports) {
  const expected = resolve(compareRoot, 'results', port.id, 'bench')
  if (options['skip-existing'] && existsSync(expected)) {
    console.log(`skip ${port.id}: ${expected} already exists`)
    continue
  }
  console.log(`\n=== runtime bench: ${port.label} (${port.id}) ===`)
  const args = [
    runner,
    '--port', port.id,
    '--minified', 'true',
    '--browsers', browsers,
    '--scenarios', scenarios,
    '--runs', runs,
    '--warmups', warmups,
  ]
  const result = spawnSync(process.execPath, args, { cwd: compareRoot, stdio: 'inherit' })
  if (result.status !== 0) process.exit(result.status ?? 1)
}

console.log(`\nruntime matrix complete for ${ports.map((port) => port.id).join(', ')}`)
