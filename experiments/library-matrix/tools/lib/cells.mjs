// Running one cell in a fresh Node process, with a hang watchdog (Node only). The child writes
// `\u0001heartbeat <label>` lines to stderr before each call (check) or each timed run (bench); a
// child silent for longer than `callTimeoutMs` is killed and the cell is recorded as `hang`, naming
// the last call it started, instead of waiting for a whole-cell timeout.

import { spawn } from 'node:child_process'

export function runNodeCell(script, config, { cwd, callTimeoutMs, totalTimeoutMs = 3_600_000 }) {
  return new Promise((resolvePromise) => {
    const child = spawn(process.execPath, [script], { cwd, stdio: ['pipe', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let pending = ''
    let lastBeat = Date.now()
    let lastLabel = 'start'
    let hang = null
    child.stdout.setEncoding('utf8')
    child.stderr.setEncoding('utf8')
    child.stdout.on('data', (chunk) => { stdout += chunk })
    child.stderr.on('data', (chunk) => {
      pending += chunk
      const lines = pending.split('\n')
      pending = lines.pop()
      for (const line of lines) {
        if (line.startsWith('\u0001heartbeat ')) {
          lastBeat = Date.now()
          lastLabel = line.slice('\u0001heartbeat '.length)
        } else {
          stderr = `${stderr}${line}\n`.slice(-8_000)
        }
      }
    })
    const started = Date.now()
    const watchdog = setInterval(() => {
      const silent = Date.now() - lastBeat
      if (silent > callTimeoutMs || Date.now() - started > totalTimeoutMs) {
        hang = { lastCall: lastLabel, silentSeconds: Math.round(silent / 1000), limitSeconds: Math.round(callTimeoutMs / 1000) }
        child.kill('SIGKILL')
      }
    }, 250)
    child.on('close', (code, signal) => {
      clearInterval(watchdog)
      if (hang) {
        resolvePromise({ status: 'hang', error: { message: `no progress for ${hang.silentSeconds} s (limit ${hang.limitSeconds} s) after: ${hang.lastCall}` }, hang })
        return
      }
      try {
        resolvePromise(JSON.parse(stdout))
      } catch {
        resolvePromise({ status: 'error', error: { message: `cell exited ${code}${signal ? ` (${signal})` : ''}`, stderr: stderr.slice(-4_000) } })
      }
    })
    child.stdin.end(JSON.stringify(config))
  })
}
