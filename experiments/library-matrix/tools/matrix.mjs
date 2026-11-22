#!/usr/bin/env node
// Renders MATRIX.md and matrix-results.json from the receipts: one row per library x language
// with build status, parity per environment, static sizes, and Node and browser p50 ratios
// against the JS reference with the A/A control beside each.
//
//   node tools/matrix.mjs [--matrix <registry>] [--receipts <dir>] [--output <dir>]
//
// Refuses (exit 1, nothing written) when any port's dist/ is not the output of its current build
// receipt. A check, size or timing receipt measured on another artifact than the current build's
// is shown as `stale` and never counted.

import { existsSync, mkdirSync, readdirSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  artifactOf, bindingDifference, checkReceiptProblem, currentBinding, guardProblem, harnessIdentity, harnessRoot, loadRegistry,
  majorVersion, parseOptions, readJson, receiptPaths, receiptsRoot, repoPath, usageError, writeJson,
} from './lib/registry.mjs'

import { compositeFreshness } from './lib/composite.mjs'

const USAGE = 'usage: node tools/matrix.mjs [--matrix <registry>] [--receipts <dir>] [--output <dir>]'
const fail = (message) => usageError('matrix.mjs', USAGE, message)
const options = parseOptions(process.argv.slice(2), { tool: 'matrix.mjs', usage: USAGE, known: ['matrix', 'receipts', 'output'] })
const registry = loadRegistry(options.matrix && options.matrix !== true ? options.matrix : undefined)
const paths = receiptPaths(receiptsRoot(options))
const outputDir = resolve(options.output && options.output !== true ? options.output : harnessRoot)

const states = registry.ports.map((port) => ({ port, state: compositeFreshness(registry, paths, port) }))
const stale = states.filter(({ state }) => state.status === 'stale')
if (stale.length) {
  process.stderr.write('matrix.mjs: refusing stale dist/ (not the output of its current build receipt):\n')
  for (const { port, state } of stale) process.stderr.write(`  ${port.key}: ${state.reasons.join('; ')}\n`)
  process.stderr.write('rebuild with tools/build.mjs; MATRIX.md and matrix-results.json were not written\n')
  process.exit(1)
}

const sizes = existsSync(paths.sizes) ? readJson(paths.sizes) : null
// The current binding of every library (corpus, reference, normalisation, output form): check and
// timing receipts measured under another binding are stale and never counted.
const bindings = {}
for (const [id, library] of Object.entries(registry.libraries)) bindings[id] = (await currentBinding(library)).binding
// Receipts are bound to the harness code that made them, and to the major version of their
// environment: Node's is this process's; a browser's must match between parity and timing.
const checkHarness = harnessIdentity('check')
const benchHarness = harnessIdentity('bench')
const nodeMajor = majorVersion({ version: process.version })
const results = { schema: 1, generatedAt: new Date().toISOString(), tool: 'experiments/library-matrix/tools/matrix.mjs', registry: { path: repoPath(registry.path), sha256: registry.sha256 }, libraries: {}, rows: [] }

function checkCell(port, environment, artifact) {
  const path = resolve(paths.checks, `${port.key}--${environment}.json`)
  if (!existsSync(path)) return { status: 'not-run' }
  const cell = readJson(path)
  const problem = checkReceiptProblem(cell, {
    artifact, binding: bindings[port.library], harness: checkHarness, environmentMajor: environment === 'node' ? nodeMajor : null, requirePass: false,
  })
  if (problem) return { status: problem.status, reason: problem.reason, receipt: repoPath(path) }
  return {
    major: majorVersion(cell.environmentIdentity), status: cell.status, equal: cell.summary?.equal ?? null, cases: cell.summary?.cases ?? null, environment: cell.environmentIdentity?.label ?? environment, receipt: repoPath(path), checkedAt: cell.checkedAt }
}

function benchCells(port, environment, artifact) {
  const dir = resolve(paths.bench, port.library)
  if (!existsSync(dir)) return []
  const prefix = `${port.key}--${environment}--`
  const cells = readdirSync(dir).filter((name) => name.startsWith(prefix) && name.endsWith('.json')).map((name) => ({ name, cell: readJson(resolve(dir, name)) }))
  const binding = bindings[port.library]
  return cells.map(({ name, cell }) => ({
    ...cell,
    receipt: repoPath(resolve(dir, name)),
    current: cell.status === 'unavailable'
      || (Boolean(artifact) && cell.artifact?.sha256 === artifact.sha256 && cell.binding?.digest === binding.digest && cell.harness?.sha256 === benchHarness.sha256),
    staleReason: cell.harness?.sha256 !== benchHarness.sha256 ? `measured by other bench code (${cell.benchProtocol ?? 'protocol 1'})`
      : cell.binding?.digest !== binding.digest ? `measured under another ${bindingDifference(cell.binding, binding)}`
        : 'measured on another artifact than the current build',
  }))
}

