// Each distinct URL evaluates the pinned module's feature detection once.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { performance } from 'node:perf_hooks'
import Long from '../../node_modules/long/index.js'
import { evaluate } from './reference.mjs'
import { variant } from './bench-variant.mjs'

const wasm = globalThis.WebAssembly
let fallback
try {
  globalThis.WebAssembly = undefined
  fallback = (await import('../../node_modules/long/index.js?without-wasm')).default
} finally {
  globalThis.WebAssembly = wasm
}

const corpus = JSON.parse(readFileSync(new URL('corpus.json', import.meta.url)))
const guard = corpus.cases.find(c => c.id === 'wasm-divide-guard')
assert.notEqual(evaluate(Long, guard.input), evaluate(fallback, guard.input))
const median = values => [...values].sort((a,b) => a-b)[Math.floor(values.length / 2)]
for (const { id, input } of corpus.cases.filter(c => c.id.startsWith('bench-'))) {
  const durations = [[], []]
  for (let k = 0; k < 11; k++) {
    const program = variant(input, k)
    const outputs = []
    for (const index of k % 2 ? [1, 0] : [0, 1]) {
      const start = performance.now()
      outputs[index] = evaluate([Long, fallback][index], program)
      const elapsed = performance.now() - start
      if (k >= 2) durations[index].push(elapsed)
    }
    assert.equal(outputs[0], outputs[1])
  }
  console.log(JSON.stringify({ id, node: process.version, samples: 9,
    wasmMedianMs: median(durations[0]), jsMedianMs: median(durations[1]) }))
}
