// One parity cell: every corpus case through the JS reference, then through one port, in this
// fresh Node process (tools/check-cell.mjs) or this freshly loaded browser page (tools/check.mjs).
// Browser-safe: no Node imports.
//
// The reference is imported and run on every case first, so nothing the port does at import time
// (a GopherJS global, a Dart facade) can reach the reference's outputs.
//
// Each case is a call sequence on ONE input object, run identically on both sides:
//   first      a fresh structuredClone of the case input;
//   repeat     the same object again, unchanged (a call that mutated its input, or that caches by
//              input identity, behaves differently here);
//   rewritten  the same object with its contents replaced in place by the next case's input (when
//              both inputs are plain objects or both arrays): a cache keyed by input identity
//              returns a stale result here.
// Every step compares the canonical output AND the input's canonical JSON after the call, so
// mutating the argument is part of the observable behaviour the port must reproduce.
//
// A step is equal when both sides return (or, for an async library, resolve) the same canonical
// text and leave the same input state, or when both THROW (`both-threw`; messages are recorded,
// never compared). Returning a value the output form rejects (a Map for `json`, a number for
// `string`) is not throwing: when the reference throws, such a port fails the step; when the
// reference itself returns such a value, the step is `reference-error`. A port that returns a
// Promise for a synchronous library, or a plain value for an async one, fails the step.
//
// Runtime guard, while the port is imported and called: fetch, XMLHttpRequest, WebSocket,
// EventSource and navigator.sendBeacon record a violation and throw, unless the library declares
// `runtimeGuard.network: "allowed"`; and the own property names of globalThis are compared before
// and after the port phase: a new global outside `config.allowedGlobals` (the family's documented
// runtime globals) is a violation. Module and file access are guarded by the caller (a resolve hook
// and process stubs in tools/check-cell.mjs; the page's CSP and the server's per-load request log
// in tools/check.mjs), which receives the phases through `hooks.referencePhase` and
// `hooks.portPhase`. `hooks.heartbeat(label)` is called before every call so the caller can stop a
// cell that hangs.

import {
  canonicalText, describeEnvironment, digestText, errorRecord, firstDifference, inputState, isThenable, rewritable, rewriteInPlace,
} from './canonical.mjs'

// 1: one call per case. 2: the first / repeat / rewritten sequence (skipped `rewritten` in Firefox).
// 3: realm-independent inputs. 4: uncanonicalisable outputs are not "throwing"; runtime guard.
// 5: globalThis diff, reference-phase guard, per-load request attribution, CSP.
// The version string is for people: receipts are bound to this code's digest (registry.mjs).
export const CHECK_PROTOCOL = 'library-matrix-check/5'

async function evaluate(run, object, output, normalise) {
  let result
  let value
  let thenable = false
  try {
    value = run(object)
    thenable = isThenable(value)
    if (thenable) value = await value
  } catch (error) {
    result = { ok: false, threw: true, error: errorRecord(error) }
  }
  if (!result && thenable !== Boolean(output.async)) {
    result = { ok: false, contract: true, error: { name: 'ContractError', message: output.async ? 'returned a plain value for an async library' : 'returned a Promise for a synchronous library' } }
  }
  if (!result) {
    try {
      const text = canonicalText(normalise(value), output.form)
      result = { ok: true, text, digest: await digestText(text) }
    } catch (error) {
      result = { ok: false, uncanonical: true, error: errorRecord(error) }
    }
  }
  const state = inputState(object)
  return { ...result, inputState: state, inputDigest: await digestText(state) }
}

async function sequence(run, cases, index, output, normalise, beat, side) {
  const object = structuredClone(cases[index].input)
  beat(`${side} ${cases[index].id} first`)
  const steps = [{ step: 'first', ...(await evaluate(run, object, output, normalise)) }]
  beat(`${side} ${cases[index].id} repeat`)
  steps.push({ step: 'repeat', ...(await evaluate(run, object, output, normalise)) })
  const next = cases[(index + 1) % cases.length].input
  if (cases.length > 1 && rewritable(object, next)) {
    rewriteInPlace(object, structuredClone(next))
    beat(`${side} ${cases[index].id} rewritten`)
    steps.push({ step: 'rewritten', ...(await evaluate(run, object, output, normalise)) })
  }
  return steps
}

function compareStep(want, got) {
  const record = { step: want.step, referenceDigest: want.digest ?? null, portDigest: got.digest ?? null }
  if (want.threw) {
    record.referenceError = want.error
    if (got.threw) {
      record.status = 'equal'
      record.note = 'both-threw'
      record.portError = got.error
    } else {
      record.status = 'different'
      record.reason = got.ok ? 'the reference threw and the port returned'
        : got.uncanonical ? `the reference threw and the port returned a value outside the output form (${got.error.message})`
          : got.error.message
      if (!got.ok) record.portError = got.error
    }
  } else if (!want.ok) {
    record.status = 'reference-error'
    record.referenceError = want.error
    record.reason = want.uncanonical ? 'the reference returned a value outside the output form' : want.error.message
  } else if (!got.ok) {
    record.status = got.threw ? 'error' : 'different'
    record.reason = got.threw ? 'the port threw' : got.uncanonical ? `the port returned a value outside the output form (${got.error.message})` : got.error.message
    record.portError = got.error
  } else if (want.text !== got.text) {
    record.status = 'different'
    record.difference = firstDifference(want.text, got.text)
  } else {
    record.status = 'equal'
  }
  if (record.status === 'equal' && want.inputDigest !== got.inputDigest) {
    record.status = 'different'
    record.reason = 'the input object differs after the call (mutation of the argument)'
    record.difference = firstDifference(want.inputState, got.inputState)
  }
  return record
}

