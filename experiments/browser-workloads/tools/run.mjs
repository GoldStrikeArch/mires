#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { createServer } from 'node:http'
import os from 'node:os'
import { gunzipSync } from 'node:zlib'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'

import { args, fileRecord, stableJson } from '../../fingerprintjs-rust-compact/tools/common.mjs'
import { browserIdentity, createDriver } from '../../fingerprintjs-rust-compact/tests/webdriver.mjs'
import { PROTOCOL_VERSION, TIMED_WORKLOADS, canonicalJson, cellSeed, digest } from './timing-protocol.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = resolve(root, '../..')
const distRoot = resolve(root, 'dist')
const referenceRoot = resolve(root, 'reference')
const webdriverRoot = resolve(repositoryRoot, 'experiments/fingerprintjs')
const options = args()
const BROWSER_NAMES = ['chrome', 'firefox', 'safari']
const USAGE = `usage: node experiments/browser-workloads/tools/run.mjs [--pure | --browsers ${BROWSER_NAMES.join(',')}]
  [--samples 9] [--warmups 2] [--min-sample-ms 20] [--seed 1] [--cycles 8]
  [--output results/runtime.json] [--build-receipt results/build.json]`
// An invalid option is a usage error before any work; it never becomes a receipt or a number.
// An unknown browser name is a usage error too (as in measure.mjs); `unavailable` is reserved for
// a known browser whose WebDriver session could not start.
const browsers = [...new Set(String(options.browsers || 'chrome').split(',').map((name) => name.trim()).filter(Boolean))]
for (const name of browsers) if (!BROWSER_NAMES.includes(name)) usageError(`unknown browser: ${name} (expected ${BROWSER_NAMES.join(', ')})`)
const pureOnly = options.pure === true
const output = resolve(options.output || resolve(root, 'results/runtime.json'))
const buildReceiptPath = resolve(options['build-receipt'] || resolve(root, 'results/build.json'))
const samples = integerOption('samples', 9, 1)
const warmups = integerOption('warmups', 2, 0)
const lifecycleCycles = Number(options.cycles || 8)
const asyncLifecycleCycles = Number(options.asyncCycles || lifecycleCycles)
const minSampleMs = positiveNumberOption('min-sample-ms', 20)
const seed = integerOption('seed', 1, Number.MIN_SAFE_INTEGER)
const workloads = ['minisearch', 'marked', 'fflate', 'idb', 'idb_lines', 'ky', 'uplot', 'explorer', 'workbox', 'workbox_direct']

// Paired Rust-vs-JavaScript timing (tools/timing-protocol.mjs). Every timed workload runs two
// cells per environment: Rust against the JS reference, and the A/A control, Rust against a second
// Rust instance of the same artifact.
const TIMED = ['marked', 'minisearch', 'fflate']
const PAIRS = [
  { pair: 'rust-vs-js', comparison: 'rust/js', arms: [
    { key: 'rust', label: 'R', implementation: 'rust', instance: 'a' },
    { key: 'js', label: 'J', implementation: 'js' },
  ] },
  { pair: 'rust-vs-rust', comparison: 'A/A', arms: [
    { key: 'rust-a', label: 'A', implementation: 'rust', instance: 'a' },
    { key: 'rust-b', label: 'B', implementation: 'rust', instance: 'b' },
  ] },
]
// The library file each reference adapter imports, by a relative path into the repository's
// node_modules: the package's published ESM entry for `specifier` (checked against Node's
// resolution of that specifier). fflate uses its browser build in every environment. Browsers load
// the adapters and these files from the timing server at their repository-relative URLs, so the
// relative imports resolve the same way as in Node, with no import map. (Firefox does not apply a
// page's import map to modules imported from a WebDriver script.)
const REFERENCE_LIBRARIES = {
  marked: { specifier: 'marked', package: 'marked', file: 'lib/marked.esm.js' },
  minisearch: { specifier: 'minisearch', package: 'minisearch', file: 'dist/es/index.js' },
  fflate: { specifier: 'fflate/browser', package: 'fflate', file: 'esm/browser.js' },
}
const TIMING_PAGE = '/timing.html'
const BROWSER_SCRIPT_TIMEOUT_MS = 600_000
// Runs one cell in the freshly loaded timing page; the page imports the shared protocol itself.
const BROWSER_CELL_SCRIPT = `
  const done = arguments[arguments.length - 1]
  const config = arguments[0]
  import(config.protocolUrl)
    .then((protocol) => protocol.runCell(config))
    .then(done, (error) => done({
      workload: config.workload, pair: config.pair, status: 'error',
      error: { message: String(error), stack: String(error && error.stack) },
    }))
`
// Node checksums per workload, from the Node rust/js cell; every later cell must reproduce them.
const nodeChecksums = {}

