#!/usr/bin/env node
// Timing: each port against the JS reference on the library's bench cases, with an A/A noise
// control per port, in Node and in browsers, on the N-arm protocol of tools/lib/protocol.mjs.
//
//   node tools/bench.mjs --library <id,...|all> [--language <id,...|all>]
//     [--environments node,chrome,firefox,safari] [--design triplet|pairs]
//     [--samples 9] [--warmups 2] [--min-sample-ms 20] [--seed 1] [--matrix <registry>] [--receipts <dir>]
//
// Cell designs (README "Timing"):
//   triplet (default)  one cell per port, case and environment with three arms: the port (instance
//                      a), the JS reference, and a second, separately imported instance of the
//                      same port artifact (b). port/js = a/js and A/A = a/b come from the same
//                      process or page, under the same load, in Williams-balanced orders.
//   pairs              browser-workloads' design: a port-vs-js cell and a separate A/A cell.
// Every cell runs in a fresh Node process (tools/bench-cell.mjs) or a fresh browser page load.
// A port is timed in an environment only when its check receipt there is a pass for the current
// artifact, corpus, reference, normalisation, output form and check protocol (`parity-not-passed`
// otherwise), and its dist/ is its build receipt's output (`stale` otherwise). Inside the cell the
// arms must agree on the canonical checksum and match the Node reference checksum before any
// timing; every call gets a fresh clone of the input, and sampled outputs of every timed run must
// keep the gate checksum (protocol.mjs).
//
// These numbers are not a performance claim; see README "Timing" for what they can and cannot say.

import { existsSync, readFileSync } from 'node:fs'
import os from 'node:os'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { openBrowser, runWatched, startServer } from './lib/browser.mjs'
import { runNodeCell } from './lib/cells.mjs'
import { digestText, inputText } from './lib/canonical.mjs'
import { PROTOCOL_VERSION, cellSeed } from './lib/protocol.mjs'
import {
  artifactOf, checkReceiptProblem, currentBinding, guardProblem, freshness, harnessIdentity, harnessRoot, integerOption, loadRegistry, majorVersion, numberOption, parseOptions, readJson, sha256,
  receiptPaths, receiptsRoot, repoPath, repositoryRoot, selectPorts, usageError, writeJson,
} from './lib/registry.mjs'

const USAGE = 'usage: node tools/bench.mjs --library <id,...|all> [--language <id,...|all>] [--environments node,chrome,firefox,safari] [--design triplet|pairs] [--samples 24] [--warmups 2] [--min-sample-ms 20] [--seed 1] [--call-timeout 30] [--aa-range 0.5,2] [--matrix <registry>] [--receipts <dir>]'
const fail = (message) => usageError('bench.mjs', USAGE, message)
const options = parseOptions(process.argv.slice(2), {
  tool: 'bench.mjs', usage: USAGE,
  known: ['library', 'language', 'environments', 'design', 'samples', 'warmups', 'min-sample-ms', 'seed', 'call-timeout', 'aa-range', 'matrix', 'receipts'],
})
if (options.library === undefined) fail('--library is required')
// 24 samples: four whole Williams blocks for the triplet (6 orders) and twelve for pairs (2), and
// at least 20, so the reported p95 is not simply the maximum.
const samples = integerOption(options, 'samples', 24, 1, fail)
const callTimeoutMs = numberOption(options, 'call-timeout', 30, fail) * 1000
// A/A p50 outside this range refuses the cell's ratios (`aa-out-of-range`).
const aaRange = options['aa-range'] === undefined ? [0.5, 2] : String(options['aa-range']).split(',').map(Number)
if (aaRange.length !== 2 || !aaRange.every((value) => Number.isFinite(value) && value > 0) || aaRange[0] >= aaRange[1]) fail(`--aa-range must be two positive numbers low,high (got ${options['aa-range']})`)
const checkHarness = harnessIdentity('check')
const benchHarness = harnessIdentity('bench')
const warmups = integerOption(options, 'warmups', 2, 0, fail)
const minSampleMs = numberOption(options, 'min-sample-ms', 20, fail)
const seed = integerOption(options, 'seed', 1, Number.MIN_SAFE_INTEGER, fail)
const design = options.design ?? 'triplet'
if (!['triplet', 'pairs'].includes(design)) fail(`--design must be triplet or pairs (got ${design})`)
const registry = loadRegistry(options.matrix && options.matrix !== true ? options.matrix : undefined)
// Libraries without bench cases (the floor library) are not timed.
const skipped = new Set(Object.values(registry.libraries).filter((library) => !library.bench?.cases?.length).map((library) => library.id))
const ports = selectPorts(registry, { library: options.library, language: options.language ?? 'all' }, fail)
  .filter((port) => !skipped.has(port.library))
