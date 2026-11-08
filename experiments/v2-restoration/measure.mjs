#!/usr/bin/env node
// Stage 0 measurement loop for the mires v2 restoration.
//
// Builds the declared target matrix with fixed flags, one build at a time, then records sizes,
// hashes, link cost, optimizer decision counts and Node timings under
// build/v2-restoration/<label>/ and appends one block to experiments/v2-restoration/RESULTS.md.
// The package build tools only compile and bundle; every number here is measured by this script.
// See README.md beside this file for the protocol and the meaning of each field.
//
//   node experiments/v2-restoration/measure.mjs --label R0
//     [--targets compact,idiomatic,pro,corpus] [--no-build] [--force]
//     [--pro-timeout-min 15] [--pro-optimizer on|off] [--samples 9] [--warmups 2]
//     [--browsers chrome,firefox,safari]

import { spawn, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import {
  appendFileSync, closeSync, copyFileSync, existsSync, mkdirSync, openSync, readFileSync, rmSync, statSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'
import { brotliCompressSync, constants as zlibConstants, gzipSync } from 'node:zlib'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const resultsPath = resolve(repoRoot, 'experiments/v2-restoration/RESULTS.md')
const TARGETS = ['compact', 'idiomatic', 'pro', 'corpus']
// The workload list of experiments/browser-workloads/tools/build.mjs, and the subset its
// `run.mjs --pure` benchmarks.
const CORPUS_WORKLOADS = ['minisearch', 'marked', 'fflate', 'idb', 'idb_lines', 'ky', 'uplot', 'explorer', 'workbox', 'workbox_direct']
const CORPUS_TIMED = ['marked', 'minisearch', 'fflate']
// Browsers the corpus runner can time in (run.mjs --browsers); none means `run.mjs --pure`.
const CORPUS_BROWSERS = ['chrome', 'firefox', 'safari']
const COST_PROFILE_PREFIX = 'rcgjs-cost-profile:'
const SCHEMA = 1

const RESULTS_HEADER = `# v2 restoration results

Append-only log written by \`measure.mjs\`. Each block is one run; receipts live in
\`build/v2-restoration/<label>/\`. Never edit or delete a recorded block. See README.md for the protocol.
`

// Pro is private. Its observations (sizes, decisions, compile cost, logs, commands) go only to
// build/v2-restoration/<label>/pro/ and to the git-ignored private RESULTS.md below. The public
// receipts and RESULTS.md carry one line per private target: its build status, plus wall time and
// peak RSS when the build completed.
const PRIVATE_TARGETS = ['pro']
const privateResultsPath = resolve(repoRoot, 'experiments-pro/v2-restoration/RESULTS.md')
const PRIVATE_RESULTS_HEADER = `# v2 restoration results (private: Pro)

Append-only log of Pro observations written by \`experiments/v2-restoration/measure.mjs\`. Each block
is one run; receipts live in \`build/v2-restoration/<label>/pro/\`. Never edit or delete a recorded
block. Do not copy these numbers into public files.
`
const isPrivate = (name) => PRIVATE_TARGETS.includes(name)

// ---------------------------------------------------------------------------------------------
// Target matrix. Flags are fixed: changing them is a protocol change, not a run option.
// ---------------------------------------------------------------------------------------------

function minArtifact(file, fetched) {
  return { file, kind: 'min', raw: file.replace(/\.min\.js$/, '.js'), fetched, table: true }
}

function rawArtifact(file, table) {
  return { file, kind: 'raw', raw: null, fetched: false, table }
}

export function targetSpecs({ proOptimizer = 'on' } = {}) {
  const packageFlags = ['--profile', 'release-balanced', '--optimizer', 'on', '--lane', 'jir', '--cleanup', 'on']
  const proDist = proOptimizer === 'on'
    ? 'experiments-pro/fp-pro-rust/dist/mires-v2'
    : 'experiments-pro/fp-pro-rust/dist/mires-v2-opt-off'
  // Optimizer report names are the build tools' `modeName` for exactly these flags.
  const proReports = `experiments-pro/fp-pro-rust/bench/results/optimizer-reports/profile-release-balanced-cleanup-on-${proOptimizer}`
  return {
    compact: {
      tool: 'experiments/fingerprintjs-rust-compact/tools/build.mjs',
      args: [...packageFlags, '--scheduler', 'boundary', '--lane-report',
        '--output', 'experiments/fingerprintjs-rust-compact/dist/mires-v2'],
      dist: 'experiments/fingerprintjs-rust-compact/dist/mires-v2',
      artifacts: [
        minArtifact('fp.esm.min.js', true), minArtifact('fp.min.js', false), minArtifact('fp.umd.min.js', false),
        rawArtifact('raw/program.js', true), rawArtifact('raw/shim.js', false),
      ],
      fetchedTotal: true,
      links: [{
        name: 'main', program: 'raw/program.js', laneReport: 'link-lane-report.json', itemLanes: 'lane-report.json',
        optimizerReport: 'experiments/fingerprintjs-rust-compact/bench/results/optimizer-reports/profile-release-balanced-cleanup-on-on-rust-z-mir-inline-boundary.json',
      }],
      kernels: 'fp.esm.min.js',
    },
    idiomatic: {
      tool: 'experiments/fingerprintjs-rust/tools/build.mjs',
      args: [...packageFlags, '--scheduler', 'boundary', '--lane-report',
        '--output', 'experiments/fingerprintjs-rust/dist/mires-v2'],
      // The idiomatic tool writes each scheduler variant into <output>/<variant>/.
      dist: 'experiments/fingerprintjs-rust/dist/mires-v2/boundary',
      artifacts: [
        minArtifact('fp.esm.min.js', true), minArtifact('fp.min.js', false), minArtifact('fp.umd.min.js', false),
        rawArtifact('raw/program.js', true), rawArtifact('raw/shim.js', false),
      ],
      fetchedTotal: true,
      links: [{
        name: 'main', program: 'raw/program.js', laneReport: 'link-lane-report.json', itemLanes: 'lane-report.json',
        optimizerReport: 'experiments/fingerprintjs-rust/bench/results/optimizer-reports/profile-release-balanced-cleanup-on-on-boundary.json',
      }],
      kernels: 'fp.esm.min.js',
    },
    pro: {
      tool: 'experiments-pro/fp-pro-rust/tools/build.mjs',
      args: ['--profile', 'release-balanced', '--optimizer', proOptimizer, '--lane', 'jir', '--cleanup', 'on',
        '--lane-report', '--output', proDist],
      dist: proDist,
      optimizer: proOptimizer,
      artifacts: [
        minArtifact('fp.esm.min.js', true), minArtifact('fp.min.js', false), minArtifact('fp.worker.min.js', true),
        rawArtifact('raw/program.js', true), rawArtifact('raw/shim.js', false),
        rawArtifact('raw/worker-program.js', true), rawArtifact('raw/worker-shim.js', false),
      ],
      fetchedTotal: true,
      links: [
        { name: 'main', program: 'raw/program.js', laneReport: 'link-lane-report.json', itemLanes: 'lane-report.json',
          optimizerReport: `${proReports}.json` },
        { name: 'worker', program: 'raw/worker-program.js', laneReport: 'worker-link-lane-report.json',
          itemLanes: 'worker-lane-report.json', optimizerReport: `${proReports}-worker.json` },
      ],
    },
    corpus: {
      // Same as `npm run build:corpus`, invoked directly so npm is not part of the timed command.
      tool: 'experiments/browser-workloads/tools/build.mjs',
      args: [],
      dist: 'experiments/browser-workloads/dist',
      receipt: 'experiments/browser-workloads/results/build.json',
      artifacts: CORPUS_WORKLOADS.flatMap((name) => [
        minArtifact(`${name}/index.min.js`, true),
        rawArtifact(`${name}/raw/program.js`, true),
        rawArtifact(`${name}/raw/shim.js`, false),
      ]),
      fetchedTotal: false,
      // The corpus publishes the whole optimizer report per workload; its `jir_lane` section is
      // the link lane report.
      links: CORPUS_WORKLOADS.map((name) => ({
        name, program: `${name}/raw/program.js`, laneReport: null, itemLanes: `${name}/lane-report.json`,
        optimizerReport: `experiments/browser-workloads/dist/${name}/optimizer-report.json`,
      })),
    },
  }
}

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

const rel = (path) => relative(repoRoot, path) || '.'
const sha256 = (value) => createHash('sha256').update(value).digest('hex')
const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)

function gzipBytes(buffer) {
  return gzipSync(buffer, { level: 9 }).length
}

function brotliBytes(buffer) {
  return brotliCompressSync(buffer, {
    params: {
      [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
      [zlibConstants.BROTLI_PARAM_LGWIN]: 22,
      [zlibConstants.BROTLI_PARAM_SIZE_HINT]: buffer.length,
    },
  }).length
}

function statOrNull(path) {
  try {
    return statSync(path)
  } catch {
    return null
  }
}

function readJsonOrNull(path) {
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : null
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: repoRoot, encoding: 'utf8', maxBuffer: 64 << 20, ...options })
  if (result.error) throw new Error(`${command} could not start: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} exited ${result.status}: ${result.stderr}`)
  return result.stdout
}

