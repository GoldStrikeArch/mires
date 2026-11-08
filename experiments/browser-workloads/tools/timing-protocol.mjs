// Paired Rust-vs-JavaScript timing protocol for the corpus workloads.
//
// One module, browser-safe (no Node imports), so every environment runs the same protocol code:
// tools/timing-cell.mjs runs a cell in a fresh Node process, and tools/run.mjs serves this file to
// a fresh browser page per cell. A cell is one workload and two arms (Rust vs JS, or Rust vs a
// second Rust instance for the A/A noise control). See ../README.md for the protocol.

export const PROTOCOL_VERSION = 1

// The workloads `run.mjs --pure` already benchmarks. `baseIterations` is the call count of the
// existing pureBenchmarks; calibration only raises it, so a sample is at least `minSampleMs` long.
// `canonicalize` maps an arm's output to the behaviour both implementations share; its digest is
// the cell checksum, and a cell whose arms disagree is never timed.
export const TIMED_WORKLOADS = {
  marked: {
    baseIterations: 5,
    reference: { package: 'marked', adapter: 'reference/marked.mjs' },
    shared: 'the complete rendered HTML string (compared byte for byte)',
    differences: [
      'The Rust port renders a line-based subset (ATX headings, "- " lists, "> " quotes, fenced code, paragraphs, **strong**, *em*, `code`, [text](url)); marked renders CommonMark + GFM. Inputs outside the subset produce different HTML and fail the checksum gate.',
      'Raw HTML: the Rust port has no HTML blocks or inline tags and escapes & < > in them; marked passes raw HTML through. The reference overrides marked\'s `html` renderer to escape exactly those three characters (marked still lexes the HTML).',
      'Text escaping: marked also escapes " and \' in text (&quot;, &#39;); the Rust port does not. The timed input contains neither character; if it did, the checksum gate would fail.',
    ],
    canonicalize: (output) => output,
  },
  minisearch: {
    baseIterations: 4,
    reference: { package: 'minisearch', adapter: 'reference/minisearch.mjs' },
    shared: 'the set of matched document ids (sorted, comma-joined); scores and ranking are not compared',
    differences: [
      'Scoring: MiniSearch uses BM25+ with prefix matches weighted 0.375 and the score multiplied by the number of matching query terms; the Rust port sums integer weighted term frequencies (title 3, body 1). Scores, and therefore ranking and tie order, differ by design.',
      'Tokenization: MiniSearch splits on Unicode whitespace and punctuation, the Rust port on non-ASCII-alphanumeric bytes. The reference drops one-character terms and lower-cases like the Rust port; on the timed input (ASCII letters, digits and spaces) both produce the same terms.',
      'Query syntax: neither library parses `term*`; the reference splits the query on ASCII whitespace and turns a trailing `*` into a MiniSearch prefix query, OR-combined, as the Rust port does.',
    ],
    canonicalize: (output) => output.split(',').filter(Boolean).map((pair) => pair.slice(0, pair.lastIndexOf(':'))).sort().join(','),
  },
  fflate: {
    baseIterations: 3,
    reference: { package: 'fflate', adapter: 'reference/fflate.mjs' },
    shared: 'every gzip byte except header offsets 8 (XFL) and 9 (OS), which are zeroed before hashing',
    differences: [
      'Header: fflate writes XFL=4 and OS=3 for level 0; the Rust port writes XFL=0 and OS=255. Both write MTIME=0 (the reference passes mtime: 0) and no file name.',
      'Algorithm: both emit stored (uncompressed) DEFLATE blocks of at most 65,535 bytes and the same CRC-32/ISIZE trailer, but fflate computes CRC-32 with a lookup table and the Rust port bit by bit. The ratio therefore measures implementation choices as well as compiler output.',
      'Block boundary: for an input whose length is a positive multiple of 65,535, fflate appends an empty final stored block and the Rust port does not; such an input fails the checksum gate. The timed input is 61,264 bytes.',
    ],
    canonicalize: (output) => {
      const bytes = Uint8Array.from(output)
      if (bytes.length > 9) bytes[8] = bytes[9] = 0
      return bytes
    },
  },
}

const MAX_ITERATIONS = 1_000_000
const now = () => globalThis.performance.now()
const textEncoder = new TextEncoder()

// Keeps every call's result observable so no engine can drop a call.
let sinkValue = 0
function sink(value) {
  sinkValue = (sinkValue + (value?.length ?? 1)) | 0
}

export function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`
  if (value && typeof value === 'object') {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(',')}}`
  }
  return JSON.stringify(value)
}

function toBytes(value) {
  if (typeof value === 'string') return textEncoder.encode(value)
  if (value instanceof Uint8Array) return value
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
  return textEncoder.encode(canonicalJson(value))
}

