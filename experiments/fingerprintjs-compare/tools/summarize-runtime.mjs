#!/usr/bin/env node

import { existsSync } from 'node:fs'
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'

import { compareRoot, loadRegistry, parseArgs } from './ports.mjs'

const options = parseArgs()
const registry = await loadRegistry()
const matrixPath = resolve(compareRoot, String(options.matrix || 'matrix.json'))
const matrix = JSON.parse(await readFile(matrixPath, 'utf8'))
const matrixById = new Map(matrix.implementations.map((impl) => [impl.id, impl]))
const resultRoot = resolve(compareRoot, String(options.results || 'results'))
const requested = options.ports
  ? new Set(String(options.ports).split(',').map((value) => value.trim()).filter(Boolean))
  : null
const ports = registry.ports.filter((port) => requested ? requested.has(port.id) : port.runtimeDefault)

const metricSpecs = [
  ['total', (record) => record.metrics?.total],
  ['evaluate', (record) => record.metrics?.evaluate],
  ['getImmediate', (record) => record.metrics?.getImmediate],
  ['load', (record) => record.metrics?.load],
  ['hashLazy', (record) => record.metrics?.hashLazy],
  ['eventLoopP95', (record) => record.responsiveness?.eventLoopP95],
  ['maxRafGap', (record) => record.responsiveness?.maxRafGap],
  ['heapDelta', (record) => record.heapDelta],
]

const implementations = []
for (const port of ports) {
  const benchDir = resolve(resultRoot, port.id, 'bench')
  const cells = []
  if (existsSync(benchDir)) {
    const files = (await readdir(benchDir)).filter((name) => name.endsWith('.json')).sort()
    for (const file of files) {
      const data = JSON.parse(await readFile(resolve(benchDir, file), 'utf8'))
      const browser = data.browser?.name || file.replace(/\.json$/, '')
      for (const scenario of data.scenarios || ['idle']) {
        const candidate = data.records.filter((record) => record.implementation === 'boundary' && record.scenario === scenario)
        const oracle = data.records.filter((record) => record.implementation === 'upstream' && record.scenario === scenario)
        if (!candidate.length || !oracle.length) continue
        const metrics = {}
        for (const [name, read] of metricSpecs) {
          const left = candidate.map(read).filter(finite)
          const right = oracle.map(read).filter(finite)
          if (!left.length || !right.length) continue
          const p50 = percentile(left, 0.50)
          const p95 = percentile(left, 0.95)
          const oracleP50 = percentile(right, 0.50)
          const oracleP95 = percentile(right, 0.95)
          metrics[name] = {
            p50,
            p95,
            oracleP50,
            oracleP95,
            p50Ratio: oracleP50 > 0 ? p50 / oracleP50 : null,
            p95Ratio: oracleP95 > 0 ? p95 / oracleP95 : null,
          }
        }
        cells.push({ browser, scenario, metrics })
      }
    }
  }

  const aggregate = {}
  for (const [name] of metricSpecs) {
    aggregate[name] = {
      p50Ratio: geometricMean(cells.map((cell) => cell.metrics[name]?.p50Ratio).filter(positive)),
      p95Ratio: geometricMean(cells.map((cell) => cell.metrics[name]?.p95Ratio).filter(positive)),
    }
  }
  const compilerOverheadP95 = geometricMean([
    aggregate.evaluate.p95Ratio,
    aggregate.getImmediate.p95Ratio,
    aggregate.hashLazy.p95Ratio,
  ].filter(positive))
  const totalCellRatios = cells.map((cell) => cell.metrics.total?.p95Ratio).filter(positive)
  const maxTotalP95Ratio = totalCellRatios.length ? Math.max(...totalCellRatios) : null
  const primaryRuntimeRatio = aggregate.total.p95Ratio
  const runtimeGate = primaryRuntimeRatio == null || maxTotalP95Ratio == null
    ? 'missing'
    : primaryRuntimeRatio <= 1.05 && maxTotalP95Ratio <= 1.05
      ? 'pass'
      : primaryRuntimeRatio <= 1.10 && maxTotalP95Ratio <= 1.10
        ? 'watch'
        : 'fail'
  const size = matrixById.get(port.id)?.artifacts?.iifeMin || null
  const oracleSize = matrixById.get('oracle')?.artifacts?.iifeMin || null
  implementations.push({
    id: port.id,
    label: port.label,
    profile: port.profile,
    cells: cells.length,
    primaryRuntimeRatio,
    maxTotalP95Ratio,
    compilerOverheadP95,
    runtimeGate,
    aggregate,
    size: size ? {
      bytes: size.bytes,
      gzip9: size.gzip9,
      brotli11: size.brotli11,
      gzipRatio: oracleSize?.gzip9 ? size.gzip9 / oracleSize.gzip9 : null,
    } : null,
    details: cells,
  })
}