function countBy(values, key) {
  const counts = {}
  for (const value of values ?? []) {
    const name = String(value?.[key] ?? 'null')
    counts[name] = (counts[name] ?? 0) + 1
  }
  return sortCounts(counts)
}

function sortCounts(counts) {
  return Object.fromEntries(Object.entries(counts).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])))
}

function occurrences(text, needle) {
  return text.split(needle).length - 1
}

// Status of a file read after a target's build step:
//   missing     the file does not exist
//   existing    --no-build: whatever is on disk, not produced by this run
//   stale       a build ran but the file predates it (left over from an earlier build)
//   ok          written during this run's successful build
//   unverified  written during this run, but the build failed or timed out
function fileStatus(stat, build) {
  if (!stat) return 'missing'
  if (!build.built) return 'existing'
  if (stat.mtimeMs < build.startedMs) return 'stale'
  return build.status === 'ok' ? 'ok' : 'unverified'
}

function fileInfo(path, build) {
  const stat = statOrNull(path)
  if (!stat) return { path: rel(path), status: 'missing' }
  return {
    path: rel(path),
    status: fileStatus(stat, build),
    bytes: stat.size,
    sha256: sha256(readFileSync(path)),
    mtime: stat.mtime.toISOString(),
  }
}

// ---------------------------------------------------------------------------------------------
// Timed child processes with a process-tree kill on timeout
// ---------------------------------------------------------------------------------------------

let activeChild = null

function isAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error.code === 'EPERM'
  }
}

// Every process descending from `rootPid` by parent links, plus every member of its process group
// (a descendant orphaned to launchd keeps the group).
function processTree(rootPid) {
  const rows = run('ps', ['-A', '-o', 'pid=,ppid=,pgid=']).trim().split('\n')
    .map((line) => line.trim().split(/\s+/).map(Number))
  const tree = new Set([rootPid])
  let grew = true
  while (grew) {
    grew = false
    for (const [pid, ppid, pgid] of rows) {
      if (!tree.has(pid) && (tree.has(ppid) || pgid === rootPid)) {
        tree.add(pid)
        grew = true
      }
    }
  }
  return [...tree]
}

function signalTree(groupId, pids, signal) {
  for (const target of [-groupId, ...pids]) {
    try {
      process.kill(target, signal)
    } catch {
      // Already gone.
    }
  }
}

async function waitFor(predicate, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (predicate()) return true
    await new Promise((done) => setTimeout(done, 100))
  }
  return predicate()
}

// SIGTERM the child's process group and every process in its tree; SIGKILL whatever survives a
// grace period. The child was spawned detached, so its pid is also its process-group id.
export async function killTree(rootPid, graceMs = 5000) {
  const pids = processTree(rootPid)
  const anyAlive = () => isAlive(-rootPid) || pids.some(isAlive)
  signalTree(rootPid, pids, 'SIGTERM')
  let escalated = false
  if (!(await waitFor(() => !anyAlive(), graceMs))) {
    escalated = true
    signalTree(rootPid, pids, 'SIGKILL')
    await waitFor(() => !anyAlive(), graceMs)
  }
  const survivors = pids.filter(isAlive)
  return { processes: pids.length, escalatedToSigkill: escalated, groupGone: !isAlive(-rootPid), survivors }
}

// macOS `/usr/bin/time -l` output. Its max RSS is the largest single process among the command
// and its reaped descendants (BSD rusage merges children by max), not a sum.
export function parseTimeOutput(text) {
  const times = text.match(/([\d.]+)\s+real\s+([\d.]+)\s+user\s+([\d.]+)\s+sys/)
  const rss = text.match(/(\d+)\s+maximum resident set size/)
  return {
    wallSeconds: times ? Number(times[1]) : null,
    userSeconds: times ? Number(times[2]) : null,
    sysSeconds: times ? Number(times[3]) : null,
    maxRssBytes: rss ? Number(rss[1]) : null,
  }
}

// jsc-link under RCGJS_COST_PROFILE=1 prints `rcgjs-cost-profile: key=value` lines on stderr, one
// block per link. Blocks are split where a key repeats, and at an `output=` key, which a linker
// that names its output prints first in its block. A block carrying `output=` is named by it;
// otherwise only its position in the log (`order`) identifies the link, and `attributedBy` says
// which applies. A payload that is not whitespace-separated key=value pairs is kept verbatim in
// `unparsed`.
export function parseCostProfile(text) {
  const blocks = []
  const unparsed = []
  let current = null
  for (const line of text.split('\n')) {
    const at = line.indexOf(COST_PROFILE_PREFIX)
    if (at < 0) continue
    const payload = line.slice(at + COST_PROFILE_PREFIX.length).trim()
    const tokens = payload.split(/\s+/).filter(Boolean)
    if (tokens.length === 0 || !tokens.every((token) => /^[^=]+=/.test(token))) {
      unparsed.push(payload)
      continue
    }
    for (const token of tokens) {
      const key = token.slice(0, token.indexOf('='))
      const value = token.slice(token.indexOf('=') + 1)
      if (!current || key in current || (key === 'output' && Object.keys(current).length > 0)) {
        current = {}
        blocks.push(current)
      }
      current[key] = value !== '' && Number.isFinite(Number(value)) ? Number(value) : value
    }
  }
  const links = blocks.map(({ output, ...counters }, order) => ({
    order,
    output: output === undefined ? null : String(output),
    attributedBy: output === undefined ? 'log-order' : 'output',
    counters,
  }))
  return { links, unparsed }
}