// SHA-256 through WebCrypto (Node, and browsers in a secure context such as http://127.0.0.1).
// Without WebCrypto the digest is FNV-1a 32 and carries that prefix, so it can never compare equal
// to a SHA-256 digest by accident.
export async function digest(value) {
  const bytes = toBytes(value)
  const subtle = globalThis.crypto?.subtle
  if (subtle) {
    const hash = new Uint8Array(await subtle.digest('SHA-256', bytes))
    return `sha256:${Array.from(hash, (byte) => byte.toString(16).padStart(2, '0')).join('')}`
  }
  let hash = 0x811c9dc5
  for (const byte of bytes) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
  return `fnv1a32:${hash.toString(16).padStart(8, '0')}`
}

// Deterministic seed for one cell from the run seed and the cell's identity.
export function cellSeed(seed, ...parts) {
  let hash = 0x811c9dc5
  for (const byte of textEncoder.encode([seed, ...parts].join('/'))) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
  return hash
}

function mulberry32(seed) {
  let state = seed >>> 0
  return () => {
    state = (state + 0x6d2b79f5) >>> 0
    let value = state
    value = Math.imul(value ^ (value >>> 15), value | 1)
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61)
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296
  }
}

function shuffled(length, random) {
  const order = Array.from({ length }, (_, index) => index)
  for (let index = length - 1; index > 0; index--) {
    const other = Math.floor(random() * (index + 1))
    ;[order[index], order[other]] = [order[other], order[index]]
  }
  return order
}

// Arm orders for `rounds` rounds of a two-arm cell, in balanced blocks: every two consecutive
// rounds hold both orders, the first chosen at random, so each order runs equally often (within
// one) and an order effect cancels instead of landing on one arm by chance.
function balancedOrders(rounds, random) {
  const orders = []
  while (orders.length < rounds) {
    const first = random() < 0.5 ? [0, 1] : [1, 0]
    orders.push(first, [first[1], first[0]])
  }
  return orders.slice(0, rounds)
}

// Order statistics with the same definition as run.mjs `summarize`.
export function summarize(values) {
  const sorted = [...values].sort((left, right) => left - right)
  const at = (quantile) => sorted[Math.max(0, Math.ceil(sorted.length * quantile) - 1)]
  return {
    count: sorted.length,
    min: sorted[0],
    p50: at(0.5),
    p95: at(0.95),
    max: sorted.at(-1),
    mean: sorted.reduce((sum, value) => sum + value, 0) / sorted.length,
  }
}

