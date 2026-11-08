// node --test experiments/v2-restoration/measure.test.mjs
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'

import {
  corpusRunMode, corpusRuntimeFromReceipt, corpusRuntimeLines, parseCostProfile, parseOptions, parseTimeOutput,
  privateStatusLine, runTimed, targetSpecs,
} from './measure.mjs'

function isAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error.code === 'EPERM'
  }
}

function scratch(t) {
  const dir = mkdtempSync(join(tmpdir(), 'measure-test-'))
  t.after(() => rmSync(dir, { recursive: true, force: true }))
  return dir
}

test('timeout kills the process group and a descendant that left it, and records no duration', async (t) => {
  const dir = scratch(t)
  const pids = join(dir, 'pids')
  // A shell (group leader's child) starts: a background sleep in the same process group, and a
  // node process that spawns a detached sleep (new session, new group) — the jsc-link-under-rustc
  // shape plus the worst case of a descendant outside the group. Each records its pid.
  const detached = `const c=require('child_process').spawn('sleep',['300'],{detached:true,stdio:'ignore'});`
    + `require('fs').appendFileSync(${JSON.stringify(pids)},c.pid+'\\n');setTimeout(()=>{},300000)`
  const script = [
    `sleep 300 & echo $! >> ${pids}`,
    `${JSON.stringify(process.execPath)} -e ${JSON.stringify(detached)} & echo $! >> ${pids}`,
    'sleep 300',
  ].join('; ')
  const started = Date.now()
  const result = await runTimed({
    command: '/bin/sh', args: ['-c', script], cwd: dir, env: process.env,
    logPath: join(dir, 'fake-build.log'), timeoutMs: 0.05 * 60000,
  })
  const recorded = readFileSync(pids, 'utf8').trim().split('\n').map(Number)
  assert.equal(recorded.length, 3, 'all three descendants started')
  assert.equal(result.status, 'timed-out')
  assert.equal(result.timeoutMinutes, 0.05)
  assert.equal(result.wallSeconds, null)
  assert.equal(result.maxRssBytes, null)
  assert.equal('endedAt' in result, false)
  assert.ok(result.kill.processes >= 5, `tree snapshot covered time, node, sh and children (${result.kill.processes})`)
  assert.equal(result.kill.groupGone, true)
  assert.deepEqual(result.kill.survivors, [])
  for (const pid of recorded) assert.equal(isAlive(pid), false, `pid ${pid} was killed`)
  assert.ok(Date.now() - started < 20000, 'returned promptly after the timeout')
})

test('a finished command records wall time and peak RSS from /usr/bin/time', async (t) => {
  const dir = scratch(t)
  const result = await runTimed({
    command: process.execPath, args: ['-e', 'Buffer.alloc(200e6).fill(1); console.error("rcgjs-cost-profile: lower_calls=12 phase=link")'],
    cwd: dir, env: process.env, logPath: join(dir, 'ok-build.log'),
  })
  assert.equal(result.status, 'ok')
  assert.equal(result.exitCode, 0)
  assert.ok(result.wallSeconds >= 0)
  assert.ok(result.maxRssBytes > 200e6, `max RSS ${result.maxRssBytes}`)
  const profile = parseCostProfile(readFileSync(join(dir, 'ok-build.log'), 'utf8'))
  assert.deepEqual(profile, {
    links: [{ order: 0, output: null, attributedBy: 'log-order', counters: { lower_calls: 12, phase: 'link' } }],
    unparsed: [],
  })
})

test('a failing command is recorded as failed with its exit code', async (t) => {
  const dir = scratch(t)
  const result = await runTimed({
    command: '/bin/sh', args: ['-c', 'echo boom >&2; exit 3'], cwd: dir, env: process.env,
    logPath: join(dir, 'bad-build.log'),
  })
  assert.equal(result.status, 'failed')
  assert.equal(result.exitCode, 3)
  assert.match(readFileSync(join(dir, 'bad-build.log'), 'utf8'), /boom/)
})

