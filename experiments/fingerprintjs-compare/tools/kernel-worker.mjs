#!/usr/bin/env node

import { pathToFileURL } from 'node:url'

const [artifact, roundsText = '15', iterationsText = '1000'] = process.argv.slice(2)
if (!artifact) throw new Error('usage: kernel-worker.mjs <artifact> [rounds] [iterations]')
const rounds = Number(roundsText)
const iterations = Number(iterationsText)
const importStart = process.hrtime.bigint()
const module = await import(`${pathToFileURL(artifact).href}?kernel=${process.pid}-${Date.now()}`)
const importNs = Number(process.hrtime.bigint() - importStart)
if (typeof module.murmurX64Hash128 !== 'function') throw new Error(`${artifact} has no murmurX64Hash128 export`)

const cases = {
  ascii: 'abcdefghijklmnopqrstuvwxyz0123456789'.repeat(64),
  unicode: '😀Zażółć中𝌆—browser-fingerprint—'.repeat(48),
}
const output = {}
for (const [name, input] of Object.entries(cases)) {
  for (let i = 0; i < 200; i++) module.murmurX64Hash128(input, 17)
  const samples = []
  let guard = 0
  let hash = ''
  for (let round = 0; round < rounds; round++) {
    const started = process.hrtime.bigint()
    for (let i = 0; i < iterations; i++) {
      hash = module.murmurX64Hash128(input, 17)
      guard ^= hash.charCodeAt(i & 31)
    }
    const elapsed = Number(process.hrtime.bigint() - started)
    samples.push(elapsed / iterations)
  }
  output[name] = {
    hash,
    guard,
    p50Ns: percentile(samples, 0.50),
    p95Ns: percentile(samples, 0.95),
    samples,
  }
}

console.log(JSON.stringify({ artifact, rounds, iterations, importNs, cases: output }))

function percentile(values, q) {
  const sorted = [...values].sort((a, b) => a - b)
  if (sorted.length === 1) return sorted[0]
  const index = (sorted.length - 1) * q
  const lower = Math.floor(index)
  const upper = Math.ceil(index)
  const weight = index - lower
  return sorted[lower] * (1 - weight) + sorted[upper] * weight
}