// Timing for one port and environment. A ratio is reported only when parity in that environment
// is a current pass: a port that fails (or has no current) parity has no timing in the report,
// whatever its bench receipts say.
function timing(port, environment, artifact, cases, parity) {
  const cells = benchCells(port, environment, artifact)
  if (!cells.length) return { status: 'not-run' }
  if (cells.every((cell) => cell.status === 'unavailable')) return { status: 'unavailable', reason: cells[0].reason }
  const current = cells.filter((cell) => cell.current)
  if (!current.length) return { status: 'stale', reason: cells[0].staleReason }
  if (parity?.status !== 'pass') return { status: 'parity-not-passed', reason: `parity in ${environment} is ${parity?.status ?? 'not checked'}` }
  const nodePath = resolve(paths.checks, `${port.key}--node.json`)
  const guard = guardProblem(existsSync(nodePath) ? readJson(nodePath) : null, { artifact, binding: bindings[port.library], harness: checkHarness })
  if (guard) return { status: 'runtime-guard', reason: guard.reason }
  const timedMajor = majorVersion(current.find((cell) => cell.environmentIdentity)?.environmentIdentity)
  if (timedMajor !== null && parity.major !== null && timedMajor !== parity.major) {
    return { status: 'stale', reason: `parity was checked in major version ${parity.major}, timing measured in ${timedMajor}` }
  }
  const perCase = {}
  for (const item of cases) {
    // One design per case: the triplet cell, or the pairs design's two cells, whichever was
    // measured last, so ratios from two runs are never mixed.
    const groups = new Map()
    for (const cell of current.filter((candidate) => candidate.case === item)) {
      const group = cell.design === 'triplet' ? 'triplet' : 'pairs'
      if (!groups.has(group)) groups.set(group, [])
      groups.get(group).push(cell)
    }
    const latest = (list) => list.map((cell) => cell.measuredAt || '').sort().at(-1)
    const forCase = [...groups.values()].sort((a, b) => (latest(a) < latest(b) ? 1 : -1))[0] ?? []
    if (!forCase.length) {
      const prior = cells.filter((candidate) => candidate.case === item)
      if (!prior.length) {
        perCase[item] = { status: 'not-run' }
      } else if (prior.every((cell) => cell.status === 'unavailable')) {
        perCase[item] = { status: 'unavailable', reason: prior[0].reason, receipts: prior.map((cell) => cell.receipt) }
      } else {
        const latestPrior = [...prior].sort((a, b) => ((a.measuredAt || '') < (b.measuredAt || '') ? 1 : -1))[0]
        perCase[item] = { status: 'stale', reason: latestPrior.staleReason, receipts: prior.map((cell) => cell.receipt) }
      }
      continue
    }
    if (forCase.every((cell) => cell.status === 'unavailable')) {
      perCase[item] = { status: 'unavailable', reason: forCase[0].reason, receipts: forCase.map((cell) => cell.receipt) }
      continue
    }
    const comparison = (name) => forCase.map((cell) => cell.result?.comparisons?.find((candidate) => candidate.name === name)).find(Boolean) ?? null
    const statuses = forCase.map((cell) => cell.status)
    const status = statuses.every((candidate) => candidate === 'ok') ? 'ok' : statuses.find((candidate) => candidate !== 'ok')
    const failure = status !== 'ok' ? forCase.find((cell) => cell.status === status) : null
    perCase[item] = {
      status,
      ...(failure?.reason ? { reason: failure.reason } : {}),
      design: [...new Set(forCase.map((cell) => cell.design))].join('+'),
      ratioP50: comparison('port/js')?.p50 ?? null,
      pairedP50: comparison('port/js')?.paired?.p50 ?? null,
      aaP50: comparison('A/A')?.p50 ?? null,
      environment: forCase[0].environmentIdentity?.label ?? environment,
      loadAverage: forCase.map((cell) => cell.loadAverage).filter(Boolean),
      receipts: forCase.map((cell) => cell.receipt),
    }
  }
  const incompleteStatuses = new Set(['not-run', 'stale', 'unavailable'])
  const caseStatuses = cases.map((item) => perCase[item]?.status)
  const status = cases.length > 0 && caseStatuses.every((itemStatus) => itemStatus === 'ok')
    ? 'ok'
    : caseStatuses.some((itemStatus) => itemStatus !== 'ok' && !incompleteStatuses.has(itemStatus))
      ? 'failed'
      : 'incomplete'
  return { status, cases: perCase }
}