test('parsers', () => {
  const time = parseTimeOutput('      135.20 real       120.10 user        10.50 sys\n  2147483648  maximum resident set size\n')
  assert.deepEqual(time, { wallSeconds: 135.2, userSeconds: 120.1, sysSeconds: 10.5, maxRssBytes: 2147483648 })
  assert.deepEqual(parseTimeOutput(''), { wallSeconds: null, userSeconds: null, sysSeconds: null, maxRssBytes: null })
  // The jsc-link shape: one key per line, one block per link; a repeated key starts the next link.
  // A block with `output=` is attributed by name, one without only by its position in the log.
  const log = [
    'rcgjs-cost-profile: phase.link.read.ms=155.770',
    'rcgjs-cost-profile: phase.link.read.calls=1',
    '   Compiling something',
    'rcgjs-cost-profile: output=/x/marked/program.js',
    'rcgjs-cost-profile: phase.link.read.ms=12.5',
    'rcgjs-cost-profile: lower.final=x7',
    'rcgjs-cost-profile: free text',
  ].join('\n')
  assert.deepEqual(parseCostProfile(log), {
    links: [
      { order: 0, output: null, attributedBy: 'log-order', counters: { 'phase.link.read.ms': 155.77, 'phase.link.read.calls': 1 } },
      { order: 1, output: '/x/marked/program.js', attributedBy: 'output', counters: { 'phase.link.read.ms': 12.5, 'lower.final': 'x7' } },
    ],
    unparsed: ['free text'],
  })
})

test('the public line for a private target carries only status, wall time and peak RSS', () => {
  const on = targetSpecs({ proOptimizer: 'on' })
  const off = targetSpecs({ proOptimizer: 'off' })
  assert.equal(privateStatusLine('pro', { status: 'ok', wallSeconds: 523.44, peakRssMiB: 2890, log: 'x' }, on),
    'pro: ok, wall 523.4 s, peak RSS 2890 MiB')
  assert.equal(privateStatusLine('pro', { status: 'timed-out', wallSeconds: null, peakRssMiB: null, timeoutMinutes: 15 }, on), 'pro: timed-out')
  assert.equal(privateStatusLine('pro', { status: 'failed', wallSeconds: 12.3, peakRssMiB: 900 }, on), 'pro: failed')
  assert.equal(privateStatusLine('pro', { status: 'not-built', wallSeconds: null, peakRssMiB: null }, off), 'pro (opt-off): skipped')
})

test('--browsers defaults to none (pure only), validates names and passes through to the corpus runner', () => {
  assert.deepEqual(parseOptions(['--label', 'x']).options.browsers, [])
  assert.deepEqual(corpusRunMode([]), ['--pure'])
  const { options } = parseOptions(['--label', 'x', '--browsers', 'chrome, firefox,chrome,,safari'])
  assert.deepEqual(options.browsers, ['chrome', 'firefox', 'safari'])
  assert.deepEqual(corpusRunMode(options.browsers), ['--browsers', 'chrome,firefox,safari'])
  assert.throws(() => parseOptions(['--label', 'x', '--browsers', 'chrome,opera']), /unknown browser: opera/)
})

