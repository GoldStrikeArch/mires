import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib'
import { spawnSync } from 'node:child_process'

export function args(argv = process.argv.slice(2)) {
  const values = { _: [] }
  for (let index = 0; index < argv.length; ++index) {
    const value = argv[index]
    if (!value.startsWith('--')) {
      values._.push(value)
      continue
    }
    const [name, inline] = value.slice(2).split('=', 2)
    if (inline !== undefined) {
      values[name] = inline
    } else if (argv[index + 1] && !argv[index + 1].startsWith('--')) {
      values[name] = argv[++index]
    } else {
      values[name] = true
    }
  }
  return values
}

export function command(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, {
    encoding: options.input === undefined ? 'utf8' : undefined,
    ...options,
  })
  if (result.status !== 0) {
    const stderr = Buffer.isBuffer(result.stderr) ? result.stderr.toString() : result.stderr
    const stdout = Buffer.isBuffer(result.stdout) ? result.stdout.toString() : result.stdout
    throw new Error(`${command} ${commandArgs.join(' ')} failed: ${stderr || stdout || `exit ${result.status}`}`)
  }
  return result.stdout
}

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

export function gzip9(value) {
  const result = spawnSync('gzip', ['-9', '-n', '-c'], { input: value })
  if (result.status === 0) return result.stdout.length
  return gzipSync(value, { level: 9, mtime: 0 }).length
}

export function brotli11(value) {
  return brotliCompressSync(value, {
    params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
  }).length
}

export async function fileRecord(path) {
  const value = await readFile(path)
  return {
    bytes: value.length,
    sha256: sha256(value),
    gzip9: gzip9(value),
    brotli11: brotli11(value),
  }
}

export async function importEsm(path) {
  const source = await readFile(path, 'utf8')
  return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`)
}

export function stableJson(value) {
  return `${JSON.stringify(sortObject(value), null, 2)}\n`
}

function sortObject(value) {
  if (Array.isArray(value)) return value.map(sortObject)
  if (!value || typeof value !== 'object') return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortObject(value[key])]))
}

export function assertEqual(actual, expected, label) {
  const left = JSON.stringify(sortObject(actual))
  const right = JSON.stringify(sortObject(expected))
  if (left !== right) {
    throw new Error(`${label} differs\nexpected ${right}\nactual   ${left}`)
  }
}