for (const [libraryId, library] of Object.entries(registry.libraries)) {
  const libraryPorts = states.filter(({ port }) => port.library === libraryId)
  results.libraries[libraryId] = {
    label: library.label,
    role: library.role ?? 'library',
    npm: library.npm,
    package: library.package ?? null,
    reference: library.reference,
    contract: library.contract ?? null,
    corpus: library.corpus,
    output: library.output,
    normalisation: library.normalisation,
    environments: library.environments,
    bench: library.bench ?? null,
    jsReferenceSize: !sizes?.references?.[libraryId] ? null
      : JSON.stringify(sizes.references[libraryId].binding?.reference) !== JSON.stringify(bindings[libraryId].reference) ? { status: 'stale' }
        : { min: sizes.references[libraryId].min, method: sizes.references[libraryId].method },
  }
  for (const { port, state } of libraryPorts) {
    const artifact = artifactOf(state.receipt)
    const sizeRow = sizes?.rows?.find((row) => row.library === libraryId && row.port === port.id) ?? null
    const sizeCurrent = Boolean(sizeRow?.production && artifact && sizeRow.production.sha256 === artifact.sha256)
    const parity = Object.fromEntries(library.environments.map((environment) => [environment, checkCell(port, environment, artifact)]))
    const row = {
      library: libraryId,
      port: port.id,
      language: port.language,
      label: registry.languages[port.language].label + (sizeRow?.mostlyJs?.label ? ' [mostly JS]' : ''),
      production: registry.languages[port.language].production,
      build: {
        status: state.status === 'fresh' ? 'pass' : state.status,
        seconds: state.receipt?.seconds ?? null,
        reproducible: state.receipt?.reproducibility?.identical ?? null,
        receipt: state.receipt ? repoPath(resolve(paths.builds, `${port.key}.json`)) : null,
        artifact,
        toolchain: state.receipt?.meta?.toolchain ?? null,
      },
      parity,
      sizes: sizeRow?.production ? (sizeCurrent ? {
        min: { bytes: sizeRow.production.bytes, gzip9: sizeRow.production.gzip9, brotli11: sizeRow.production.brotli11 },
        runtimeFloor: sizeRow.runtimeFloor ? { bytes: sizeRow.runtimeFloor.bytes, gzip9: sizeRow.runtimeFloor.gzip9, brotli11: sizeRow.runtimeFloor.brotli11 } : null,
        targetLanguageLines: sizeRow.targetLanguage?.code ?? null,
        embeddedJsLines: sizeRow.embeddedJs?.lines ?? null,
        hostJsLines: sizeRow.hostJs?.nonBlank ?? null,
        evalReferences: sizeRow.embeddedJs?.evalReferences?.length ?? null,
        moduleState: sizeRow.moduleState?.count ?? null,
        mostlyJs: sizeRow.mostlyJs ?? null,
        attribution: sizeRow.attribution ?? null,
      } : { status: 'stale' }) : { status: 'not-measured' },
      timing: library.bench?.cases?.length
        ? Object.fromEntries(library.environments.map((environment) => [environment, timing(port, environment, artifact, library.bench.cases, parity[environment])]))
        : null,
    }
    results.rows.push(row)
  }
}

// ---- Markdown ----
const number = (value, digits = 2) => (typeof value === 'number' && Number.isFinite(value) ? value.toFixed(digits) : '-')
const integer = (value) => (typeof value === 'number' ? value.toLocaleString('en-US') : '-')
const parityText = (cell) => {
  if (!cell) return '-'
  if (cell.status === 'pass') return `pass ${cell.equal}/${cell.cases}`
  if (cell.status === 'fail') return `**FAIL** ${cell.equal}/${cell.cases}`
  return cell.status
}
const timingText = (entry, item) => {
  if (!entry) return '-'
  if (entry.status !== 'ok' && entry.status !== 'failed' && entry.status !== 'incomplete') return entry.status
  const value = entry.cases?.[item]
  if (!value) return '-'
  if (value.status !== 'ok') return value.status
  return `${number(value.ratioP50)} (A/A ${number(value.aaP50)})`
}