if (ports.length === 0) fail('the selection names no port of a library with bench cases')
const requested = options.environments === undefined ? null : String(options.environments).split(',').map((name) => name.trim()).filter(Boolean)
for (const name of requested || []) if (!registry.environments[name]) fail(`unknown environment ${name}`)
const paths = receiptPaths(receiptsRoot(options))
const protocolPath = resolve(harnessRoot, 'tools/lib/protocol.mjs')

const DESIGNS = {
  triplet: [{
    cell: 'triplet',
    arms: [
      { key: 'port-a', label: 'A', kind: 'port', instance: 'a' },
      { key: 'js', label: 'J', kind: 'reference' },
      { key: 'port-b', label: 'B', kind: 'port', instance: 'b' },
    ],
    comparisons: [{ name: 'port/js', of: ['port-a', 'js'] }, { name: 'A/A', of: ['port-a', 'port-b'] }, { name: 'port-b/js', of: ['port-b', 'js'] }],
  }],
  pairs: [
    { cell: 'port-vs-js', arms: [{ key: 'port-a', label: 'A', kind: 'port', instance: 'a' }, { key: 'js', label: 'J', kind: 'reference' }], comparisons: [{ name: 'port/js', of: ['port-a', 'js'] }] },
    { cell: 'a-a', arms: [{ key: 'port-a', label: 'A', kind: 'port', instance: 'a' }, { key: 'port-b', label: 'B', kind: 'port', instance: 'b' }], comparisons: [{ name: 'A/A', of: ['port-a', 'port-b'] }] },
  ],
}

const run = {
  schema: 1,
  tool: 'experiments/library-matrix/tools/bench.mjs',
  protocol: {
    version: PROTOCOL_VERSION,
    code: 'experiments/library-matrix/tools/lib/protocol.mjs',
    design,
    designs: DESIGNS[design].map(({ cell, arms, comparisons }) => ({ cell, arms: arms.map(({ key, kind }) => `${key} (${kind})`), comparisons })),
    isolation: 'fresh node process per cell (tools/bench-cell.mjs); fresh page load per cell in a browser',
    gate: 'the port has a current passing check receipt for the environment (same artifact, library binding, check code and environment major version); all arms agree on the canonical checksum, equal to the reference digest that receipt recorded in this environment, before timing; stale dist/ refused',
    aaRange: `A/A p50 outside [${aaRange.join(', ')}] refuses the cell (aa-out-of-range)`,
    inputs: 'every call receives its own structuredClone of the input, made outside the timed region',
    outputs: 'after every calibration, warmup and sample run, a seeded random call and the last call must have the gate checksum (output-mismatch otherwise)',
    calibration: `iterationsPerSample = max(baseIterations, calls needed for ${minSampleMs} ms), per arm, calibrated cold and again after the warmups (the samples use the warm value)`,
    picks: 'the verified outputs of each run are a crypto.getRandomValues-seeded pick and the last call (pickSeed recorded per cell)',
    variants: 'when the library declares bench.variant, every timed call gets a distinct variant(input, k), verified against the reference on the same k',
    watchdog: `a cell with no heartbeat for ${callTimeoutMs / 1000} s is stopped and recorded as hang`,
    order: 'Williams design per block, rows shuffled and labels permuted (mulberry32, seeded per cell)',
    unit: 'p50/p95/min/max are milliseconds per call (sample time / iterationsPerSample)',
    ratio: 'first arm / second arm of per-call p50; paired = median over rounds of the per-round per-call ratio',
  },
  configuration: { samples, warmups, minSampleMs, seed, design, aaRange, callTimeoutSeconds: callTimeoutMs / 1000 },
  registry: { path: repoPath(registry.path), sha256: registry.sha256 },
  startedAt: new Date().toISOString(),
  loadAverage: { start: os.loadavg(), end: null },
  host: { platform: process.platform, arch: process.arch, cpu: os.cpus()[0]?.model ?? null, logicalCpus: os.cpus().length },
  libraries: {},
  environments: {},
  cells: [],
}