// Runs `command args` under `/usr/bin/time -l`, stdout and stderr appended to `logPath`, in its own
// process group. On timeout the whole tree is killed and no duration is recorded.
export async function runTimed({ command, args, cwd, env, logPath, timeoutMs = null }) {
  const timePath = `${logPath.replace(/\.log$/, '')}.time.txt`
  rmSync(timePath, { force: true })
  const log = openSync(logPath, 'a')
  const startedMs = Date.now()
  const child = spawn('/usr/bin/time', ['-l', '-o', timePath, command, ...args], {
    cwd, env, detached: true, stdio: ['ignore', log, log],
  })
  closeSync(log)
  activeChild = child
  let killing = null
  const timer = timeoutMs === null ? null : setTimeout(() => {
    killing = killTree(child.pid)
  }, timeoutMs)
  const exit = await new Promise((done, fail) => {
    child.once('error', fail)
    child.once('exit', (code, signal) => done({ code, signal }))
  }).finally(() => {
    clearTimeout(timer)
    activeChild = null
  })
  const base = {
    command: [command, ...args],
    cwd: rel(cwd),
    log: rel(logPath),
    startedAt: new Date(startedMs).toISOString(),
    startedMs,
    timeoutMinutes: timeoutMs === null ? null : timeoutMs / 60000,
  }
  if (killing) {
    return { ...base, status: 'timed-out', wallSeconds: null, maxRssBytes: null, kill: await killing }
  }
  const timing = parseTimeOutput(existsSync(timePath) ? readFileSync(timePath, 'utf8') : '')
  return {
    ...base,
    status: exit.code === 0 ? 'ok' : 'failed',
    exitCode: exit.code,
    signal: exit.signal,
    endedAt: new Date().toISOString(),
    timeFile: rel(timePath),
    ...timing,
  }
}

// ---------------------------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------------------------

// docs/v2-source-identity.json's digest is sha256 over the compact JSON of
// [{bytes, path, sha256}] sorted by path (Python json.dumps(..., separators=(',', ':'),
// sort_keys=True), so non-ASCII is \u-escaped). Its inventory is every git-visible (tracked or
// untracked, not ignored) file under the recorded top-level directories plus the recorded root files.
function inventoryDigest(entries) {
  const json = JSON.stringify(entries.map(({ bytes, path, sha256: hash }) => ({ bytes, path, sha256: hash })))
    .replace(/[\u0080-￿]/g, (char) => `\\u${char.charCodeAt(0).toString(16).padStart(4, '0')}`)
  return sha256(json)
}

function sourceIdentity() {
  const recordPath = resolve(repoRoot, 'docs/v2-source-identity.json')
  const record = JSON.parse(readFileSync(recordPath, 'utf8'))
  const methodVerified = inventoryDigest(record.files) === record.sha256
  const roots = [...new Set(record.files.map(({ path }) => (path.includes('/') ? path.split('/')[0] : path)))]
  const listed = run('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', ...roots])
    .split('\0').filter(Boolean)
  const paths = [...new Set(listed)].filter((path) => statOrNull(resolve(repoRoot, path))?.isFile()).sort()
  const entries = paths.map((path) => {
    const value = readFileSync(resolve(repoRoot, path))
    return { bytes: value.length, path, sha256: sha256(value) }
  })
  const recorded = new Map(record.files.map((file) => [file.path, file.sha256]))
  const current = new Map(entries.map((file) => [file.path, file.sha256]))
  const digest = inventoryDigest(entries)
  return {
    method: 'docs/v2-source-identity.json method: sha256 of compact sorted-key JSON of [{bytes,path,sha256}] sorted by path',
    methodVerified,
    scope: `git-visible files under ${roots.join(', ')}`,
    digest,
    files: entries.length,
    recordedDigest: record.sha256,
    matchesRecorded: digest === record.sha256,
    changesFromRecorded: {
      added: paths.filter((path) => !recorded.has(path)),
      removed: [...recorded.keys()].filter((path) => !current.has(path)),
      modified: paths.filter((path) => recorded.has(path) && recorded.get(path) !== current.get(path)),
    },
  }
}

function binaryIdentity(path) {
  const stat = statOrNull(resolve(repoRoot, path))
  if (!stat) return { path, status: 'missing' }
  return { path, bytes: stat.size, sha256: sha256(readFileSync(resolve(repoRoot, path))), mtime: stat.mtime.toISOString() }
}

function packageVersion(name) {
  return readJsonOrNull(resolve(repoRoot, 'node_modules', name, 'package.json'))?.version ?? null
}

function collectInputs({ label, options, specs, argv }) {
  const porcelain = run('git', ['status', '--porcelain']).split('\n').filter(Boolean)
  return {
    schema: SCHEMA,
    label,
    createdAt: new Date().toISOString(),
    argv,
    options,
    git: { head: run('git', ['rev-parse', 'HEAD']).trim(), dirtyCount: porcelain.length, dirty: porcelain },
    sourceIdentity: sourceIdentity(),
    compiler: {
      jscLink: binaryIdentity('target/release/jsc-link'),
      codegenDylib: binaryIdentity('target/release/librustc_codegen_mires.dylib'),
    },
    toolchain: {
      node: process.version,
      rustc: run('rustc', ['--version', '--verbose']).trim(),
      rollup: packageVersion('rollup'),
      terser: packageVersion('terser'),
      zlib: process.versions.zlib,
      brotli: process.versions.brotli,
    },
    host: {
      platform: process.platform,
      arch: process.arch,
      osRelease: os.release(),
      cpu: os.cpus()[0]?.model ?? null,
      logicalCpus: os.cpus().length,
      memoryBytes: os.totalmem(),
    },
    environment: options.build ? { RCGJS_COST_PROFILE: '1' } : {},
    protocol: {
      gzip: 'node:zlib gzipSync level 9',
      brotli: 'node:zlib brotliCompressSync quality 11, lgwin 22',
      fetchedTotal: 'each fetched resource counted once: a package entry bundle plus any worker bundle it loads; shims are inside the bundles',
      linkCost: '/usr/bin/time -l around the whole package build tool: wall seconds, and max RSS of the largest single process in the tree',
      runtime: `experiments/browser-workloads/tools/run.mjs ${corpusRunMode(options.browsers).join(' ')}, ${options.samples} samples after ${options.warmups} warmups; `
        + 'pureBenchmarks: ms per sample of N calls in Node; paired timing (tools/timing-protocol.mjs): ms per call of the Rust build against the pinned JS reference '
        + 'and against a second Rust instance (A/A), per environment, rust/js ratio = Rust p50 / JS p50',
    },
    targets: Object.fromEntries(options.targets.map((name) => [name, {
      command: ['node', specs[name].tool, ...specs[name].args],
      dist: specs[name].dist,
      ...(specs[name].optimizer ? { optimizer: specs[name].optimizer } : {}),
    }])),
  }
}

