#!/usr/bin/env node
// Runs one parity cell (tools/lib/check-runner.mjs `runCheck`) in this fresh Node process, the
// Node counterpart of a fresh browser page. tools/check.mjs spawns it once per port, writes the
// cell configuration to stdin and reads the JSON result from stdout.
//
// Runtime guard (README "Runtime guard"):
//   - port phase (its import and every call): a synchronous module resolve hook refuses every module
//     load except the port artifact itself (an import() hidden in an eval string, a data: URL, a
//     module the process already loaded, a Node builtin);
//   - reference phase (the adapter's import and every reference call): the hook refuses Node
//     builtins, so neither side can read files (contract rule: no file reads at run time);
//   - both phases: process.getBuiltinModule, process.binding, process._linkedBinding and
//     process.dlopen throw.
// A guard error the code under test leaves unhandled (an unawaited import() promise) is recorded,
// not fatal, so it ends as a named violation. Heartbeats go to stderr synchronously before every
// call, so the parent can stop a cell that hangs.
import { writeSync } from 'node:fs'
import { isBuiltin, registerHooks } from 'node:module'

import { runCheck } from './lib/check-runner.mjs'

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const config = JSON.parse(Buffer.concat(chunks).toString('utf8'))

let phase = null
const loads = []
const referenceAccess = []
const unhandled = []
const GUARD = 'library-matrix runtime guard'

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (phase === 'reference' && (isBuiltin(specifier) || specifier.startsWith('node:'))) {
      referenceAccess.push(`import ${specifier}`)
      throw new Error(`${GUARD}: the reference imports the builtin ${specifier}`)
    }
    const resolved = nextResolve(specifier, context)
    if (phase === 'port' && resolved.url !== config.portUrl) {
      loads.push({ specifier: specifier.slice(0, 200), parent: context.parentURL ?? null })
      throw new Error(`${GUARD}: module load of ${specifier.slice(0, 120)} during the port phase`)
    }
    return resolved
  },
})

// Process internals that reach files, native code or bindings without an import.
const internals = ['getBuiltinModule', 'binding', '_linkedBinding', 'dlopen']
const originals = Object.fromEntries(internals.map((name) => [name, process[name]]))
function stubInternals(active) {
  for (const name of internals) {
    if (typeof originals[name] !== 'function') continue
    process[name] = active
      ? (argument) => {
          const record = `process.${name}(${String(argument?.id ?? argument).slice(0, 80)})`
          if (phase === 'reference') referenceAccess.push(record)
          else loads.push({ specifier: record, parent: null })
          throw new Error(`${GUARD}: ${record} during the ${phase} phase`)
        }
      : originals[name]
  }
}

process.on('unhandledRejection', (reason) => {
  unhandled.push(String(reason?.message ?? reason).slice(0, 300))
})
process.on('uncaughtException', (error) => {
  unhandled.push(String(error?.message ?? error).slice(0, 300))
})

const hooks = {
  heartbeat: (label) => writeSync(2, `\u0001heartbeat ${label}\n`),
  referencePhase: (active) => {
    if (active) phase = 'reference'
    else if (phase === 'reference') phase = null
    stubInternals(active)
  },
  portPhase: (active) => {
    if (active) phase = 'port'
    else if (phase === 'port') phase = null
    stubInternals(active)
  },
}
const result = await runCheck(config, hooks)
// Let pending rejections from the port phase surface before reporting.
await new Promise((resolvePromise) => setTimeout(resolvePromise, 50))
result.guard.moduleLoads = loads
result.guard.referenceAccess = referenceAccess
result.guard.unhandled = unhandled
if (referenceAccess.length && result.status !== 'error') {
  result.status = 'error'
  result.error = { name: 'ReferenceGuard', message: `the reference adapter reached ${[...new Set(referenceAccess)].join('; ')}; adapters must not read files at run time` }
}
process.stdout.write(JSON.stringify(result))
