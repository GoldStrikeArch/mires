import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { args } from '../../fingerprintjs-rust/tools/common.mjs'

const here = dirname(fileURLToPath(import.meta.url))
export const compareRoot = resolve(here, '..')
export const rustRoot = resolve(compareRoot, '../fingerprintjs-rust')
export const oracleRoot = resolve(compareRoot, '../fingerprintjs')

export function parseArgs(argv) {
  return args(argv)
}

export async function loadRegistry() {
  return JSON.parse(await readFile(resolve(compareRoot, 'ports.json'), 'utf8'))
}

export async function resolvePort(options) {
  const registry = await loadRegistry()
  const all = [registry.oracle, ...registry.ports]
  if (options.dir) {
    const dir = resolve(compareRoot, String(options.dir))
    const match = all.find((port) => resolve(compareRoot, port.dir) === dir)
    return {
      id: match?.id || 'custom',
      label: match?.label || dir,
      dir,
      dist: resolve(dir, 'dist'),
      kind: match?.kind || 'custom',
      frozen: false,
    }
  }
  const id = String(options.port || options._[0] || '')
  if (!id) throw new Error('pass --port <id> or --dir <path>')
  const match = all.find((port) => port.id === id)
  if (!match) throw new Error(`unknown port ${id}; expected one of ${all.map((port) => port.id).join(', ')}`)
  return {
    ...match,
    dir: resolve(compareRoot, match.dir),
    dist: resolve(compareRoot, match.dist),
  }
}

export function artifactPaths(dist) {
  return {
    esm: resolve(dist, 'fp.esm.js'),
    esmMin: resolve(dist, 'fp.esm.min.js'),
    iifeMin: resolve(dist, 'fp.min.js'),
    types: resolve(dist, 'fp.d.ts'),
    attribution: resolve(dist, 'attribution.json'),
    variant: resolve(dist, 'variant.json'),
    rawProgram: resolve(dist, 'raw/program.js'),
    rawShim: resolve(dist, 'raw/shim.js'),
    rawFacade: resolve(dist, 'raw/facade.js'),
  }
}
