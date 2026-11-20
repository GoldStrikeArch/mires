#!/usr/bin/env node
// Static sizes, by the method of experiments/tools/measure-comparators.mjs: raw bytes, gzip-9
// (Node zlib, mtime 0) and Brotli-11 (Node zlib), each resource compressed on its own. Also the
// port's target-language lines, embedded-JS lines and host-JS lines, and the family's runtime
// floor as its builder recorded it (dist/meta.json). Sizes are static sizes, not a ranking.
//
//   node tools/sizes.mjs [--library <id,...|all>] [--language <id,...|all>] [--matrix <registry>] [--receipts <dir>]
//
// A port whose dist/ is not the output of its current build receipt (an artifact or a build input
// changed since) is refused: the tool exits 1 and writes nothing. A port without a successful
// build is listed as not built.

import { readFileSync } from 'node:fs'
import { extname, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { brotliCompressSync, constants, gzipSync } from 'node:zlib'

import { SYNTAX, codeLines, embeddedJs, moduleState } from './lib/source-stats.mjs'

import {
  currentBinding, harnessRoot, loadRegistry, parseOptions, receiptPaths, receiptsRoot, repoPath, repositoryRoot,
  selectPorts, sha256, usageError, walkFiles, writeJson,
} from './lib/registry.mjs'

import { componentsOf, compositeFreshness } from './lib/composite.mjs'

const USAGE = 'usage: node tools/sizes.mjs [--library <id,...|all>] [--language <id,...|all>] [--matrix <registry>] [--receipts <dir>]'
const fail = (message) => usageError('sizes.mjs', USAGE, message)
const options = parseOptions(process.argv.slice(2), { tool: 'sizes.mjs', usage: USAGE, known: ['library', 'language', 'matrix', 'receipts'] })
const registry = loadRegistry(options.matrix && options.matrix !== true ? options.matrix : undefined)
const selected = selectPorts(registry, { library: options.library ?? 'all', language: options.language ?? 'all' }, fail)
// The floor library's ports are always measured: they are every other row's runtime floor.
const floorLibraries = new Set(Object.values(registry.libraries).filter((library) => library.role === 'floor').map((library) => library.id))
const ports = [...selected, ...registry.ports.filter((port) => floorLibraries.has(port.library) && !selected.includes(port))]
const paths = receiptPaths(receiptsRoot(options))

export function measure(bytes) {
  return {
    bytes: bytes.length,
    gzip9: gzipSync(bytes, { level: 9, mtime: 0 }).length,
    brotli11: brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length,
    sha256: sha256(bytes),
  }
}

function sourceStats(port) {
  const language = registry.languages[port.language]
  const sources = [port, ...componentsOf(registry, port)]
  const hostDirs = sources.flatMap(source => (language.hostJs || []).map(dir => resolve(source.dir, dir)))
  const files = sources.flatMap(source => walkFiles(source.dir, {
    skip: (path, entry) => entry.name.startsWith('.') || (entry.isDirectory() && path === resolve(source.dir, 'dist')),
  }))
  const target = { files: 0, nonBlank: 0, code: 0 }
  const embedded = { lines: 0, constructs: [], evalReferences: [] }
  const host = { files: 0, nonBlank: 0, bytes: 0, paths: [] }
  const state = []
  for (const path of files) {
    const text = readFileSync(path, 'utf8')
    if (['.js', '.mjs', '.cjs'].includes(extname(path))) {
      // Handwritten JS: the allow-listed host directory (Elm's js/), or product JS, flagged.
      const allowed = hostDirs.some((dir) => path.startsWith(`${dir}/`))
      host.files++
      host.nonBlank += text.split('\n').filter((line) => line.trim()).length
      host.bytes += Buffer.byteLength(text)
      host.paths.push(allowed ? repoPath(path) : `${repoPath(path)} (outside the host allow-list)`)
      for (const finding of moduleState(text, 'js')) state.push({ file: repoPath(path), ...finding })
      const found = embeddedJs(text, 'js')
      for (const reference of found.evalReferences) embedded.evalReferences.push({ file: repoPath(path), ...reference })
      continue
    }
    if (!language.sources.extensions.includes(extname(path))) continue
    const counted = codeLines(text, SYNTAX[port.language])
    target.files++
    target.nonBlank += counted.nonBlank
    target.code += counted.code
    const found = embeddedJs(text, port.language)
    embedded.lines += found.lines
    for (const construct of found.constructs) embedded.constructs.push({ file: repoPath(path), ...construct })
    for (const reference of found.evalReferences) embedded.evalReferences.push({ file: repoPath(path), ...reference })
    for (const finding of moduleState(text, port.language)) state.push({ file: repoPath(path), ...finding })
  }
  return { targetLanguage: target, embeddedJs: embedded, hostJs: host, moduleState: { count: state.length, findings: state } }
}

// The JS reference, for scale: the adapter and the modules it imports, bundled by Rollup and
// minified with the common Terser options. Not a port and not a row of the ranking it is not.
async function referenceSize(library) {
  const { rollup } = await import(pathToFileURL(resolve(repositoryRoot, 'node_modules/rollup/dist/es/rollup.js')))
  const { minify } = await import(pathToFileURL(resolve(repositoryRoot, 'node_modules/terser/main.js')))
  const bundle = await rollup({ input: library.referencePath, onwarn() {} })
  const generated = await bundle.generate({ format: 'es', exports: 'named' })
  await bundle.close()
  const code = generated.output.find((output) => output.type === 'chunk').code
  const minified = (await minify(code, { format: { comments: false }, safari10: true, module: true })).code
  return {
    method: 'Rollup bundle of the reference adapter and the package modules it imports, common Terser pass (comments off, safari10, module)',
    bundle: measure(Buffer.from(code)),
    min: measure(Buffer.from(`${minified}\n`)),
  }
}

const refused = []
const rows = []
for (const port of ports) {
  const state = compositeFreshness(registry, paths, port)
  if (state.status === 'stale') {
    refused.push({ port: port.key, reasons: state.reasons })
    continue
  }
  const language = registry.languages[port.language]
  const row = { library: port.library, port: port.id, language: port.language, label: language.label, productionVariant: language.production, status: state.status, production: null }
  if (state.fresh) {
    const meta = state.receipt.meta
    const dist = resolve(port.dir, 'dist')
    row.build = { receipt: repoPath(resolve(paths.builds, `${port.key}.json`)), startedAt: state.receipt.startedAt }
    row.production = { path: repoPath(resolve(dist, 'index.min.js')), ...measure(readFileSync(resolve(dist, 'index.min.js'))) }
    try {
      row.readable = { path: repoPath(resolve(dist, 'index.js')), ...measure(readFileSync(resolve(dist, 'index.js'))) }
    } catch {
      row.readable = null
    }
    row.variant = meta.variant
    row.toolchain = meta.toolchain
    row.attribution = meta.attribution ?? null
    Object.assign(row, sourceStats(port))
    if (language.hostJs?.length) {
      // The comparator's rule (experiments/fingerprintjs-compare/CONTRACT.md, Elm): raw host-JS
      // bytes over the minified production artifact; above half, the row is "mostly JS". The
      // builder's share of rendered bytes (both unminified) is kept beside it.
      const comparatorShare = row.hostJs.bytes / row.production.bytes
      const rendered = meta.attribution?.jsHost !== undefined ? meta.attribution.jsHost / (meta.attribution.elmModule + meta.attribution.jsHost) : null
      row.mostlyJs = { label: comparatorShare > 0.5, comparatorShare, renderedShare: rendered, rule: 'raw host-JS bytes / index.min.js bytes > 0.5 (comparator); renderedShare = host / (program + host) in the unminified bundle' }
      row.label = row.mostlyJs.label ? `${language.label} [mostly JS]` : language.label
    }
  }
  rows.push(row)
}
// Runtime floor of each row: the production artifact of the floor library's port in the same
// language (the identity program: only the boundary, the runtime and the packaging remain).
for (const row of rows) {
  const floorRow = rows.find((candidate) => floorLibraries.has(candidate.library) && candidate.language === row.language && candidate.production)
  row.runtimeFloor = floorRow
    ? { library: floorRow.library, port: floorRow.port, method: 'production artifact of the identity program (libraries/floor)', ...floorRow.production }
    : null
}

if (refused.length) {
  process.stderr.write('sizes.mjs: refusing stale dist/ (not the output of its current build receipt):\n')
  for (const item of refused) process.stderr.write(`  ${item.port}: ${item.reasons.join('; ')}\n`)
  process.stderr.write('rebuild with tools/build.mjs; no sizes receipt was written\n')
  process.exit(1)
}

const references = {}
for (const id of [...new Set(ports.map((port) => port.library))]) {
  const library = registry.libraries[id]
  const { identity, binding } = await currentBinding(library)
  references[id] = { identity, binding, ...(await referenceSize(library)) }
}
// A partial run (--library / --language) updates its own rows and libraries in the shared
// receipt and keeps every other row; rows of ports no longer in the registry are dropped. Each row
// carries its own `measuredAt`, and the report checks every row against the current artifact.
const measuredAt = new Date().toISOString()
for (const row of rows) row.measuredAt = measuredAt
const keyOf = (row) => `${row.library}--${row.port}`
const measured = new Set(rows.map(keyOf))
const inRegistry = new Set(registry.ports.map((port) => port.key))
let previous = null
try {
  previous = JSON.parse(readFileSync(paths.sizes, 'utf8'))
} catch {
  previous = null
}
const keptRows = (previous?.rows ?? []).filter((row) => !measured.has(keyOf(row)) && inRegistry.has(keyOf(row)))
const keptReferences = Object.fromEntries(Object.entries(previous?.references ?? {})
  .filter(([id]) => !references[id] && registry.libraries[id]))
const receipt = {
  schema: 1,
  tool: 'experiments/library-matrix/tools/sizes.mjs',
  generatedAt: new Date().toISOString(),
  node: process.version,
  zlib: process.versions.zlib,
  brotli: process.versions.brotli,
  method: 'each resource compressed on its own with Node zlib: gzip level 9 with mtime 0, Brotli quality 11 (experiments/tools/measure-comparators.mjs)',
  statement: 'Static sizes of each port\'s production artifact, not a ranking of languages or compilers: they say nothing about speed, and a port\'s size also reflects how it was written.',
  qualifiedLanguageRanking: false,
  registry: { path: repoPath(registry.path), sha256: registry.sha256 },
  selection: { library: options.library ?? 'all', language: options.language ?? 'all' },
  merged: { keptRows: keptRows.length, keptReferences: Object.keys(keptReferences) },
  references: { ...keptReferences, ...references },
  rows: [...rows, ...keptRows].sort((left, right) => (keyOf(left) < keyOf(right) ? -1 : keyOf(left) > keyOf(right) ? 1 : 0)),
}
writeJson(paths.sizes, receipt)
const header = ['port', 'status', 'min', 'gzip-9', 'brotli-11', 'lines', 'embedded JS', 'eval refs', 'host JS', 'module state', 'floor min/gzip-9']
const lines = [header, ...rows.map((row) => [row.library + '/' + row.port, row.status, row.production?.bytes ?? '-', row.production?.gzip9 ?? '-', row.production?.brotli11 ?? '-',
  row.targetLanguage?.code ?? '-', row.embeddedJs?.lines ?? '-', row.embeddedJs?.evalReferences.length ?? '-', row.hostJs?.nonBlank ?? '-', row.moduleState?.count ?? '-', row.runtimeFloor ? `${row.runtimeFloor.bytes}/${row.runtimeFloor.gzip9}` : 'n/a'])]
for (const [id, reference] of Object.entries(references)) lines.push([`${id}/JS reference`, 'for scale', reference.min.bytes, reference.min.gzip9, reference.min.brotli11, '', '', '', '', '', ''])
const widths = header.map((_, column) => Math.max(...lines.map((line) => String(line[column]).length)))
for (const line of lines) process.stdout.write(`${line.map((value, column) => String(value).padStart(column === 0 || column === 1 ? 0 : widths[column]).padEnd(widths[column])).join('  ')}\n`)
process.stdout.write(`${rows.length} rows measured, ${keptRows.length} kept from the previous receipt -> ${repoPath(paths.sizes)}\nStatic sizes, not a ranking.\n`)