const receipt = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  configuration: {
    browsers: pureOnly ? [] : browsers,
    samples,
    warmups,
    lifecycleCycles: pureOnly ? 0 : lifecycleCycles,
    asyncLifecycleCycles: pureOnly ? 0 : asyncLifecycleCycles,
    minSampleMs,
    seed,
  },
  artifacts: {},
  pure: {},
  browsers: {},
  workbox: {},
  timing: {
    protocol: {
      version: PROTOCOL_VERSION,
      code: 'tools/timing-protocol.mjs',
      cell: 'one workload, two arms (rust vs js, or rust vs a second rust instance for A/A), run in a fresh Node process (tools/timing-cell.mjs) or a fresh browser page load',
      coldImport: 'first import() of each arm in the fresh process/page, seeded random order, timed alone; browsers fetch from a loopback server with cache-control: no-store and a unique URL per load',
      firstCall: 'the first call of each arm, also its validation call; not part of the samples',
      gate: 'arms must produce the same canonical checksum, equal to the Node rust/js cell checksum, before any timing; a cell that fails the gate has no timings',
      calibration: `iterationsPerSample = max(base iterations of pureBenchmarks, calls needed for ${minSampleMs} ms), per arm`,
      order: 'every warmup and sample round runs the two arms back to back; orders are seeded random (mulberry32) in balanced blocks of two rounds (AB+BA or BA+AB), so each order runs equally often within one',
      unit: 'p50/p95/min/max are milliseconds per call (sample time / iterationsPerSample); sampleMs holds the per-sample times',
      ratio: 'first arm / second arm of per-call p50 (rust/js, or rust-a/rust-b for A/A); paired = median over rounds of the per-round per-call ratio',
      browsers: 'headless where supported (Chrome, Firefox; not Safari); the timing page is served with COOP same-origin and COEP require-corp so performance.now() gets cross-origin-isolated resolution; the protocol, adapters and library files are served at their repository-relative URLs (no import map)',
    },
    loadAverage: { start: os.loadavg(), end: null },
    host: { platform: process.platform, arch: process.arch, cpu: os.cpus()[0]?.model ?? null, logicalCpus: os.cpus().length },
    references: {},
    workloads: {},
    environments: {},
  },
}
const buildReceipt = JSON.parse(await readFile(buildReceiptPath, 'utf8'))
receipt.buildReceipt = {
  path: portablePath(buildReceiptPath),
  ...await fileRecord(buildReceiptPath),
}
for (const name of workloads) {
  receipt.artifacts[name] = await fileRecord(resolve(distRoot, name, 'index.min.js'))
}
receipt.buildReceipt.artifactsMatch = workloads.every((name) =>
  buildReceipt.workloads?.[name]?.artifact?.sha256 === receipt.artifacts[name].sha256,
)

const timingInputs = await prepareTiming()
await runPure()
await runNodeTiming()
if (!pureOnly) {
  await runBrowsers()
  await runBrowserTiming()
}
finishTiming()

receipt.status = {
  buildReceiptPassed: buildReceipt.status?.passed === true,
  buildReceiptMatched: receipt.buildReceipt.artifactsMatch,
  browserPassed: pureOnly || browsers.every((browser) => receipt.browsers[browser]?.passed),
  purePassed: Object.values(receipt.pure).every((result) => result.passed),
  // The service-worker lifecycle is deliberately Chrome-scoped. A Firefox- or Safari-only
  // receipt records that scope as skipped without turning otherwise successful browser execution
  // into a process failure; production qualification below still requires the async control.
  workboxPassed: pureOnly || !browsers.includes('chrome') || receipt.workbox.hostPromise?.passed,
  // Every requested timing environment ran every cell through the checksum gate. An unavailable
  // browser is recorded with its reason and fails this check; it never contributes a number.
  timingPassed: Object.values(receipt.timing.environments).every((environment) => environment.status === 'ok'),
}
receipt.status.passed = Object.values(receipt.status).every(Boolean)
const browserMatrixComplete = !pureOnly
  && ['chrome', 'firefox', 'safari'].every((browser) => browsers.includes(browser))
const corpusQualified = receipt.status.passed
  && browserMatrixComplete
  && receipt.pure.minisearch?.stress?.passed === true
  && receipt.workbox.asyncContinuation?.passed === true
  && browsers.every((browser) => receipt.browsers[browser]?.controls?.idbLines?.passed)
receipt.qualification = {
  browserMatrixComplete,
  minisearchStressPassed: receipt.pure.minisearch?.stress?.passed === true,
  asyncWorkerPassed: pureOnly || receipt.workbox.asyncContinuation?.passed === true,
  idbLinesSupported: pureOnly || browsers.every((browser) => receipt.browsers[browser]?.controls?.idbLines?.passed),
  corpusQualified,
  productionQualified: false,
  productionQualificationReason: 'This is a reduced diagnostic corpus, not five complete medium-size applications.',
}
receipt.classification = {
  admissionPassed: corpusQualified,
  executionPassed: receipt.status.passed,
  expectedFailuresObserved: !pureOnly && receipt.workbox.asyncContinuation?.passed === false,
  productionQualified: false,
  semanticParityPassed: receipt.status.passed
    && receipt.pure.minisearch?.stress?.passed === true
    && (pureOnly || receipt.workbox.asyncContinuation?.passed === true)
    && (pureOnly || browsers.every((browser) => receipt.browsers[browser]?.controls?.idbLines?.passed)),
}

function usageError(message) {
  process.stderr.write(`run.mjs: ${message}\n${USAGE}\n`)
  process.exit(1)
}

function shown(raw) {
  return raw === true ? 'no value' : JSON.stringify(raw)
}

// A decimal integer of at least `min`; absent means `fallback`, a bare flag is an error.
function integerOption(name, fallback, min) {
  const raw = options[name]
  if (raw === undefined) return fallback
  const value = typeof raw === 'string' && /^[+-]?\d+$/.test(raw) ? Number(raw) : NaN
  if (!Number.isSafeInteger(value) || value < min) {
    usageError(`--${name} must be an integer${min > Number.MIN_SAFE_INTEGER ? ` >= ${min}` : ''} (got ${shown(raw)})`)
  }
  return value
}

