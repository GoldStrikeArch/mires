#!/usr/bin/env node
// Keep the historical measurement workers byte-identical. The command that starts
// them checks the transitive source lock of any selected composite applications.
import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { parseArgs } from 'node:util'
import { compositeFreshness, hasComponents } from './lib/composite.mjs'
import { harnessRoot, loadRegistry, receiptPaths, receiptsRoot, selectPorts } from './lib/registry.mjs'

const [mode, ...args] = process.argv.slice(2)
if (!['check', 'bench'].includes(mode))
  throw new Error('usage: node tools/qualify.mjs <check|bench> [worker options]')
const options = Object.fromEntries(
  ['library', 'language', 'matrix', 'receipts'].map((name) => [name, { type: 'string' }])
)
const { values } = parseArgs({ args, options, strict: false, allowPositionals: true })
const registry = loadRegistry(values.matrix)
const paths = receiptPaths(receiptsRoot(values))
const selected = selectPorts(registry, { library: values.library, language: values.language }, (message) => {
  throw new Error(message)
})
const applications = selected.filter(hasComponents)
function verify() {
  for (const app of applications) {
    const state = compositeFreshness(registry, paths, app)
    if (state.status === 'stale') throw new Error(app.key + ': ' + state.reasons.join('; '))
  }
}
verify()
const result = spawnSync(process.execPath, [resolve(harnessRoot, 'tools', mode + '.mjs'), ...args], {
  stdio: 'inherit'
})
if (result.error) throw result.error
verify()
process.exitCode = result.status ?? 1