// Per library: bench cases, their inputs and the Node reference checksum (a one-arm cell).
const libraries = {}
for (const id of [...new Set(ports.map((port) => port.library))]) {
  const library = registry.libraries[id]
  const { identity, corpus, binding } = await currentBinding(library)
  if (identity.errors.length) throw new Error(`${id} reference: ${identity.errors.join('; ')}`)
  // The gate's expected checksum is the reference's own output IN THE ENVIRONMENT BEING TIMED,
  // taken from the port's current check receipt there (parityGate): a reference may legitimately
  // produce different output per engine (MiniSearch's BM25 scores go through Math.log, which is not
  // correctly rounded and differs between V8 and SpiderMonkey), and parity is defined per
  // environment. The in-page reference arm must reproduce that digest, and all arms must agree.
  const cases = []
  for (const caseId of library.bench.cases) {
    const item = corpus.cases.find((candidate) => candidate.id === caseId)
    if (!item) throw new Error(`${id}: bench case ${caseId} is not in the corpus`)
    const inputSha256 = await digestText(inputText(item.input))
    cases.push({ id: caseId, note: item.note ?? null, input: item.input, inputSha256 })
  }
  const environments = requested || library.environments
  libraries[id] = { library, identity, cases, environments, binding }
  run.libraries[id] = {
    binding,
    reference: identity,
    corpus: corpus.record,
    output: library.output,
    normalisation: library.normalisation ?? null,
    baseIterations: library.bench.baseIterations ?? 1,
    cases: cases.map(({ id: caseId, note, inputSha256 }) => ({ id: caseId, note, inputSha256 })),
  }
}

const states = new Map(ports.map((port) => [port.key, freshness(registry, paths, port)]))

function cellConfigs(port, context, item, environment, urlFor, normalisationUrl, variantUrl = null, referenceUrl = null, expected = null) {
  return DESIGNS[design].map((spec) => ({
    library: port.library,
    port: port.id,
    case: item.id,
    design: spec.cell,
    arms: spec.arms.map((arm) => ({ key: arm.key, label: arm.label, kind: arm.kind, url: urlFor(arm) })),
    comparisons: spec.comparisons,
    input: item.input,
    expectedInputSha256: item.inputSha256,
    expectedChecksum: expected?.digest ?? null,
    expectedChecksumSource: expected?.source ?? null,
    aaRange,
    output: context.library.output,
    normalisationUrl,
    variantUrl,
    referenceUrl,
    warmups,
    samples,
    minSampleMs,
    baseIterations: context.library.bench.baseIterations ?? 1,
    seed: cellSeed(seed, environment, port.library, port.id, item.id, spec.cell),
  }))
}

function nodeCell(config) {
  return runNodeCell(resolve(harnessRoot, 'tools/bench-cell.mjs'), config, { cwd: harnessRoot, callTimeoutMs })
}

// Timing is only for ports whose parity holds: the port's check receipt for this environment must
// be a pass for the current artifact, under the current library binding and check protocol.
function parityGate(port, environmentKey, environmentMajor) {
  const path = resolve(paths.checks, `${port.key}--${environmentKey}.json`)
  const cell = existsSync(path) ? readJson(path) : null
  const artifact = artifactOf(states.get(port.key).receipt)
  const nodePath = resolve(paths.checks, `${port.key}--node.json`)
  const problem = checkReceiptProblem(cell, { artifact, binding: libraries[port.library].binding, harness: checkHarness, environmentMajor })
    ?? guardProblem(existsSync(nodePath) ? readJson(nodePath) : null, { artifact, binding: libraries[port.library].binding, harness: checkHarness })
  const receipt = cell ? { path: repoPath(path), sha256: sha256(readFileSync(path)), status: cell.status, checkedAt: cell.checkedAt ?? null, checkProtocol: cell.checkProtocol ?? null, environment: cell.environmentIdentity?.label ?? null } : null
  if (problem) return { refusal: { status: 'parity-not-passed', reason: `${problem.status}: ${problem.reason}`, parity: problem }, receipt, expected: {} }
  // This environment's reference digest for every bench case, from the same receipt.
  const expected = {}
  for (const item of libraries[port.library].cases) {
    const recorded = cell.cases?.find((candidate) => candidate.id === item.id)?.referenceDigest
    if (!recorded) {
      const reason = `the check receipt records no reference digest for bench case ${item.id}`
      return { refusal: { status: 'parity-not-passed', reason, parity: { status: 'not-checked', reason } }, receipt, expected: {} }
    }
    expected[item.id] = { digest: recorded, source: `reference digest of case ${item.id} in ${receipt.path} (${receipt.environment ?? environmentKey})` }
  }
  return { refusal: null, receipt, expected }
}

