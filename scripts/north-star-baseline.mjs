#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { closeSync, lstatSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import {
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  symlink,
  writeFile,
} from 'node:fs/promises'
import { dirname, extname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib'

const compilerRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = resolve(compilerRoot, '../..')
const options = parseArgs(process.argv.slice(2))
const scratch = resolve(options.scratch || resolve(compilerRoot, 'build/north-star-baseline'))
const output = resolve(options.output || resolve(compilerRoot, 'checkpoints/north-star-baseline.json'))
const browsers = String(options.browsers || 'chrome,firefox,safari').split(',')
const benchmarkRuns = Number(options['benchmark-runs'] || 30)
const benchmarkWarmups = Number(options['benchmark-warmups'] || 5)
const skipBrowsers = options['skip-browsers'] === true
const resumeSource = options['resume-source'] === true
const sourceSnapshot = resolve(scratch, 'source')
const oracleRoot = resolve(repositoryRoot, 'experiments/fingerprintjs')
const sourceRoots = {
  compilerV1: resolve(repositoryRoot, 'spike/rustc-codegen-js'),
  compilerV2: compilerRoot,
  fingerprintjs: oracleRoot,
  fingerprintjsCompare: resolve(repositoryRoot, 'experiments/fingerprintjs-compare'),
  rescript: resolve(repositoryRoot, 'experiments/fingerprintjs-rescript'),
  idiomatic: resolve(repositoryRoot, 'experiments/fingerprintjs-rust'),
  compact: resolve(repositoryRoot, 'experiments/fingerprintjs-rust-compact'),
}

for (const path of Object.values(sourceRoots)) await requireDirectory(path)
await requireDirectory(resolve(oracleRoot, 'node_modules'))
await mkdir(dirname(output), { recursive: true })
if (resumeSource) {
  await requireDirectory(sourceSnapshot)
  if (!await exists(resolve(sourceSnapshot, 'Cargo.toml'))) {
    copyWorkspaceSupport(repositoryRoot, sourceSnapshot)
  }
  // Resume preserves the frozen inputs. When the receipt learns about a new required tree, add
  // only that missing tree rather than silently refreshing the snapshot beneath earlier runs.
  for (const [name, source] of Object.entries(sourceRoots)) {
    const destination = copiedRoot(sourceSnapshot, name)
    if (!await exists(destination)) {
      await mkdir(dirname(destination), { recursive: true })
      copySource(name, source, destination)
    }
  }
  await rm(resolve(scratch, 'checkout'), { recursive: true, force: true })
  await rm(resolve(scratch, 'runs'), { recursive: true, force: true })
} else {
  await rm(scratch, { recursive: true, force: true })
  await mkdir(scratch, { recursive: true })
  copyWorkspaceSupport(repositoryRoot, sourceSnapshot)
  for (const [name, source] of Object.entries(sourceRoots)) {
    const destination = copiedRoot(sourceSnapshot, name)
    await mkdir(dirname(destination), { recursive: true })
    copySource(name, source, destination)
  }
}

const receipt = {
  schema: 2,
  generatedAt: new Date().toISOString(),
  configuration: {
    benchmarkRuns,
    benchmarkWarmups,
    browsers,
    compression: { brotliQuality: 11, gzipLevel: 9 },
    esTarget: 'ES2018',
    optimizer: 'on',
    schedulerVariants: { compact: ['boundary'], idiomatic: ['boundary', 'fiber'] },
    workloads: ['fingerprintjs-idiomatic', 'fingerprintjs-compact', 'murmur-x64'],
    v1: { lane: 'baseline', rustOptLevel: null, mirInline: false },
    v2: { lane: 'jir', rustOptLevel: null, mirInline: false },
    v1Compact: { lane: 'baseline', rustOptLevel: 'z', mirInline: true },
    v2Compact: { lane: 'jir', rustOptLevel: 'z', mirInline: true },
  },
  provenance: await provenance(),
  builds: {},
  checks: {},
  runtime: {},
  murmur: null,
  status: {},
}

for (const compiler of ['v1', 'v2']) {
  receipt.builds[compiler] = { runs: [] }
  for (let run = 1; run <= 2; ++run) {
    process.stdout.write(`\n==> ${compiler} clean build ${run}/2\n`)
    receipt.builds[compiler].runs.push(await cleanBuild(compiler, run))
  }
  receipt.builds[compiler].determinism = compareBuilds(receipt.builds[compiler].runs)
}

if (!skipBrowsers) prepareBrowsers()
for (const compiler of ['v1', 'v2']) {
  process.stdout.write(`\n==> ${compiler} semantic and package checks\n`)
  receipt.checks[compiler] = await checkArtifacts(compiler)
}

if (!skipBrowsers) {
  for (const compiler of ['v1', 'v2']) {
    process.stdout.write(`\n==> ${compiler} protected browser runtime\n`)
    receipt.runtime[compiler] = await browserRuntime(compiler)
  }
} else {
  receipt.runtime.skipped = 'requested by --skip-browsers'
}

process.stdout.write('\n==> current v2 Murmur gate\n')
receipt.murmur = await murmurGate()
receipt.compression = compareCompression(receipt.builds)

receipt.status = {
  browserParity: skipBrowsers || Object.values(receipt.checks).every((check) => check.browserParity?.passed),
  deterministic: Object.values(receipt.builds).every((build) => build.determinism.passed),
  gzipNoRegression: receipt.compression.passed,
  packageAndSemantic: Object.values(receipt.checks).every((check) => check.passed),
  runtime: skipBrowsers || Object.values(receipt.runtime).every((run) => run.passed && run.summaryRun?.passed),
  murmurGatePassed: receipt.murmur.passed,
  passed: false,
}
receipt.status.passed = receipt.status.browserParity
  && receipt.status.deterministic
  && receipt.status.gzipNoRegression
  && receipt.status.packageAndSemantic
  && receipt.status.runtime
  && receipt.status.murmurGatePassed

await writeFile(output, stableJson(receipt))
process.stdout.write(`\nWrote ${output}\n`)
if (!receipt.status.passed) process.exitCode = 1

async function cleanBuild(compiler, run) {
  const checkout = resolve(scratch, 'checkout')
  await rm(checkout, { recursive: true, force: true })
  await mkdir(checkout, { recursive: true })
  copyTreePart(sourceSnapshot, checkout)
  await symlink(resolve(oracleRoot, 'node_modules'), resolve(copiedRoot(checkout, 'fingerprintjs'), 'node_modules'))

  const copiedCompiler = copiedRoot(checkout, compiler === 'v1' ? 'compilerV1' : 'compilerV2')
  const runRoot = resolve(scratch, 'runs', compiler, String(run))
  const logs = resolve(runRoot, 'logs')
  const artifacts = resolve(runRoot, 'artifacts')
  await mkdir(logs, { recursive: true })
  await mkdir(artifacts, { recursive: true })

  const compilerBuild = timed(
    resolve(copiedCompiler, 'scripts/build.sh'),
    [],
    { cwd: copiedCompiler, log: resolve(logs, 'compiler-build.log') },
  )
  const idiomatic = await buildPort({
    compiler,
    copiedCompiler,
    port: copiedRoot(checkout, 'idiomatic'),
    outputRoot: resolve(artifacts, 'idiomatic'),
    compact: false,
    log: resolve(logs, 'idiomatic-build.log'),
  })
  const compact = await buildPort({
    compiler,
    copiedCompiler,
    port: copiedRoot(checkout, 'compact'),
    outputRoot: resolve(artifacts, 'compact'),
    compact: true,
    log: resolve(logs, 'compact-build.log'),
  })

  const dylib = resolve(copiedCompiler, `target/release/librustc_codegen_mires.${process.platform === 'darwin' ? 'dylib' : 'so'}`)
  const binaries = { backend: await fileRecord(dylib) }
  const linker = resolve(copiedCompiler, 'target/release/jsc-link')
  if (await exists(linker)) binaries.linker = await fileRecord(linker)
  return { compilerBuild, binaries, compact, idiomatic, run }
}

async function buildPort({ compiler, copiedCompiler, port, outputRoot, compact, log }) {
  const args = [
    resolve(port, 'tools/build.mjs'),
    '--compiler-root', copiedCompiler,
    '--optimizer', 'on',
    '--scheduler', compact ? 'boundary' : 'all',
    '--output', outputRoot,
  ]
  if (compiler === 'v2') args.push('--lane', 'jir', '--lane-report')
  if (compact) args.push('--rust-opt-level', 'z', '--mir-inline', 'on')
  const timing = timed(process.execPath, args, { cwd: port, log })
  const files = await treeRecords(outputRoot)
  const buildTree = resolve(port, 'target/js-build')
  const schedulerRoots = compact
    ? { boundary: outputRoot }
    : { boundary: resolve(outputRoot, 'boundary'), fiber: resolve(outputRoot, 'fiber') }
  const finalJsAllocations = {}
  for (const [scheduler, root] of Object.entries(schedulerRoots)) {
    finalJsAllocations[scheduler] = await allocationCounts(resolve(root, 'fp.esm.js'))
  }
  return {
    artifacts: files,
    finalJsAllocations,
    objectFiles: await objectStats(buildTree),
    timing,
  }
}

function compareBuilds(runs) {
  const mismatches = []
  const binaryNames = [...new Set([
    ...Object.keys(runs[0].binaries),
    ...Object.keys(runs[1].binaries),
  ])].sort()
  for (const name of binaryNames) {
    const left = runs[0].binaries[name]?.sha256 || null
    const right = runs[1].binaries[name]?.sha256 || null
    if (left !== right) mismatches.push({ kind: 'compiler-binary', name, run1: left, run2: right })
  }
  for (const port of ['idiomatic', 'compact']) {
    const left = Object.fromEntries(runs[0][port].artifacts.map((file) => [file.path, file.sha256]))
    const right = Object.fromEntries(runs[1][port].artifacts.map((file) => [file.path, file.sha256]))
    for (const path of [...new Set([...Object.keys(left), ...Object.keys(right)])].sort()) {
      if (left[path] !== right[path]) {
        mismatches.push({ kind: 'artifact', port, path, run1: left[path] || null, run2: right[path] || null })
      }
    }
    const leftObjects = runs[0][port].objectFiles.sha256
    const rightObjects = runs[1][port].objectFiles.sha256
    if (leftObjects !== rightObjects) {
      mismatches.push({ kind: 'object-tree', port, run1: leftObjects, run2: rightObjects })
    }
  }
  return { mismatches, passed: mismatches.length === 0 }
}

function compareCompression(builds) {
  const files = {
    compact: 'fp.esm.min.js',
    idiomatic: 'boundary/fp.esm.min.js',
  }
  const ports = Object.entries(files).map(([port, path]) => {
    const record = (compiler) => builds[compiler].runs[1][port].artifacts.find((file) => file.path === path)
    const v1 = record('v1')
    const v2 = record('v2')
    if (!v1 || !v2) return { path, port, passed: false, v1: null, v2: null }
    return {
      delta: v2.gzip9 - v1.gzip9,
      passed: v2.gzip9 <= v1.gzip9,
      path,
      port,
      v1: v1.gzip9,
      v2: v2.gzip9,
    }
  })
  return { passed: ports.every((port) => port.passed), ports }
}

async function checkArtifacts(compiler) {
  const runRoot = resolve(scratch, 'runs', compiler, '2')
  const copied = resolve(scratch, 'checkout')
  const checks = []
  for (const portName of ['idiomatic', 'compact']) {
    const port = copiedRoot(copied, portName)
    const dist = resolve(
      runRoot,
      'artifacts',
      portName,
      ...(portName === 'compact' ? [] : ['boundary']),
    )
    for (const [name, script, args] of [
      ['package', 'tools/check-package.mjs', ['--dist', dist, '--allow-size-failure']],
      ['ssr', 'tools/check-ssr.mjs', ['--dist', dist]],
      ['types', 'tools/check-types.mjs', ['--types', resolve(dist, 'fp.d.ts')]],
    ]) {
      checks.push(runCheck(`${portName}.${name}`, process.execPath, [resolve(port, script), ...args], port))
    }
  }
  if (compiler === 'v2') {
    const v2 = copiedRoot(copied, 'compilerV2')
    checks.push(runCheck('sourceMaps', resolve(v2, 'scripts/repr-sourcemap-test.sh'), [], v2, { SKIP_BUILD: '1' }))
  }

  let browserParity
  if (!skipBrowsers) {
    const results = []
    for (const portName of ['idiomatic', 'compact']) {
      const port = copiedRoot(copied, portName)
      const candidate = resolve(
        runRoot,
        'artifacts',
        portName,
        ...(portName === 'compact' ? ['fp.esm.js'] : ['boundary/fp.esm.js']),
      )
      const destination = resolve(runRoot, 'parity', portName)
      const result = runCheck(
        `${portName}.browserParity`,
        process.execPath,
        [resolve(port, 'tests/differential.mjs'), '--candidate', candidate, '--browsers', browsers.join(','), '--output', destination],
        port,
      )
      const records = result.passed ? await parityRecords(destination) : []
      results.push({ ...result, records })
    }
    browserParity = { passed: results.every((result) => result.passed), results }
  }
  return { browserParity, checks, passed: checks.every((check) => check.passed) && (browserParity?.passed ?? true) }
}

async function browserRuntime(compiler) {
  const checkout = resolve(scratch, 'checkout')
  const port = copiedRoot(checkout, 'idiomatic')
  const artifacts = resolve(scratch, 'runs', compiler, '2', 'artifacts/idiomatic')
  const outputRoot = resolve(scratch, 'runs', compiler, '2', 'runtime')
  const args = [
    resolve(port, 'bench/run.mjs'),
    '--minified',
    '--boundary', resolve(artifacts, 'boundary/fp.esm.min.js'),
    '--fiber', resolve(artifacts, 'fiber/fp.esm.min.js'),
    '--browsers', browsers.join(','),
    '--runs', String(benchmarkRuns),
    '--warmups', String(benchmarkWarmups),
    '--rerun-threshold', '-1',
    '--output', outputRoot,
  ]
  const attempts = []
  let result
  for (let attempt = 1; attempt <= 2; ++attempt) {
    await rm(outputRoot, { recursive: true, force: true })
    result = runCheck('protectedRuntime', process.execPath, args, port)
    attempts.push({ attempt, ...result })
    if (result.passed || !isTransientBrowserStartupFailure(result.failure)) break
    process.stdout.write(`browser startup failed for ${compiler}; retrying once\n`)
  }
  if (!result.passed) return { ...result, attempts }
  const summaryPath = resolve(outputRoot, 'summary.json')
  const summaryRun = runCheck(
    'runtimeSummary',
    process.execPath,
    [resolve(port, 'bench/summarize.mjs'), '--input', outputRoot, '--output', summaryPath],
    port,
  )
  const summary = JSON.parse(await readFile(summaryPath, 'utf8'))
  return {
    ...result,
    attempts,
    heap: await heapSummary(outputRoot),
    summary: {
      gates: summary.gates,
      p95GeometricMeans: summary.p95GeometricMeans,
      sha256: sha256(await readFile(summaryPath)),
    },
    summaryRun,
  }
}

function isTransientBrowserStartupFailure(failure) {
  return /SessionNotCreatedError|automation session ended unexpected|ECONNRESET/.test(failure || '')
}

async function murmurGate() {
  const checkout = resolve(scratch, 'checkout')
  const v2 = copiedRoot(checkout, 'compilerV2')
  const outputPath = resolve(scratch, 'murmur-gate.txt')
  const result = runCheck(
    'murmur',
    resolve(v2, 'scripts/alloc-gate-test.sh'),
    ['--fingerprintjs'],
    v2,
    { SKIP_BUILD: '1' },
    outputPath,
  )
  const output = await readFile(outputPath, 'utf8')
  const allocations = output.match(/\((\d+) final JS allocation sites/)
  const bytes = output.match(/murmur size: (?:PASS|FAIL) \((\d+) B/)
  const runtime = output.match(/murmur kernel: ([0-9.]+)x/)
  return {
    ...result,
    metrics: {
      itemBytes: bytes ? Number(bytes[1]) : null,
      loopAllocationSites: allocations ? Number(allocations[1]) : null,
      runtimeRatio: runtime ? Number(runtime[1]) : null,
    },
    output,
  }
}

async function provenance() {
  const inputs = []
  const workspaceRoot = resumeSource ? sourceSnapshot : repositoryRoot
  for (const file of await sourceFiles(workspaceRoot)) {
    const path = relative(workspaceRoot, file)
    if (path.startsWith('spike/') || path.startsWith('experiments/')) continue
    inputs.push({ path: `workspace/${path}`, sha256: sha256(await readFile(file)) })
  }
  for (const [rootName, originalRoot] of Object.entries(sourceRoots)) {
    const root = resumeSource ? copiedRoot(sourceSnapshot, rootName) : originalRoot
    // The oracle and ReScript distributions are copied into every isolated build and therefore
    // are inputs, not generated output. The Rust ports' dist trees remain excluded because those
    // are precisely what the receipt rebuilds and compares.
    const includeDist = rootName === 'fingerprintjs' || rootName === 'rescript'
    for (const file of await sourceFiles(root, { includeDist })) {
      const path = relative(root, file)
      // The receipt is an output, even though it is published beside source-controlled
      // checkpoints. Including yesterday's receipt would make today's input digest recursively
      // depend on the result it replaces.
      if (rootName === 'compilerV2' && path === 'checkpoints/north-star-baseline.json') continue
      inputs.push({
        path: `${rootName}/${path}`,
        sha256: sha256(await readFile(file)),
      })
    }
  }
  inputs.sort((left, right) => left.path.localeCompare(right.path))
  const inputDigest = sha256(Buffer.from(inputs.map((item) => `${item.sha256}  ${item.path}\n`).join('')))
  const terser = JSON.parse(await readFile(resolve(oracleRoot, 'node_modules/terser/package.json'), 'utf8'))
  const rollup = JSON.parse(await readFile(resolve(oracleRoot, 'node_modules/rollup/package.json'), 'utf8'))
  return {
    git: {
      compilerV1Revision: capture('git', ['rev-parse', 'HEAD'], sourceRoots.compilerV1).trim(),
      compilerV2Revision: capture('git', ['rev-parse', 'HEAD'], sourceRoots.compilerV2).trim(),
      revision: capture('git', ['rev-parse', 'HEAD'], repositoryRoot).trim(),
      statusSha256: resumeSource
        ? null
        : sha256(Buffer.from(capture('git', ['status', '--short', '--untracked-files=all'], repositoryRoot))),
      fingerprintjsRevision: capture('git', ['-C', sourceRoots.fingerprintjs, 'rev-parse', 'HEAD'], repositoryRoot).trim(),
    },
    inputDigest,
    inputs,
    tools: {
      cargoV1: capture('cargo', ['--version'], sourceRoots.compilerV1).trim(),
      cargoV2: capture('cargo', ['--version'], sourceRoots.compilerV2).trim(),
      node: process.version,
      platform: `${process.platform}-${process.arch}`,
      rollup: rollup.version,
      rustcV1: capture('rustc', ['--version', '--verbose'], sourceRoots.compilerV1).trim(),
      rustcV2: capture('rustc', ['--version', '--verbose'], sourceRoots.compilerV2).trim(),
      terser: terser.version,
      zlib: process.versions.zlib,
    },
  }
}

function copiedRoot(checkout, name) {
  const paths = {
    compilerV1: 'spike/rustc-codegen-js',
    compilerV2: 'spike/rustc-codegen-js-v2',
    fingerprintjs: 'experiments/fingerprintjs',
    fingerprintjsCompare: 'experiments/fingerprintjs-compare',
    rescript: 'experiments/fingerprintjs-rescript',
    idiomatic: 'experiments/fingerprintjs-rust',
    compact: 'experiments/fingerprintjs-rust-compact',
  }
  return resolve(checkout, paths[name])
}

function copySource(name, source, destination) {
  const args = [
    '-a', '--delete',
    '--exclude', '.git',
    '--exclude', 'build',
    '--exclude', 'target',
    '--exclude', 'dist',
    '--exclude', 'node_modules',
    '--exclude', 'bench/results',
    '--exclude', 'tests/results',
  ]
  if (name === 'compilerV2') args.push('--exclude', 'checkpoints/north-star-baseline.json')
  args.push(`${source}/`, `${destination}/`)
  const result = spawnSync('rsync', args, { stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`rsync failed for ${source}`)
  if (name === 'fingerprintjs' || name === 'rescript') {
    copyTreePart(resolve(source, 'dist'), resolve(destination, 'dist'))
  }
}

function copyWorkspaceSupport(source, destination) {
  const result = spawnSync('rsync', [
    '-a',
    '--exclude', '.git',
    '--exclude', '.agents',
    '--exclude', 'build',
    '--exclude', 'dist',
    '--exclude', 'experiments',
    '--exclude', 'node_modules',
    '--exclude', 'spike',
    '--exclude', 'target',
    `${source}/`, `${destination}/`,
  ], { stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`rsync failed for workspace support ${source}`)
}

function copyTreePart(source, destination) {
  const result = spawnSync('rsync', ['-a', '--delete', `${source}/`, `${destination}/`], { stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`rsync failed for ${source}`)
}

function timed(command, args, { cwd, log }) {
  const timePath = `${log}.time`
  const descriptor = openSync(log, 'w')
  const result = spawnSync('/usr/bin/time', ['-l', '-p', '-o', timePath, command, ...args], {
    cwd,
    env: process.env,
    stdio: ['ignore', descriptor, descriptor],
  })
  closeSync(descriptor)
  if (result.status !== 0) {
    throw new Error(`${command} failed with ${result.status}; see ${log}`)
  }
  const timing = lstatSync(timePath).size ? readFileSyncText(timePath) : ''
  return {
    logSha256: sha256(Buffer.from(readFileSyncText(log))),
    peakRssBytes: numberBefore(timing, 'maximum resident set size'),
    realSeconds: numberAfter(timing, 'real'),
    systemSeconds: numberAfter(timing, 'sys'),
    userSeconds: numberAfter(timing, 'user'),
  }
}

function runCheck(name, command, args, cwd, extraEnvironment = {}, outputPath) {
  const result = spawnSync(command, args, {
    cwd,
    env: { ...process.env, ...extraEnvironment },
    encoding: 'utf8',
    maxBuffer: 128 * 1024 * 1024,
  })
  const output = `${result.stdout || ''}${result.stderr || ''}`
  if (outputPath) writeFileSyncText(outputPath, output)
  return {
    failure: result.status === 0 ? null : output.trim(),
    name,
    outputSha256: sha256(Buffer.from(output)),
    passed: result.status === 0,
    status: result.status,
  }
}

function prepareBrowsers() {
  if (process.platform !== 'darwin' || !browsers.includes('safari')) return
  // SafariDriver normally launches Safari itself, but a cold local Safari can time out before its
  // remote-automation endpoint is ready. Opening it once is idempotent and keeps the receipt from
  // turning that host startup race into four identical test failures.
  const result = spawnSync('open', ['-a', 'Safari'])
  if (result.status !== 0) throw new Error('could not open Safari for the requested browser checks')
}

async function parityRecords(root) {
  const records = []
  for (const browser of browsers) {
    const value = JSON.parse(await readFile(resolve(root, `${browser}.json`), 'utf8'))
    records.push({
      browser: value.browser,
      differences: value.comparison.differences.length,
      candidateSha256: value.artifacts.candidate.sha256,
    })
  }
  return records
}

async function heapSummary(root) {
  const groups = new Map()
  for (const file of (await readdir(root)).filter((name) => /^(chrome|firefox|safari)\.json$/.test(name))) {
    const value = JSON.parse(await readFile(resolve(root, file), 'utf8'))
    for (const record of value.records) {
      if (!Number.isFinite(record.heapDelta)) continue
      const key = `${value.browser.name}/${record.scenario}/${record.implementation}`
      const values = groups.get(key) || []
      values.push(record.heapDelta)
      groups.set(key, values)
    }
  }
  return Object.fromEntries([...groups].sort().map(([key, values]) => [key, {
    count: values.length,
    p50: percentile(values, 0.5),
    p95: percentile(values, 0.95),
  }]))
}

async function allocationCounts(path) {
  const require = createRequire(import.meta.url)
  const acornPath = resolve(oracleRoot, 'node_modules/acorn')
  const { parse } = require(acornPath)
  // The target controls transforms, not what syntax the backend may need for Rust integers.
  // In particular, exact i64/u64 constants are BigInts, which post-date ES2018.
  const ast = parse(await readFile(path, 'utf8'), { ecmaVersion: 'latest', sourceType: 'module' })
  const counts = { arrays: 0, news: 0, objects: 0, total: 0 }
  visit(ast, (node) => {
    if (node.type === 'ArrayExpression') ++counts.arrays
    if (node.type === 'NewExpression') ++counts.news
    if (node.type === 'ObjectExpression') ++counts.objects
  })
  counts.total = counts.arrays + counts.news + counts.objects
  return counts
}

function visit(value, callback) {
  if (!value || typeof value !== 'object') return
  if (typeof value.type === 'string') callback(value)
  for (const child of Object.values(value)) {
    if (Array.isArray(child)) child.forEach((item) => visit(item, callback))
    else visit(child, callback)
  }
}

async function objectStats(root) {
  if (!await exists(root)) return { bytes: 0, files: 0, sha256: sha256(Buffer.alloc(0)) }
  const files = await allFiles(root)
  const objects = files.filter((path) => ['.o', '.rlib', '.rmeta'].includes(extname(path)))
  let bytes = 0
  const hashes = []
  for (const path of objects) {
    const value = await readFile(path)
    bytes += value.length
    hashes.push(`${sha256(value)}  ${relative(root, path)}\n`)
  }
  return { bytes, files: objects.length, sha256: sha256(Buffer.from(hashes.join(''))) }
}

async function treeRecords(root) {
  const records = []
  for (const path of await allFiles(root)) {
    records.push({ path: relative(root, path), ...await fileRecord(path) })
  }
  return records
}

async function fileRecord(path) {
  const value = await readFile(path)
  return {
    bytes: value.length,
    brotli11: brotliCompressSync(value, { params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 } }).length,
    gzip9: gzipSync(value, { level: 9, mtime: 0 }).length,
    sha256: sha256(value),
  }
}

async function sourceFiles(root, { includeDist = false } = {}) {
  const files = []
  const excluded = new Set(['.git', 'build', 'target', 'node_modules'])
  if (!includeDist) excluded.add('dist')
  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name)
      const name = relative(root, path)
      if (entry.name === '.git') continue
      if (entry.isDirectory()) {
        if (excluded.has(entry.name) || name === 'bench/results' || name === 'tests/results') continue
        await walk(path)
      } else if (entry.isFile()) {
        files.push(path)
      }
    }
  }
  await walk(root)
  files.sort()
  return files
}

async function allFiles(root) {
  const files = []
  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name)
      if (entry.isDirectory()) await walk(path)
      else if (entry.isFile()) files.push(path)
    }
  }
  await walk(root)
  files.sort()
  return files
}

