// N-arm timing protocol for the library matrix. Browser-safe (no Node imports), so Node
// (tools/bench-cell.mjs, a fresh process per cell) and every browser (a fresh page load per cell)
// run the same code.
//
// Generalised from experiments/browser-workloads/tools/timing-protocol.mjs (protocol version 1,
// two arms per cell), which is not modified: the steps, the calibration rule, the sink, the
// mulberry32 seeding, the order statistics and the digest rule are that file's; what changes is
// that a cell holds any number of arms, that arm orders come from a Williams design (balanced
// Latin squares) instead of AB/BA blocks, that ratios are named comparisons between arm pairs, and
// that outputs go through the library's canonical form and optional async contract.
//
// A cell runs, all inside one fresh process or page:
//   1. the input's canonical-JSON digest must equal the one the driver computed;
//   2. cold import of each arm, in seeded random order, each timed alone;
//   3. one validation call per arm (`firstCallMs`, not a sample); every arm's canonical checksum
//      must agree and equal `expectedChecksum` (the Node reference digest), or the cell stops with
//      `checksum-mismatch` and no timings;
//   4. per-arm calibration in seeded random order: `iterationsPerSample` = max(baseIterations,
//      calls needed to fill minSampleMs);
//   5. `warmups` rounds, then a second (warm) calibration per arm, whose iterations the samples
//      use, then `samples` rounds; in a round every arm runs `iterationsPerSample` calls back to
//      back, in an order taken from a Williams design whose rows are shuffled and whose arm labels
//      are permuted per block (seeded), so every arm holds every position equally often and every
//      arm directly follows every other equally often, within each block (`samplesBalanced` says
//      whether the sample count is a whole number of blocks);
//   6. one more call per arm whose checksum must still match (`nondeterministic` otherwise).
// Every timed call receives its own input, made outside the timed region: a structuredClone of
// the case input, or, when the library declares a bench variant (registry `bench.variant`), a
// distinct variant(input, k) per call, so neither an identity cache nor a content cache can hit.
// After every calibration, warmup and sample run, the outputs of two of its calls (one picked by
// a crypto.getRandomValues-seeded stream, recorded as `pickSeed`, and the last one) must have the
// expected checksum (the gate checksum, or the reference's output on the same variant); otherwise
// the cell stops with `output-mismatch` and no ratio. `hooks.heartbeat(label)` reports liveness
// (at most every 250 ms, never inside a timed batch) to the caller's hang watchdog. With fewer
// than 20 samples the reported p95 is the maximum (`p95IsMax`).
// Ratios are first arm / second arm of per-call milliseconds.

import { canonicalText, describeEnvironment, digestText, errorRecord, inputText, isThenable } from './canonical.mjs'

// 2: fresh inputs, in-sample output verification. 3: crypto-seeded output picks, bench variants,
// warm re-calibration, p95IsMax, heartbeats. 4: disjoint variant ranges per arm, A/A range gate,
// per-environment expected checksum. 5: browser cells run in the page realm (tools/lib/page.mjs),
// and the input reaches the page as JSON text, in its key order. Receipts are bound to this code's
// digest.
export const PROTOCOL_VERSION = 'library-matrix/5 (N-arm generalisation of browser-workloads timing protocol 1)'

const MAX_ITERATIONS = 1_000_000
// Memory for the fresh input clones of one batch of timed calls (estimated at 4 bytes per
// character of the input's canonical JSON).
const INPUT_POOL_BYTES = 32 * 1024 * 1024
const now = () => globalThis.performance.now()

// Keeps every call's result observable so no engine can drop a call.
let sinkValue = 0
function sink(value) {
  sinkValue = (sinkValue + (value?.length ?? 1)) | 0
}