function record(port, environmentKey, environment, config, result, loadAverage, refused = null, gate = null) {
  const state = states.get(port.key)
  const artifact = artifactOf(state.receipt)
  const cell = {
    schema: 1,
    benchProtocol: PROTOCOL_VERSION,
    harness: benchHarness,
    node: process.version,
    binding: libraries[port.library].binding,
    checkReceipt: gate?.receipt ?? null,
    library: port.library,
    port: port.id,
    language: port.language,
    environment: environmentKey,
    environmentIdentity: environment,
    case: config?.case ?? null,
    design: config?.design ?? null,
    measuredAt: new Date().toISOString(),
    status: refused ? refused.status : result.status,
    reason: refused ? refused.reason : (result.error?.message ?? (result.mismatch ? `output-mismatch in ${result.mismatch.arm} ${result.mismatch.phase} ${result.mismatch.round} (${result.mismatch.output})` : null)),
    parity: refused?.parity ?? null,
    artifact,
    loadAverage,
    result: refused ? null : result,
  }
  const name = `${port.key}--${environmentKey}--${cell.case ?? 'all'}--${cell.design ?? design}.json`
  writeJson(resolve(paths.bench, port.library, name), cell)
  run.cells.push({ ...cell, receipt: repoPath(resolve(paths.bench, port.library, name)) })
  // A cell that is not `ok` (aa-out-of-range included) has no ratio to show.
  const ratio = (label) => (cell.status === 'ok' ? result?.comparisons?.find((comparison) => comparison.name === label) : null)
  const parts = [`${environmentKey}: ${port.key} ${cell.case ?? ''} ${cell.design ?? ''} ${cell.status}`]
  if (ratio('port/js')) parts.push(`port/js p50 ${ratio('port/js').p50.toFixed(3)}`)
  if (ratio('A/A')) parts.push(`A/A ${ratio('A/A').p50.toFixed(3)}`)
  if (cell.status === 'aa-out-of-range') parts.push(`(A/A p50 ${result.aaOutOfRange.p50.toFixed(3)} outside [${result.aaOutOfRange.range.join(', ')}]; no ratio)`)
  else if (cell.reason && cell.status !== 'ok') parts.push(`(${String(cell.reason).split('\n')[0].slice(0, 200)})`)
  process.stdout.write(`${parts.join('  ')}\n`)
}

function refusedCells(port, environmentKey, environment, refusal, gate = null) {
  const context = libraries[port.library]
  for (const item of context.cases) for (const spec of DESIGNS[design]) {
    record(port, environmentKey, environment, { case: item.id, design: spec.cell }, null, null, refusal, gate)
  }
}

// Node.
const nodeIdentity = { name: 'node', version: process.version, label: `node ${process.version}`, v8: process.versions.v8, isolation: 'fresh node process per cell' }
if (ports.some((port) => libraries[port.library].environments.includes('node'))) {
  run.environments.node = { status: 'ok', environment: nodeIdentity }
  for (const port of ports) {
    const context = libraries[port.library]
    if (!context.environments.includes('node')) continue
    const state = states.get(port.key)
    if (!state.fresh) {
      refusedCells(port, 'node', nodeIdentity, { status: state.status, reason: state.reasons.join('; ') })
      continue
    }
    const gate = parityGate(port, 'node', majorVersion(nodeIdentity))
    if (gate.refusal) {
      refusedCells(port, 'node', nodeIdentity, gate.refusal, gate)
      continue
    }
    const artifactUrl = pathToFileURL(resolve(port.dir, 'dist/index.min.js')).href
    const referenceUrl = pathToFileURL(context.library.referencePath).href
    for (const item of context.cases) {
      const normalisationUrl = context.library.normalisationPath ? pathToFileURL(context.library.normalisationPath).href : null
      const variantUrl = context.library.benchVariantPath ? pathToFileURL(context.library.benchVariantPath).href : null
      for (const config of cellConfigs(port, context, item, 'node', (arm) => (arm.kind === 'port' ? `${artifactUrl}?instance=${arm.instance}` : referenceUrl), normalisationUrl, variantUrl, referenceUrl, gate.expected[item.id])) {
        const loadStart = os.loadavg()
        const result = await nodeCell(config)
        record(port, 'node', nodeIdentity, config, result, { start: loadStart, end: os.loadavg() }, null, gate)
      }
    }
  }
}