function positiveNumberOption(name, fallback) {
  const raw = options[name]
  if (raw === undefined) return fallback
  const value = typeof raw === 'string' && raw.trim() !== '' ? Number(raw) : NaN
  if (!(Number.isFinite(value) && value > 0)) usageError(`--${name} must be a positive number (got ${shown(raw)})`)
  return value
}

function portablePath(path) {
  const local = relative(root, path)
  return local && !local.startsWith('..') ? local : path
}
await mkdir(dirname(output), { recursive: true })
await writeFile(output, stableJson(receipt))
printTimingTable()
process.stdout.write(`${output}\n`)
if (!receipt.status.passed) process.exitCode = 1

async function runPure() {
  const mini = await load('minisearch')
  const marked = await load('marked')
  const fflate = await load('fflate')
  const uplot = await load('uplot')
  const documents = makeDocuments(12)
  const markdown = makeMarkdown(80)
  const compressionText = markdown.repeat(8)

  receipt.pure.minisearch = capture(() => {
    const miniValue = mini.run({ documents, query: 'browser comp*' })
    const stress = progressiveStress((count) => mini.run({ documents: makeDocuments(count), query: 'browser comp*' }))
    return {
      passed: miniValue.startsWith('d0:') && miniValue.includes('d11:'),
      value: miniValue,
      stress,
    }
  })
  receipt.pure.marked = capture(() => {
    const markedValue = marked.run({ markdown: '# Hi\n\n- **one**\n- `two`' })
    return result(
      markedValue === '<h1>Hi</h1>\n<ul>\n<li><strong>one</strong></li>\n<li><code>two</code></li>\n</ul>\n',
      markedValue,
    )
  })
  receipt.pure.fflate = capture(() => {
    const gzip = fflate.run({ text: compressionText })
    return result(gunzipSync(Buffer.from(gzip)).toString() === compressionText, gzip.length)
  })

  const calls = []
  const context = new Proxy({}, {
    get(target, key) { return target[key] ?? ((...values) => calls.push([key, ...values])) },
    set(target, key, value) { target[key] = value; return true },
  })
  const canvas = { clientWidth: 100, clientHeight: 50, getContext: () => context }
  receipt.pure.uplot = capture(() => {
    const plotValue = uplot.run({ canvas, xs: [0, 1, 2], ys: [2, 4, 3] })
    return result(plotValue.points === 3 && calls.some(([name]) => name === 'stroke'), plotValue)
  })

  receipt.pureBenchmarks = {
    minisearch: capture(() => benchmark(() => mini.run({ documents, query: 'browser comp*' }), 4)),
    marked: capture(() => benchmark(() => marked.run({ markdown }), 5)),
    fflate: capture(() => benchmark(() => fflate.run({ text: compressionText }), 3)),
  }
}