const lines = []
lines.push('# Library × compiler matrix', '')
lines.push(`Generated by \`tools/matrix.mjs\` on ${results.generatedAt} from the receipts under \`${repoPath(paths.root)}/\`. Do not edit by hand; \`matrix-results.json\` holds the same data.`, '')
lines.push('**The sizes are static sizes, not a ranking.** They are the bytes of each port\'s production artifact (`dist/index.min.js`, raw, gzip-9 and Brotli-11 by Node zlib); they say nothing about speed, and a port\'s size also reflects how it was written.', '')
lines.push('**The timing ratios are not a performance claim.** Each is the port\'s p50 per call divided by the JS reference\'s, measured in one fresh process or page with the A/A ratio (the same artifact imported twice) beside it; an A/A far from 1.00 means that run was noisy. The machine load is recorded with every cell and is not controlled.', '')
lines.push('"module state" counts module-level mutable declarations in a port\'s sources (a lint, `tools/lib/source-stats.mjs`): a port must not cache or memoise across calls unless the reference does, so every flagged line needs a reason; "eval refs" counts references to eval or Function. "[mostly JS]" uses the comparator rule: raw host-JS bytes over the minified artifact above one half. Ratios are of p50; the receipts\' p95 is the maximum when a cell has fewer than 20 samples (`p95IsMax`).', '')
lines.push('Parity is exact equality of canonical outputs (and of the input state after each call) against the reference on every corpus case, over a fresh, a repeated and an in-place-rewritten input object. `unavailable` is a browser whose WebDriver session could not start (never a pass); `stale` is a receipt measured on another artifact than the current build, under another corpus, reference (including every module it reaches and its package directory), normalisation, output form or bench variant, by other check or bench code (bound by digest), or in another major version of Node (or, for timing, of the browser its parity was checked in); `not-run` has no receipt; `incomplete` means at least one expected bench case has no current receipt (`not-run`, `stale`, or `unavailable`) and none failed. A timing ratio is shown only where parity in that environment is a current pass (`parity-not-passed` otherwise), the Node runtime guard saw nothing (`runtime-guard`), and the A/A p50 is inside the cell\'s bound (`aa-out-of-range`). The harness catches accidental divergence and common shortcuts; it is not a security sandbox, and source review plus the lint columns are part of every port\'s acceptance (README "Threat model").', '')