// Browsers.
const browsers = [...new Set(ports.flatMap((port) => libraries[port.library].environments))].filter((name) => registry.environments[name].kind === 'browser')
if (browsers.length) {
  const files = new Set([protocolPath, resolve(harnessRoot, 'tools/lib/canonical.mjs')])
  for (const context of Object.values(libraries)) {
    for (const module of [...context.identity.modules, ...context.identity.normalisationModules, ...context.identity.benchVariantModules]) files.add(resolve(repositoryRoot, module.path))
  }
  for (const port of ports) files.add(resolve(port.dir, 'dist/index.min.js'))
  const server = await startServer([...files])
  let load = 0
  try {
    for (const browser of browsers) {
      const targets = ports.filter((port) => libraries[port.library].environments.includes(browser))
      let session = await openBrowser(registry.environments[browser].webdriver, { scriptTimeoutMs: 1_800_000 })
      if (session.error) {
        run.environments[browser] = { status: 'unavailable', reason: session.error }
        process.stdout.write(`${browser}: timing unavailable: ${session.error.split('\n')[0]}\n`)
        for (const port of targets) refusedCells(port, browser, null, { status: 'unavailable', reason: session.error })
        continue
      }
      run.environments[browser] = { status: 'ok', environment: { ...session.environment, isolation: 'fresh page load per cell' } }
      process.stdout.write(`${browser}: timing in ${session.environment.label}\n`)
      try {
        for (const port of targets) {
          const state = states.get(port.key)
          if (!state.fresh) {
            refusedCells(port, browser, run.environments[browser].environment, { status: state.status, reason: state.reasons.join('; ') })
            continue
          }
          if (session.error) {
            refusedCells(port, browser, run.environments[browser].environment, { status: 'unavailable', reason: `the session could not be reopened after a hang: ${session.error}` })
            continue
          }
          const gate = parityGate(port, browser, majorVersion(session.environment))
          if (gate.refusal) {
            refusedCells(port, browser, run.environments[browser].environment, gate.refusal, gate)
            continue
          }
          const context = libraries[port.library]
          for (const item of context.cases) {
            for (const spec of DESIGNS[design]) {
              if (session.error) {
                record(port, browser, run.environments[browser].environment, { case: item.id, design: spec.cell }, null, null, { status: 'unavailable', reason: `the session could not be reopened after a hang: ${session.error}` }, gate)
                continue
              }
              load++
              const query = `?load=${load}`
              const [config] = cellConfigs(port, context, item, browser, (arm) => (arm.kind === 'port'
                ? server.url(resolve(port.dir, 'dist/index.min.js'), `${query}&instance=${arm.instance}`)
                : server.url(context.library.referencePath, query)),
              context.library.normalisationPath ? server.url(context.library.normalisationPath, query) : null,
              context.library.benchVariantPath ? server.url(context.library.benchVariantPath, query) : null,
              server.url(context.library.referencePath, query), gate.expected[item.id]).filter((candidate) => candidate.design === spec.cell)
              const loadStart = os.loadavg()
              const { result, sessionLost } = await runWatched(session, server, load, server.url(protocolPath, query), 'runCell', config, { callTimeoutMs })
              if (result?.cspViolations?.length && result.status === 'ok') {
                result.status = 'runtime-guard'
                result.error = { message: `CSP refused ${result.cspViolations.map((item) => `${item.directive} ${item.blocked}`).join('; ')}` }
              }
              record(port, browser, run.environments[browser].environment, config, result, { start: loadStart, end: os.loadavg() }, null, gate)
              if (sessionLost) session = await openBrowser(registry.environments[browser].webdriver, { scriptTimeoutMs: 1_800_000 })
            }
          }
        }
      } finally {
        await session.driver?.quit().catch(() => {})
      }
    }
  } finally {
    await server.close()
  }
}