async function runBrowsers() {
  const server = await startWorkloadServer()
  try {
    for (const browser of browsers) {
      let driver
      try {
        driver = await createDriver(browser, webdriverRoot)
      } catch (error) {
        receipt.browsers[browser] = { passed: false, unavailable: errorRecord(error).message }
        process.stdout.write(`${browser}: unavailable: ${receipt.browsers[browser].unavailable}\n`)
        continue
      }
      try {
        process.stdout.write(`${browser}: loading workload page\n`)
        await driver.get(`${server.origin}/index.html`)
        const value = await driver.executeAsyncScript(`
          const done = arguments[arguments.length - 1]
          let stage = 'module-import'
          let complete = false
          const finish = (value) => {
            if (complete) return
            complete = true
            clearTimeout(deadline)
            done(value)
          }
          const deadline = setTimeout(() => finish({ error: 'browser workload timeout', stage }), 30000)
          ;(async () => {
            const origin = ${JSON.stringify(server.origin)}
            const load = (name) => import(origin + '/' + name + '.js')
            const [idb, idbLines, ky, uplot, explorer] = await Promise.all(['idb', 'idb_lines', 'ky', 'uplot', 'explorer'].map(load))
            stage = 'idb'
            const database = 'browser-workloads-' + Math.random().toString(36).slice(2)
            const records = 'b=beta=2\\na=alpha=1\\nc=gamma=3'
            const idbValue = await idb.run({ database, records })
            stage = 'idb-lines'
            let idbLinesValue
            let idbLinesError
            try {
              idbLinesValue = await idbLines.run({ database: database + '-lines', records })
            } catch (error) {
              idbLinesError = { message: String(error), stack: error && error.stack }
            }
            stage = 'ky'
            const token = Math.random().toString(36).slice(2)
            const kyValue = await ky.run({ url: origin + '/retry/' + token, attempts: 4, timeout: 5000 })
            stage = 'uplot'
            const canvas = document.createElement('canvas')
            Object.defineProperty(canvas, 'clientWidth', { value: 320 })
            Object.defineProperty(canvas, 'clientHeight', { value: 160 })
            document.body.appendChild(canvas)
            const xs = Array.from({ length: 256 }, (_, index) => index)
            const ys = xs.map((value) => Math.sin(value / 13) * 20 + value / 8)
            const plotValue = uplot.run({ canvas, xs, ys })
            stage = 'explorer'
            const explorerRecords = [
              'a=a|Rust compiler|# Rust browser compiler',
              'b=b|Offline search|# Search indexed browser data',
              'c=c|Canvas chart|# Chart time series data',
            ].join('\\n')
            const explorerValue = await explorer.run({
              database: database + '-explorer', records: explorerRecords,
              query: 'browser comp*', canvas, xs, ys,
            })
            const deleteDatabase = (name) => new Promise((resolve) => {
              const request = indexedDB.deleteDatabase(name)
              request.onsuccess = request.onerror = request.onblocked = () => resolve()
            })
            await deleteDatabase(database)
            await deleteDatabase(database + '-lines')
            await deleteDatabase(database + '-explorer')
            finish({
              idbValue,
              idbLinesValue,
              idbLinesError,
              kyValue,
              plotValue,
              explorerValue: {
                hits: explorerValue.hits,
                html: explorerValue.html,
                gzipBytes: explorerValue.gzipBytes,
                storedBytes: explorerValue.storedBytes,
                points: explorerValue.chart.points,
              },
            })
          })().catch((error) => finish({ error: String(error), stack: error && error.stack, stage }))
        `)
        const checks = {
          idb: value.idbValue === 'a=alpha=1\nb=beta=2\nc=gamma=3',
          ky: /^200\|ok-[a-z0-9]+\|3$/.test(value.kyValue),
          uplot: value.plotValue?.points === 256 && Number.isFinite(value.plotValue?.checksum),
          explorer: value.explorerValue?.hits?.startsWith('a:') &&
            value.explorerValue?.html?.includes('<h1>Rust browser compiler</h1>') &&
            value.explorerValue?.gzipBytes > value.explorerValue?.storedBytes &&
            value.explorerValue?.points === 256,
          noError: !value.error,
        }
        receipt.browsers[browser] = {
          browser: await browserIdentity(driver),
          checks,
          controls: {
            idbLines: {
              passed: value.idbLinesValue === 'a=alpha=1\nb=beta=2\nc=gamma=3',
              expectedFailure: value.idbLinesError?.message?.includes('not a byte view of 4-byte elements') === true,
              error: value.idbLinesError,
            },
          },
          passed: Object.values(checks).every(Boolean),
          value,
        }
        process.stdout.write(`${browser}: browser workloads ${receipt.browsers[browser].passed ? 'PASS' : 'FAIL'}\n`)
        if (browser === 'chrome') {
          process.stdout.write(`${browser}: running service-worker lifecycle\n`)
          receipt.workbox = {
            asyncContinuation: await runWorkboxLifecycle(
              driver,
              server,
              'workbox.js',
              asyncLifecycleCycles,
            ),
            hostPromise: await runWorkboxLifecycle(
              driver,
              server,
              'workbox_direct.js',
              lifecycleCycles,
            ),
          }
          try {
            receipt.workbox.browserLog = (await driver.manage().logs().get('browser')).map((entry) => ({
              level: entry.level?.name || String(entry.level),
              message: entry.message,
              timestamp: entry.timestamp,
            }))
          } catch (error) {
            receipt.workbox.browserLog = [{ level: 'UNAVAILABLE', message: String(error) }]
          }
        }
      } finally {
        await driver.quit()
      }
    }
    if (!browsers.includes('chrome')) {
      receipt.workbox = { passed: true, skipped: 'Workbox lifecycle is qualified in Chrome by default.' }
    }
  } finally {
    await server.close()
  }
}

async function runWorkboxLifecycle(driver, server, script, cycles) {
  server.setOffline(false)
  const records = []
  await driver.executeAsyncScript(`
    const done = arguments[arguments.length - 1]
    caches.open('browser-workloads-stale').then(() => done(true), (error) => done({ error: String(error) }))
  `)
  for (let cycle = 1; cycle <= cycles; cycle++) {
    process.stdout.write(`workbox: cycle ${cycle}/${cycles}\n`)
    const installed = await driver.executeAsyncScript(`
      const done = arguments[arguments.length - 1]
      let stage = 'register'
      let registration
      let worker
      let complete = false
      const finish = (value) => {
        if (complete) return
        complete = true
        clearTimeout(deadline)
        done(value)
      }
      const states = () => ({
        worker: worker?.state || null,
        installing: registration?.installing?.state || null,
        waiting: registration?.waiting?.state || null,
        active: registration?.active?.state || null,
      })
      const deadline = setTimeout(() => finish({ error: 'service-worker cycle timeout', stage, states: states() }), 30000)
      ;(async () => {
        registration = await navigator.serviceWorker.register(${JSON.stringify(`${server.origin}/`)} + ${JSON.stringify(script)} + '?v=${cycle}', { scope: '/' })
        stage = 'activation'
        worker = registration.installing || registration.waiting || registration.active
        if (worker && worker.state !== 'activated') {
          worker.postMessage({ type: 'SKIP_WAITING' })
          await new Promise((resolve) => {
            const timeout = setTimeout(resolve, 10000)
            worker.addEventListener('statechange', () => {
              if (worker.state === 'activated') { clearTimeout(timeout); resolve() }
            })
          })
        }
        stage = 'ready'
        await navigator.serviceWorker.ready
        const expectedVersion = '?v=${cycle}'
        if (!navigator.serviceWorker.controller || !navigator.serviceWorker.controller.scriptURL.includes(expectedVersion)) {
          await new Promise((resolve) => {
            const timeout = setTimeout(resolve, 10000)
            const changed = () => {
              if (navigator.serviceWorker.controller?.scriptURL.includes(expectedVersion)) {
                clearTimeout(timeout)
                navigator.serviceWorker.removeEventListener('controllerchange', changed)
                resolve()
              }
            }
            navigator.serviceWorker.addEventListener('controllerchange', changed)
            changed()
          })
        }
        stage = 'online-fetch'
        const cacheNames = await caches.keys()
        const cacheEntries = Object.fromEntries(await Promise.all(cacheNames.map(async (name) => [
          name,
          (await (await caches.open(name)).keys()).map((request) => request.url),
        ])))
        const online = await (await fetch('/asset-a.txt')).text()
        finish({ online, scriptURL: (registration.active || worker).scriptURL, cacheNames, cacheEntries })
      })().catch((error) => finish({ error: String(error), stack: error && error.stack, stage }))
    `)
    if (installed.error) {
      records.push({ cycle, installed, offline: { skipped: true } })
      break
    }
    server.setOffline(true)
    const offline = await driver.executeAsyncScript(`
      const done = arguments[arguments.length - 1]
      fetch('/asset-a.txt').then((response) => response.text()).then(
        (text) => caches.keys().then((caches) => done({ text, caches })),
        (error) => done({ error: String(error) }),
      )
    `)
    server.setOffline(false)
    records.push({ cycle, installed, offline })
  }
  const cleanup = await driver.executeAsyncScript(`
    const done = arguments[arguments.length - 1]
    ;(async () => {
      const registrations = await navigator.serviceWorker.getRegistrations()
      await Promise.all(registrations.map((registration) => registration.unregister()))
      const names = await caches.keys()
      await Promise.all(names.filter((name) => name.startsWith('browser-workloads')).map((name) => caches.delete(name)))
      done({ registrationsBefore: registrations.length, cachesAfter: await caches.keys() })
    })().catch((error) => done({ error: String(error) }))
  `)
  const passed = records.every((record) =>
    !record.installed.error && record.installed.online === 'asset-a-v1' &&
    record.offline.text === 'asset-a-v1' &&
    record.offline.caches.filter((name) => name.startsWith('browser-workloads')).length === 1,
  ) && cleanup.registrationsBefore === 1 && cleanup.cachesAfter.filter((name) => name.startsWith('browser-workloads')).length === 0
  return { cycles, cleanup, passed, records, serverRequests: server.snapshot() }
}

