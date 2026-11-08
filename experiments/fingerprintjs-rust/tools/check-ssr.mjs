#!/usr/bin/env node

import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { args, command } from './common.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const distRoot = resolve(options.dist || resolve(artifactRoot, 'dist/boundary'))
const poison = `
for (const name of ['window', 'document', 'navigator', 'screen', 'location', 'self']) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    get() { throw new Error('browser global touched during import: ' + name) },
  })
}
`

command(process.execPath, ['--input-type=module', '--eval', `${poison}
  const fs = await import('node:fs/promises')
  const source = await fs.readFile(process.argv[1], 'utf8')
  await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'))
`, resolve(distRoot, options.esm || 'fp.esm.js')])

command(process.execPath, ['--eval', `${poison}
  require(process.argv[1])
`, resolve(distRoot, options.cjs || 'fp.cjs.js')])

console.log(`ESM and CJS imports are SSR-safe: ${distRoot}`)
