#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

import { artifactPaths, compareRoot, loadRegistry, parseArgs } from './ports.mjs'

const options = parseArgs()
const registry = await loadRegistry()
const requested = options.ports
  ? new Set(String(options.ports).split(',').map((value) => value.trim()).filter(Boolean))
  : null
const ports = [registry.oracle, ...registry.ports].filter((port) => {
  if (requested) return requested.has(port.id) || port.id === 'oracle'
  return port.id === 'oracle' || port.runtimeDefault
})
const rounds = String(options.rounds || 15)
const iterations = String(options.iterations || 1000)
const worker = resolve(compareRoot, 'tools/kernel-worker.mjs')
const records = []

for (const port of ports) {
  const dist = resolve(compareRoot, port.dist)
  const files = artifactPaths(dist)
  const artifact = port.id === 'oracle' ? files.esm : files.esmMin
  const result = spawnSync(process.execPath, [worker, artifact, rounds, iterations], {
    cwd: compareRoot,
    encoding: 'utf8',
  })
  if (result.status !== 0) {
    process.stderr.write(result.stderr || result.stdout)
    process.exit(result.status ?? 1)
  }
  const line = result.stdout.trim().split('\n').at(-1)
  const measurement = JSON.parse(line)
  records.push({
    id: port.id,
    label: port.label,
    profile: port.profile,
    artifact,
    ...measurement,
  })
  console.log(`${port.id}: ascii ${(measurement.cases.ascii.p50Ns / 1000).toFixed(2)} µs; unicode ${(measurement.cases.unicode.p50Ns / 1000).toFixed(2)} µs`)
}

const oracle = records.find((record) => record.id === 'oracle')
if (!oracle) throw new Error('oracle measurement missing')
for (const record of records) {
  for (const name of ['ascii', 'unicode']) {
    if (record.cases[name].hash !== oracle.cases[name].hash) {
      throw new Error(`${record.id} ${name} hash mismatch: ${record.cases[name].hash} != ${oracle.cases[name].hash}`)
    }
    record.cases[name].p50Ratio = record.cases[name].p50Ns / oracle.cases[name].p50Ns
    record.cases[name].p95Ratio = record.cases[name].p95Ns / oracle.cases[name].p95Ns
  }
  record.importRatio = record.importNs / oracle.importNs
  record.kernelRatio = Math.sqrt(record.cases.ascii.p50Ratio * record.cases.unicode.p50Ratio)
}

const candidates = records.filter((record) => record.id !== 'oracle').sort((a, b) => a.kernelRatio - b.kernelRatio)
const output = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  methodology: {
    runtime: 'Node.js isolated-process warm benchmark of exported murmurX64Hash128',
    inputs: ['ASCII', 'Unicode/UTF-16'],
    primary: 'geometric mean of ASCII and Unicode p50 ratios against the oracle',
    rounds: Number(rounds),
    iterations: Number(iterations),
    caveat: 'diagnostic hot-kernel benchmark; protected browser performance remains authoritative',
  },
  oracle,
  implementations: candidates,
}
const jsonPath = resolve(compareRoot, String(options.output || 'kernel-matrix.json'))
await mkdir(dirname(jsonPath), { recursive: true })
await writeFile(jsonPath, `${JSON.stringify(output, null, 2)}\n`)
const markdown = renderMarkdown(output)
const markdownPath = resolve(compareRoot, String(options.markdown || 'kernel-matrix.md'))
await writeFile(markdownPath, markdown)
process.stdout.write(markdown)

function renderMarkdown(data) {
  const lines = [
    '# FingerprintJS production kernel matrix',
    '',
    `Generated ${data.generatedAt}.`,
    '',
    'This is an isolated Node.js diagnostic for the exported MurmurHash3 path. It complements, but does not replace, the protected browser benchmark.',
    '',
    '| Implementation | Kernel p50 | ASCII p50 | Unicode p50 | ASCII p95 | Unicode p95 | Module import |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: |',
  ]
  for (const record of data.implementations) {
    lines.push([
      `| ${record.label}`,
      ratio(record.kernelRatio),
      ratio(record.cases.ascii.p50Ratio),
      ratio(record.cases.unicode.p50Ratio),
      ratio(record.cases.ascii.p95Ratio),
      ratio(record.cases.unicode.p95Ratio),
      ratio(record.importRatio),
      '',
    ].join(' | '))
  }
  lines.push('')
  return `${lines.join('\n')}\n`
}

function ratio(value) {
  return Number.isFinite(value) ? `${value.toFixed(2)}×` : '—'
}