async function startWorkloadServer() {
  let offline = false
  const retryCounts = new Map()
  const requests = []
  const modules = Object.fromEntries(workloads.map((name) => [`/${name}.js`, resolve(distRoot, name, 'index.min.js')]))
  // Timing page resources, a whitelist served at their repository-relative URLs: the shared
  // protocol, the reference adapters and the library files they import.
  const timingModules = Object.fromEntries(timingFiles().map((path) => [`/${relative(repositoryRoot, path)}`, path]))
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://127.0.0.1')
      requests.push({ offline, path: url.pathname, search: url.search })
      if (url.pathname === '/index.html' || url.pathname === '/') {
        response.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'service-worker-allowed': '/' })
        response.end('<!doctype html><meta charset="utf-8"><title>Browser workloads</title>')
        return
      }
      if (modules[url.pathname]) {
        response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8', 'cache-control': 'no-store', 'service-worker-allowed': '/' })
        response.end(await readFile(modules[url.pathname]))
        return
      }
      if (url.pathname === TIMING_PAGE) {
        response.writeHead(200, {
          'content-type': 'text/html; charset=utf-8',
          'cache-control': 'no-store',
          'cross-origin-opener-policy': 'same-origin',
          'cross-origin-embedder-policy': 'require-corp',
        })
        response.end(timingPage())
        return
      }
      if (timingModules[url.pathname]) {
        response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8', 'cache-control': 'no-store' })
        response.end(await readFile(timingModules[url.pathname]))
        return
      }
      if (url.pathname === '/workbox.js') {
        response.writeHead(200, { 'content-type': 'text/javascript; charset=utf-8', 'cache-control': 'no-store', 'service-worker-allowed': '/' })
        response.end(await readFile(resolve(distRoot, 'workbox/index.min.js')))
        return
      }
      if (offline) {
        response.destroy()
        return
      }
      if (url.pathname === '/asset-a.txt') {
        response.writeHead(200, { 'content-type': 'text/plain', 'cache-control': 'no-store' }).end('asset-a-v1')
        return
      }
      if (url.pathname === '/asset-b.txt') {
        response.writeHead(200, { 'content-type': 'text/plain', 'cache-control': 'no-store' }).end('asset-b-v1')
        return
      }
      if (url.pathname.startsWith('/retry/')) {
        const token = url.pathname.slice('/retry/'.length)
        const count = (retryCounts.get(token) || 0) + 1
        retryCounts.set(token, count)
        response.writeHead(count < 3 ? 503 : 200, { 'content-type': 'text/plain' }).end(count < 3 ? 'retry' : `ok-${token}`)
        return
      }
      response.writeHead(404).end('not found')
    } catch (error) {
      response.writeHead(500).end(String(error))
    }
  })
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolvePromise)
  })
  const { port } = server.address()
  return {
    origin: `http://127.0.0.1:${port}`,
    setOffline(value) { offline = value },
    snapshot: () => requests.slice(-200),
    close: () => new Promise((resolvePromise, reject) => server.close((error) => error ? reject(error) : resolvePromise())),
  }
}

function timingPage() {
  return '<!doctype html><meta charset="utf-8"><title>Browser workload timing</title>'
}

function timingFiles() {
  return [
    resolve(root, 'tools/timing-protocol.mjs'),
    ...TIMED.map((name) => resolve(referenceRoot, `${name}.mjs`)),
    ...Object.values(REFERENCE_LIBRARIES).map((library) => resolve(repositoryRoot, 'node_modules', library.package, library.file)),
  ]
}