// ---------------------------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------------------------

function measureArtifact(spec, artifact, build) {
  const path = resolve(repoRoot, spec.dist, artifact.file)
  const info = fileInfo(path, build)
  const record = { file: artifact.file, kind: artifact.kind, fetched: artifact.fetched, ...info }
  if (info.status === 'missing') return record
  const value = readFileSync(path)
  record.gzip = gzipBytes(value)
  record.brotli = brotliBytes(value)
  if (artifact.raw) {
    const rawPath = resolve(repoRoot, spec.dist, artifact.raw)
    const rawStat = statOrNull(rawPath)
    record.raw = rawStat ? { file: artifact.raw, bytes: rawStat.size, status: fileStatus(rawStat, build) } : { file: artifact.raw, status: 'missing' }
  }
  return record
}

function fetchedTotal(artifacts) {
  const fetched = artifacts.filter((artifact) => artifact.fetched)
  if (fetched.length === 0 || fetched.some((artifact) => artifact.status === 'missing')) return null
  return {
    files: fetched.map((artifact) => artifact.file),
    bytes: fetched.reduce((sum, artifact) => sum + artifact.bytes, 0),
    gzip: fetched.reduce((sum, artifact) => sum + artifact.gzip, 0),
    brotli: fetched.reduce((sum, artifact) => sum + artifact.brotli, 0),
  }
}

// Compares the corpus artifacts with the corpus build receipt. The receipt's gzip9 comes from the
// system `gzip -9 -n` when present, so it is reported but not expected to match Node zlib.
function corpusReceiptCheck(spec, artifacts) {
  const receiptPath = resolve(repoRoot, spec.receipt)
  const receipt = readJsonOrNull(receiptPath)
  if (!receipt) return { receipt: spec.receipt, status: 'missing' }
  const counts = { bytes: 0, sha256: 0, brotli11: 0, gzip9: 0 }
  const mismatches = []
  let compared = 0
  for (const name of CORPUS_WORKLOADS) {
    const mine = artifacts.find((artifact) => artifact.file === `${name}/index.min.js`)
    const theirs = receipt.workloads?.[name]?.artifact
    if (!mine || mine.status === 'missing' || !theirs) {
      mismatches.push({ workload: name, reason: 'artifact or receipt entry missing' })
      continue
    }
    compared++
    for (const [field, value] of [['bytes', mine.bytes], ['sha256', mine.sha256], ['brotli11', mine.brotli], ['gzip9', mine.gzip]]) {
      if (theirs[field] === value) counts[field]++
      else if (field !== 'gzip9') mismatches.push({ workload: name, field, receipt: theirs[field], measured: value })
    }
  }
  return {
    receipt: spec.receipt,
    generatedAt: receipt.generatedAt ?? null,
    receiptPassed: receipt.status?.passed ?? null,
    compared,
    matching: counts,
    identical: compared === CORPUS_WORKLOADS.length && mismatches.length === 0,
    gzipNote: 'receipt gzip9 uses system gzip -9 -n when available; measured gzip is Node zlib level 9',
    mismatches,
  }
}

function emittedCounts(text) {
  const helpers = {}
  for (const match of text.matchAll(/__rt\.([A-Za-z_$][\w$]*)/g)) helpers[match[1]] = (helpers[match[1]] ?? 0) + 1
  return {
    bytes: Buffer.byteLength(text),
    functionItems: (text.match(/^function /gm) ?? []).length,
    tryBlocks: occurrences(text, 'try{'),
    isAbortChecks: occurrences(text, 'if(__rt.is_abort('),
    rtReferences: Object.values(helpers).reduce((sum, count) => sum + count, 0),
    rtHelpers: sortCounts(helpers),
  }
}

function decisionCounts(lane, itemLanes, optimizer) {
  const declined = lane?.inline?.declined ?? {}
  const items = itemLanes?.items ?? null
  return {
    applied: lane?.passes?.applied ?? null,
    wouldApply: lane?.passes?.would_apply ?? null,
    range: lane?.range ?? null,
    inline: lane?.inline ? {
      applied: lane.inline.applied,
      crossObjectApplied: lane.inline.cross_object_applied,
      declinedBudget: lane.inline.declined_budget,
      exceptionalControlFlowDeclines: declined['inline.exceptional-control-flow'] ?? 0,
      declined: sortCounts(declined),
    } : null,
    devirtApplied: lane?.devirt?.applied ?? null,
    fallbackCount: lane?.fallback_count ?? null,
    planRetries: lane?.plan_retries ?? null,
    witnesses: lane?.witnesses ?? null,
    abiDeclines: lane?.abi ? countBy(lane.abi.declined, 'reason') : null,
    physical: lane?.physical ? {
      selected: lane.physical.selected_calls?.length ?? 0,
      reasons: countBy(lane.physical.decisions, 'reason'),
    } : null,
    reprApplied: lane?.repr?.final_emitted?.applied ?? null,
    itemLanes: items ? {
      total: items.length,
      byLane: countBy(items, 'lane'),
      declines: countBy(items.filter((item) => item.decline), 'decline'),
    } : null,
    optimizerReport: optimizer ? {
      passesApplied: optimizer.passes?.applied ?? null,
      programBytes: optimizer.program_bytes ?? null,
      droppedItems: optimizer.dropped_items ?? null,
      dedupGroups: optimizer.dedup?.length ?? null,
    } : null,
  }
}

function measureLink(spec, link, build) {
  const distPath = (file) => resolve(repoRoot, spec.dist, file)
  const optimizerPath = resolve(repoRoot, link.optimizerReport)
  const sources = {
    program: fileInfo(distPath(link.program), build),
    laneReport: link.laneReport ? fileInfo(distPath(link.laneReport), build) : null,
    itemLanes: fileInfo(distPath(link.itemLanes), build),
    optimizerReport: fileInfo(optimizerPath, build),
  }
  const optimizer = readJsonOrNull(optimizerPath)
  const lane = link.laneReport ? readJsonOrNull(distPath(link.laneReport)) : optimizer?.jir_lane ?? null
  const program = existsSync(distPath(link.program)) ? readFileSync(distPath(link.program), 'utf8') : null
  return {
    sources,
    decisions: lane ? decisionCounts(lane, readJsonOrNull(distPath(link.itemLanes)), optimizer) : null,
    emitted: program === null ? null : emittedCounts(program),
  }
}

