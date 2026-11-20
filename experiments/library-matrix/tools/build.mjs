#!/usr/bin/env node
// Builds ports with their language builders and writes one receipt per port, following the
// receipt pattern of experiments/tools/run-comparator-builds.py: hash the build inputs before and
// after, move the old dist/ aside so it can never pass for fresh output, record every artifact's
// digest, record failures and timeouts as such, and keep the previous receipt and log.
//
//   node tools/build.mjs --library <id,...|all> --language <id,...|all>
//     [--clean] [--timeout 1200] [--matrix matrix.json] [--receipts build/library-matrix]
//
// Work directories live under <receipts>/work/<library>/<port>, so --receipts relocates them too.
//
// --clean deletes each port's work directory first, so nothing cached there (Dune _build, the
// Gradle project's build/ and .gradle/, scala-cli .scala-build, elm-stuff, .dart_tool) carries
// over; global toolchain caches (~/.gradle, ~/.elm, GOCACHE, the opam switch) stay.

import { spawn } from 'node:child_process'
import { copyFileSync, createWriteStream, existsSync, mkdirSync, readFileSync, renameSync, rmSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import {
  buildReceiptPath, harnessIdentity, harnessRoot, integerOption, loadRegistry, parseOptions, readJson, receiptPaths, receiptsRoot, repoPath,
  repositoryRoot, selectPorts, sha256, sourceInventory, usageError, walkFiles, writeJson,
} from './lib/registry.mjs'

import { hasComponents, compositeBuilder } from './lib/composite.mjs'

const USAGE = 'usage: node tools/build.mjs --library <id,...|all> --language <id,...|all> [--clean] [--timeout 1200] [--matrix <registry>] [--receipts <dir>]'
const fail = (message) => usageError('build.mjs', USAGE, message)
const options = parseOptions(process.argv.slice(2), { tool: 'build.mjs', usage: USAGE, known: ['library', 'language', 'clean', 'timeout', 'matrix', 'receipts'] })
if (options.clean !== undefined && options.clean !== true) fail('--clean takes no value')
if (options.library === undefined || options.language === undefined) fail('--library and --language are required')
const timeoutSeconds = integerOption(options, 'timeout', 1200, 1, fail)
const registry = loadRegistry(options.matrix && options.matrix !== true ? options.matrix : undefined)
const ports = selectPorts(registry, { library: options.library, language: options.language }, fail)
if (ports.length === 0) fail('the selection names no port')
const paths = receiptPaths(receiptsRoot(options))
for (const dir of [paths.builds, paths.logs, paths.attempts, paths.previousDist]) mkdirSync(dir, { recursive: true })
const { moduleShape } = await import(pathToFileURL(resolve(harnessRoot, 'languages/common.mjs')))

// The environment of the comparator runner, so builders see the same toolchains.
const env = { ...process.env }
env.PATH = [resolve(repositoryRoot, 'node_modules/.bin'), resolve(process.env.HOME || '', '.local/bin'), env.PATH || ''].join(':')
env.MIRES_OPAM_ROOT ||= resolve(repositoryRoot, '.toolchains/opam')
env.MIRES_OPAM_SWITCH ||= 'mires-comparators'
env.KOTLIN_BIN ||= resolve(repositoryRoot, 'experiments/tools/kotlin')
env.DART ||= 'dart'
env.DUNE_JOBS ||= '4'
env.MIRES_GO_ROOT ||= resolve(repositoryRoot, '.toolchains/go')
env.COURSIER_REPOSITORIES ||= 'https://repo.maven.apache.org/maven2'
delete env.CARGO_INCREMENTAL

const results = []
for (const port of ports) results.push(await buildPort(port))

process.stdout.write('\nport                               status   seconds  identical-to-previous  index.min.js\n')
for (const result of results) {
  const min = result.artifacts.find((artifact) => artifact.path.endsWith('/dist/index.min.js'))
  process.stdout.write(`${result.port.padEnd(34)} ${(result.built ? 'PASS' : 'FAIL').padEnd(8)} ${String(result.seconds).padStart(7)}  ${String(result.reproducibility.identical ?? `n/a (${result.reproducibility.reason})`).padEnd(21)}  ${min ? `${min.bytes} bytes ${min.sha256.slice(0, 12)}` : '-'}\n`)
}
const passed = results.filter((result) => result.built).length
process.stdout.write(`${passed}/${results.length} builds PASS\n`)
process.exitCode = passed === results.length ? 0 : 1

async function buildPort(port) {
  const language = registry.languages[port.language]
  const receiptPath = buildReceiptPath(paths, port)
  const logPath = resolve(paths.logs, `${port.key}.build.log`)
  const stamp = new Date().toISOString().replace(/[:.]/g, '-')
  const previous = existsSync(receiptPath) ? readJson(receiptPath) : null
  if (existsSync(receiptPath) || existsSync(logPath)) {
    const attempt = resolve(paths.attempts, port.key, stamp)
    mkdirSync(attempt, { recursive: true })
    if (existsSync(receiptPath)) copyFileSync(receiptPath, resolve(attempt, 'build.json'))
    if (existsSync(logPath)) copyFileSync(logPath, resolve(attempt, 'build.log'))
  }
  const workDir = resolve(paths.work, port.library, port.id)
  if (options.clean) rmSync(workDir, { recursive: true, force: true })
  const buildInfoPath = resolve(workDir, 'build-info.json')
  rmSync(buildInfoPath, { force: true })
  const sourceBefore = sourceInventory(registry, port)

  // Old output stays identifiable under previous-dist/ but cannot be mistaken for this build's.
  const distDir = resolve(port.dir, 'dist')
  let movedAside = null
  if (existsSync(distDir)) {
    const aside = resolve(paths.previousDist, port.library, port.id)
    rmSync(aside, { recursive: true, force: true })
    mkdirSync(resolve(aside, '..'), { recursive: true })
    renameSync(distDir, aside)
    movedAside = repoPath(aside)
  }

  const command = hasComponents(port)
    ? [process.execPath, compositeBuilder, '--port', port.dir, '--work', workDir, '--matrix', registry.path, '--receipts', paths.root]
    : [process.execPath, language.builderPath, '--port', port.dir, '--work', workDir]
  const startedAt = new Date().toISOString()
  const started = process.hrtime.bigint()
  process.stdout.write(`BUILD ${port.key} (${language.label})\n`)
  const { exitCode, timedOut, signal } = await runLogged(command, logPath, timeoutSeconds)
  const seconds = Number((Number(process.hrtime.bigint() - started) / 1e9).toFixed(3))

  const artifacts = walkFiles(distDir).map((path) => {
    const bytes = readFileSync(path)
    return { path: repoPath(path), bytes: bytes.length, sha256: sha256(bytes) }
  })
  const sourceAfter = sourceInventory(registry, port)
  const shapes = {}
  for (const name of ['index.min.js', 'index.js']) {
    const path = resolve(distDir, name)
    if (existsSync(path)) {
      try {
        shapes[name] = await moduleShape(readFileSync(path, 'utf8'))
      } catch (error) {
        shapes[name] = { ok: false, error: String(error.message || error) }
      }
    }
  }
  const meta = existsSync(resolve(distDir, 'meta.json')) ? readJson(resolve(distDir, 'meta.json')) : null
  const buildInfo = existsSync(buildInfoPath) ? readJson(buildInfoPath) : null
  const checks = {
    exitedZero: exitCode === 0 && !timedOut,
    productionArtifact: Boolean(shapes['index.min.js']),
    moduleShape: Object.values(shapes).length > 0 && Object.values(shapes).every((shape) => shape.ok),
    sourceUnchanged: sourceBefore.digest === sourceAfter.digest,
    metaWritten: meta !== null,
  }
  const built = Object.values(checks).every(Boolean)
  const comparable = previous?.built && previous.sourceAfter?.digest === sourceAfter.digest
  const reproducibility = !built
    ? { identical: null, reason: 'this build failed' }
    : !previous ? { identical: null, reason: 'no previous build' }
      : !comparable ? { identical: null, reason: previous.built ? 'build inputs changed' : 'previous build failed' }
        : {
            identical: JSON.stringify(previous.artifacts.map(({ path, sha256 }) => [path, sha256])) === JSON.stringify(artifacts.map(({ path, sha256 }) => [path, sha256])),
            reason: 'same build inputs as the previous receipt',
            previousStartedAt: previous.startedAt,
          }
  const receipt = {
    schema: 1,
    tool: 'experiments/library-matrix/tools/build.mjs',
    library: port.library,
    port: port.id,
    language: port.language,
    label: language.label,
    production: language.production,
    registry: { path: repoPath(registry.path), sha256: registry.sha256 },
    node: process.version,
    harness: harnessIdentity('build'),
    workDir: repoPath(workDir),
    startedAt,
    finishedAt: new Date().toISOString(),
    command: command.map((part) => (part.startsWith(repositoryRoot) ? repoPath(part) : part)),
    cwd: repoPath(harnessRoot),
    timeoutSeconds,
    cleanWorkDir: options.clean === true,
    exitCode,
    signal,
    timedOut,
    seconds,
    log: repoPath(logPath),
    previousDistMovedTo: movedAside,
    sourceBefore,
    sourceAfter,
    sourceUnchanged: checks.sourceUnchanged,
    checks,
    built,
    artifacts,
    moduleShape: shapes,
    meta,
    buildInfo,
    reproducibility,
    qualification: 'build only; parity, sizes and timing have their own receipts',
  }
  writeJson(receiptPath, receipt)
  process.stdout.write(`${port.key} ${built ? 'PASS' : 'FAIL'} exit ${exitCode}${timedOut ? ' (timed out)' : ''} ${seconds} s\n`)
  if (!built) {
    const failed = Object.entries(checks).filter(([, ok]) => !ok).map(([name]) => name)
    process.stdout.write(`  failed checks: ${failed.join(', ')}\n`)
    const lines = readFileSync(logPath, 'utf8').split('\n').slice(-16).join('\n')
    process.stdout.write(`${lines}\n`)
  }
  return { port: port.key, built, seconds, artifacts, reproducibility }
}

// Runs the builder in its own process group with stdout and stderr in the log. On timeout the
// whole group gets SIGTERM, then SIGKILL after ten seconds; the build is recorded as failed.
function runLogged(command, logPath, timeout) {
  return new Promise((resolvePromise) => {
    const log = createWriteStream(logPath)
    const child = spawn(command[0], command.slice(1), { cwd: harnessRoot, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
    child.stdout.pipe(log, { end: false })
    child.stderr.pipe(log, { end: false })
    let timedOut = false
    let killer = null
    const timer = setTimeout(() => {
      timedOut = true
      try { process.kill(-child.pid, 'SIGTERM') } catch {}
      killer = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL') } catch {} }, 10_000)
    }, timeout * 1000)
    child.on('error', (error) => log.write(`spawn failed: ${error.message}\n`))
    child.on('close', (code, signal) => {
      clearTimeout(timer)
      if (killer) clearTimeout(killer)
      // A timed-out build exits like `timeout(1)`: 124.
      const exitCode = timedOut ? 124 : code ?? 128
      log.end(`\n[build.mjs] exit ${code}${signal ? ` signal ${signal}` : ''}${timedOut ? ' (timed out)' : ''}\n`, () => resolvePromise({ exitCode, timedOut, signal }))
    })
  })
}