// Browser URL of a repository file on the timing server.
function servedUrl(origin, path, load) {
  return `${origin}/${relative(repositoryRoot, path)}?load=${load}`
}

// The timed inputs are exactly those of `pureBenchmarks` in runPure.
async function prepareTiming() {
  const markdown = makeMarkdown(80)
  const inputs = {
    marked: { input: { markdown }, description: 'makeMarkdown(80)' },
    minisearch: { input: { documents: makeDocuments(12), query: 'browser comp*' }, description: "makeDocuments(12), query 'browser comp*'" },
    fflate: { input: { text: markdown.repeat(8) }, description: 'makeMarkdown(80).repeat(8)' },
  }
  const rootPackage = JSON.parse(await readFile(resolve(repositoryRoot, 'package.json'), 'utf8'))
  const sha256File = async (path) => {
    const value = await readFile(path)
    return { bytes: value.length, sha256: createHash('sha256').update(value).digest('hex') }
  }
  for (const name of TIMED) {
    const spec = TIMED_WORKLOADS[name]
    const json = canonicalJson(inputs[name].input)
    receipt.timing.workloads[name] = {
      input: { description: inputs[name].description, sha256: await digest(json), canonicalJsonBytes: Buffer.byteLength(json) },
      baseIterations: spec.baseIterations,
      shared: spec.shared,
      differences: spec.differences,
    }
    const library = REFERENCE_LIBRARIES[name]
    const entry = resolve(repositoryRoot, 'node_modules', library.package, library.file)
    const record = {
      package: library.package,
      specifier: library.specifier,
      pinned: rootPackage.devDependencies?.[library.package] ?? null,
      adapter: { path: `reference/${name}.mjs`, ...await sha256File(resolve(referenceRoot, `${name}.mjs`)) },
      entry: { path: relative(repositoryRoot, entry) },
    }
    try {
      record.version = JSON.parse(await readFile(resolve(repositoryRoot, 'node_modules', library.package, 'package.json'), 'utf8')).version
      Object.assign(record.entry, await sha256File(entry))
      // The adapter's relative import names this file; Node's package resolution must agree that
      // it is the package's published entry for the specifier.
      record.nodeResolvesSameEntry = fileURLToPath(import.meta.resolve(library.specifier)) === entry
      const adapterSource = await readFile(resolve(referenceRoot, `${name}.mjs`), 'utf8')
      record.adapterImportsEntry = adapterSource.includes(`from '${relative(referenceRoot, entry)}'`)
    } catch (error) {
      record.error = errorRecord(error)
    }
    receipt.timing.references[name] = record
  }
  return Object.fromEntries(TIMED.map((name) => [name, inputs[name].input]))
}

function cellConfig(environment, workload, pairSpec, urlFor) {
  return {
    workload,
    pair: pairSpec.pair,
    arms: pairSpec.arms.map((arm) => ({ key: arm.key, label: arm.label, implementation: arm.implementation, url: urlFor(arm) })),
    input: timingInputs[workload],
    expectedInputSha256: receipt.timing.workloads[workload].input.sha256,
    expectedChecksum: nodeChecksums[workload] ?? null,
    warmups,
    samples,
    minSampleMs,
    seed: cellSeed(seed, environment, workload, pairSpec.pair),
  }
}

function cellLine(environment, cell) {
  const p50 = cell.ratio?.p50
  const ratio = typeof p50 === 'number' && Number.isFinite(p50) ? ` ${cell.ratio.of} p50 ratio ${p50.toFixed(3)}` : ''
  const error = cell.error ? ` (${cell.error.message})` : ''
  return `${environment}: timing ${cell.workload} ${cell.pair} ${cell.status}${ratio}${error}\n`
}

function referencesUsable() {
  return TIMED.every((name) => {
    const reference = receipt.timing.references[name]
    return !reference.error && reference.nodeResolvesSameEntry === true && reference.adapterImportsEntry === true
  })
}

async function runNodeTiming() {
  const record = {
    status: 'ok',
    environment: {
      name: 'node',
      version: process.version,
      label: `node ${process.version}`,
      v8: process.versions.v8,
      isolation: 'fresh node process per cell (tools/timing-cell.mjs)',
    },
    cells: [],
  }
  receipt.timing.environments.node = record
  if (!referencesUsable()) {
    record.status = 'failed'
    record.reason = 'a JS reference library is missing or resolves to an unexpected file; see timing.references'
    return
  }
  const cellScript = resolve(root, 'tools/timing-cell.mjs')
  for (const workload of TIMED) {
    for (const pairSpec of PAIRS) {
      const config = cellConfig('node', workload, pairSpec, (arm) => (arm.implementation === 'rust'
        ? `${pathToFileURL(resolve(distRoot, workload, 'index.min.js')).href}?instance=${arm.instance}`
        : pathToFileURL(resolve(referenceRoot, `${workload}.mjs`)).href))
      const loadStart = os.loadavg()
      const child = spawnSync(process.execPath, [cellScript], {
        input: JSON.stringify(config), encoding: 'utf8', maxBuffer: 64 << 20,
      })
      let cell
      try {
        cell = JSON.parse(child.stdout)
      } catch {
        cell = {
          workload, pair: pairSpec.pair, status: 'error',
          error: { message: `timing cell exited ${child.status}${child.error ? `: ${child.error.message}` : ''}`, stderr: String(child.stderr ?? '').slice(-4_000) },
        }
      }
      cell.loadAverage = { start: loadStart, end: os.loadavg() }
      record.cells.push(cell)
      if (pairSpec.pair === 'rust-vs-js' && cell.status === 'ok') nodeChecksums[workload] = cell.checksum
      process.stdout.write(cellLine('node', cell))
    }
  }
  if (!record.cells.every((cell) => cell.status === 'ok')) record.status = 'failed'
}