// Deterministic seed for one cell from the run seed and the cell's identity.
export function cellSeed(seed, ...parts) {
  let hash = 0x811c9dc5
  for (const byte of new TextEncoder().encode([seed, ...parts].join('/'))) hash = Math.imul(hash ^ byte, 0x01000193) >>> 0
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

// Williams design for n treatments: n rows for even n, 2n (each row and its reverse) for odd n.
// Each treatment appears once per row and equally often in each position, and each ordered pair of
// distinct treatments is adjacent equally often. For n = 2 it is AB, BA: the original protocol.
export function williamsRows(n) {
  if (n === 1) return [[0]]
  const first = [0]
  for (let low = 1, high = n - 1, takeLow = true; first.length < n; takeLow = !takeLow) first.push(takeLow ? low++ : high--)
  const rows = Array.from({ length: n }, (_, shift) => first.map((value) => (value + shift) % n))
  if (n % 2 === 1) rows.push(...rows.map((row) => [...row].reverse()))
  return rows
}

// Arm orders for `rounds` rounds in balanced blocks: each block is the Williams design with its
// rows shuffled and its labels permuted (seeded), so an order effect cancels across a block
// instead of landing on one arm by chance.
export function balancedOrders(armCount, rounds, random) {
  const orders = []
  while (orders.length < rounds) {
    const rows = williamsRows(armCount)
    const labels = shuffled(armCount, random)
    for (const row of shuffled(rows.length, random)) orders.push(rows[row].map((symbol) => labels[symbol]))
  }
  return orders.slice(0, rounds)
}

// Order statistics with the definition of browser-workloads run.mjs `summarize`.
export function summarize(values) {
  const sorted = [...values].sort((left, right) => left - right)
  const at = (quantile) => sorted[Math.max(0, Math.ceil(sorted.length * quantile) - 1)]
  return {
    count: sorted.length,
    min: sorted[0],
    p50: at(0.5),
    p95: at(0.95),
    // With fewer than 20 values the 95th percentile by this definition is the maximum.
    p95IsMax: sorted.length < 20,
    max: sorted.at(-1),
    mean: sorted.reduce((sum, value) => sum + value, 0) / sorted.length,
  }
}

const pause = () => new Promise((resolve) => setTimeout(resolve, 0))

export async function runCell(config, hooks = {}) {
  // Liveness for the caller's hang watchdog, at most every 250 ms (never inside a timed batch).
  let lastBeat = 0
  const beat = (label, force = false) => {
    if (typeof hooks.heartbeat !== 'function') return
    const time = now()
    if (!force && time - lastBeat < 250) return
    lastBeat = time
    hooks.heartbeat(label)
  }
  const result = {
    protocol: PROTOCOL_VERSION,
    library: config.library,
    port: config.port,
    case: config.case,
    design: config.design,
    seed: config.seed,
    warmups: config.warmups,
    samples: config.samples,
    minSampleMs: config.minSampleMs,
    baseIterations: config.baseIterations,
    output: config.output,
    environment: describeEnvironment(),
    status: 'ok',
  }
  let arms = []
  try {
    if (!(Array.isArray(config.arms) && config.arms.length >= 1)) throw new Error('a timing cell has at least one arm')
    if (!(Number.isInteger(config.samples) && config.samples >= 1)) throw new Error(`samples must be a positive integer (got ${config.samples})`)
    if (!(Number.isInteger(config.warmups) && config.warmups >= 0)) throw new Error(`warmups must be a non-negative integer (got ${config.warmups})`)
    if (!(Number.isFinite(config.minSampleMs) && config.minSampleMs > 0)) throw new Error(`minSampleMs must be a positive number (got ${config.minSampleMs})`)
    if (!(Number.isInteger(config.baseIterations) && config.baseIterations >= 1)) throw new Error(`baseIterations must be a positive integer (got ${config.baseIterations})`)
    const random = mulberry32(config.seed)
    const asynchronous = Boolean(config.output.async)
    const normalise = config.normalisationUrl ? (await import(config.normalisationUrl)).apply : (value) => value
    result.inputSha256 = await digestText(inputText(config.input))
    result.inputVerified = !config.expectedInputSha256 || config.expectedInputSha256 === result.inputSha256
    if (!result.inputVerified) return finish({ ...result, status: 'input-mismatch' }, arms)

    arms = config.arms.map(({ key, label, kind, url }) => ({ key, label, kind, url }))
    result.arms = arms
    result.importOrder = shuffled(arms.length, random).map((index) => arms[index].key)
    for (const key of result.importOrder) {
      const arm = arms.find((candidate) => candidate.key === key)
      const started = now()
      beat(`import ${arm.key}`, true)
      const module = await import(arm.url)
      arm.coldImportMs = now() - started
      arm.run = module.run
      if (typeof arm.run !== 'function') throw new Error(`${arm.url} does not export run()`)
    }
    // Every call gets its own input, made outside the timed region, in batches of at most
    // `poolSize` (bounded by INPUT_POOL_BYTES): a structuredClone of the pristine input, or, when
    // the library declares a bench variant, `variant(input, k)` for a k no earlier call of this arm
    // used (k >= 1), so that no call sees an input object or an input VALUE that another call has
    // seen: neither an identity cache nor a content cache can hit.
    const inputBytes = Math.max(64, 4 * inputText(config.input).length)
    const poolSize = Math.max(1, Math.min(MAX_ITERATIONS, Math.floor(INPUT_POOL_BYTES / inputBytes)))
    const variantModule = config.variantUrl ? await import(config.variantUrl) : null
    if (variantModule && typeof variantModule.variant !== 'function') throw new Error('the bench-variant module does not export variant(input, index)')
    result.inputPool = {
      estimatedBytesPerClone: inputBytes,
      clonesPerBatch: poolSize,
      rule: variantModule ? 'one distinct variant(input, k) per call (k never repeats within an arm), made outside the timed region' : 'one fresh structuredClone per call, made outside the timed region',
    }
    const makeInput = variantModule ? (k) => structuredClone(variantModule.variant(config.input, k)) : () => structuredClone(config.input)
    const fresh = () => structuredClone(config.input)
    const digestOf = async (value) => digestText(canonicalText(normalise(value), config.output.form))
    const contract = (arm, value) => {
      const thenable = isThenable(value)
      if (thenable !== asynchronous) throw new Error(`${arm.key} ${asynchronous ? 'returned a plain value for an async library' : 'returned a Promise for a synchronous library'}`)
      return thenable
    }
    const checksum = async (arm) => {
      beat(`${arm.key} validation`, true)
      const input = fresh()
      const started = now()
      let value = arm.run(input)
      if (contract(arm, value)) value = await value
      const milliseconds = now() - started
      return { digest: await digestOf(value), milliseconds }
    }
    for (const arm of arms) {
      const first = await checksum(arm)
      arm.checksum = first.digest
      arm.firstCallMs = first.milliseconds
    }
    result.checksumsAgree = arms.every((arm) => arm.checksum === arms[0].checksum)
    result.checksum = result.checksumsAgree ? arms[0].checksum : null
    result.checksumMatchesExpected = config.expectedChecksum ? result.checksum === config.expectedChecksum : null
    result.expectedChecksum = config.expectedChecksum ?? null
    result.expectedChecksumSource = config.expectedChecksumSource ?? null
    if (!result.checksumsAgree || result.checksumMatchesExpected === false) return finish({ ...result, status: 'checksum-mismatch' }, arms)
    if (config.mode === 'checksum') return finish(result, arms)

    // With variants, a kept output is checked against the reference's output on the same variant,
    // computed outside the timed region (the reference arm, or the reference imported untimed).
    let oracle = null
    if (variantModule) {
      const referenceArm = arms.find((arm) => arm.kind === 'reference')
      oracle = referenceArm ? referenceArm.run : (await import(config.referenceUrl)).run
      if (typeof oracle !== 'function') throw new Error('no reference to verify bench variants against')
    }
    const expectedFor = async (k) => {
      if (!oracle) return result.checksum
      let value = oracle(makeInput(k))
      if (isThenable(value)) value = await value
      return digestOf(value)
    }

    // `iterations` calls on fresh inputs. Only the calls are timed (a synchronous library's with no
    // await inside the timed region; an async library's awaited one by one). Two outputs are kept
    // for verification: the call at index `pick` and the last one. Picks come from a stream seeded
    // by crypto.getRandomValues per cell (recorded as `pickSeed`), so a port cannot predict them
    // from the cell's identity.
    const timedSync = (arm, iterations, pick) => {
      const run = arm.run
      let elapsed = 0
      let done = 0
      let picked
      let last
      while (done < iterations) {
        const count = Math.min(poolSize, iterations - done)
        const base = arm.nextVariant
        const pool = new Array(count)
        for (let index = 0; index < count; index++) pool[index] = makeInput(kOf(arm, base + index))
        arm.nextVariant += count
        const local = pick - done
        const started = now()
        for (let index = 0; index < count; index++) {
          const value = run(pool[index])
          sink(value)
          if (index === local) picked = value
          last = value
        }
        elapsed += now() - started
        done += count
      }
      return { elapsed, kept: [['seeded random call', picked, kOf(arm, arm.nextVariant - iterations + pick)], ['last call', last, kOf(arm, arm.nextVariant - 1)]] }
    }
    const timedAsync = async (arm, iterations, pick) => {
      const run = arm.run
      let elapsed = 0
      let done = 0
      let picked
      let last
      while (done < iterations) {
        const count = Math.min(poolSize, iterations - done)
        const base = arm.nextVariant
        const pool = new Array(count)
        for (let index = 0; index < count; index++) pool[index] = makeInput(kOf(arm, base + index))
        arm.nextVariant += count
        const local = pick - done
        const started = now()
        for (let index = 0; index < count; index++) {
          const value = await run(pool[index])
          sink(value)
          if (index === local) picked = value
          last = value
        }
        elapsed += now() - started
        done += count
      }
      return { elapsed, kept: [['seeded random call', picked, kOf(arm, arm.nextVariant - iterations + pick)], ['last call', last, kOf(arm, arm.nextVariant - 1)]] }
    }
    // Each arm (each instance of the port, and the reference) takes variants from its own disjoint
    // range, k = 1 + armIndex + armCount * n, so no two arms ever see the same variant: a cache the
    // instances share through the realm (globalThis) cannot hit either.
    arms.forEach((arm, index) => { arm.variantIndex = index })
    const kOf = (arm, sequence) => 1 + arm.variantIndex + arms.length * sequence
    const pickSeed = globalThis.crypto?.getRandomValues ? globalThis.crypto.getRandomValues(new Uint32Array(1))[0] : null
    if (pickSeed === null) throw new Error('crypto.getRandomValues is unavailable for the output picks')
    result.pickSeed = pickSeed
    const pickRandom = mulberry32(pickSeed)
    for (const arm of arms) arm.nextVariant = 0
    const timed = (arm, iterations) => {
      const pick = Math.floor(pickRandom() * iterations)
      return asynchronous ? timedAsync(arm, iterations, pick) : timedSync(arm, iterations, pick)
    }
    // The kept outputs of every calibration, warmup and sample run must have the expected
    // checksum: a port that is wrong or cheap only on the hot path fails the cell.
    const verify = async (arm, kept, phase, round) => {
      for (const [which, value, k] of kept) {
        let digest
        try {
          digest = await digestOf(value)
        } catch (error) {
          digest = `uncanonicalisable: ${error?.message ?? error}`
        }
        const expected = await expectedFor(k)
        if (digest !== expected) {
          result.mismatch = { arm: arm.key, phase, round, output: which, ...(variantModule ? { variant: k } : {}), digest, expected }
          return false
        }
      }
      arm.verifiedOutputs = (arm.verifiedOutputs ?? 0) + kept.length
      return true
    }
    // Calibration: calls until minSampleMs, then iterationsPerSample = max(baseIterations, calls
    // filling minSampleMs). It runs once on cold code and again after the warmups; the samples use
    // the warm calibration.
    const calibrate = async (arm, phase) => {
      let calls = 0
      let elapsed = 0
      let kept
      do {
        beat(`${arm.key} ${phase}`)
        const run = asynchronous ? await timed(arm, 1) : timed(arm, 1)
        elapsed += run.elapsed
        kept = run.kept
        calls++
      } while (elapsed < config.minSampleMs && calls < MAX_ITERATIONS)
      if (!(await verify(arm, kept.slice(1), phase, 0))) return null
      const perCall = elapsed / calls
      return {
        calls,
        milliseconds: elapsed,
        iterationsPerSample: Math.max(config.baseIterations, Math.min(MAX_ITERATIONS, perCall > 0 ? Math.ceil(config.minSampleMs / perCall) : MAX_ITERATIONS)),
      }
    }

    for (const index of shuffled(arms.length, random)) {
      const arm = arms[index]
      const cold = await calibrate(arm, 'calibration')
      if (!cold) return finish({ ...result, status: 'output-mismatch' }, arms)
      arm.calibration = { cold }
      arm.iterationsPerSample = cold.iterationsPerSample
    }
    let round = 0
    for (const order of balancedOrders(arms.length, config.warmups, random)) {
      round++
      for (const index of order) {
        const arm = arms[index]
        beat(`${arm.key} warmup ${round}`)
        const run = asynchronous ? await timed(arm, arm.iterationsPerSample) : timed(arm, arm.iterationsPerSample)
        if (!(await verify(arm, run.kept, 'warmup', round))) return finish({ ...result, status: 'output-mismatch' }, arms)
      }
      await pause()
    }
    for (const index of shuffled(arms.length, random)) {
      const arm = arms[index]
      const warm = await calibrate(arm, 'recalibration')
      if (!warm) return finish({ ...result, status: 'output-mismatch' }, arms)
      arm.calibration.warm = warm
      arm.iterationsPerSample = warm.iterationsPerSample
    }
    for (const arm of arms) arm.sampleMs = []
    result.order = []
    result.samplesBalanced = config.samples % williamsRows(arms.length).length === 0
    round = 0
    for (const order of balancedOrders(arms.length, config.samples, random)) {
      round++
      result.order.push(order.map((index) => arms[index].label).join(''))
      for (const index of order) {
        const arm = arms[index]
        beat(`${arm.key} sample ${round}`)
        const run = asynchronous ? await timed(arm, arm.iterationsPerSample) : timed(arm, arm.iterationsPerSample)
        arm.sampleMs.push(run.elapsed)
        if (!(await verify(arm, run.kept, 'sample', round))) return finish({ ...result, status: 'output-mismatch' }, arms)
      }
      await pause()
    }
    for (const arm of arms) {
      arm.finalChecksum = (await checksum(arm)).digest
      arm.sampleSummary = summarize(arm.sampleMs)
      arm.perCallMs = summarize(arm.sampleMs.map((value) => value / arm.iterationsPerSample))
    }
    result.deterministic = arms.every((arm) => arm.finalChecksum === result.checksum)
    // The samples stay in the record for diagnosis, but a cell whose output drifted has no ratio.
    if (!result.deterministic) return finish({ ...result, status: 'nondeterministic' }, arms)
    result.comparisons = (config.comparisons || []).map(({ name, of: [firstKey, secondKey] }) => {
      const first = arms.find((arm) => arm.key === firstKey)
      const second = arms.find((arm) => arm.key === secondKey)
      const paired = first.sampleMs.map((value, index) =>
        (value / first.iterationsPerSample) / (second.sampleMs[index] / second.iterationsPerSample))
      return {
        name,
        of: `${firstKey}/${secondKey}`,
        p50: first.perCallMs.p50 / second.perCallMs.p50,
        p95: first.perCallMs.p95 / second.perCallMs.p95,
        p95IsMax: first.perCallMs.p95IsMax || second.perCallMs.p95IsMax,
        min: first.perCallMs.min / second.perCallMs.min,
        paired: summarize(paired),
      }
    })
    // The A/A control must be plausible: two instances of one artifact in one process or page
    // cannot honestly differ by 2x. Outside `aaRange` the cell has no ratio (`aa-out-of-range`):
    // the instances interfere (a cache shared through the realm, a warm-up asymmetry) or the run
    // was too noisy to read.
    const aaRange = Array.isArray(config.aaRange) && config.aaRange.length === 2 ? config.aaRange : [0.5, 2]
    result.aaRange = aaRange
    const aa = result.comparisons.find((comparison) => comparison.name === 'A/A')
    if (aa && !(aa.p50 >= aaRange[0] && aa.p50 <= aaRange[1])) {
      result.status = 'aa-out-of-range'
      result.aaOutOfRange = { p50: aa.p50, range: aaRange }
    }
  } catch (error) {
    result.status = 'error'
    result.error = errorRecord(error)
  }
  return finish(result, arms)
}

function finish(result, arms) {
  for (const arm of arms) delete arm.run
  result.sink = sinkValue
  return result
}
