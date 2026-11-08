#!/usr/bin/env node

import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

import { artifactPaths, parseArgs, resolvePort } from './ports.mjs'

const options = parseArgs()
const port = await resolvePort(options)
const files = artifactPaths(port.dist)
const poison = `
for (const name of ['window', 'document', 'navigator', 'screen', 'location', 'self']) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    get() { throw new Error('browser global touched during import: ' + name) },
  })
}
`

const result = spawnSync(process.execPath, [
  '--input-type=module',
  '--eval',
  `${poison}
    const fs = await import('node:fs/promises')
    const source = await fs.readFile(process.argv[1], 'utf8')
    await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'))
  `,
  files.esm,
], { encoding: 'utf8' })

if (result.status !== 0) {
  process.stderr.write(result.stderr || result.stdout || `exit ${result.status}\n`)
  process.exit(result.status ?? 1)
}
console.log(`ESM import is SSR-safe: ${files.esm}`)