run.loadAverage.end = os.loadavg()
run.finishedAt = new Date().toISOString()

// Rows: one per port, environment and case, with port/js and A/A side by side.
run.rows = []
for (const port of ports) {
  for (const environment of libraries[port.library].environments) {
    for (const item of libraries[port.library].cases) {
      const cells = run.cells.filter((cell) => cell.port === port.id && cell.library === port.library && cell.environment === environment && cell.case === item.id)
      if (!cells.length) continue
      const comparison = (name) => cells.filter((cell) => cell.status === 'ok').map((cell) => cell.result?.comparisons?.find((candidate) => candidate.name === name)).find(Boolean) ?? null
      const arm = (key) => cells.map((cell) => cell.result?.arms?.find((candidate) => candidate.key === key)).find(Boolean) ?? null
      run.rows.push({
        library: port.library,
        port: port.id,
        language: port.language,
        environment,
        case: item.id,
        status: cells.every((cell) => cell.status === 'ok') ? 'ok' : cells.map((cell) => cell.status).find((status) => status !== 'ok'),
        artifactSha256: cells[0].artifact?.sha256 ?? null,
        portP50: arm('port-a')?.perCallMs?.p50 ?? null,
        portP95: arm('port-a')?.perCallMs?.p95 ?? null,
        jsP50: arm('js')?.perCallMs?.p50 ?? null,
        jsP95: arm('js')?.perCallMs?.p95 ?? null,
        ratioP50: comparison('port/js')?.p50 ?? null,
        ratioP95: comparison('port/js')?.p95 ?? null,
        p95IsMax: comparison('port/js')?.p95IsMax ?? null,
        portBRatioP50: comparison('port-b/js')?.p50 ?? null,
        pairedP50: comparison('port/js')?.paired?.p50 ?? null,
        aaP50: comparison('A/A')?.p50 ?? null,
        aaPairedP50: comparison('A/A')?.paired?.p50 ?? null,
        iterationsPerSample: { port: arm('port-a')?.iterationsPerSample ?? null, js: arm('js')?.iterationsPerSample ?? null },
        coldImportMs: { port: arm('port-a')?.coldImportMs ?? null, js: arm('js')?.coldImportMs ?? null },
        loadAverage: cells.map((cell) => cell.loadAverage).filter(Boolean),
      })
    }
  }
}
for (const id of Object.keys(libraries)) {
  writeJson(resolve(paths.bench, `${id}.summary.json`), {
    ...run,
    libraries: { [id]: run.libraries[id] },
    cells: run.cells.filter((cell) => cell.library === id).map(({ result, ...cell }) => ({ ...cell, comparisons: result?.comparisons ?? null })),
    rows: run.rows.filter((row) => row.library === id),
  })
}

const number = (value, digits = 3) => (typeof value === 'number' && Number.isFinite(value) ? value.toFixed(digits) : '-')
const header = ['environment', 'port', 'case', 'port p50', 'js p50', 'port/js', 'paired', 'A/A', 'status']
const lines = [header, ...run.rows.map((row) => [row.environment, row.port, row.case, number(row.portP50, 4), number(row.jsP50, 4),
  number(row.ratioP50, 2), number(row.pairedP50, 2), number(row.aaP50, 3), row.status])]
const widths = header.map((_, column) => Math.max(...lines.map((line) => String(line[column]).length)))
process.stdout.write(`\ntiming (ms per call; port/js = p50/p50; paired = median per-round ratio; A/A = port-a/port-b p50)\n`)
for (const line of lines) process.stdout.write(`${line.map((value, column) => String(value).padEnd(widths[column])).join('  ').trimEnd()}\n`)
for (const [name, environment] of Object.entries(run.environments)) if (environment.status !== 'ok') process.stdout.write(`${name}: ${environment.status} (${environment.reason.split('\n')[0]})\n`)
process.stdout.write(`load average start ${run.loadAverage.start.map((value) => value.toFixed(2)).join(' ')}, end ${run.loadAverage.end.map((value) => value.toFixed(2)).join(' ')}\n`)
process.stdout.write('These numbers are not a performance claim: read every ratio next to its A/A and the load average.\n')
const bad = run.cells.filter((cell) => cell.status !== 'ok' && cell.status !== 'unavailable')
process.exitCode = bad.length ? 1 : 0