function timerResolutionMs() {
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

function describeEnvironment() {
  const node = globalThis.process?.versions?.node
  return {
    runtime: node ? 'node' : 'browser',
    ...(node ? { node: globalThis.process.version, v8: globalThis.process.versions.v8 } : {}),
    userAgent: globalThis.navigator?.userAgent ?? null,
    crossOriginIsolated: typeof globalThis.crossOriginIsolated === 'boolean' ? globalThis.crossOriginIsolated : null,
    timerResolutionMs: timerResolutionMs(),
  }
}

function runMany(run, input, iterations) {
  for (let index = 0; index < iterations; index++) sink(run(input))
}

// Calls the arm until `minSampleMs` has elapsed (at least once), then sets its iterations per
// sample so one sample lasts at least that long, never below the workload's base count.
function calibrate(arm, input, baseIterations, minSampleMs) {
  let calls = 0
  let elapsed = 0
  const started = now()
  do {
    sink(arm.run(input))
    calls++
    elapsed = now() - started
  } while (elapsed < minSampleMs && calls < MAX_ITERATIONS)
  const perCall = elapsed / calls
  arm.calibration = { calls, milliseconds: elapsed }
  arm.iterationsPerSample = Math.max(baseIterations,
    Math.min(MAX_ITERATIONS, perCall > 0 ? Math.ceil(minSampleMs / perCall) : MAX_ITERATIONS))
}

const pause = () => new Promise((resolve) => setTimeout(resolve, 0))

function errorRecord(error) {
  return {
    name: error?.name || 'Error',
    message: String(error?.message || error).slice(0, 2_000),
    stack: String(error?.stack || '').split('\n').slice(0, 8).join('\n').slice(0, 4_000),
  }
}

// Runs one cell. `config.arms` is two `{key, label, implementation, url}` records; each `url` is a
// module exporting `run(options)`. Steps, all inside this one fresh process or page:
//   1. cold import of each arm, in seeded random order, timed separately;
//   2. one validation call per arm (its time is `firstCallMs`); the canonical checksums must agree,
//      and equal `config.expectedChecksum` when given, or the cell stops with no timing;
//   3. per-arm calibration, then `warmups` rounds and `samples` rounds; in every round the two arms
//      run back to back, `iterationsPerSample` calls each, in seeded random order drawn in balanced
//      blocks of two rounds (AB then BA, or BA then AB);
//   4. one more call per arm whose checksum must still agree (determinism after timing).
// Ratios are first arm / second arm on milliseconds per call.
export async function runCell(config) {
  const spec = TIMED_WORKLOADS[config.workload]
  const result = {
    protocol: PROTOCOL_VERSION,
    workload: config.workload,
    pair: config.pair,
    seed: config.seed,
    warmups: config.warmups,
    samples: config.samples,
    minSampleMs: config.minSampleMs,
    environment: describeEnvironment(),
    status: 'ok',
  }
  try {
    if (!spec) throw new Error(`unknown timed workload ${config.workload}`)
    if (config.arms?.length !== 2) throw new Error('a timing cell has exactly two arms')
    // run.mjs validates these; a cell configured another way fails instead of reporting NaN.
    if (!(Number.isInteger(config.samples) && config.samples >= 1)) throw new Error(`samples must be a positive integer (got ${config.samples})`)
    if (!(Number.isInteger(config.warmups) && config.warmups >= 0)) throw new Error(`warmups must be a non-negative integer (got ${config.warmups})`)
    if (!(Number.isFinite(config.minSampleMs) && config.minSampleMs > 0)) throw new Error(`minSampleMs must be a positive number (got ${config.minSampleMs})`)
    const random = mulberry32(config.seed)
    result.inputSha256 = await digest(canonicalJson(config.input))
    result.inputVerified = !config.expectedInputSha256 || config.expectedInputSha256 === result.inputSha256
    if (!result.inputVerified) return { ...result, status: 'input-mismatch' }
    const arms = config.arms.map(({ key, label, implementation, url }) => ({ key, label, implementation, url }))
    result.arms = arms
    result.importOrder = shuffled(arms.length, random).map((index) => arms[index].key)
    for (const key of result.importOrder) {
      const arm = arms.find((candidate) => candidate.key === key)
      const started = now()
      const module = await import(arm.url)
      arm.coldImportMs = now() - started
      arm.run = module.run
      if (typeof arm.run !== 'function') throw new Error(`${arm.url} does not export run()`)
    }
    for (const arm of arms) {
      const started = now()
      const output = arm.run(config.input)
      arm.firstCallMs = now() - started
      arm.checksum = await digest(spec.canonicalize(output))
      arm.rawChecksum = await digest(output)
      arm.rawBytes = toBytes(output).length
    }
    result.checksumsAgree = arms.every((arm) => arm.checksum === arms[0].checksum)
    result.checksum = result.checksumsAgree ? arms[0].checksum : null
    result.checksumMatchesExpected = config.expectedChecksum ? result.checksum === config.expectedChecksum : null
    if (!result.checksumsAgree || result.checksumMatchesExpected === false) {
      result.status = 'checksum-mismatch'
      return finish(result, arms)
    }
    for (const index of shuffled(arms.length, random)) {
      calibrate(arms[index], config.input, spec.baseIterations, config.minSampleMs)
    }
    for (const order of balancedOrders(config.warmups, random)) {
      for (const index of order) runMany(arms[index].run, config.input, arms[index].iterationsPerSample)
      await pause()
    }
    for (const arm of arms) arm.sampleMs = []
    result.order = []
    for (const order of balancedOrders(config.samples, random)) {
      result.order.push(order.map((index) => arms[index].label).join(''))
      for (const index of order) {
        const arm = arms[index]
        const started = now()
        runMany(arm.run, config.input, arm.iterationsPerSample)
        arm.sampleMs.push(now() - started)
      }
      await pause()
    }
    for (const arm of arms) {
      arm.finalChecksum = await digest(spec.canonicalize(arm.run(config.input)))
      arm.sampleSummary = summarize(arm.sampleMs)
      arm.perCallMs = summarize(arm.sampleMs.map((value) => value / arm.iterationsPerSample))
    }
    result.deterministic = arms.every((arm) => arm.finalChecksum === result.checksum)
    if (!result.deterministic) {
      // The samples stay in the record for diagnosis, but a cell whose output drifted has no ratio.
      result.status = 'nondeterministic'
      return finish(result, arms)
    }
    const [first, second] = arms
    const paired = first.sampleMs.map((value, index) =>
      (value / first.iterationsPerSample) / (second.sampleMs[index] / second.iterationsPerSample))
    result.ratio = {
      of: `${first.key}/${second.key}`,
      p50: first.perCallMs.p50 / second.perCallMs.p50,
      p95: first.perCallMs.p95 / second.perCallMs.p95,
      min: first.perCallMs.min / second.perCallMs.min,
      paired: summarize(paired),
    }
  } catch (error) {
    result.status = 'error'
    result.error = errorRecord(error)
  }
  return finish(result, result.arms ?? [])
}

function finish(result, arms) {
  for (const arm of arms) delete arm.run
  result.sink = sinkValue
  return result
}
