import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { SourceMap } from 'node:module'
import { createContext, runInContext } from 'node:vm'

// These expectations specify values and observable effects independently of either compiler.
// The nsz intrinsic permits either zero on a tie; this lowerer consistently selects the right.
const min = (left, right) => Number.isNaN(left) || right <= left ? right : left
const max = (left, right) => Number.isNaN(left) || right >= left ? right : left
const copysign = (left, right) => right < 0 || Object.is(right, -0) ? -Math.abs(left) : Math.abs(left)
const operations = { min, max, copysign }
for (const [name, operation] of Object.entries({ ...operations })) {
  operations[`${name}32`] = (left, right) => operation(Math.fround(left), Math.fround(right))
}
const integer = (value) => Number.isNaN(value) ? 0 : Math.min(2147483647, Math.max(-2147483648, Math.trunc(value)))
operations.cmp = (left, right) => integer(left) < integer(right) ? -1 : integer(left) === integer(right) ? 0 : 1
operations.and = (left, right) => Boolean((left !== 0) & (right !== 0))
operations.or = (left, right) => Boolean((left !== 0) | (right !== 0))
operations.repeat = (left, right) => left + right
operations.repeat_empty = () => 0
operations.discriminant = (left, right) => left < right ? 0 : left === right ? 1 : 2
operations.unsize = (left) => integer(left) + 2

const values = [-Infinity, -7, -0, 0, 3, Infinity, NaN]
let trace = []
let throwAt = 0
let calls = 0
let stacks
let runtime
const runtimeSource = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
const sentinel = { thrown: 'operand' }
const observer = new Proxy(() => {}, {
  apply(_target, receiver, [value]) {
    assert.equal(receiver, runtime)
    trace.push(['call', value])
    if (stacks) stacks.push(new Error().stack)
    if (++calls === throwAt) throw sentinel
  },
})
const runtimeObserver = {
  get(target, name) {
    if (name !== 'js_log_f64') return target[name]
    trace.push(['get'])
    return observer
  },
}
const expectedTrace = (left, right) => [['get'], ['call', left], ['get'], ['call', right]]
const assertTrace = (actual, expected) => assert.deepEqual(actual, expected)

// Prove that duplicate, skipped, and reordered effects fail even when a value could match.
const expected = expectedTrace(7, 3)
for (const wrong of [
  [...expected, ...expected.slice(2)],
  expected.slice(0, 2),
  expectedTrace(3, 7),
]) assert.throws(() => assertTrace(wrong, expected), assert.AssertionError)

const paths = process.argv.slice(2)
assert.ok(paths.length > 0, 'provide freshly compiled 106_operand_effects.js artifacts')
for (const path of paths) {
  const source = await readFile(path, 'utf8')
  let laneReport
  try {
    laneReport = JSON.parse(await readFile(`${path}.lane.json`, 'utf8'))
  } catch (error) {
    if (error.code !== 'ENOENT') throw error
  }
  if (process.env.RCGJS_EXPECT_LANE === 'jir') assert.ok(laneReport, 'missing JIR lane receipt')
  if (laneReport) {
    assert.equal(laneReport.lane, 'jir')
    assert.equal(laneReport.fallback_count, 0, 'the JIR regression must execute JIR lowering')
    for (const name of Object.keys(operations)) {
      assert.equal(laneReport.items.find(item => item.name === `observed_${name}`)?.lane, 'jir')
    }
  }
  const context = createContext({})
  runInContext(runtimeSource, context, { filename: 'runtime/shim.js' })
  runtime = new Proxy(context.__rt, runtimeObserver)
  context.__rt = runtime
  runInContext(source, context, { filename: path })
  let cases = 0
  for (const [name, oracle] of Object.entries(operations)) {
    const fn = context[`observed_${name}`]
    assert.equal(typeof fn, 'function', `missing export observed_${name}`)
    for (const left of values) for (const right of values) {
      trace = []
      calls = 0
      throwAt = 0
      const actual = fn(left, right)
      assert.ok(Object.is(actual, oracle(left, right)), `${path}: ${name}(${left}, ${right}): ${actual}`)
      assertTrace(trace, expectedTrace(left, right))
      ++cases
      for (throwAt of [1, 2]) {
        trace = []
        calls = 0
        assert.throws(() => fn(left, right), (error) => error === sentinel)
        assertTrace(trace, expectedTrace(left, right).slice(0, throwAt * 2))
        ++cases
      }
    }
  }
  const mapBytes = await readFile(`${path}.map`, 'utf8')
  const map = JSON.parse(mapBytes)
  const sourceMap = new SourceMap(map)
  const fixtureIndex = map.sources.findIndex(source => source.endsWith('/106_operand_effects.rs'))
  const fixtureSource = await readFile(new URL('../examples/core-tests/106_operand_effects.rs', import.meta.url), 'utf8')
  assert.equal(map.sourcesContent[fixtureIndex], fixtureSource, 'source map must embed the tested Rust source')
  const authoredLine = fixtureSource.split('\n').findIndex(line => line.trim() === 'print_f64(value);')
  for (const name of Object.keys(operations)) {
    throwAt = 0
    calls = 0
    stacks = []
    context[`observed_${name}`](7, 3)
    assert.equal(stacks.length, 2)
    for (const stack of stacks) {
      const origins = stack.split('\n').flatMap(frame => {
        const at = frame.indexOf(`${path}:`)
        if (at < 0) return []
        const position = frame.slice(at + path.length + 1).match(/^(\d+):(\d+)/)
        return position ? [sourceMap.findEntry(Number(position[1]) - 1, Number(position[2]) - 1)] : []
      })
      assert.ok(origins.some(origin => origin.originalSource === map.sources[fixtureIndex]
        && origin.originalLine === authoredLine), `${name}: host call lost its authored Rust origin`)
    }
    stacks = undefined
  }
  console.log(JSON.stringify({
    path,
    sha256: createHash('sha256').update(source).digest('hex'),
    cases,
    negativeControls: 3,
    lane: laneReport?.lane ?? 'baseline',
    semanticParity: true,
    sourceOrigins: true,
    sourceMapSha256: createHash('sha256').update(mapBytes).digest('hex'),
    runtimeSha256: createHash('sha256').update(runtimeSource).digest('hex'),
  }))
}
