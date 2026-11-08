// node --test experiments/v2-restoration/measure.test.mjs
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'

import { parseCostProfile, parseTimeOutput, privateStatusLine, runTimed, targetSpecs } from './measure.mjs'

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
