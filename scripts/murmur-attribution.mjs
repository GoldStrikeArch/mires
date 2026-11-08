#!/usr/bin/env node

// Produces the reproducible seven-item Murmur byte attribution required by Phase 6C.
//
// Usage:
//   node scripts/murmur-attribution.mjs \
//     <program.js.lane.json> <program.js> <Hashing.res.mjs> <terser>

import fs from 'node:fs'
import { spawnSync } from 'node:child_process'

const [, , sidecarPath, programPath, rescriptPath, terserPath] = process.argv
if (!sidecarPath || !programPath || !rescriptPath || !terserPath) {
  process.stderr.write(
    'usage: murmur-attribution.mjs <sidecar> <program.js> <Hashing.res.mjs> <terser>\n',
  )
  process.exit(2)
}

const report = JSON.parse(fs.readFileSync(sidecarPath, 'utf8'))
const program = fs.readFileSync(programPath, 'utf8')
const minified = spawnSync(terserPath, ['--module', '-c', '-m', '--', rescriptPath], {
  encoding: 'utf8',
})
if (minified.status !== 0) {
  process.stderr.write(minified.stderr || minified.stdout)
  process.exit(minified.status ?? 1)
}

const hashingRows = (report.items ?? [])
  .filter((row) => row.name.includes('$hashing$'))
  .sort((left, right) => left.name.localeCompare(right.name))
if (hashingRows.length !== 7) {
  throw new Error(`expected seven hashing items, found ${hashingRows.length}`)
}
if (hashingRows.some((row) => row.lane !== 'jir' || !Number.isInteger(row.bytes_min))) {
  throw new Error('every hashing item must have an exact JIR bytes_min measurement')
}

const shortName = (name) => {
  const match = name.match(/\$hashing\$([^$]+)\$h[0-9a-f]+(?:\$n\d+)?$/)
  if (!match) throw new Error(`unrecognized hashing item name: ${name}`)
  return match[1]
}

const roles = new Map([
  ['impl_0_final_mix', 'u64-arithmetic'],
  ['impl_0_multiply', 'u64-arithmetic'],
  ['impl_0_rotate_left', 'u64-arithmetic'],
  ['push_hex', 'hex-formatting'],
  ['x64hash128', 'public-wrappers'],
  ['x64hash128_bytes', 'hash-core'],
  ['x64hash128_seed', 'public-wrappers'],
])

const decisions = new Map(
  (report.physical?.decisions ?? [])
    .filter((decision) => decision.item.includes('$hashing$'))
    .map((decision) => [decision.item, decision]),
)

const extractFunction = (name) => {
  const marker = `function ${name}(`
  const start = program.indexOf(marker)
  if (start < 0) throw new Error(`program has no function ${name}`)
  const body = program.indexOf('{', start + marker.length)
  let depth = 0
  let quote = null
  let escaped = false
  let lineComment = false
  let blockComment = false
  for (let index = body; index < program.length; index += 1) {
    const char = program[index]
    const next = program[index + 1]
    if (lineComment) {
      if (char === '\n') lineComment = false
      continue
    }
    if (blockComment) {
      if (char === '*' && next === '/') {
        blockComment = false
        index += 1
      }
      continue
    }
    if (quote) {
      if (escaped) escaped = false
      else if (char === '\\') escaped = true
      else if (char === quote) quote = null
      continue
    }
    if (char === '/' && next === '/') {
      lineComment = true
      index += 1
    } else if (char === '/' && next === '*') {
      blockComment = true
      index += 1
    } else if (char === '"' || char === "'" || char === '`') {
      quote = char
    } else if (char === '{') {
      depth += 1
    } else if (char === '}') {
      depth -= 1
      if (depth === 0) return program.slice(start, index + 1)
    }
  }
  throw new Error(`unterminated function ${name}`)
}

const occurrences = (text, pattern) => text.match(pattern)?.length ?? 0
const items = hashingRows.map((row) => {
  const item = shortName(row.name)
  const role = roles.get(item)
  if (!role) throw new Error(`no attribution role for ${item}`)
  const source = extractFunction(row.name)
  const decision = decisions.get(row.name)
  return {
    item,
    role,
    bytes_min: row.bytes_min,
    final_js_allocations: row.js_alloc?.object + row.js_alloc?.array + row.js_alloc?.closure +
      row.js_alloc?.new ?? null,
    final_js_loop_allocations: row.js_alloc?.in_loop ?? null,
    jir_view_sites: (row.alloc?.sites ?? []).filter((site) => site.kind === 'view').length,
    physical: decision
      ? {
          shape: decision.shape,
          selected: decision.selected,
          reason: decision.reason,
        }
      : null,
    syntax: {
      chunks_exact_calls: occurrences(source, /chunks_exact/g),
      iterator_identifiers: new Set(source.match(/[A-Za-z_$][\w$]*/g)?.filter((name) => name.includes('iter')))
        .size,
      panic_sites: occurrences(source, /throw new Error/g),
      source_location_literals: occurrences(source, /filename:/g),
    },
  }
})

const clusterOrder = ['hash-core', 'u64-arithmetic', 'hex-formatting', 'public-wrappers']
const clusters = clusterOrder.map((role) => {
  const members = items.filter((item) => item.role === role)
  return {
    role,
    bytes_min: members.reduce((total, item) => total + item.bytes_min, 0),
    items: members.map((item) => item.item),
  }
})

const rustBytes = items.reduce((total, item) => total + item.bytes_min, 0)
const rescriptBytes = Buffer.byteLength(minified.stdout)
const targetBytes = rescriptBytes * 2
const clusterBytes = clusters.reduce((total, cluster) => total + cluster.bytes_min, 0)
if (clusterBytes !== rustBytes) throw new Error('cluster attribution does not sum to item bytes')

const core = items.find((item) => item.item === 'x64hash128_bytes')
const output = {
  schema: 1,
  compiler_report_schema: report.schema,
  input: {
    sidecar: sidecarPath,
    program: programPath,
    rescript_source: rescriptPath,
  },
  target: {
    rust_bytes: rustBytes,
    rescript_bytes: rescriptBytes,
    ratio: Number((rustBytes / rescriptBytes).toFixed(4)),
    two_x_bytes: targetBytes,
    excess_bytes: Math.max(0, rustBytes - targetBytes),
  },
  clusters,
  items,
  next_structural_target: {
    cluster: 'hash-core',
    exact_outer_bytes: core.bytes_min,
    reason:
      'The core owns every raw/slice view site, the ChunksExact construction, both tail iterator state machines, all source-location and panic residue, and 70% of the seven-item graph.',
    required_evidence:
      'Measure a hypothetical direct tail/slice-state lowering with the same compiler-link artifact economics; no sub-item byte saving is claimed before that candidate exists.',
  },
}

process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
