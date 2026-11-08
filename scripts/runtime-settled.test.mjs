import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import vm from 'node:vm'
import { minimalShim } from './minimal-shim.mjs'

function runtime(source) {
  const jobs = [], context = vm.createContext({ queueMicrotask: callback => jobs.push(callback) })
  vm.runInContext(source, context)
  return { settled: context.__rt.settled, drain: () => { while (jobs.length) jobs.shift()() } }
}

function getterContract(rt) {
  const trace = [], result = [], token = {}
  const method = new Proxy(function () {}, {
    get() { throw new Error('must not read the callable call property') },
    apply(target, receiver, args) {
      assert.equal(receiver, thenable)
      trace.push('apply')
      args[0](token)
    },
  })
  const thenable = Object.create({
    get then() {
      trace.push('get')
      assert.equal(trace.length, 1, 'then getter must run once')
      assert.equal(this, thenable)
      return method
    },
  })
  rt.settled(thenable, (value, ok) => { trace.push('callback'); result.push([value, ok]) })
  assert.deepEqual(trace, ['get', 'apply', 'callback'])
  assert.deepEqual(result, [[token, true]])
}

// The historical harness compared two sibling repositories. Exercise this standalone runtime.
for (const compiler of ['mires-v2']) {
  const source = await readFile(new URL('../runtime/shim.js', import.meta.url), 'utf8')
  test(`${compiler}: then lookup executes once with its original receiver`, () => getterContract(runtime(source)))
  test(`${compiler}: the extracted runtime preserves the getter contract`, () => {
    const extracted = minimalShim(['__rt.settled(value, callback)'], source)
    assert.deepEqual(extracted.names, ['settled'])
    getterContract(runtime(extracted.source))
  })
  test(`${compiler}: nullish/non-callable values remain deferred and errors retain identity`, () => {
    const rt = runtime(source), result = [], value = { then: 42 }
    for (const input of [null, undefined, 7, 'plain', value]) rt.settled(input, (x, ok) => result.push([x, ok]))
    assert.equal(result.length, 0)
    rt.drain()
    assert.deepEqual(result, [[null, true], [undefined, true], [7, true], ['plain', true], [value, true]])
    const token = {}
    assert.throws(() => rt.settled({ get then() { throw token } }, () => {}), error => error === token)
    assert.throws(() => rt.settled({ then() { throw token } }, () => {}), error => error === token)
    assert.throws(() => rt.settled({ then(resolve) { resolve(1) } }, () => { throw token }), error => error === token)
    rt.settled({ then(resolve, reject) { reject(token) } }, (x, ok) => result.push([x, ok]))
    assert.deepEqual(result.at(-1), [token, false])
  })
  test(`${compiler}: the original double-lookup implementation fails the contract`, () => {
    const broken = source.replace(
      'const then = value !== null && value !== undefined ? value.then : undefined;',
      'const then = value !== null && value !== undefined ? value.then : undefined; if (typeof then === "function") value.then;',
    )
    assert.notEqual(broken, source)
    assert.throws(() => getterContract(runtime(broken)), /then getter must run once/)
  })
}
