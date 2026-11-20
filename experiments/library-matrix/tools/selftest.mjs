#!/usr/bin/env node
// Self-test of the harness's own logic, on synthetic modules (no toolchain needed): canonical
// forms, the Williams design's balance, the checker's case semantics (async contract, both-threw,
// normalisation, bytes and json forms) and the timing protocol's gate and async path.
//
//   node tools/selftest.mjs [--receipts <dir>]

import assert from 'node:assert/strict'
import { mkdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { CanonicalError, canonicalText } from './lib/canonical.mjs'
import { runCheck } from './lib/check-runner.mjs'
import { balancedOrders, runCell, williamsRows } from './lib/protocol.mjs'
import { attributeRequest, startServer } from './lib/browser.mjs'
import { componentsOf, componentSources, compositeFreshness, lockPath } from './lib/composite.mjs'
import { directoryDigest, fileRecord, moduleGraph, receiptPaths, repositoryRoot, sourceInventory, writeJson } from './lib/registry.mjs'
import { codeLines, embeddedJs, moduleState, SYNTAX } from './lib/source-stats.mjs'

let passed = 0
async function test(name, body) {
  await body()
  passed++
  process.stdout.write(`ok ${name}\n`)
}

// Scratch modules go to <receipts>/selftest (default build/library-matrix/selftest) and are removed.
const receiptsIndex = process.argv.indexOf('--receipts')
const dir = resolve(receiptsIndex !== -1 && process.argv[receiptsIndex + 1] ? resolve(process.argv[receiptsIndex + 1]) : resolve(repositoryRoot, 'build/library-matrix'), 'selftest')
rmSync(dir, { recursive: true, force: true })
mkdirSync(dir, { recursive: true })
const module = (name, source) => {
  const path = resolve(dir, `${name}.mjs`)
  writeFileSync(path, source)
  return pathToFileURL(path).href
}

await test('string form is exact, lone surrogates included', () => {
  assert.notEqual(canonicalText('\ud800', 'string'), canonicalText('�', 'string'))
  assert.equal(canonicalText('a\nb', 'string'), '"a\\nb"')
  assert.throws(() => canonicalText(1, 'string'), CanonicalError)
})

await test('bytes form requires a Uint8Array', () => {
  assert.equal(canonicalText(new Uint8Array([0, 255, 16]), 'bytes'), 'bytes:00ff10')
  assert.throws(() => canonicalText([0, 255], 'bytes'), CanonicalError)
  assert.throws(() => canonicalText(new Uint16Array([1]), 'bytes'), CanonicalError)
})

await test('json form keeps key order, -0, NaN and undefined apart, refuses non-plain objects', () => {
  assert.notEqual(canonicalText({ a: 1, b: 2 }, 'json'), canonicalText({ b: 2, a: 1 }, 'json'))
  assert.notEqual(canonicalText(-0, 'json'), canonicalText(0, 'json'))
  assert.equal(canonicalText([NaN, undefined, null, 1n, Infinity], 'json'), '[NaN,undefined,null,1n,Infinity]')
  assert.throws(() => canonicalText(new Map(), 'json'), CanonicalError)
  assert.throws(() => canonicalText({ when: new Date(0) }, 'json'), CanonicalError)
  const cycle = {}
  cycle.self = cycle
  assert.throws(() => canonicalText(cycle, 'json'), CanonicalError)
})

await test('Williams design: positions and adjacent pairs balanced for 2..6 arms', () => {
  for (let n = 2; n <= 6; n++) {
    const rows = williamsRows(n)
    for (let position = 0; position < n; position++) {
      const counts = new Array(n).fill(0)
      for (const row of rows) counts[row[position]]++
      assert.equal(new Set(counts).size, 1, `n=${n} position ${position}`)
    }
    const pairs = new Map()
    for (const row of rows) for (let index = 1; index < n; index++) pairs.set(`${row[index - 1]}>${row[index]}`, (pairs.get(`${row[index - 1]}>${row[index]}`) || 0) + 1)
    assert.equal(pairs.size, n * (n - 1), `n=${n} every ordered pair adjacent`)
    assert.equal(new Set(pairs.values()).size, 1, `n=${n} pairs equally often`)
  }
  let state = 1
  const random = () => ((state = (state * 48271) % 2147483647) / 2147483647)
  const orders = balancedOrders(3, 12, random)
  for (let arm = 0; arm < 3; arm++) assert.equal(orders.filter((order) => order[0] === arm).length, 4)
})

const reference = module('reference', 'export function run(input) { if (input.fail) throw new Error("bad input"); return input.text.toUpperCase() }')
const cases = [{ id: 'a', input: { text: 'abc' } }, { id: 'b', input: { text: 'ü😀' } }, { id: 'c', input: { text: '', fail: true } }]

await test('checker: equal, different, both-threw, port-error', async () => {
  const good = module('good', 'export function run(input) { if (input.fail) throw new TypeError("other message"); return input.text.toUpperCase() }')
  const wrong = module('wrong', 'export function run(input) { if (input.fail) return ""; return input.text.toLowerCase() }')
  const throwing = module('throwing', 'export function run(input) { throw new Error("port bug") }')
  const output = { form: 'string', async: false }
  const ok = await runCheck({ referenceUrl: reference, portUrl: good, output, cases })
  assert.deepEqual(ok.cases.map((item) => item.status), ['equal', 'equal', 'equal'])
  assert.equal(ok.cases[2].note, 'both-threw')
  const bad = await runCheck({ referenceUrl: reference, portUrl: wrong, output, cases })
  assert.deepEqual(bad.cases.map((item) => item.status), ['different', 'different', 'different'])
  assert.equal(bad.cases[0].difference.index, 1)
  const error = await runCheck({ referenceUrl: reference, portUrl: throwing, output, cases: cases.slice(0, 1) })
  assert.equal(error.cases[0].status, 'error')
})

await test('checker: a Promise from a synchronous library fails; an async library is awaited', async () => {
  const asyncPort = module('async-port', 'export async function run(input) { return input.text.toUpperCase() }')
  const asyncReference = module('async-reference', 'export async function run(input) { return input.text.toUpperCase() }')
  const sync = await runCheck({ referenceUrl: reference, portUrl: asyncPort, output: { form: 'string', async: false }, cases: cases.slice(0, 2) })
  assert.deepEqual(sync.cases.map((item) => item.status), ['different', 'different'])
  const awaited = await runCheck({ referenceUrl: asyncReference, portUrl: asyncPort, output: { form: 'string', async: true }, cases: cases.slice(0, 2) })
  assert.deepEqual(awaited.cases.map((item) => item.status), ['equal', 'equal'])
})

await test('checker: bytes and json forms, and a named normalisation', async () => {
  const bytesReference = module('bytes-reference', 'export function run(input) { return Uint8Array.from([input.n, 1, 2]) }')
  const bytesPort = module('bytes-port', 'export function run(input) { return Uint8Array.from([input.n, 9, 2]) }')
  const zeroSecond = module('normalise', 'export const name = "zero-byte-1"; export function apply(value) { const copy = Uint8Array.from(value); copy[1] = 0; return copy }')
  const bytes = [{ id: 'x', input: { n: 7 } }]
  const strict = await runCheck({ referenceUrl: bytesReference, portUrl: bytesPort, output: { form: 'bytes', async: false }, cases: bytes })
  assert.equal(strict.cases[0].status, 'different')
  const normalised = await runCheck({ referenceUrl: bytesReference, portUrl: bytesPort, normalisationUrl: zeroSecond, output: { form: 'bytes', async: false }, cases: bytes })
  assert.equal(normalised.cases[0].status, 'equal')
  const jsonReference = module('json-reference', 'export function run() { return { a: 1, b: [1, 2] } }')
  const jsonReordered = module('json-reordered', 'export function run() { return { b: [1, 2], a: 1 } }')
  const json = await runCheck({ referenceUrl: jsonReference, portUrl: jsonReordered, output: { form: 'json', async: false }, cases: bytes })
  assert.equal(json.cases[0].status, 'different')
})

await test('protocol: gate refuses a mismatch before timing; sync and async cells time', async () => {
  const wrong = module('wrong-cell', 'export function run(input) { return input.text }')
  const config = (arms, output, extra = {}) => ({
    library: 'self', port: 'p', case: 'a', design: 'triplet', arms, input: { text: 'abc' }, output,
    comparisons: [{ name: 'port/js', of: ['port-a', 'js'] }, { name: 'A/A', of: ['port-a', 'port-b'] }],
    warmups: 1, samples: 4, minSampleMs: 2, baseIterations: 1, seed: 7, aaRange: [0.001, 1000], ...extra,
  })
  const arms = (port) => [
    { key: 'port-a', label: 'A', kind: 'port', url: `${port}?instance=a` },
    { key: 'js', label: 'J', kind: 'reference', url: reference },
    { key: 'port-b', label: 'B', kind: 'port', url: `${port}?instance=b` },
  ]
  const refused = await runCell(config(arms(wrong), { form: 'string', async: false }))
  assert.equal(refused.status, 'checksum-mismatch')
  assert.ok(refused.arms.every((arm) => arm.sampleMs === undefined))
  const good = module('good-cell', 'export function run(input) { return input.text.toUpperCase() }')
  const timed = await runCell(config(arms(good), { form: 'string', async: false }))
  assert.equal(timed.status, 'ok', JSON.stringify(timed.error))
  assert.equal(timed.order.length, 4)
  assert.deepEqual(timed.comparisons.map((comparison) => comparison.name), ['port/js', 'A/A'])
  const expectedWrong = await runCell(config(arms(good), { form: 'string', async: false }, { expectedChecksum: 'sha256:00' }))
  assert.equal(expectedWrong.status, 'checksum-mismatch')
  const asyncGood = module('async-cell', 'export async function run(input) { return input.text.toUpperCase() }')
  const asyncReference = module('async-cell-reference', 'export async function run(input) { return input.text.toUpperCase() }')
  const asyncArms = [
    { key: 'port-a', label: 'A', kind: 'port', url: `${asyncGood}?instance=a` },
    { key: 'js', label: 'J', kind: 'reference', url: asyncReference },
    { key: 'port-b', label: 'B', kind: 'port', url: `${asyncGood}?instance=b` },
  ]
  const asyncCell = await runCell(config(asyncArms, { form: 'string', async: true }))
  assert.equal(asyncCell.status, 'ok', JSON.stringify(asyncCell.error))
})

await test('checker: identity memoisation and argument mutation fail the call sequence', async () => {
  const output = { form: 'string', async: false }
  const objects = [{ id: 'a', input: { text: 'abc' } }, { id: 'b', input: { text: 'xyz' } }]
  const memo = module('memo', 'let last; let cached; export function run(input) { if (input === last) return cached; last = input; cached = input.text.toUpperCase(); return cached }')
  const memoised = await runCheck({ referenceUrl: reference, portUrl: memo, output, cases: objects })
  assert.deepEqual(memoised.cases.map((item) => [item.status, item.failedStep]), [['different', 'rewritten'], ['different', 'rewritten']])
  const mutating = module('mutating', 'export function run(input) { const out = input.text.toUpperCase(); input.text = ""; return out }')
  const mutated = await runCheck({ referenceUrl: reference, portUrl: mutating, output, cases: objects })
  assert.ok(mutated.cases.every((item) => item.status === 'different' && item.failedStep === 'first'), JSON.stringify(mutated.cases))
  assert.match(mutated.cases[0].reason, /mutation/)
})

await test('protocol: fresh input per call; hot-path divergence and argument mutation are handled', async () => {
  const config = (port, extra = {}) => ({
    library: 'self', port: 'p', case: 'a', design: 'triplet', input: { text: 'abc' }, output: { form: 'string', async: false },
    arms: [
      { key: 'port-a', label: 'A', kind: 'port', url: `${port}?instance=a` },
      { key: 'js', label: 'J', kind: 'reference', url: reference },
      { key: 'port-b', label: 'B', kind: 'port', url: `${port}?instance=b` },
    ],
    comparisons: [{ name: 'port/js', of: ['port-a', 'js'] }, { name: 'A/A', of: ['port-a', 'port-b'] }],
    warmups: 1, samples: 3, minSampleMs: 2, baseIterations: 1, seed: 11, aaRange: [0.001, 1000], ...extra,
  })
  const hot = module('hot', 'let calls = 0; export function run(input) { calls++; return calls > 20 ? "" : input.text.toUpperCase() }')
  const hotCell = await runCell(config(hot))
  assert.equal(hotCell.status, 'output-mismatch', JSON.stringify(hotCell.mismatch))
  assert.equal(hotCell.comparisons, undefined)
  const memo = module('memo-cell', 'globalThis.__matrixMemoHits = 0; let last; let cached; export function run(input) { if (input === last) { globalThis.__matrixMemoHits++; return cached } last = input; cached = input.text.toUpperCase(); return cached }')
  const memoCell = await runCell(config(memo))
  assert.equal(memoCell.status, 'ok')
  assert.equal(globalThis.__matrixMemoHits, 0, 'no timed call may see an input object twice')
  const mutating = module('mutating-cell', 'export function run(input) { const out = input.text.toUpperCase(); input.text += "!"; return out }')
  const mutatingCell = await runCell(config(mutating))
  assert.equal(mutatingCell.status, 'ok', 'a port mutating its own clone cannot change any other call')
  assert.ok(mutatingCell.arms.every((arm) => arm.verifiedOutputs > 0))
})

await test('checker: a port returning a value the output form rejects is not "both threw"', async () => {
  const throwing = module('always-throws', 'export function run() { throw new Error("no") }')
  const mapPort = module('returns-map', 'export function run() { return new Map() }')
  const thrower = module('port-throws', 'export function run() { throw new TypeError("other") }')
  const objects = [{ id: 'a', input: { text: 'x' } }]
  const loophole = await runCheck({ referenceUrl: throwing, portUrl: mapPort, output: { form: 'json', async: false }, cases: objects })
  assert.equal(loophole.cases[0].status, 'different')
  assert.match(loophole.cases[0].reason, /outside the output form/)
  const both = await runCheck({ referenceUrl: throwing, portUrl: thrower, output: { form: 'json', async: false }, cases: objects })
  assert.equal(both.cases[0].status, 'equal')
  const badReference = await runCheck({ referenceUrl: mapPort, portUrl: mapPort, output: { form: 'json', async: false }, cases: objects })
  assert.equal(badReference.cases[0].status, 'reference-error')
})

await test('checker: the network guard blocks fetch during the port phase only', async () => {
  const fetching = module('fetching', 'export function run(input) { try { fetch("http://127.0.0.1:9/x").catch(() => {}) } catch {} return input.text.toUpperCase() }')
  const result = await runCheck({ referenceUrl: reference, portUrl: fetching, output: { form: 'string', async: false }, cases: cases.slice(0, 1) })
  assert.ok(result.guard.violations.includes('fetch'))
  assert.equal(typeof globalThis.fetch, 'function')
  const allowed = await runCheck({ referenceUrl: reference, portUrl: fetching, guard: { network: 'allowed' }, output: { form: 'string', async: false }, cases: cases.slice(0, 1) })
  assert.deepEqual(allowed.guard.violations, [])
})

await test('protocol: bench variants give every timed call a distinct input; picks are crypto-seeded', async () => {
  const variantModule = module('variant', 'export function variant(input, index) { return { text: input.text + " v" + index.toString(36) } }')
  const contentMemo = module('content-memo', 'globalThis.__matrixContentHits = 0; const cache = new Map(); export function run(input) { const hit = cache.get(input.text); if (hit !== undefined) { globalThis.__matrixContentHits++; return hit } const out = input.text.toUpperCase(); cache.set(input.text, out); return out }')
  const config = {
    library: 'self', port: 'p', case: 'a', design: 'triplet', input: { text: 'abc' }, output: { form: 'string', async: false },
    arms: [
      { key: 'port-a', label: 'A', kind: 'port', url: `${contentMemo}?instance=a` },
      { key: 'js', label: 'J', kind: 'reference', url: reference },
      { key: 'port-b', label: 'B', kind: 'port', url: `${contentMemo}?instance=b` },
    ],
    comparisons: [{ name: 'port/js', of: ['port-a', 'js'] }], warmups: 1, samples: 6, minSampleMs: 2, baseIterations: 1, seed: 3, aaRange: [0.001, 1000],
    variantUrl: variantModule, referenceUrl: reference,
  }
  const first = await runCell(config)
  assert.equal(first.status, 'ok', JSON.stringify(first.error ?? first.mismatch))
  // Only the untimed final determinism call repeats the base input (once per port arm); no timed
  // call shares an input value with any other call.
  assert.equal(globalThis.__matrixContentHits, 2, 'no timed call may share an input value')
  const second = await runCell(config)
  assert.notEqual(first.pickSeed, second.pickSeed)
  assert.equal(first.samplesBalanced, true)
  const wrongVariant = module('wrong-on-variants', 'export function run(input) { return input.text.includes(" v") ? "" : input.text.toUpperCase() }')
  const refused = await runCell({ ...config, arms: config.arms.map((arm) => (arm.kind === 'port' ? { ...arm, url: `${wrongVariant}?instance=${arm.key}` } : arm)) })
  assert.equal(refused.status, 'output-mismatch')
})

await test('protocol: arms take disjoint variant ranges; an implausible A/A refuses the cell', async () => {
  const variantModule = module('variant-k', 'export function variant(input, index) { return { text: input.text, k: index } }')
  const recorder = module('recorder', 'const tag = new URL(import.meta.url).searchParams.get("instance"); (globalThis.__matrixSeen ||= {})[tag] = new Set(); export function run(input) { globalThis.__matrixSeen[tag].add(input.k); return input.text.toUpperCase() }')
  const config = (extra = {}) => ({
    library: 'self', port: 'p', case: 'a', design: 'triplet', input: { text: 'abc' }, output: { form: 'string', async: false },
    arms: [
      { key: 'port-a', label: 'A', kind: 'port', url: `${recorder}?instance=a` },
      { key: 'js', label: 'J', kind: 'reference', url: reference },
      { key: 'port-b', label: 'B', kind: 'port', url: `${recorder}?instance=b` },
    ],
    comparisons: [{ name: 'port/js', of: ['port-a', 'js'] }, { name: 'A/A', of: ['port-a', 'port-b'] }],
    warmups: 1, samples: 6, minSampleMs: 2, baseIterations: 1, seed: 5, variantUrl: variantModule, referenceUrl: reference, ...extra,
  })
  const cell = await runCell(config({ aaRange: [0.001, 1000] }))
  assert.equal(cell.status, 'ok', JSON.stringify(cell.error ?? cell.mismatch))
  const seen = globalThis.__matrixSeen
  const overlap = [...seen.a].filter((k) => k !== undefined && seen.b.has(k))
  assert.deepEqual(overlap, [], 'instances a and b must never get the same k')
  const refused = await runCell(config({ aaRange: [5, 10] }))
  assert.equal(refused.status, 'aa-out-of-range')
  assert.deepEqual(refused.aaOutOfRange.range, [5, 10])
})

await test('registry: literal dynamic imports are bound, computed ones are errors', async () => {
  module('lazy-target', 'export const x = 1')
  const lazy = resolve(dir, 'lazy.mjs')
  writeFileSync(lazy, 'export async function run() { return (await import("./lazy-target.mjs")).x }')
  const graph = await moduleGraph(lazy)
  assert.deepEqual(graph.modules.map((item) => item.path.split('/').pop()), ['lazy.mjs', 'lazy-target.mjs'])
  const computed = resolve(dir, 'computed.mjs')
  writeFileSync(computed, 'const name = "./lazy-target.mjs"; export async function run() { return import(name) }')
  assert.match((await moduleGraph(computed)).errors.join(' '), /computed specifier/)
})

await test('source stats: embedded JS, eval references and module state', () => {
  const kotlin = 'fun a() = js( "1 + 1")\nfun b() = js(\"\"\"\n  var x = 1;\n  x + 1\n\"\"\")\n// js("in a comment")\nval s = "js(\\"in a string\\")"\n'
  const found = embeddedJs(kotlin, 'kotlin')
  assert.deepEqual(found.constructs.map((item) => item.lines), [1, 4])
  const rescript = '%%raw(`\nfunction f() {}\n`)\nlet g = %raw(`1`)\n'
  assert.deepEqual(embeddedJs(rescript, 'rescript').constructs.map((item) => [item.marker, item.lines]), [['%%raw', 3], ['%raw', 1]])
  assert.equal(embeddedJs('[%%mel.raw {|function f() {}|}]\n', 'melange').constructs.length, 1)
  assert.equal(embeddedJs('%%raw(`\nconst m = globalThis["ev" + "al"]("import(x)")\nconst u = import.meta.url\n`)\n', 'rescript').evalReferences.length, 2)
  assert.equal(embeddedJs('let f = Js.Unsafe.global##.eval in ()\n', 'jsoo').evalReferences.length, 1)
  assert.equal(embeddedJs('val e = js.Dynamic.global.eval("1")\n', 'scalajs').evalReferences.length, 1)
  assert.equal(embeddedJs('js.Global.Get("eval").Invoke("1")\n', 'gopherjs').evalReferences.length, 1)
  assert.equal(embeddedJs('#[js(call = "eval")]\nfn e(code: JsValue) -> JsValue;\n', 'rust').evalReferences.length, 1)
  assert.equal(moduleState('let cache = Map.make()\nlet f = x => x\n', 'rescript').length, 1)
  assert.equal(moduleState('var cache = map[string]string{}\nfunc f() { var local = 1 }\n', 'gopherjs').length, 1)
  assert.equal(moduleState('private val cache = mutableMapOf<String, String>()\nfun f() { val x = mutableMapOf<String, String>() }\n', 'kotlin').length, 1)
  assert.equal(moduleState('static mut HITS: u32 = 0;\nfn f() {}\n', 'rust').length, 1)
  assert.deepEqual(codeLines('a\n// c\n/* d\n e */\nb\n', SYNTAX.rust), { nonBlank: 5, code: 2 })
})

await test('browser server: requests are charged to their own page load, late ones included', async () => {
  assert.deepEqual(attributeRequest('/__load/7/experiments/x.mjs', undefined), { load: '7', path: '/experiments/x.mjs', via: 'path' })
  assert.deepEqual(attributeRequest('/probe.png', 'http://127.0.0.1:1/__load/3/library-matrix.html?load=3'), { load: '3', path: '/probe.png', via: 'referer' })
  assert.equal(attributeRequest('/probe.png', undefined).load, null)
  const server = await startServer([])
  try {
    // Load 1's page fires a late image request after load 2 has started: it is still load 1's.
    await fetch(`${server.origin}/__load/2/library-matrix.html?load=2`)
    await fetch(`${server.origin}/late-image-probe`, { headers: { referer: `${server.origin}/__load/1/library-matrix.html?load=1` } }).catch(() => {})
    await fetch(`${server.origin}/__load/2/experiments/innocent.mjs?load=2`).catch(() => {})
    const byLoad = (load) => server.requests.filter((request) => request.load === load).map((request) => request.path)
    assert.deepEqual(byLoad('1'), ['/late-image-probe'])
    assert.deepEqual(byLoad('2'), ['/library-matrix.html', '/experiments/innocent.mjs'])
  } finally {
    await server.close()
  }
})

await test('registry: the package-directory digest follows symbolic links', () => {
  const pack = resolve(dir, 'package')
  mkdirSync(pack, { recursive: true })
  writeFileSync(resolve(pack, 'index.mjs'), 'export const x = 1')
  writeFileSync(resolve(dir, 'options-real.json'), '{"a":1}')
  symlinkSync('../options-real.json', resolve(pack, 'options.json'))
  const before = directoryDigest(pack)
  assert.equal(before.files, 2)
  assert.deepEqual(before.links, [['options.json', '../options-real.json', 'file']])
  writeFileSync(resolve(dir, 'options-real.json'), '{"a":2}')
  assert.notEqual(directoryDigest(pack).sha256, before.sha256)
  assert.equal(embeddedJs('let f: string => unit = %raw(`(b) => WebAssembly.instantiate(b)`)\nlet u = "data:text/javascript,1"\n', 'rescript').evalReferences.length, 2)
})

await test('composite freshness binds the selected language source and the linked artifact', () => {
  const app = { library: 'app', id: 'test', language: 'test', key: 'app--test', dir: resolve(dir, 'app') }
  const dep = { library: 'dep', id: 'test', language: 'test', key: 'dep--test', dir: resolve(dir, 'dep') }
  const other = { ...dep, id: 'other', language: 'other', dir: resolve(dir, 'other') }
  const registry = {
    ports: [other, dep, app],
    languages: { test: { builderPath: resolve(dir, 'compiler/build.mjs') } }
  }
  const paths = receiptPaths(resolve(dir, 'receipts'))
  writeJson(resolve(app.dir, 'port.json'), { components: ['dep'] })
  for (const p of [dep, other])
    writeJson(resolve(p.dir, 'port.json'), { library: 'dep', language: p.language })
  writeFileSync(resolve(dep.dir, 'algorithm'), 'version 1')
  assert.deepEqual(componentsOf(registry, app), [dep])
  assert.throws(() => componentsOf({ ...registry, ports: [app, other] }, app), /missing same-language/)
  const inputs = componentSources(registry, app, [dep])
  const artifacts = { dep: { production: 'production digest', readable: 'readable digest' } }
  const toolchain = { version: '1' }
  writeJson(lockPath(app), { ...inputs, toolchain, artifacts })
  const meta = resolve(app.dir, 'dist/meta.json')
  writeJson(meta, { toolchain, componentArtifacts: artifacts })
  writeJson(resolve(paths.builds, app.key + '.json'), {
    built: true,
    artifacts: [fileRecord(meta)],
    sourceAfter: sourceInventory(registry, app)
  })
  assert.equal(compositeFreshness(registry, paths, app).fresh, true)
  writeFileSync(resolve(other.dir, 'algorithm'), 'unrelated change')
  assert.equal(compositeFreshness(registry, paths, app).fresh, true)
  writeFileSync(resolve(dep.dir, 'algorithm'), 'version 2')
  const state = compositeFreshness(registry, paths, app)
  assert.equal(state.status, 'stale')
  assert.match(state.reasons[0], /components.lock.json mismatch \(dep\)/)
})

rmSync(dir, { recursive: true, force: true })
process.stdout.write(`${passed} self-tests passed\n`)
