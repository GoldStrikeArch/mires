#!/usr/bin/env node

import { createServer } from 'node:http'
import { gunzipSync } from 'node:zlib'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'

import { args, fileRecord, stableJson } from '../../fingerprintjs-rust-compact/tools/common.mjs'
import { browserIdentity, createDriver } from '../../fingerprintjs-rust-compact/tests/webdriver.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = resolve(root, '../..')
const distRoot = resolve(root, 'dist')
const webdriverRoot = resolve(repositoryRoot, 'experiments/fingerprintjs')
const options = args()
const browsers = String(options.browsers || 'chrome').split(',').filter(Boolean)
const pureOnly = options.pure === true
const output = resolve(options.output || resolve(root, 'results/runtime.json'))
const buildReceiptPath = resolve(options['build-receipt'] || resolve(root, 'results/build.json'))
const samples = Number(options.samples || 9)
const warmups = Number(options.warmups || 2)
const lifecycleCycles = Number(options.cycles || 8)
const asyncLifecycleCycles = Number(options.asyncCycles || lifecycleCycles)
const workloads = ['minisearch', 'marked', 'fflate', 'idb', 'idb_lines', 'ky', 'uplot', 'explorer', 'workbox', 'workbox_direct']

const receipt = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  configuration: {
    browsers: pureOnly ? [] : browsers,
    samples,
    warmups,
    lifecycleCycles: pureOnly ? 0 : lifecycleCycles,
    asyncLifecycleCycles: pureOnly ? 0 : asyncLifecycleCycles,
  },
  artifacts: {},
  pure: {},
  browsers: {},
  workbox: {},
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

await runPure()
if (!pureOnly) await runBrowsers()

receipt.status = {
  buildReceiptPassed: buildReceipt.status?.passed === true,
  buildReceiptMatched: receipt.buildReceipt.artifactsMatch,
  browserPassed: pureOnly || browsers.every((browser) => receipt.browsers[browser]?.passed),
  purePassed: Object.values(receipt.pure).every((result) => result.passed),
  // The service-worker lifecycle is deliberately Chrome-scoped. A Firefox- or Safari-only
  // receipt records that scope as skipped without turning otherwise successful browser execution
  // into a process failure; production qualification below still requires the async control.
  workboxPassed: pureOnly || !browsers.includes('chrome') || receipt.workbox.hostPromise?.passed,
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

function portablePath(path) {
  const local = relative(root, path)
  return local && !local.startsWith('..') ? local : path
}
await mkdir(dirname(output), { recursive: true })
await writeFile(output, stableJson(receipt))
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
      const driver = await createDriver(browser, webdriverRoot)
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