const gateOrder = { pass: 0, watch: 1, fail: 2, missing: 3 }
implementations.sort((a, b) => {
  const gate = gateOrder[a.runtimeGate] - gateOrder[b.runtimeGate]
  if (gate) return gate
  if (a.primaryRuntimeRatio == null) return b.primaryRuntimeRatio == null ? a.label.localeCompare(b.label) : 1
  if (b.primaryRuntimeRatio == null) return -1
  return a.primaryRuntimeRatio - b.primaryRuntimeRatio
    || (a.compilerOverheadP95 ?? Infinity) - (b.compilerOverheadP95 ?? Infinity)
    || a.label.localeCompare(b.label)
})

const output = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  methodology: {
    primary: 'geometric mean of candidate/oracle total p95 across available browser/scenario cells',
    gate: 'pass requires aggregate and every measured total p95 cell to stay within 5% of the oracle; watch is within 10%',
    compilerOverhead: 'geometric mean of evaluate, immediate-get, and lazy-hash p95 ratios',
    ordering: 'runtime gate, then end-to-end total p95; bundle size is secondary and never affects rank',
    candidateSlot: 'boundary',
    oracleSlot: 'upstream',
  },
  implementations,
}
const jsonPath = resolve(compareRoot, String(options.output || 'runtime-matrix.json'))
await mkdir(dirname(jsonPath), { recursive: true })
await writeFile(jsonPath, `${JSON.stringify(output, null, 2)}\n`)

const markdown = renderMarkdown(output)
const markdownPath = resolve(compareRoot, String(options.markdown || 'runtime-matrix.md'))
await mkdir(dirname(markdownPath), { recursive: true })
await writeFile(markdownPath, markdown)
process.stdout.write(markdown)

function renderMarkdown(data) {
  const lines = [
    '# FingerprintJS production runtime matrix',
    '',
    `Generated ${data.generatedAt}.`,
    '',
    'Rows are ordered by the runtime gate, then end-to-end total p95. PASS requires both aggregate and every measured total-p95 cell to remain within 5% of the oracle. Bundle size is secondary and never changes the ordering.',
    '',
    '| Implementation | Gate | Cells | Total p95 | Worst total cell | Compiler overhead p95 | Eval p95 | Immediate get p95 | Lazy hash p95 | Event-loop p95 | Heap delta p50 | IIFE gzip-9 | Gzip vs oracle |',
    '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
  ]
  for (const impl of data.implementations) {
    lines.push([
      `| ${impl.label}`,
      impl.runtimeGate.toUpperCase(),
      impl.cells,
      ratio(impl.aggregate.total.p95Ratio),
      ratio(impl.maxTotalP95Ratio),
      ratio(impl.compilerOverheadP95),
      ratio(impl.aggregate.evaluate.p95Ratio),
      ratio(impl.aggregate.getImmediate.p95Ratio),
      ratio(impl.aggregate.hashLazy.p95Ratio),
      ratio(impl.aggregate.eventLoopP95.p95Ratio),
      ratio(impl.aggregate.heapDelta.p50Ratio),
      impl.size?.gzip9 ?? '—',
      ratio(impl.size?.gzipRatio),
      '',
    ].join(' | '))
  }
  lines.push('', 'Ratios below 1.00× are faster or smaller than the oracle; ratios above 1.00× are slower or larger.', '')
  return `${lines.join('\n')}\n`
}

function percentile(values, q) {
  const sorted = [...values].sort((a, b) => a - b)
  if (sorted.length === 1) return sorted[0]
  const index = (sorted.length - 1) * q
  const lower = Math.floor(index)
  const upper = Math.ceil(index)
  const weight = index - lower
  return sorted[lower] * (1 - weight) + sorted[upper] * weight
}

function geometricMean(values) {
  if (!values.length) return null
  return Math.exp(values.reduce((sum, value) => sum + Math.log(value), 0) / values.length)
}

function finite(value) {
  return Number.isFinite(value)
}

function positive(value) {
  return Number.isFinite(value) && value > 0
}

function ratio(value) {
  return Number.isFinite(value) ? `${value.toFixed(2)}×` : '—'
}