async function browserEnvironment(driver, browser) {
  const identity = await browserIdentity(driver)
  const capabilities = await driver.getCapabilities()
  let driverVersion = capabilities.get('chrome')?.chromedriverVersion ?? capabilities.get('moz:geckodriverVersion') ?? null
  if (browser === 'safari') {
    const version = spawnSync('safaridriver', ['--version'], { encoding: 'utf8' })
    driverVersion = String(version.stdout || version.stderr || '').trim() || null
  }
  return {
    name: identity.name,
    version: identity.version,
    label: `${identity.name} ${identity.version}`,
    platform: identity.platform,
    headless: browser !== 'safari',
    driver: driverVersion,
    userAgent: await driver.executeScript('return navigator.userAgent'),
    isolation: 'fresh page load per cell',
  }
}

async function runBrowserTiming() {
  const server = await startWorkloadServer()
  try {
    for (const browser of browsers) {
      const record = { status: 'ok', environment: null, cells: [] }
      receipt.timing.environments[browser] = record
      if (!referencesUsable()) {
        record.status = 'failed'
        record.reason = 'a JS reference library is missing or resolves to an unexpected file; see timing.references'
        continue
      }
      let driver
      try {
        driver = await createDriver(browser, webdriverRoot)
      } catch (error) {
        record.status = 'unavailable'
        record.reason = errorRecord(error).message
        process.stdout.write(`${browser}: timing unavailable: ${record.reason}\n`)
        continue
      }
      try {
        await driver.manage().setTimeouts({ pageLoad: 60000, script: BROWSER_SCRIPT_TIMEOUT_MS })
        record.environment = await browserEnvironment(driver, browser)
        process.stdout.write(`${browser}: timing in ${record.environment.label}\n`)
        let load = 0
        for (const workload of TIMED) {
          for (const pairSpec of PAIRS) {
            load++
            const config = cellConfig(browser, workload, pairSpec, (arm) => (arm.implementation === 'rust'
              ? `${server.origin}/${workload}.js?load=${load}&instance=${arm.instance}`
              : servedUrl(server.origin, resolve(referenceRoot, `${workload}.mjs`), load)))
            config.protocolUrl = servedUrl(server.origin, resolve(root, 'tools/timing-protocol.mjs'), load)
            const loadStart = os.loadavg()
            let cell
            try {
              await driver.get(`${server.origin}${TIMING_PAGE}?load=${load}`)
              cell = await driver.executeAsyncScript(BROWSER_CELL_SCRIPT, config)
            } catch (error) {
              cell = { workload, pair: pairSpec.pair, status: 'error', error: errorRecord(error) }
            }
            cell.loadAverage = { start: loadStart, end: os.loadavg() }
            record.cells.push(cell)
            process.stdout.write(cellLine(browser, cell))
          }
        }
        if (!record.cells.every((cell) => cell.status === 'ok')) record.status = 'failed'
      } catch (error) {
        record.status = 'failed'
        record.reason = errorRecord(error).message
      } finally {
        await driver.quit().catch(() => {})
      }
    }
  } finally {
    await server.close()
  }
}

// Flat rows (one per arm) and comparisons (one per cell) over every environment.
function finishTiming() {
  const timing = receipt.timing
  timing.loadAverage.end = os.loadavg()
  timing.rows = []
  timing.comparisons = []
  for (const [key, environment] of Object.entries(timing.environments)) {
    const label = environment.environment?.label ?? key
    if (environment.status !== 'ok' && environment.cells.length === 0) {
      timing.comparisons.push({ environment: label, environmentKey: key, status: environment.status, reason: environment.reason ?? null })
      continue
    }
    for (const cell of environment.cells) {
      const pairSpec = PAIRS.find((candidate) => candidate.pair === cell.pair)
      for (const arm of cell.arms ?? []) {
        timing.rows.push({
          environment: label,
          environmentKey: key,
          workload: cell.workload,
          pair: cell.pair,
          implementation: arm.implementation,
          arm: arm.key,
          status: cell.status,
          inputSha256: cell.inputSha256 ?? null,
          iterationsPerSample: arm.iterationsPerSample ?? null,
          warmups: cell.warmups,
          samples: cell.samples,
          unit: 'ms/call',
          p50: arm.perCallMs?.p50 ?? null,
          p95: arm.perCallMs?.p95 ?? null,
          min: arm.perCallMs?.min ?? null,
          max: arm.perCallMs?.max ?? null,
          sampleMs: arm.sampleSummary ?? null,
          coldImportMs: arm.coldImportMs ?? null,
          firstCallMs: arm.firstCallMs ?? null,
          checksum: arm.checksum ?? null,
          rawChecksum: arm.rawChecksum ?? null,
          checksumMatchesNode: arm.checksum !== undefined && arm.checksum === nodeChecksums[cell.workload],
          timerResolutionMs: cell.environment?.timerResolutionMs ?? null,
          crossOriginIsolated: cell.environment?.crossOriginIsolated ?? null,
          loadAverage: cell.loadAverage,
        })
      }
      timing.comparisons.push({
        environment: label,
        environmentKey: key,
        workload: cell.workload,
        comparison: pairSpec?.comparison ?? cell.pair,
        status: cell.status,
        ratio: cell.ratio?.p50 ?? null,
        ratioP95: cell.ratio?.p95 ?? null,
        ratioMin: cell.ratio?.min ?? null,
        pairedRatio: cell.ratio?.paired?.p50 ?? null,
        checksum: cell.checksum ?? null,
        checksumMatchesNode: cell.checksum !== undefined && cell.checksum === nodeChecksums[cell.workload],
        loadAverage: cell.loadAverage,
      })
    }
  }
}

