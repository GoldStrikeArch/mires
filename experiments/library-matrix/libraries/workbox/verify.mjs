// node verify.mjs [chrome|firefox] [reference|<language>] [--capture]
//
// Runs the corpus and four bench variants through the reference adapter (or a built port's
// dist/index.min.js) in a real browser page and checks, beyond the checker's reference-versus-port
// comparison:
// - every call leaves CacheStorage as it found it (cleanup on every path, including failed steps);
// - every output is location-independent: no loopback origin and no scenario cache prefix leaks
//   into it, so one expected.json holds for every browser and server port (the page-realm
//   harness gives Chrome and Firefox the same reference output);
// - the corpus keeps its error-path cleanup cases (a step fails after the scenario opened caches);
// - the bench variant renames every bench URL per k and keeps the shape of the input.
// --capture (reference only) rewrites expected.json from this browser.
import assert from 'node:assert/strict'
import {readFileSync, writeFileSync} from 'node:fs'
import {fileURLToPath} from 'node:url'
import {openBrowser, runInPage, startServer} from '../../tools/lib/browser.mjs'

const [browser = 'chrome', language = 'reference', action] = process.argv.slice(2)
const file = path => fileURLToPath(new URL(path, import.meta.url))
const target = file(language === 'reference' ? 'reference.mjs' : '../../ports/workbox/' + language + '/dist/index.min.js')
const cases = JSON.parse(readFileSync(file('corpus.json'))).cases
const seeds = [1, 2 ** 32 + 1, 2 ** 52 + 1, Number.MAX_SAFE_INTEGER]

const served = ['capture.mjs', 'bench-variant.mjs', 'corpus.json', 'alpha.mjs', 'beta.mjs', '../../node_modules/workbox-precaching/matrix-entry.mjs'].map(file)
const server = await startServer([...served, target])
let session
try {
  session = await openBrowser(browser)
  if (session.error) throw Error(session.error)
  const url = path => server.url(path, '?load=1')
  const r = await runInPage(session.driver, server.origin, 1, url(file('capture.mjs')), 'capture', {moduleUrl: url(target), corpusUrl: url(file('corpus.json')), seeds})
  if (r.error) throw Error(r.error.message)
  assert.deepEqual(r.cspViolations, [])
  assert.deepEqual(r.leaks, [], 'a call left caches behind')

  const results = Object.fromEntries(Object.entries(r.results).map(([id, text]) => [id, JSON.parse(text)]))
  const inputs = Object.fromEntries(Object.entries(r.inputs).map(([id, text]) => [id, JSON.parse(text)]))
  let cleanupAfterError = 0
  for (const [id, out] of Object.entries(results)) {
    assert.ok(out && Array.isArray(out.results), id + ' ' + JSON.stringify(out).slice(0, 300))
    assert.equal(out.results.length, inputs[id].steps.length, id)
    const text = JSON.stringify(out)
    assert.ok(!text.includes('127.0.0.1') && !text.includes('mires-workbox-'), id + ' prints a location-dependent value')
    if (out.results.some(item => item && typeof item === 'object' && 'error' in item) && out.caches.length) cleanupAfterError++
  }
  assert.ok(cleanupAfterError >= 10, 'error-path cleanup cases: ' + cleanupAfterError)

  const bench = cases.find(c => c.id === 'bench-cache').input
  const variants = seeds.map((k, i) => inputs['bench-variant-' + i])
  for (const input of variants) {
    const text = JSON.stringify(input)
    assert.equal(input.steps.length, bench.steps.length)
    assert.ok(!text.includes('"bench/'), 'every bench URL is renamed')
    assert.equal(text.length, JSON.stringify(variants[0]).length, 'the variants share one shape')
  }
  assert.equal(new Set(variants.map(input => JSON.stringify(input))).size, seeds.length)
  for (const i of seeds.keys()) {
    assert.equal(JSON.stringify(results['bench-variant-' + i].results.map(Boolean)), JSON.stringify(results['bench-cache'].results.map(Boolean)))
  }

  const corpusResults = Object.fromEntries(cases.map(c => [c.id, results[c.id]]))
  if (action === '--capture') {
    assert.equal(language, 'reference')
    const lines = Object.entries(corpusResults).map(([id, out]) => ' ' + JSON.stringify(id) + ': ' + JSON.stringify(out))
    writeFileSync(file('expected.json'), '{\n' + lines.join(',\n') + '\n}\n')
  } else {
    const expected = JSON.parse(readFileSync(file('expected.json')))
    for (const c of cases) assert.equal(JSON.stringify(corpusResults[c.id]), JSON.stringify(expected[c.id]), c.id)
  }
  console.log(`${browser} ${language}: ${cases.length} cases and ${seeds.length} bench variants matched, ${cleanupAfterError} with a step failing after caches were opened, no caches left behind`)
} finally {
  await session?.driver?.quit()
  await server.close()
}