// A minimal run.mjs receipt: Node pureBenchmarks plus the paired-timing rows and comparisons for
// Node and Chrome, and an unavailable Safari.
function corpusReceipt() {
  const bench = (p50, iterations) => ({ checksum: 1, iterations, milliseconds: { count: 5, min: p50 - 1, p50, p95: p50 + 1, max: p50 + 2 } })
  const row = (environmentKey, workload, arm, p50) => ({ environmentKey, workload, pair: 'rust-vs-js', arm, p50 })
  const comparison = (environmentKey, workload, kind, ratio, status = 'ok') =>
    ({ environmentKey, environment: environmentKey, workload, comparison: kind, status, ratio, pairedRatio: ratio })
  const rows = []
  const comparisons = []
  for (const [workload, rust, js] of [['marked', 100, 0.5], ['minisearch', 1.5, 0.025], ['fflate', 6, 0.2]]) {
    for (const key of ['node', 'chrome']) {
      rows.push(row(key, workload, 'rust', rust), row(key, workload, 'js', js))
      comparisons.push(comparison(key, workload, 'rust/js', rust / js), comparison(key, workload, 'A/A', 1.01))
    }
  }
  comparisons.find((entry) => entry.environmentKey === 'chrome' && entry.workload === 'fflate' && entry.comparison === 'rust/js').status = 'checksum-mismatch'
  comparisons.push({ environmentKey: 'safari', environment: 'safari', status: 'unavailable', reason: 'Remote automation is off | enable it' })
  return {
    status: { passed: false, purePassed: true, timingPassed: false },
    pureBenchmarks: { marked: bench(400, 5), minisearch: bench(5, 4), fflate: bench(20, 3) },
    timing: {
      protocol: { version: 1 },
      loadAverage: { start: [12, 10, 9], end: [14, 11, 9] },
      references: { marked: { package: 'marked', version: '18.1.1', pinned: '18.1.1', entry: {} } },
      environments: {
        node: { status: 'ok', environment: { label: 'node v26.8.1' }, cells: [] },
        chrome: { status: 'failed', environment: { label: 'chrome 157.0', driver: '157.0 (x)' }, cells: [] },
        safari: { status: 'unavailable', reason: 'Remote automation is off | enable it', cells: [] },
      },
      rows,
      comparisons,
    },
  }
}

test('corpus runtime keeps the pureBenchmarks fields and adds the Node rust/js and A/A ratios', () => {
  const { workloads, timing } = corpusRuntimeFromReceipt(corpusReceipt())
  assert.deepEqual(Object.keys(workloads), ['marked', 'minisearch', 'fflate'])
  assert.equal(workloads.marked.p50, 400)
  assert.equal(workloads.marked.callsPerSample, 5)
  assert.equal(workloads.marked.samples, 5)
  assert.equal(workloads.marked.rustJsRatio, 200)
  assert.equal(workloads.marked.aaRatio, 1.01)
  assert.equal(workloads.marked.rustP50PerCall, 100)
  assert.equal(workloads.marked.jsP50PerCall, 0.5)
  assert.equal(workloads.marked.timingStatus, 'ok/ok')
  assert.deepEqual(timing.references.marked, { package: 'marked', version: '18.1.1', pinned: '18.1.1' })
  assert.deepEqual(timing.environments.safari, { status: 'unavailable', label: 'safari', reason: 'Remote automation is off | enable it' })
  assert.deepEqual(timing.loadAverage, { start: [12, 10, 9], end: [14, 11, 9] })
  // A receipt from before paired timing still yields the original fields and null ratios.
  const legacy = corpusRuntimeFromReceipt({ pureBenchmarks: corpusReceipt().pureBenchmarks })
  assert.equal(legacy.workloads.fflate.p50, 20)
  assert.equal(legacy.workloads.fflate.rustJsRatio, null)
  assert.equal(legacy.timing, null)
})

test('the RESULTS.md runtime table appends ratio columns; browsers get their own rows, never numbers when unavailable', () => {
  const lines = corpusRuntimeLines(corpusRuntimeFromReceipt(corpusReceipt()))
  assert.equal(lines[1], '| workload | p50 ms | p95 ms | calls/sample | samples | rust/js ratio | A/A ratio |')
  assert.equal(lines[3], '| marked | 400.00 | 401.00 | 5 | 5 | 200.00 | 1.010 |')
  assert.ok(lines.includes('| chrome 157.0 | marked | 100.000 | 0.500 | 200.00 | 1.010 | ok/ok |'))
  assert.ok(lines.includes('| chrome 157.0 | fflate | 6.000 | 0.200 | — | 1.010 | checksum-mismatch/ok |'))
  assert.ok(lines.includes('| safari | — | — | — | — | — | unavailable: Remote automation is off / enable it |'))
  assert.ok(!lines.some((line) => line.startsWith('| node')), 'Node ratios live in the first table only')
  const pureOnly = corpusRuntimeLines(corpusRuntimeFromReceipt({ ...corpusReceipt(), timing: { ...corpusReceipt().timing,
    environments: { node: corpusReceipt().timing.environments.node } } }))
  assert.equal(pureOnly.length, 6, 'no browser table without browser environments')
})