async function exists(path) {
  try {
    await stat(path)
    return true
  } catch {
    return false
  }
}

async function requireDirectory(path) {
  if (!await exists(path) || !(await stat(path)).isDirectory()) throw new Error(`required directory is missing: ${path}`)
}

function capture(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed: ${result.stderr}`)
  return result.stdout
}

function numberAfter(text, label) {
  const match = text.match(new RegExp(`^${label}\\s+([0-9.]+)$`, 'm'))
  return match ? Number(match[1]) : null
}

function numberBefore(text, label) {
  const match = text.match(new RegExp(`^\\s*([0-9]+)\\s+${label}$`, 'm'))
  return match ? Number(match[1]) : null
}

function percentile(values, fraction) {
  const sorted = values.slice().sort((left, right) => left - right)
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)]
}

function parseArgs(args) {
  const values = {}
  for (let index = 0; index < args.length; ++index) {
    const arg = args[index]
    if (!arg.startsWith('--')) throw new Error(`unexpected argument: ${arg}`)
    const [name, inline] = arg.slice(2).split('=', 2)
    if (inline !== undefined) values[name] = inline
    else if (args[index + 1] && !args[index + 1].startsWith('--')) values[name] = args[++index]
    else values[name] = true
  }
  return values
}

function stableJson(value) {
  return `${JSON.stringify(sortObject(value), null, 2)}\n`
}

function sortObject(value) {
  if (Array.isArray(value)) return value.map(sortObject)
  if (!value || typeof value !== 'object') return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortObject(value[key])]))
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

function readFileSyncText(path) {
  return lstatSync(path).size ? readFileSync(path, 'utf8') : ''
}

function writeFileSyncText(path, value) {
  writeFileSync(path, value)
}
