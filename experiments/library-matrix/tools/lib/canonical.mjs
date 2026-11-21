// Canonical output forms and digests, shared by the checker and the timing protocol. Browser-safe
// (no Node imports): the same file runs in Node and in every browser page.
//
// A library's registry entry names one output form; a value is reduced to one canonical text, and
// two outputs are equal exactly when their canonical texts are equal.
//
//   string  the value must be a string; canonical text = JSON.stringify(value), which is exact for
//           every JS string (lone surrogates are escaped, so no two strings share a text).
//   bytes   the value must be a Uint8Array; canonical text = "bytes:" + lowercase hex.
//   json    a strict serializer: plain objects (own enumerable string keys, in their order, which
//           is part of the output), arrays, strings, booleans, null, undefined, numbers with -0,
//           NaN and the infinities kept distinct, and BigInts. Any other object (Map, Date, class
//           instance, typed array) is a canonicalisation error, never a silent conversion.
//
// A library may declare a named normalisation (registry `normalisation.module`), applied to the
// value before canonicalisation; its name and description are recorded in every receipt. The
// default is none.

export class CanonicalError extends Error {
  constructor(message) {
    super(message)
    this.name = 'CanonicalError'
  }
}

const HEX = Array.from({ length: 256 }, (_, byte) => byte.toString(16).padStart(2, '0'))

function strictJson(value, path, seen) {
  if (value === null) return 'null'
  switch (typeof value) {
    case 'string':
      return JSON.stringify(value)
    case 'boolean':
      return value ? 'true' : 'false'
    case 'undefined':
      return 'undefined'
    case 'bigint':
      return `${value}n`
    case 'number':
      if (Object.is(value, -0)) return '-0'
      return Number.isFinite(value) ? JSON.stringify(value) : String(value)
    case 'object': {
      if (seen.has(value)) throw new CanonicalError(`cycle at ${path}`)
      seen.add(value)
      try {
        if (Array.isArray(value)) {
          const parts = []
          for (let index = 0; index < value.length; index++) parts.push(strictJson(value[index], `${path}[${index}]`, seen))
          return `[${parts.join(',')}]`
        }
        const prototype = Object.getPrototypeOf(value)
        if (prototype !== Object.prototype && prototype !== null) {
          throw new CanonicalError(`${path} is a ${Object.prototype.toString.call(value)}, not a plain object`)
        }
        const keys = Object.keys(value)
        return `{${keys.map((key) => `${JSON.stringify(key)}:${strictJson(value[key], `${path}.${key}`, seen)}`).join(',')}}`
      } finally {
        seen.delete(value)
      }
    }
    default:
      throw new CanonicalError(`${path} is a ${typeof value}`)
  }
}

export function canonicalText(value, form) {
  switch (form) {
    case 'string':
      if (typeof value !== 'string') throw new CanonicalError(`expected a string, got ${describe(value)}`)
      return JSON.stringify(value)
    case 'bytes': {
      if (Object.prototype.toString.call(value) !== '[object Uint8Array]') throw new CanonicalError(`expected a Uint8Array, got ${describe(value)}`)
      let text = 'bytes:'
      for (let index = 0; index < value.length; index++) text += HEX[value[index]]
      return text
    }
    case 'json':
      return strictJson(value, '$', new Set())
    default:
      throw new CanonicalError(`unknown output form ${form}`)
  }
}

// A Promise or any other thenable, as `await` would treat it.
export function isThenable(value) {
  return value !== null && (typeof value === 'object' || typeof value === 'function') && typeof value.then === 'function'
}

export function describe(value) {
  if (value === null) return 'null'
  if (typeof value === 'object') return Object.prototype.toString.call(value)
  return typeof value
}

const encoder = new TextEncoder()

// SHA-256 of the canonical text's UTF-8 bytes, through WebCrypto (Node, and browsers in a secure
// context such as http://127.0.0.1). Canonical texts are well-formed (JSON escapes lone
// surrogates), so the encoding is injective. Without WebCrypto the digest is FNV-1a 32 with its
// own prefix, which can never compare equal to a SHA-256 digest (browser-workloads' rule).
export async function digestText(text) {
  const bytes = encoder.encode(text)
  const subtle = globalThis.crypto?.subtle
  if (subtle) {
    const hash = new Uint8Array(await subtle.digest('SHA-256', bytes))
    return `sha256:${Array.from(hash, (byte) => HEX[byte]).join('')}`
  }
  let hash = 0x811c9dc5
  for (const byte of bytes) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
  return `fnv1a32:${hash.toString(16).padStart(8, '0')}`
}

// Canonical JSON of an input (sorted keys), used only to fingerprint corpus inputs.
export function inputText(value) {
  if (Array.isArray(value)) return `[${value.map(inputText).join(',')}]`
  if (value && typeof value === 'object') {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${inputText(value[key])}`).join(',')}}`
  }
  return JSON.stringify(value)
}

// Whether `target` (an input object a call has seen) can take `source`'s contents in place: both
// plain objects, or both arrays. Primitives have no identity to reuse.
export function rewritable(target, source) {
  const kind = (value) => {
    if (Array.isArray(value)) return 'array'
    return value !== null && typeof value === 'object' && Object.getPrototypeOf(value) === Object.prototype ? 'object' : null
  }
  return kind(target) !== null && kind(target) === kind(source)
}

// Replaces the contents of `target` with those of `source`, keeping `target`'s identity, so a
// call that caches by input identity (or kept a reference to it) sees new contents.
export function rewriteInPlace(target, source) {
  if (Array.isArray(target)) {
    target.length = 0
    target.push(...source)
    return target
  }
  for (const key of Object.keys(target)) delete target[key]
  return Object.assign(target, source)
}

// The canonical JSON of an input after a call, so a call that mutates its input is visible.
export function inputState(value) {
  try {
    return inputText(value)
  } catch (error) {
    return `unserialisable: ${error?.message ?? error}`
  }
}

// Where two canonical texts first differ, with a short excerpt of each around that point.
export function firstDifference(expected, actual, context = 80) {
  const limit = Math.min(expected.length, actual.length)
  let index = 0
  while (index < limit && expected.charCodeAt(index) === actual.charCodeAt(index)) index++
  const from = Math.max(0, index - context)
  return {
    index,
    expectedLength: expected.length,
    actualLength: actual.length,
    expected: expected.slice(from, index + context),
    actual: actual.slice(from, index + context),
  }
}

export function errorRecord(error) {
  return {
    name: error?.name || 'Error',
    message: String(error?.message ?? error).slice(0, 2_000),
    stack: String(error?.stack || '').split('\n').slice(0, 8).join('\n').slice(0, 4_000),
  }
}

function timerResolutionMs() {
  const now = () => globalThis.performance.now()
  let smallest = null
  let last = now()
  const deadline = last + 50
  for (let changes = 0; changes < 20;) {
    const value = now()
    if (value !== last) {
      smallest = smallest === null ? value - last : Math.min(smallest, value - last)
      last = value
      changes++
    }
    if (value > deadline) break
  }
  return smallest
}

export function describeEnvironment() {
  const node = globalThis.process?.versions?.node
  return {
    runtime: node ? 'node' : 'browser',
    ...(node ? { node: globalThis.process.version, v8: globalThis.process.versions.v8 } : {}),
    userAgent: globalThis.navigator?.userAgent ?? null,
    crossOriginIsolated: typeof globalThis.crossOriginIsolated === 'boolean' ? globalThis.crossOriginIsolated : null,
    timerResolutionMs: timerResolutionMs(),
  }
}