for (const [libraryId, info] of Object.entries(results.libraries)) {
  const rows = results.rows.filter((row) => row.library === libraryId)
  if (!rows.length) continue
  const library = registry.libraries[libraryId]
  lines.push(`## ${libraryId}: ${info.label}`, '')
  const packageLabel = info.npm
    ? `\`${info.npm.package}@${info.npm.version}\` (${info.npm.entry})`
    : info.package.entry
      ? `stand-in package \`${info.package.name}@${info.package.version}\` (\`${info.package.entry}\`)`
      : info.package.standIn
  const reference = `${packageLabel} through \`${info.reference}\``
  lines.push(`Reference: ${reference}. Output form: \`${info.output.form}\`${info.output.async ? ' (async)' : ''}. Normalisation: ${info.normalisation ? `\`${info.normalisation.name}\`` : 'none'}. Corpus: \`${info.corpus}\`. Contract: ${info.contract ? `\`${info.contract}\`` : '-'}.`, '')
  if (info.jsReferenceSize?.status === 'stale') lines.push('JS reference for scale: stale (the sizes receipt measured another reference; re-run `tools/sizes.mjs`).', '')
  else if (info.jsReferenceSize) lines.push(`JS reference for scale (${info.jsReferenceSize.method}): ${integer(info.jsReferenceSize.min.bytes)} bytes, ${integer(info.jsReferenceSize.min.gzip9)} gzip-9, ${integer(info.jsReferenceSize.min.brotli11)} Brotli-11.`, '')
  const benchCases = library.bench?.cases ?? []
  const header = ['Language', 'Build', ...library.environments.map((environment) => `Parity ${environment}`), 'min', 'gzip-9', 'brotli-11']
  if (info.role !== 'floor') header.push('floor gzip-9', 'lines', 'embedded JS', 'eval refs', 'host JS', 'module state')
  for (const item of benchCases) for (const environment of library.environments) header.push(`${environment} port/js (A/A)${benchCases.length > 1 ? ` ${item}` : ''}`)
  lines.push(`| ${header.join(' | ')} |`, `|${header.map((_, index) => (index < 2 + library.environments.length ? ' --- ' : ' ---: ')).join('|')}|`)
  for (const row of rows) {
    const size = row.sizes
    const cells = [
      row.label,
      row.build.status === 'pass' ? `pass${row.build.reproducible === true ? ', reproduced' : ''}` : `**${row.build.status}**`,
      ...library.environments.map((environment) => parityText(row.parity[environment])),
      size.min ? integer(size.min.bytes) : size.status,
      size.min ? integer(size.min.gzip9) : '-',
      size.min ? integer(size.min.brotli11) : '-',
    ]
    if (info.role !== 'floor') {
      cells.push(size.runtimeFloor ? integer(size.runtimeFloor.gzip9) : '-', size.min ? integer(size.targetLanguageLines) : '-', size.min ? integer(size.embeddedJsLines) : '-',
        size.min ? integer(size.evalReferences) : '-', size.min ? integer(size.hostJsLines) : '-', size.min ? integer(size.moduleState) : '-')
    }
    for (const item of benchCases) for (const environment of library.environments) cells.push(timingText(row.timing?.[environment], item))
    lines.push(`| ${cells.join(' | ')} |`)
  }
  lines.push('')
  if (info.role === 'floor') {
    lines.push('The floor library is the identity program (`run({ text }) => text`): its production artifact is what a language costs before any library code, so every other table\'s "floor gzip-9" column is this table\'s gzip-9 for the same language.', '')
  }
  const parityCounts = library.environments.map((environment) => {
    const cells = rows.map((row) => row.parity[environment])
    if (cells.every((cell) => cell.status === 'unavailable')) return `${environment} unavailable`
    return `${environment} ${cells.filter((cell) => cell.status === 'pass').length}/${cells.length}`
  })
  lines.push(`Exact parity: ${parityCounts.join(', ')}. Builds: ${rows.filter((row) => row.build.status === 'pass').length}/${rows.length} pass, ${rows.filter((row) => row.build.reproducible === true).length} reproduced byte for byte by the latest rebuild.`, '')
  const environmentsSeen = new Map()
  const seen = (label, use) => {
    if (!label) return
    if (!environmentsSeen.has(label)) environmentsSeen.set(label, new Set())
    environmentsSeen.get(label).add(use)
  }
  for (const row of rows) {
    for (const cell of Object.values(row.parity)) seen(cell.environment, 'parity')
    for (const entry of Object.values(row.timing || {})) for (const value of Object.values(entry.cases || {})) seen(value.environment, 'timing')
  }
  const unavailable = new Map()
  for (const row of rows) {
    for (const [environment, cell] of Object.entries(row.parity)) if (cell.status === 'unavailable') unavailable.set(environment, cell.reason)
    for (const [environment, entry] of Object.entries(row.timing || {})) if (entry.status === 'unavailable') unavailable.set(`timing ${environment}`, entry.reason)
  }
  if (environmentsSeen.size) lines.push(`Environments: ${[...environmentsSeen].map(([label, uses]) => `${label} (${[...uses].join(', ')})`).join('; ')}.`, '')
  for (const [environment, reason] of unavailable) lines.push(`- \`${environment}\` unavailable: ${String(reason).split('\n')[0]}`)
  if (unavailable.size) lines.push('')
  const loads = rows.flatMap((row) => Object.values(row.timing || {}).flatMap((entry) => Object.values(entry.cases || {}).flatMap((value) => value.loadAverage ?? [])))
  if (loads.length) {
    const ones = loads.flatMap((load) => [load.start[0], load.end[0]])
    lines.push(`Timing cells ran at a 1-minute load average between ${number(Math.min(...ones))} and ${number(Math.max(...ones))}${benchCases.length ? ` on bench case${benchCases.length > 1 ? 's' : ''} ${benchCases.map((item) => `\`${item}\``).join(', ')}` : ''}.`, '')
  }
}

lines.push('## Production variants and toolchains', '')
lines.push('| Language | Production variant | Toolchain (from the pilot build) |', '| --- | --- | --- |')
for (const [id, language] of Object.entries(registry.languages)) {
  const row = results.rows.find((candidate) => candidate.language === id && candidate.build.toolchain)
  const toolchain = row ? Object.entries(row.build.toolchain).map(([key, value]) => `${key}: ${String(value).replace(/\|/g, '/')}`).join('; ') : '-'
  lines.push(`| ${language.label} | ${language.production} | ${toolchain} |`)
}
lines.push('')

mkdirSync(outputDir, { recursive: true })
writeFileSync(resolve(outputDir, 'MATRIX.md'), `${lines.join('\n')}\n`)
writeJson(resolve(outputDir, 'matrix-results.json'), results)
process.stdout.write(`${results.rows.length} rows (${Object.keys(results.libraries).map((id) => `${id} ${results.rows.filter((row) => row.library === id).length}`).join(', ')}) -> ${repoPath(resolve(outputDir, 'MATRIX.md'))}, ${repoPath(resolve(outputDir, 'matrix-results.json'))}\n`)