function runKernels(name, spec, labelDir) {
  const input = resolve(repoRoot, spec.dist, spec.kernels)
  if (!existsSync(input)) return { status: 'skipped', reason: `${rel(input)} missing` }
  const receipt = resolve(labelDir, 'receipts', `${name}-kernels.json`)
  const logPath = resolve(labelDir, 'logs', `${name}-kernels.log`)
  const result = spawnSync(process.execPath, ['experiments/tools/check-oss-kernels.mjs', input, receipt], {
    cwd: repoRoot, encoding: 'utf8',
  })
  writeFileSync(logPath, `${result.stdout ?? ''}${result.stderr ?? ''}`)
  const record = readJsonOrNull(receipt)
  return {
    status: record ? (record.passed ? 'passed' : 'failed') : 'failed',
    exitCode: result.status,
    passed: record ? record.checks.filter((check) => check.passed).length : null,
    total: record ? record.checks.length : null,
    candidateSha256: record?.candidate?.sha256 ?? null,
    receipt: rel(receipt),
    log: rel(logPath),
  }
}

// `--pure` (Node only) without browsers; otherwise the runner's browser path, which also runs its
// browser checks and, with Chrome, the service-worker lifecycle.
export function corpusRunMode(browsers) {
  return browsers.length === 0 ? ['--pure'] : ['--browsers', browsers.join(',')]
}

// The numbers measure.mjs keeps from a corpus run.mjs receipt. Per workload: the Node
// pureBenchmarks fields (unchanged), plus the Node paired-timing ratios. `timing` carries the paired
// protocol's per-arm rows and per-cell comparisons for every environment, the reference library
// versions and the machine load average at the start and end of the run.
export function corpusRuntimeFromReceipt(record) {
  const comparisons = record?.timing?.comparisons ?? []
  const rows = record?.timing?.rows ?? []
  const comparison = (environmentKey, workload, kind) => comparisons.find((entry) =>
    entry.environmentKey === environmentKey && entry.workload === workload && entry.comparison === kind)
  const row = (environmentKey, workload, arm) => rows.find((entry) =>
    entry.environmentKey === environmentKey && entry.workload === workload && entry.pair === 'rust-vs-js' && entry.arm === arm)
  const workloads = {}
  for (const name of CORPUS_TIMED) {
    const bench = record?.pureBenchmarks?.[name]
    const main = comparison('node', name, 'rust/js')
    const control = comparison('node', name, 'A/A')
    workloads[name] = {
      ...(bench?.milliseconds ? {
        p50: bench.milliseconds.p50,
        p95: bench.milliseconds.p95,
        min: bench.milliseconds.min,
        max: bench.milliseconds.max,
        samples: bench.milliseconds.count,
        callsPerSample: bench.iterations,
        checksum: bench.checksum,
      } : { error: bench?.error ?? 'no benchmark record' }),
      rustP50PerCall: row('node', name, 'rust')?.p50 ?? null,
      jsP50PerCall: row('node', name, 'js')?.p50 ?? null,
      rustJsRatio: main?.status === 'ok' ? main.ratio : null,
      rustJsPairedRatio: main?.status === 'ok' ? main.pairedRatio : null,
      aaRatio: control?.status === 'ok' ? control.ratio : null,
      timingStatus: main ? `${main.status}/${control?.status ?? 'missing'}` : 'missing',
    }
  }
  const timing = record?.timing
  return {
    workloads,
    timing: timing ? {
      protocolVersion: timing.protocol?.version ?? null,
      loadAverage: timing.loadAverage ?? null,
      references: Object.fromEntries(Object.entries(timing.references ?? {})
        .map(([name, reference]) => [name, { package: reference.package, version: reference.version ?? null, pinned: reference.pinned ?? null }])),
      environments: Object.fromEntries(Object.entries(timing.environments ?? {}).map(([key, environment]) => [key, {
        status: environment.status,
        label: environment.environment?.label ?? key,
        ...(environment.environment?.driver ? { driver: environment.environment.driver } : {}),
        ...(environment.reason ? { reason: environment.reason } : {}),
      }])),
      rows,
      comparisons,
    } : null,
  }
}

function runCorpusTiming(options, labelDir) {
  const receipt = resolve(labelDir, 'receipts', 'corpus-runtime.json')
  const logPath = resolve(labelDir, 'logs', 'corpus-runtime.log')
  const args = ['experiments/browser-workloads/tools/run.mjs', ...corpusRunMode(options.browsers),
    '--samples', String(options.samples), '--warmups', String(options.warmups), '--output', receipt]
  const result = spawnSync(process.execPath, args, { cwd: repoRoot, encoding: 'utf8', maxBuffer: 64 << 20 })
  writeFileSync(logPath, `${result.stdout ?? ''}${result.stderr ?? ''}`)
  const record = readJsonOrNull(receipt)
  const { workloads, timing } = corpusRuntimeFromReceipt(record)
  return {
    status: record ? (record.status?.passed ? 'passed' : 'failed') : 'failed',
    exitCode: result.status,
    command: ['node', ...args.map((arg) => (arg === receipt ? rel(receipt) : arg))],
    browsers: options.browsers,
    unit: 'p50/p95/min/max: milliseconds per sample of callsPerSample calls (pureBenchmarks); *PerCall and timing rows: milliseconds per call',
    purePassed: record?.status?.purePassed ?? null,
    timingPassed: record?.status?.timingPassed ?? null,
    browserPassed: options.browsers.length === 0 ? null : record?.status?.browserPassed ?? null,
    buildReceiptMatched: record?.status?.buildReceiptMatched ?? null,
    receipt: rel(receipt),
    log: rel(logPath),
    workloads,
    timing,
  }
}

// ---------------------------------------------------------------------------------------------
// RESULTS.md block
// ---------------------------------------------------------------------------------------------

const dash = '—'
const fmt = (value, digits = 0) => (value === null || value === undefined ? dash : Number(value).toFixed(digits))

const displayName = (name, specs) => (name === 'pro' && specs.pro.optimizer === 'off' ? 'pro (opt-off)' : name)