function printTimingTable() {
  const number = (value, digits = 3) => (typeof value === 'number' && Number.isFinite(value) ? value.toFixed(digits) : '-')
  const header = ['environment', 'workload', 'rust p50', 'rust p95', 'js p50', 'js p95', 'rust/js', 'paired', 'A/A', 'checksums']
  const lines = [header]
  const notes = []
  for (const [key, environment] of Object.entries(receipt.timing.environments)) {
    const label = environment.environment?.label ?? key
    if (environment.reason) notes.push(`${label}: ${environment.status} (${environment.reason})`)
    for (const workload of TIMED) {
      const cell = (pair) => environment.cells.find((candidate) => candidate.workload === workload && candidate.pair === pair)
      const main = cell('rust-vs-js')
      const control = cell('rust-vs-rust')
      if (!main) continue
      const arm = (name) => main.arms?.find((candidate) => candidate.key === name)
      const agree = main.status === 'ok' && main.checksum === nodeChecksums[workload] && control?.status === 'ok'
      lines.push([label, workload, number(arm('rust')?.perCallMs?.p50), number(arm('rust')?.perCallMs?.p95),
        number(arm('js')?.perCallMs?.p50), number(arm('js')?.perCallMs?.p95), number(main.ratio?.p50, 2),
        number(main.ratio?.paired?.p50, 2), number(control?.ratio?.p50, 3),
        agree ? 'equal' : `${main.status}/${control?.status ?? 'missing'}`])
    }
  }
  const widths = header.map((_, column) => Math.max(...lines.map((line) => String(line[column]).length)))
  const text = lines.map((line) => line.map((value, column) => String(value).padEnd(widths[column])).join('  ').trimEnd())
  text.splice(1, 0, widths.map((width) => '-'.repeat(width)).join('  '))
  process.stdout.write(`timing (ms per call; ratio = p50/p50; paired = median per-round ratio; A/A = rust-a/rust-b p50)\n${text.join('\n')}\n`)
  for (const note of notes) process.stdout.write(`${note}\n`)
  const load = receipt.timing.loadAverage
  process.stdout.write(`load average start ${load.start.map((value) => value.toFixed(2)).join(' ')}, end ${load.end.map((value) => value.toFixed(2)).join(' ')}\n`)
}

function makeDocuments(count) {
  return Array.from({ length: count }, (_, index) =>
    `d${index}|Rust browser compiler ${index}|Offline browser tools and compiler experiments`,
  ).join('\n')
}

function makeMarkdown(count) {
  return Array.from({ length: count }, (_, index) =>
    `## Section ${index}\n\n- **browser** compiler\n- [report](https://example.test/${index})\n\nText with <escape>.`,
  ).join('\n\n')
}

function benchmark(run, iterations) {
  for (let index = 0; index < warmups; index++) runMany(run, iterations)
  const times = []
  let checksum = 0
  for (let sample = 0; sample < samples; sample++) {
    const started = performance.now()
    checksum ^= runMany(run, iterations)
    times.push(performance.now() - started)
  }
  return { checksum, milliseconds: summarize(times), iterations }
}

function runMany(run, iterations) {
  let checksum = 0
  for (let index = 0; index < iterations; index++) {
    const value = run()
    checksum ^= typeof value === 'string' ? value.length : value.length || 0
  }
  return checksum
}

function summarize(values) {
  const sorted = [...values].sort((left, right) => left - right)
  return {
    count: sorted.length,
    min: sorted[0],
    p50: sorted[Math.ceil(sorted.length * 0.5) - 1],
    p95: sorted[Math.ceil(sorted.length * 0.95) - 1],
    max: sorted.at(-1),
  }
}

function result(passed, value) {
  return { passed, value }
}

function capture(run) {
  try {
    return run()
  } catch (error) {
    return { passed: false, error: errorRecord(error) }
  }
}

function progressiveStress(run) {
  const attempts = []
  for (const count of [2, 4, 8, 12, 16, 24, 32, 64, 120, 240]) {
    try {
      const value = run(count)
      attempts.push({ count, passed: value.startsWith('d0:') && value.includes(`d${count - 1}:`), bytes: value.length })
    } catch (error) {
      attempts.push({ count, passed: false, error: errorRecord(error) })
      break
    }
  }
  return {
    passed: attempts.length === 10 && attempts.every((attempt) => attempt.passed),
    largestPassingDocuments: attempts.filter((attempt) => attempt.passed).at(-1)?.count || 0,
    firstFailingDocuments: attempts.find((attempt) => !attempt.passed)?.count || null,
    attempts,
  }
}

function errorRecord(error) {
  return {
    name: error?.name || 'Error',
    message: String(error?.message || error).slice(0, 2_000),
    stack: String(error?.stack || '').split('\n').slice(0, 8).join('\n').slice(0, 4_000),
  }
}

async function load(name) {
  return import(`${pathToFileURL(resolve(distRoot, name, 'index.min.js')).href}?run=${Date.now()}`)
}
