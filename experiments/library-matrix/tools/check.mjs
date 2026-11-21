#!/usr/bin/env node
// Exact-parity checker: every corpus case through the port's production artifact and through the
// JS reference, compared by canonical form (tools/lib/canonical.mjs) with no normalisation unless
// the library's registry entry names one. One cell = one port in one environment: a fresh Node
// process (tools/check-cell.mjs) or a fresh page load in a WebDriver session. One receipt per cell.
//
//   node tools/check.mjs --library <id,...|all> [--language <id,...|all>]
//     [--environments node,chrome,firefox,safari] [--readable] [--matrix <registry>] [--receipts <dir>]
//
// Environments default to the library's registry list. A port whose dist/ is not the output of
// its current build receipt is `stale` (or `not-built` / `build-failed`) and is not run. A browser
// whose WebDriver session cannot start is `unavailable` with the driver's error, never a pass.
// --readable also checks dist/index.js. Exit status: 1 when any cell is fail, error, stale,
// not-built or build-failed; unavailable cells are reported, not failed.

import os from 'node:os'
import { existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { BOOTSTRAP, HEARTBEAT, PAGE, openBrowser, runWatched, startServer } from './lib/browser.mjs'
import { runNodeCell } from './lib/cells.mjs'
import { inputText, digestText } from './lib/canonical.mjs'
import { CHECK_PROTOCOL } from './lib/check-runner.mjs'
import {
  artifactOf, currentBinding, fileRecord, freshness, harnessIdentity, harnessRoot, loadRegistry, numberOption, parseOptions, receiptPaths,
  receiptsRoot, repoPath, repositoryRoot, selectPorts, usageError, writeJson,
} from './lib/registry.mjs'

const USAGE = 'usage: node tools/check.mjs --library <id,...|all> [--language <id,...|all>] [--environments node,chrome,firefox,safari] [--readable] [--call-timeout 30] [--matrix <registry>] [--receipts <dir>]'
const fail = (message) => usageError('check.mjs', USAGE, message)
const options = parseOptions(process.argv.slice(2), { tool: 'check.mjs', usage: USAGE, known: ['library', 'language', 'environments', 'readable', 'call-timeout', 'matrix', 'receipts'] })
// A call that makes no progress for this long (a hung port) fails its cell instead of blocking it.
const callTimeoutMs = numberOption(options, 'call-timeout', 30, fail) * 1000
const harness = harnessIdentity('check')
if (options.library === undefined) fail('--library is required')
const registry = loadRegistry(options.matrix && options.matrix !== true ? options.matrix : undefined)
const ports = selectPorts(registry, { library: options.library, language: options.language ?? 'all' }, fail)
if (ports.length === 0) fail('the selection names no port')
const requested = options.environments === undefined ? null : String(options.environments).split(',').map((name) => name.trim()).filter(Boolean)
for (const name of requested || []) if (!registry.environments[name]) fail(`unknown environment ${name} (known: ${Object.keys(registry.environments).join(', ')})`)
const artifacts = options.readable ? ['index.min.js', 'index.js'] : ['index.min.js']
const paths = receiptPaths(receiptsRoot(options))
const runnerPath = resolve(harnessRoot, 'tools/lib/check-runner.mjs')
const startedAt = new Date().toISOString()

// Per library: reference identity, corpus, normalisation, environments.
const libraries = {}
for (const id of [...new Set(ports.map((port) => port.library))]) {
  const library = registry.libraries[id]
  const { identity, corpus, binding } = await currentBinding(library)
  let normalisation = null
  if (library.normalisation) {
    normalisation = { name: library.normalisation.name, description: library.normalisation.description, module: fileRecord(library.normalisationPath) }
  }
  const inputs = {}
  for (const item of corpus.cases) inputs[item.id] = await digestText(inputText(item.input))
  const environments = requested || library.environments
  libraries[id] = { library, corpus, identity, normalisation, inputs, environments, binding }
}

const cells = []
const portStates = new Map(ports.map((port) => [port.key, freshness(registry, paths, port)]))

// Node: one fresh process per port.
for (const port of ports) {
  const context = libraries[port.library]
  if (!context.environments.includes('node')) continue
  for (const artifactName of artifacts) {
    cells.push(await nodeCell(port, context, artifactName))
  }
}

// Browsers: one WebDriver session per browser, one fresh page load per port.
const browserNames = [...new Set(Object.values(libraries).flatMap((context) => context.environments))]
  .filter((name) => registry.environments[name].kind === 'browser')
if (browserNames.length) {
  const files = new Set([runnerPath, resolve(harnessRoot, 'tools/lib/canonical.mjs')])
  for (const context of Object.values(libraries)) {
    for (const module of [...context.identity.modules, ...context.identity.normalisationModules]) files.add(resolve(repositoryRoot, module.path))
    files.add(context.library.corpusPath)
  }
  for (const port of ports) for (const name of artifacts) files.add(resolve(port.dir, 'dist', name))
  const server = await startServer([...files].filter((path) => existsSync(path)))
  let load = 0
  try {
    for (const browser of browserNames) {
      const targets = ports.filter((port) => libraries[port.library].environments.includes(browser))
      if (targets.length === 0) continue
      let session = await openBrowser(registry.environments[browser].webdriver)
      if (session.error) {
        process.stdout.write(`${browser}: unavailable: ${session.error.split('\n')[0]}\n`)
        for (const port of targets) for (const artifactName of artifacts) {
          cells.push(writeCell(port, libraries[port.library], browser, artifactName, { status: 'unavailable', reason: session.error }))
        }
        continue
      }
      process.stdout.write(`${browser}: ${session.environment.label} (driver ${session.environment.driver})\n`)
      try {
        for (const port of targets) for (const artifactName of artifacts) {
          load++
          if (session.error) {
            cells.push(writeCell(port, libraries[port.library], browser, artifactName, { status: 'unavailable', reason: `the session could not be reopened after a hang: ${session.error}` }))
            continue
          }
          const { cell, sessionLost } = await browserCell(port, libraries[port.library], browser, artifactName, session, server, load)
          cells.push(cell)
          if (sessionLost) session = await openBrowser(registry.environments[browser].webdriver)
        }
      } finally {
        await session.driver?.quit().catch(() => {})
      }
    }
  } finally {
    await server.close()
  }
}

// Summary per library, with the reference's own digests compared across environments.
const summaries = {}
for (const [id, context] of Object.entries(libraries)) {
  const libraryCells = cells.filter((cell) => cell.library === id)
  const referenceDigests = {}
  for (const cell of libraryCells) {
    for (const item of cell.cases || []) {
      referenceDigests[item.id] ||= {}
      referenceDigests[item.id][cell.environment] = item.referenceDigest
    }
  }
  const referenceStable = Object.values(referenceDigests).every((byEnvironment) => new Set(Object.values(byEnvironment)).size <= 1)
  summaries[id] = {
    schema: 1,
    tool: 'experiments/library-matrix/tools/check.mjs',
    library: id,
    startedAt,
    finishedAt: new Date().toISOString(),
    registry: { path: repoPath(registry.path), sha256: registry.sha256 },
    environments: context.environments,
    artifacts,
    checkProtocol: CHECK_PROTOCOL,
    harness,
    node: process.version,
    binding: context.binding,
    reference: context.identity,
    corpus: { ...context.corpus.record, cases: context.corpus.cases.length },
    normalisation: context.normalisation,
    referenceStableAcrossEnvironments: referenceStable,
    cells: libraryCells.map((cell) => ({
      port: cell.port, language: cell.language, environment: cell.environment, artifact: cell.artifactName,
      status: cell.status, summary: cell.summary ?? null, reason: cell.reason ?? null, receipt: cell.receiptPath,
      artifactSha256: cell.artifact?.sha256 ?? null,
    })),
  }
  writeJson(resolve(paths.checks, `${id}.summary.json`), summaries[id])
}

// Table: one row per port, one column per environment.
for (const [id, summary] of Object.entries(summaries)) {
  for (const artifactName of artifacts) {
    process.stdout.write(`\n${id} (${artifactName})\n`)
    const header = ['port', ...summary.environments]
    const rows = [header]
    for (const port of ports.filter((candidate) => candidate.library === id)) {
      rows.push([port.id, ...summary.environments.map((environment) => {
        const cell = summary.cells.find((candidate) => candidate.port === port.id && candidate.environment === environment && candidate.artifact === artifactName)
        if (!cell) return '-'
        return cell.summary && cell.status !== 'unavailable' ? `${cell.status} ${cell.summary.equal}/${cell.summary.cases}` : cell.status
      })])
    }
    const widths = header.map((_, column) => Math.max(...rows.map((row) => String(row[column]).length)))
    for (const row of rows) process.stdout.write(`${row.map((value, column) => String(value).padEnd(widths[column])).join('  ')}\n`)
    for (const environment of summary.environments) {
      const environmentCells = summary.cells.filter((cell) => cell.environment === environment && cell.artifact === artifactName)
      const passed = environmentCells.filter((cell) => cell.status === 'pass').length
      const unavailable = environmentCells.filter((cell) => cell.status === 'unavailable').length
      process.stdout.write(unavailable === environmentCells.length
        ? `${environment}: unavailable for all ${unavailable} cells (the WebDriver session could not start; not a pass)\n`
        : `${environment}: ${passed}/${environmentCells.length} exact parity${unavailable ? `, ${unavailable} unavailable` : ''}\n`)
    }
    process.stdout.write(`reference stable across environments: ${summary.referenceStableAcrossEnvironments}\n`)
  }
}
const failed = cells.filter((cell) => ['fail', 'error', 'hang', 'stale', 'not-built', 'build-failed'].includes(cell.status))
process.exitCode = failed.length ? 1 : 0

function cellConfig(port, context, environmentKey) {
  return {
    library: port.library,
    port: port.id,
    environmentKey,
    output: context.library.output,
    guard: { network: context.library.runtimeGuard?.network === 'allowed' ? 'allowed' : 'blocked' },
    // The family's documented runtime globals (registry languages.<id>.runtimeGlobals).
    allowedGlobals: registry.languages[port.language].runtimeGlobals ?? [],
  }
}

async function nodeCell(port, context, artifactName) {
  const state = portStates.get(port.key)
  if (!state.fresh) return writeCell(port, context, 'node', artifactName, { status: state.status, reason: state.reasons.join('; '), state })
  if (context.identity.errors.length) return writeCell(port, context, 'node', artifactName, { status: 'error', reason: `reference: ${context.identity.errors.join('; ')}` })
  const config = {
    ...cellConfig(port, context, 'node'),
    referenceUrl: pathToFileURL(context.library.referencePath).href,
    portUrl: pathToFileURL(resolve(port.dir, 'dist', artifactName)).href,
    normalisationUrl: context.library.normalisationPath ? pathToFileURL(context.library.normalisationPath).href : null,
    cases: context.corpus.cases.map(({ id, input }) => ({ id, input })),
  }
  const loadStart = os.loadavg()
  const result = await runNodeCell(resolve(harnessRoot, 'tools/check-cell.mjs'), config, { cwd: harnessRoot, callTimeoutMs })
  const environment = { name: 'node', version: process.version, label: `node ${process.version}`, v8: process.versions.v8, isolation: 'fresh node process per cell (tools/check-cell.mjs)' }
  return writeCell(port, context, 'node', artifactName, { result, environment, loadAverage: { start: loadStart, end: os.loadavg() } })
}

async function browserCell(port, context, browser, artifactName, session, server, load) {
  const state = portStates.get(port.key)
  if (!state.fresh) return { cell: writeCell(port, context, browser, artifactName, { status: state.status, reason: state.reasons.join('; '), state }), sessionLost: false }
  if (context.identity.errors.length) return { cell: writeCell(port, context, browser, artifactName, { status: 'error', reason: `reference: ${context.identity.errors.join('; ')}` }), sessionLost: false }
  const query = `?load=${load}`
  const config = {
    ...cellConfig(port, context, browser),
    referenceUrl: server.url(context.library.referencePath, query),
    portUrl: server.url(resolve(port.dir, 'dist', artifactName), query),
    normalisationUrl: context.library.normalisationPath ? server.url(context.library.normalisationPath, query) : null,
    casesUrl: server.url(context.library.corpusPath, query),
  }
  const loadStart = os.loadavg()
  const { result, requests, sessionLost } = await runWatched(session, server, load, server.url(runnerPath, query), 'runCheck', config, { callTimeoutMs })
  // Request accounting: during the cell the page may fetch only the files this cell needs. An
  // import() of a module the page already loaded makes no request and is not visible here
  // (README "Runtime guard"); the Node cell's resolve hook sees those.
  const allowed = new Set([PAGE, HEARTBEAT, '/favicon.ico', server.servedPath(BOOTSTRAP), server.servedPath(runnerPath), server.servedPath(resolve(harnessRoot, 'tools/lib/canonical.mjs')),
    server.servedPath(context.library.corpusPath), server.servedPath(resolve(port.dir, 'dist', artifactName)),
    ...[...context.identity.modules, ...context.identity.normalisationModules].map((module) => `/${module.path}`)])
  const unexpected = [...new Set(requests.map((request) => request.path).filter((path) => !allowed.has(path)))]
  if (result && result.guard && unexpected.length) result.guard.unexpectedRequests = unexpected
  if (result && result.guard && result.cspViolations?.length) result.guard.cspViolations = result.cspViolations
  const cell = writeCell(port, context, browser, artifactName, {
    result,
    environment: { ...session.environment, isolation: 'fresh page load per cell' },
    loadAverage: { start: loadStart, end: os.loadavg() },
  })
  return { cell, sessionLost }
}

function writeCell(port, context, environmentKey, artifactName, { result, status, reason, environment = null, loadAverage = null, state = null }) {
  const build = state?.receipt ?? portStates.get(port.key).receipt
  const artifact = artifactOf(build, artifactName)
  let cellStatus = status
  let cellReason = reason ?? null
  const violations = result?.guard
    ? [...(result.guard.violations ?? []).map((name) => `${name} called`), ...(result.guard.moduleLoads ?? []).map((load) => `module load ${load.specifier}`),
      ...(result.guard.unexpectedRequests ?? []).map((path) => `request ${path}`),
      ...(result.guard.cspViolations ?? []).map((item) => `CSP refused ${item.directive} ${item.blocked}`),
      ...(result.guard.newGlobals ?? []).map((name) => `new global ${name}`),
      ...(result.guard.unhandled ?? []).map((message) => `unhandled asynchronous error: ${message}`)]
    : []
  if (!cellStatus) {
    if (result.status === 'hang') {
      cellStatus = 'hang'
      cellReason = result.error?.message ?? 'no progress'
    } else if (violations.length) {
      // The runtime guard saw the port load a module or touch the network: the port is not
      // self-contained, whatever its outputs.
      cellStatus = 'fail'
      cellReason = `runtime guard: ${[...new Set(violations)].join('; ')}`
    } else if (result.status === 'error') {
      cellStatus = 'error'
      cellReason = result.error?.message ?? 'cell error'
    } else {
      // Every case must have run and be equal, and the cell must have seen the whole corpus.
      const ran = result.cases.length === context.corpus.cases.length
      const inputsMatch = result.cases.every((item, index) => item.id === context.corpus.cases[index].id)
      cellStatus = ran && inputsMatch && result.summary.equal === result.summary.cases ? 'pass' : 'fail'
      if (cellStatus === 'fail') {
        const bad = result.cases.filter((item) => item.status !== 'equal').map((item) => `${item.id}: ${item.status}${item.failedStep && item.failedStep !== 'first' ? ` (${item.failedStep} call: ${item.reason ?? 'output differs'})` : ''}`)
        cellReason = ran ? bad.join('; ') : `ran ${result.cases.length} of ${context.corpus.cases.length} cases`
      }
    }
  }
  const suffix = artifactName === 'index.min.js' ? '' : '.readable'
  const receiptPath = resolve(paths.checks, `${port.key}--${environmentKey}${suffix}.json`)
  const receipt = {
    schema: 1,
    tool: 'experiments/library-matrix/tools/check.mjs',
    library: port.library,
    port: port.id,
    language: port.language,
    environment: environmentKey,
    environmentIdentity: environment,
    artifactName,
    status: cellStatus,
    reason: cellReason,
    checkProtocol: CHECK_PROTOCOL,
    harness,
    node: process.version,
    binding: context.binding,
    guard: result?.guard ?? null,
    checkedAt: new Date().toISOString(),
    loadAverage,
    artifact: artifact ? { ...artifact } : null,
    build: build ? { receipt: repoPath(resolve(paths.builds, `${port.key}.json`)), startedAt: build.startedAt, built: build.built } : null,
    reference: { adapter: context.identity.adapter, modules: context.identity.modules, npm: context.identity.npm ?? null, standIn: context.identity.standIn ?? null },
    corpus: { ...context.corpus.record, cases: context.corpus.cases.length, inputDigests: context.inputs },
    output: context.library.output,
    normalisation: context.normalisation,
    comparison: 'exact equality of canonical texts (tools/lib/canonical.mjs) and of the input state after each call, over the call sequence first / repeat / rewritten (tools/lib/check-runner.mjs)',
    summary: result?.summary ?? null,
    runtime: result?.environment ?? null,
    error: result?.status === 'error' ? result.error : null,
    cases: result?.cases ?? [],
  }
  writeJson(receiptPath, receipt)
  const line = receipt.summary ? ` ${receipt.summary.equal}/${receipt.summary.cases} equal` : ''
  process.stdout.write(`${environmentKey}: ${port.key}${suffix} ${cellStatus}${line}${cellReason && cellStatus !== 'pass' ? ` (${String(cellReason).split('\n')[0].slice(0, 200)})` : ''}\n`)
  return { ...receipt, receiptPath: repoPath(receiptPath) }
}
