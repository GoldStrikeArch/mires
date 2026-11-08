import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import vm from 'node:vm'

const [work, shim] = process.argv.slice(2)
const reports = {}
for (const mode of ['off', 'report', 'on', 'without-pea', 'repeat', 'replace', 'replace-without-pea']) {
  const source = readFileSync(resolve(work, `${mode}.js`), 'utf8')
  const report = JSON.parse(readFileSync(resolve(work, `${mode}.js.lane.json`), 'utf8'))
  reports[mode] = report
  assert.equal(report.schema, 17)
  assert.equal(report.fallback_count, 0, `${mode}: no compatibility fallback`)
  assert.equal(report.witnesses.failed, 0, `${mode}: no failed witnesses`)
  const context = vm.createContext({ TextEncoder, TextDecoder })
  vm.runInContext(readFileSync(shim, 'utf8'), context)
  vm.runInContext(source, context)
  for (const seed of [0, 3, 17]) {
    for (const rounds of [0, 1, 2, 17]) {
      for (const early of [false, true]) {
        const expected = [seed, seed, seed + 2,
          seed + rounds * (rounds - 1) / 2, 2 * seed + 3 + rounds]
        const x = seed + 3 + rounds * (seed + 2) + rounds * (rounds - 1) / 2
        for (const throwAt of [-2, -1, 0, 3, 4]) {
          const events = []
          const observed = []
          const marker = { mode, seed, rounds, early, throwAt }
          const fields = early ? [seed, seed + 2] : [x, seed + 2 + rounds]
          const observe = pair => {
            observed.push([pair.x, pair.y])
            pair.x = 100
            if (throwAt === -2) throw marker
            return fields[0] + fields[1]
          }
          context.__rt.js_log_f64 = value => {
            events.push(value)
            if (events.length - 1 === throwAt) throw marker
          }
          if (throwAt === -1) {
            assert.equal(context.box_pea(seed, rounds, early, observe),
              (early ? x : x + seed + 2 + rounds) + fields[0] + fields[1])
            assert.deepEqual(events, expected)
          } else {
            assert.throws(() => context.box_pea(seed, rounds, early, observe), error => error === marker)
            assert.deepEqual(events, expected.slice(0, throwAt === -2 ? 3 : throwAt + 1))
          }
          assert.deepEqual(observed, throwAt === 0 ? [] : [fields])
        }
      }
    }
  }
}

for (const mode of ['on', 'without-pea', 'replace', 'replace-without-pea']) {
  const report = reports[mode]
  assert.ok(report.repr.final_emitted.applied > 0, `${mode}: a real carrier commit survives`)
  assert.ok(report.repr.decisions.some(row => row.subject.startsWith('box_pea/') && row.chosen === 'native-box'),
    `${mode}: the tested function contains a committed NativeBox`)
}
const item = mode => reports[mode].items.find(row => row.name === 'box_pea')
const before = item('without-pea')
const after = item('on')
assert.ok(after.alloc.materialize + after.alloc.copy < before.alloc.materialize + before.alloc.copy,
  'PEA must replace product construction/copy with edge materialization beside NativeBox')
assert.ok(after.alloc.copy < before.alloc.copy, 'PEA must eliminate the independent product copy')
// The final AST scalarizer already removes the same local objects in the control.
// PEA still has to improve the emitted function, and cannot add allocation sites.
assert.ok(after.js_alloc.object <= before.js_alloc.object, 'final JavaScript must not add object allocation sites')
assert.ok(after.bytes_min < before.bytes_min, 'the composed function must shrink after final local minification')
console.log(`carrier PEA: PASS (NativeBox, copy snapshots, pointee mutation, early return, drop/throw traces; ${before.bytes_min} -> ${after.bytes_min} item bytes)`)
