#!/usr/bin/env node
// Refresh an application's content-addressed set of native component ports.
import { resolve } from 'node:path'
import { run } from '../languages/common.mjs'
import { compositeBuilder, hasComponents } from './lib/composite.mjs'
import {
  loadRegistry,
  parseOptions,
  receiptPaths,
  receiptsRoot,
  selectPorts,
  usageError
} from './lib/registry.mjs'

const usage =
  'usage: node tools/lock-components.mjs --library <app> [--language <id,...|all>] [--matrix <path>] [--receipts <dir>]'
const fail = (message) => usageError('lock-components.mjs', usage, message)
const options = parseOptions(process.argv.slice(2), {
  tool: 'lock-components.mjs',
  usage,
  known: ['library', 'language', 'matrix', 'receipts']
})
if (typeof options.library !== 'string') fail('--library is required')
const registry = loadRegistry(options.matrix)
const paths = receiptPaths(receiptsRoot(options))
const ports = selectPorts(registry, { library: options.library, language: options.language ?? 'all' }, fail)
for (const port of ports) {
  if (!hasComponents(port)) fail(`${port.key} has no declared components`)
  run(process.execPath, [
    compositeBuilder,
    '--port',
    port.dir,
    '--work',
    resolve(paths.work, port.library, port.id + '-lock'),
    '--matrix',
    registry.path,
    '--receipts',
    paths.root,
    '--lock-only'
  ])
}