// The RESULTS.md runtime tables for a corpus runtime record. The first table keeps the original
// Node pureBenchmarks columns and appends the Node paired-timing ratios; browser environments, when
// timed, get a second table, and an unavailable browser is a status row with no numbers.
export function corpusRuntimeLines(corpus) {
  const lines = ['', '| workload | p50 ms | p95 ms | calls/sample | samples | rust/js ratio | A/A ratio |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
  for (const [name, bench] of Object.entries(corpus.workloads)) {
    lines.push(`| ${name} | ${fmt(bench.p50, 2)} | ${fmt(bench.p95, 2)} | ${fmt(bench.callsPerSample)} | ${fmt(bench.samples)} | ${fmt(bench.rustJsRatio, 2)} | ${fmt(bench.aaRatio, 3)} |`)
  }
  const environments = Object.entries(corpus.timing?.environments ?? {}).filter(([key]) => key !== 'node')
  if (environments.length === 0) return lines
  lines.push('', '| environment | workload | rust ms/call p50 | js ms/call p50 | rust/js ratio | A/A ratio | status |',
    '| --- | --- | ---: | ---: | ---: | ---: | --- |')
  const { rows, comparisons } = corpus.timing
  for (const [key, environment] of environments) {
    const cells = comparisons.filter((entry) => entry.environmentKey === key && entry.workload)
    if (cells.length === 0) {
      lines.push(`| ${environment.label} | ${dash} | ${dash} | ${dash} | ${dash} | ${dash} | ${environment.status}${environment.reason ? `: ${String(environment.reason).split('\n')[0].replaceAll('|', '/').slice(0, 160)}` : ''} |`)
      continue
    }
    for (const workload of CORPUS_TIMED) {
      const main = cells.find((entry) => entry.workload === workload && entry.comparison === 'rust/js')
      const control = cells.find((entry) => entry.workload === workload && entry.comparison === 'A/A')
      const p50 = (arm) => rows.find((entry) => entry.environmentKey === key && entry.workload === workload && entry.pair === 'rust-vs-js' && entry.arm === arm)?.p50
      lines.push(`| ${environment.label} | ${workload} | ${fmt(p50('rust'), 3)} | ${fmt(p50('js'), 3)} | ${fmt(main?.status === 'ok' ? main.ratio : null, 2)} | ${fmt(control?.status === 'ok' ? control.ratio : null, 3)} | ${main?.status ?? 'missing'}/${control?.status ?? 'missing'} |`)
    }
  }
  return lines
}

// The only Pro information allowed into public receipts: status, and wall time and peak RSS when
// the build completed.
function privateStatus(build) {
  const status = build.status === 'not-built' ? 'skipped' : build.status
  return {
    status,
    wallSeconds: status === 'ok' ? build.wallSeconds : null,
    peakRssMiB: status === 'ok' ? build.peakRssMiB : null,
  }
}

export function privateStatusLine(name, build, specs) {
  const { status, wallSeconds, peakRssMiB } = privateStatus(build)
  const timing = status === 'ok' ? `, wall ${fmt(wallSeconds, 1)} s, peak RSS ${fmt(peakRssMiB)} MiB` : ''
  return `${displayName(name, specs)}: ${status}${timing}`
}

// One RESULTS.md block for the given (already filtered) targets. `extraLines` are appended as
// bullets; the public block uses them for the private targets' status lines.
function resultsBlock({ inputs, specs, sizes, cost, runtime, receiptsDir, extraLines = [] }) {
  const short = (hash) => (hash ? hash.slice(0, 12) : dash)
  const identity = inputs.sourceIdentity
  const lines = [
    `## ${inputs.label} — ${inputs.createdAt}`,
    '',
    `- HEAD \`${inputs.git.head}\`, ${inputs.git.dirtyCount} dirty path(s); compiler source digest \`${short(identity.digest)}\` (${identity.matchesRecorded ? 'matches' : 'differs from'} docs/v2-source-identity.json)`,
    `- jsc-link \`${short(inputs.compiler.jscLink.sha256)}\`, codegen dylib \`${short(inputs.compiler.codegenDylib.sha256)}\`, node ${inputs.toolchain.node}, built: ${inputs.options.build ? 'yes' : 'no'}`,
    `- receipts: \`${receiptsDir}\``,
  ]
  const artifactRows = []
  for (const [name, target] of Object.entries(sizes.targets)) {
    const tableFiles = new Set(specs[name].artifacts.filter((artifact) => artifact.table).map((artifact) => artifact.file))
    for (const artifact of target.artifacts.filter((entry) => tableFiles.has(entry.file))) {
      const raw = artifact.kind === 'min' ? artifact.raw?.bytes : artifact.bytes
      const min = artifact.kind === 'min' ? artifact.bytes : null
      artifactRows.push(`| ${displayName(name, specs)} | ${artifact.file} | ${fmt(raw)} | ${fmt(min)} | ${fmt(artifact.gzip)} | ${fmt(artifact.brotli)} | ${artifact.status} |`)
    }
    if (target.fetchedTotal && target.fetchedTotal.files.length > 1) {
      const total = target.fetchedTotal
      artifactRows.push(`| ${displayName(name, specs)} | fetched total (${total.files.join(' + ')}) | ${dash} | ${total.bytes} | ${total.gzip} | ${total.brotli} | ${dash} |`)
    }
  }
  if (artifactRows.length > 0) {
    lines.push('', '| target | file | raw | min | gzip | brotli | status |', '| --- | --- | ---: | ---: | ---: | ---: | --- |', ...artifactRows)
  }
  if (Object.keys(cost.targets).length > 0) {
    lines.push('', '| target | wall s | peak RSS MiB | status |', '| --- | ---: | ---: | --- |')
    for (const [name, build] of Object.entries(cost.targets)) {
      const status = build.status === 'timed-out' ? `timed-out (${build.timeoutMinutes} min)` : build.status
      lines.push(`| ${displayName(name, specs)} | ${fmt(build.wallSeconds, 1)} | ${fmt(build.peakRssMiB)} | ${status} |`)
    }
  }
  if (runtime.corpus) lines.push(...corpusRuntimeLines(runtime.corpus))
  const notes = []
  if (runtime.corpus) {
    const browsers = runtime.corpus.browsers ?? []
    notes.push(`corpus ${browsers.length === 0 ? 'pure run' : `run (browsers: ${browsers.join(',')})`}: ${runtime.corpus.status}`)
    const timing = runtime.corpus.timing
    if (timing) {
      const references = Object.values(timing.references).map((reference) => `${reference.package} ${reference.version ?? 'missing'}`).join(', ')
      const load = (values) => (values ? values.map((value) => fmt(value, 1)).join('/') : dash)
      notes.push(`paired timing ${runtime.corpus.timingPassed ? 'passed' : 'failed'} vs ${references}; load avg (1/5/15 min) ${load(timing.loadAverage?.start)} at start, ${load(timing.loadAverage?.end)} at end`)
    }
  }
  for (const [name, kernels] of Object.entries(runtime.kernels)) {
    notes.push(`${name} kernels: ${kernels.total === null || kernels.total === undefined ? kernels.status : `${kernels.passed}/${kernels.total}`}`)
  }
  if (sizes.targets.corpus?.receiptCheck) {
    const check = sizes.targets.corpus.receiptCheck
    notes.push(`corpus artifacts vs build.json: ${check.status === 'missing' ? 'receipt missing' : check.identical ? 'identical' : 'MISMATCH'}`)
  }
  if (notes.length > 0 || extraLines.length > 0) lines.push('')
  if (notes.length > 0) lines.push(`- ${notes.join('; ')}`)
  for (const line of extraLines) lines.push(`- ${line}`)
  return `${lines.join('\n')}\n`
}

function appendResults(path, header, block) {
  mkdirSync(dirname(path), { recursive: true })
  if (!existsSync(path)) writeFileSync(path, header)
  appendFileSync(path, `\n${block}`)
}

// ---------------------------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------------------------

const USAGE = `usage: node experiments/v2-restoration/measure.mjs --label <name>
  [--targets ${TARGETS.join(',')}] [--no-build] [--force]
  [--pro-timeout-min 15] [--pro-optimizer on|off] [--samples 9] [--warmups 2]
  [--browsers ${CORPUS_BROWSERS.join(',')}]`

export function parseOptions(argv) {
  const { values } = parseArgs({
    args: argv,
    strict: true,
    options: {
      label: { type: 'string' },
      targets: { type: 'string', default: TARGETS.join(',') },
      'no-build': { type: 'boolean', default: false },
      force: { type: 'boolean', default: false },
      'pro-timeout-min': { type: 'string', default: '15' },
      'pro-optimizer': { type: 'string', default: 'on' },
      samples: { type: 'string', default: '9' },
      warmups: { type: 'string', default: '2' },
      browsers: { type: 'string', default: '' },
      help: { type: 'boolean', short: 'h', default: false },
    },
  })
  if (values.help) {
    process.stdout.write(`${USAGE}\n`)
    process.exit(0)
  }
  const fail = (message) => {
    throw new Error(`${message}\n${USAGE}`)
  }
  if (!values.label || !/^[A-Za-z0-9._-]+$/.test(values.label)) fail('--label is required and must match [A-Za-z0-9._-]+')
  const requested = values.targets.split(',').map((name) => name.trim()).filter(Boolean)
  for (const name of requested) if (!TARGETS.includes(name)) fail(`unknown target: ${name}`)
  if (requested.length === 0) fail('--targets is empty')
  if (!['on', 'off'].includes(values['pro-optimizer'])) fail('--pro-optimizer must be on or off')
  const proTimeoutMinutes = Number(values['pro-timeout-min'])
  if (!(proTimeoutMinutes > 0)) fail('--pro-timeout-min must be a positive number')
  const samples = Number(values.samples)
  const warmups = Number(values.warmups)
  if (!Number.isInteger(samples) || samples < 1) fail('--samples must be a positive integer')
  if (!Number.isInteger(warmups) || warmups < 0) fail('--warmups must be a non-negative integer')
  const browsers = [...new Set(values.browsers.split(',').map((name) => name.trim()).filter(Boolean))]
  for (const name of browsers) if (!CORPUS_BROWSERS.includes(name)) fail(`unknown browser: ${name} (expected ${CORPUS_BROWSERS.join(', ')})`)
  return {
    label: values.label,
    force: values.force,
    options: {
      targets: TARGETS.filter((name) => requested.includes(name)),
      build: !values['no-build'],
      proTimeoutMinutes,
      proOptimizer: values['pro-optimizer'],
      samples,
      warmups,
      browsers,
    },
  }
}

function checkTools(options, specs) {
  const required = [['git', ['--version']], ['rustc', ['--version']], ['ps', ['-o', 'pid=', '-p', String(process.pid)]]]
  for (const [command, args] of required) run(command, args)
  const files = []
  if (options.build) {
    files.push('/usr/bin/time')
    for (const name of options.targets) files.push(resolve(repoRoot, specs[name].tool))
  }
  if (options.targets.includes('corpus')) files.push(resolve(repoRoot, 'experiments/browser-workloads/tools/run.mjs'))
  if (options.targets.some((name) => specs[name].kernels)) files.push(resolve(repoRoot, 'experiments/tools/check-oss-kernels.mjs'))
  files.push(resolve(repoRoot, 'docs/v2-source-identity.json'))
  const missing = files.filter((path) => !existsSync(path))
  if (missing.length > 0) throw new Error(`required tool(s) missing:\n  ${missing.join('\n  ')}`)
}

async function buildTarget(name, spec, options, labelDir) {
  const logPath = resolve(labelDir, 'logs', `${name}-build.log`)
  const timeoutMs = name === 'pro' ? options.proTimeoutMinutes * 60000 : null
  process.stdout.write(`measure: building ${name}${timeoutMs ? ` (timeout ${options.proTimeoutMinutes} min)` : ''}; log ${rel(logPath)}\n`)
  const result = await runTimed({
    command: process.execPath,
    args: [spec.tool, ...spec.args],
    cwd: repoRoot,
    env: { ...process.env, RCGJS_COST_PROFILE: '1' },
    logPath,
    timeoutMs,
  })
  result.command = ['node', ...result.command.slice(1)]
  const log = readFileSync(logPath, 'utf8')
  const compiled = [...log.matchAll(/^\s+Compiling (\S+) v/gm)].map((match) => match[1])
  process.stdout.write(`measure: ${name} ${result.status}${result.wallSeconds !== null ? ` in ${result.wallSeconds}s` : ''}\n`)
  return {
    built: true,
    ...result,
    peakRssMiB: result.maxRssBytes === null ? null : Math.round(result.maxRssBytes / 1048576),
    // Wall time includes these when they happen; a non-empty list or a sysroot rebuild means the
    // number is not a clean link-cost observation.
    cargoCompiled: compiled,
    sysrootRebuilt: /^==> building core/m.test(log),
    costProfile: parseCostProfile(log),
  }
}

async function main() {
  const argv = process.argv.slice(2)
  const { label, force, options } = parseOptions(argv)
  const specs = targetSpecs({ proOptimizer: options.proOptimizer })
  checkTools(options, specs)

  const labelDir = resolve(repoRoot, 'build/v2-restoration', label)
  const privateDir = resolve(labelDir, 'pro')
  const publicNames = options.targets.filter((name) => !isPrivate(name))
  const privateNames = options.targets.filter(isPrivate)
  const targetDir = (name) => (isPrivate(name) ? privateDir : labelDir)
  if (existsSync(labelDir)) {
    if (!force) throw new Error(`${rel(labelDir)} already exists; choose a new --label or pass --force`)
    rmSync(labelDir, { recursive: true, force: true })
  }
  mkdirSync(resolve(labelDir, 'logs'), { recursive: true })
  mkdirSync(resolve(labelDir, 'receipts'), { recursive: true })
  if (privateNames.length > 0) mkdirSync(resolve(privateDir, 'logs'), { recursive: true })

  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.once(signal, async () => {
      process.stderr.write(`measure: ${signal}; stopping the active build\n`)
      if (activeChild) await killTree(activeChild.pid)
      process.exit(130)
    })
  }

  // Every receipt keyed by target is split: public targets under labelDir, private ones under pro/.
  const pick = (record, names) => ({
    ...record,
    targets: Object.fromEntries(names.filter((name) => name in record.targets).map((name) => [name, record.targets[name]])),
  })
  const inputs = collectInputs({ label, options, specs, argv })
  writeJson(resolve(labelDir, 'inputs.json'), pick(inputs, publicNames))
  if (privateNames.length > 0) writeJson(resolve(privateDir, 'inputs.json'), pick(inputs, privateNames))

  // Builds run strictly one after another; nothing else is measured while a build runs.
  const builds = {}
  for (const name of options.targets) {
    builds[name] = options.build
      ? await buildTarget(name, specs[name], options, targetDir(name))
      : { built: false, status: 'not-built', wallSeconds: null, peakRssMiB: null }
  }

  const sizes = { schema: SCHEMA, label, protocol: inputs.protocol, targets: {} }
  const decisions = { schema: SCHEMA, label, targets: {} }
  for (const name of options.targets) {
    const spec = specs[name]
    const artifacts = spec.artifacts.map((artifact) => measureArtifact(spec, artifact, builds[name]))
    sizes.targets[name] = { dist: spec.dist, artifacts, fetchedTotal: spec.fetchedTotal ? fetchedTotal(artifacts) : null }
    if (name === 'corpus') {
      sizes.targets.corpus.receiptCheck = corpusReceiptCheck(spec, artifacts)
      if (existsSync(resolve(repoRoot, spec.receipt))) {
        copyFileSync(resolve(repoRoot, spec.receipt), resolve(labelDir, 'receipts', 'corpus-build.json'))
      }
    }
    decisions.targets[name] = Object.fromEntries(spec.links.map((link) => [link.name, measureLink(spec, link, builds[name])]))
  }

  const runtime = { schema: SCHEMA, label, corpus: null, kernels: {} }
  if (options.targets.includes('corpus')) {
    process.stdout.write(`measure: timing corpus workloads (${options.samples} samples, ${options.warmups} warmups${options.browsers.length ? `, browsers ${options.browsers.join(',')}` : ''})\n`)
    runtime.corpus = runCorpusTiming(options, labelDir)
  }
  for (const name of options.targets.filter((target) => specs[target].kernels)) {
    runtime.kernels[name] = runKernels(name, specs[name], labelDir)
  }

  const cost = { schema: SCHEMA, label, environment: inputs.environment, targets: builds }
  for (const build of Object.values(builds)) delete build.startedMs

  // Public receipts and block. Private targets appear only as status lines.
  const publicReceipts = {
    inputs: pick(inputs, publicNames), sizes: pick(sizes, publicNames), cost: pick(cost, publicNames),
    decisions: pick(decisions, publicNames), runtime,
  }
  writeJson(resolve(labelDir, 'sizes.json'), publicReceipts.sizes)
  writeJson(resolve(labelDir, 'compile-cost.json'), publicReceipts.cost)
  writeJson(resolve(labelDir, 'decisions.json'), publicReceipts.decisions)
  writeJson(resolve(labelDir, 'runtime.json'), runtime)
  const summary = summarize(publicReceipts)
  summary.privateTargets = Object.fromEntries(privateNames.map((name) => [name, privateStatus(builds[name])]))
  writeJson(resolve(labelDir, 'summary.json'), summary)
  appendResults(resultsPath, RESULTS_HEADER, resultsBlock({
    ...publicReceipts, specs, receiptsDir: `build/v2-restoration/${label}/`,
    extraLines: privateNames.map((name) => privateStatusLine(name, builds[name], specs)),
  }))
  process.stdout.write(`measure: wrote ${rel(labelDir)}/ and appended ${rel(resultsPath)}\n`)

  // Private receipts and block.
  if (privateNames.length > 0) {
    const privateReceipts = {
      inputs: pick(inputs, privateNames), sizes: pick(sizes, privateNames), cost: pick(cost, privateNames),
      decisions: pick(decisions, privateNames), runtime: { schema: SCHEMA, label, corpus: null, kernels: {} },
    }
    writeJson(resolve(privateDir, 'sizes.json'), privateReceipts.sizes)
    writeJson(resolve(privateDir, 'compile-cost.json'), privateReceipts.cost)
    writeJson(resolve(privateDir, 'decisions.json'), privateReceipts.decisions)
    writeJson(resolve(privateDir, 'summary.json'), summarize(privateReceipts))
    appendResults(privateResultsPath, PRIVATE_RESULTS_HEADER, resultsBlock({
      ...privateReceipts, specs, receiptsDir: `build/v2-restoration/${label}/pro/`,
    }))
    process.stdout.write(`measure: wrote ${rel(privateDir)}/ and appended ${rel(privateResultsPath)}\n`)
  }

  for (const [name, build] of Object.entries(builds)) {
    if (!['ok', 'not-built'].includes(build.status)) process.stdout.write(`measure: note: ${name} build ${build.status} (${build.log})\n`)
  }
}