// Replaces the network entry points with recorders that throw, and returns a restore function.
// Host-internal globals the engine itself may create lazily at any time (Node's undici fetch
// machinery, Node's own `nodejs.*` registry symbols); they never count against a port.
const HOST_INTERNAL = /^@@(?:undici|nodejs)\./

// The own property names (and symbol descriptions) of globalThis.
function globalNames() {
  return new Set([...Object.getOwnPropertyNames(globalThis), ...Object.getOwnPropertySymbols(globalThis).map((symbol) => `@@${symbol.description ?? ''}`)])
}

function guardNetwork(violations) {
  const restores = []
  const blocked = (name) => function () {
    violations.push(name)
    throw new Error(`library-matrix runtime guard: ${name} during a port call`)
  }
  const replace = (owner, key, name) => {
    if (!owner || typeof owner[key] !== 'function') return
    const own = Object.prototype.hasOwnProperty.call(owner, key)
    const original = owner[key]
    owner[key] = blocked(name)
    // A property inherited from the prototype (Firefox's fetch) is deleted again, not re-set.
    restores.push(() => {
      if (own) owner[key] = original
      else delete owner[key]
    })
  }
  replace(globalThis, 'fetch', 'fetch')
  replace(globalThis, 'XMLHttpRequest', 'XMLHttpRequest')
  replace(globalThis, 'WebSocket', 'WebSocket')
  replace(globalThis, 'EventSource', 'EventSource')
  if (globalThis.navigator) replace(globalThis.navigator, 'sendBeacon', 'navigator.sendBeacon')
  return () => { for (const restore of restores.reverse()) restore() }
}

export async function runCheck(config, hooks = {}) {
  const beat = typeof hooks.heartbeat === 'function' ? hooks.heartbeat : () => {}
  const result = {
    checkProtocol: CHECK_PROTOCOL,
    library: config.library,
    port: config.port,
    environmentKey: config.environmentKey,
    environment: describeEnvironment(),
    status: 'ok',
    guard: { network: config.guard?.network === 'allowed' ? 'allowed' : 'blocked', violations: [], newGlobals: [], allowedGlobals: config.allowedGlobals ?? [] },
    cases: [],
  }
  let restore = () => {}
  try {
    // Parsed by this module's own JSON.parse, so the inputs belong to the page realm (Firefox gives
    // Response.json() objects of another realm to a module imported from a WebDriver script).
    const cases = config.cases ?? JSON.parse(await (await fetch(config.casesUrl)).text()).cases
    const normalise = config.normalisationUrl ? (await import(config.normalisationUrl)).apply : (value) => value
    if (typeof normalise !== 'function') throw new Error('the normalisation module does not export apply(value)')
    hooks.referencePhase?.(true)
    beat('import reference')
    const reference = await import(config.referenceUrl)
    if (typeof reference.run !== 'function') throw new Error('the reference adapter does not export run()')
    const expected = []
    for (let index = 0; index < cases.length; index++) expected.push(await sequence(reference.run, cases, index, config.output, normalise, beat, 'reference'))
    hooks.referencePhase?.(false)

    // The port phase: from its import to its last call, under the runtime guard. The globals are
    // listed after the guard is installed (touching Node's lazy fetch getter creates globals).
    if (result.guard.network === 'blocked') restore = guardNetwork(result.guard.violations)
    const globalsBefore = globalNames()
    hooks.portPhase?.(true)
    beat('import port')
    const port = await import(config.portUrl)
    if (typeof port.run !== 'function') throw new Error(`${config.portUrl} does not export run()`)
    const actual = []
    for (let index = 0; index < cases.length; index++) actual.push(await sequence(port.run, cases, index, config.output, normalise, beat, 'port'))
    hooks.portPhase?.(false)
    restore()
    restore = () => {}
    const allowed = new Set(config.allowedGlobals ?? [])
    result.guard.newGlobals = [...globalNames()].filter((name) => !globalsBefore.has(name) && !allowed.has(name) && !HOST_INTERNAL.test(name))

    for (let index = 0; index < cases.length; index++) {
      const steps = expected[index].map((want, step) => compareStep(want, actual[index][step]))
      const failed = steps.find((step) => step.status !== 'equal')
      const first = steps[0]
      result.cases.push({
        id: cases[index].id,
        status: failed ? failed.status : 'equal',
        ...(failed ? { failedStep: failed.step } : {}),
        referenceDigest: first.referenceDigest,
        portDigest: first.portDigest,
        ...(first.note ? { note: first.note } : {}),
        ...(failed?.reason ? { reason: failed.reason } : {}),
        ...(failed?.difference ? { difference: failed.difference } : {}),
        ...(failed?.referenceError ? { referenceError: failed.referenceError } : {}),
        ...(failed?.portError ? { portError: failed.portError } : {}),
        steps: steps.map(({ step, status, referenceDigest, portDigest, note }) => ({ step, status, referenceDigest, portDigest, ...(note ? { note } : {}) })),
      })
    }
  } catch (error) {
    result.status = 'error'
    result.error = errorRecord(error)
  } finally {
    hooks.referencePhase?.(false)
    hooks.portPhase?.(false)
    restore()
  }
  const count = (status) => result.cases.filter((item) => item.status === status).length
  result.summary = { cases: result.cases.length, equal: count('equal'), different: count('different'), errors: count('error') + count('reference-error') }
  return result
}