function summarize({ inputs, sizes, cost, decisions, runtime }) {
  const targets = {}
  for (const [name, size] of Object.entries(sizes.targets)) {
    const build = cost.targets[name]
    targets[name] = {
      build: {
        status: build.status,
        wallSeconds: build.wallSeconds ?? null,
        peakRssMiB: build.peakRssMiB ?? null,
        ...(build.log ? { log: build.log } : {}),
        ...(build.status === 'timed-out' ? { timeoutMinutes: build.timeoutMinutes } : {}),
      },
      fetchedTotal: size.fetchedTotal,
      artifacts: Object.fromEntries(size.artifacts.filter((artifact) => artifact.kind === 'min' || artifact.file.endsWith('program.js'))
        .map((artifact) => [artifact.file, artifact.status === 'missing'
          ? { status: 'missing' }
          : { bytes: artifact.bytes, gzip: artifact.gzip, brotli: artifact.brotli, status: artifact.status }])),
      links: Object.fromEntries(Object.entries(decisions.targets[name]).map(([link, record]) => [link, {
        applied: record.decisions?.applied ?? null,
        rangeAssertsRemoved: record.decisions?.range?.asserts_removed ?? null,
        inlineExceptionalControlFlowDeclines: record.decisions?.inline?.exceptionalControlFlowDeclines ?? null,
        fallbackCount: record.decisions?.fallbackCount ?? null,
        programBytes: record.emitted?.bytes ?? null,
        functionItems: record.emitted?.functionItems ?? null,
        tryBlocks: record.emitted?.tryBlocks ?? null,
        isAbortChecks: record.emitted?.isAbortChecks ?? null,
      }])),
    }
    if (size.receiptCheck) targets[name].receiptCheck = { identical: size.receiptCheck.identical ?? null, matching: size.receiptCheck.matching ?? null }
  }
  return {
    schema: SCHEMA,
    label: inputs.label,
    createdAt: inputs.createdAt,
    head: inputs.git.head,
    dirtyCount: inputs.git.dirtyCount,
    sourceDigest: inputs.sourceIdentity.digest,
    sourceDigestMatchesRecorded: inputs.sourceIdentity.matchesRecorded,
    jscLinkSha256: inputs.compiler.jscLink.sha256 ?? null,
    codegenDylibSha256: inputs.compiler.codegenDylib.sha256 ?? null,
    node: inputs.toolchain.node,
    built: inputs.options.build,
    targets,
    runtime: runtime.corpus ? {
      status: runtime.corpus.status,
      workloads: Object.fromEntries(Object.entries(runtime.corpus.workloads)
        .map(([name, bench]) => [name, {
          p50: bench.p50 ?? null, p95: bench.p95 ?? null, callsPerSample: bench.callsPerSample ?? null,
          rustJsRatio: bench.rustJsRatio ?? null, aaRatio: bench.aaRatio ?? null,
        }])),
      browsers: runtime.corpus.browsers ?? [],
      timing: runtime.corpus.timing ? {
        environments: runtime.corpus.timing.environments,
        loadAverage: runtime.corpus.timing.loadAverage,
        comparisons: runtime.corpus.timing.comparisons.map((entry) => ({
          environment: entry.environment, workload: entry.workload ?? null, comparison: entry.comparison ?? null,
          status: entry.status, ratio: entry.ratio ?? null,
        })),
      } : null,
    } : null,
    kernels: Object.fromEntries(Object.entries(runtime.kernels)
      .map(([name, kernels]) => [name, { status: kernels.status, passed: kernels.passed ?? null, total: kernels.total ?? null }])),
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`measure: ${error.message}\n`)
    process.exit(1)
  })
}
