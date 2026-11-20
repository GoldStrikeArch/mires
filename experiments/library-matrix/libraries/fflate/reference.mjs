// Reference adapter for fflate 0.8.3: the published ESM entry, imported by a relative path (no
// bare specifier, no import map), so Node and the browsers load the same file.
//
// `run(input)` executes one fflate operation (CONTRACT.md, "Input") and returns a Uint8Array
// (CONTRACT.md, "Output"). One-shot byte-returning ops (`deflateSync`, `inflateSync`, `gzipSync`,
// `gunzipSync`, `zlibSync`, `unzlibSync`, `decompressSync`, `strToU8`) return the raw Uint8Array
// directly with zero framing overhead (including `bench-compress` and `bench-decompress`).
//
// - `strFromU8` encodes the resulting JS string as exact UTF-16LE bytes (2 bytes per code unit).
// - Streaming classes (`Deflate`, `Inflate`, `Gzip`, `Gunzip`, `Zlib`, `Unzlib`, `Decompress`)
//   concatenate `[u32le(chunk.length), final ? 1 : 0, ...chunk]` for every `ondata` callback.
// - When `input.expectErrorCode === true`, a caught `FlateError` with a numeric `.code` is
//   returned as `Uint8Array([0xff, error.code])`; otherwise errors propagate (testing `both-threw`).
import {
  Deflate,
  Decompress,
  Gunzip,
  Gzip,
  Inflate,
  Unzlib,
  Zlib,
  decompressSync,
  deflateSync,
  gunzipSync,
  gzipSync,
  inflateSync,
  strFromU8,
  strToU8,
  unzlibSync,
  zlibSync,
} from '../../../../node_modules/fflate/esm/browser.js'

function payload(spec) {
  if (spec.bytes !== undefined) return new Uint8Array(spec.bytes)
  return strToU8(spec.text, Boolean(spec.latin1))
}

function compressOptions(json) {
  if (json === undefined) return undefined
  const opts = {}
  if (json.level !== undefined) opts.level = json.level
  if (json.mem !== undefined) opts.mem = json.mem
  if (json.dictionary !== undefined) opts.dictionary = payload(json.dictionary)
  if (json.mtime !== undefined) opts.mtime = json.mtime
  if (json.filename !== undefined) opts.filename = json.filename
  return opts
}

function decompressOptions(json) {
  if (json === undefined) return undefined
  const opts = {}
  if (json.dictionary !== undefined) opts.dictionary = payload(json.dictionary)
  if (json.outSize !== undefined) opts.out = new Uint8Array(json.outSize)
  return opts
}

function stringToUtf16Le(text) {
  const out = new Uint8Array(text.length * 2)
  for (let index = 0; index < text.length; index++) {
    const unit = text.charCodeAt(index)
    out[index * 2] = unit & 0xff
    out[index * 2 + 1] = unit >>> 8
  }
  return out
}

function frameRecords(records) {
  let total = 0
  for (const [chunk] of records) total += 5 + chunk.length
  const out = new Uint8Array(total)
  let offset = 0
  for (const [chunk, isFinal] of records) {
    const len = chunk.length
    out[offset] = len & 0xff
    out[offset + 1] = (len >>> 8) & 0xff
    out[offset + 2] = (len >>> 16) & 0xff
    out[offset + 3] = (len >>> 24) & 0xff
    out[offset + 4] = isFinal ? 1 : 0
    out.set(chunk, offset + 5)
    offset += 5 + len
  }
  return out
}

function runStream(StreamClass, input, isCompress) {
  const opts = isCompress ? compressOptions(input.options) : decompressOptions(input.options)
  const records = []
  const cb = input.noHandler ? undefined : (chunk, isFinal) => {
    records.push([new Uint8Array(chunk), Boolean(isFinal)])
  }
  const stream = opts !== undefined ? new StreamClass(opts, cb) : new StreamClass(cb)
  for (const step of input.steps) {
    if (step.action === 'push') {
      stream.push(payload(step.data), step.final)
    } else if (step.action === 'flush') {
      stream.flush(step.sync)
    } else {
      throw new TypeError(`unknown stream action ${step.action}`)
    }
  }
  return frameRecords(records)
}

function execute(input) {
  switch (input.op) {
    case 'deflateSync': return deflateSync(payload(input.data), compressOptions(input.options))
    case 'inflateSync': return inflateSync(payload(input.data), decompressOptions(input.options))
    case 'gzipSync': return gzipSync(payload(input.data), compressOptions(input.options))
    case 'gunzipSync': return gunzipSync(payload(input.data), decompressOptions(input.options))
    case 'zlibSync': return zlibSync(payload(input.data), compressOptions(input.options))
    case 'unzlibSync': return unzlibSync(payload(input.data), decompressOptions(input.options))
    case 'decompressSync': return decompressSync(payload(input.data), decompressOptions(input.options))
    case 'strToU8': return strToU8(input.text, Boolean(input.latin1))
    case 'strFromU8': return stringToUtf16Le(strFromU8(payload(input.data), Boolean(input.latin1)))
    case 'Deflate': return runStream(Deflate, input, true)
    case 'Inflate': return runStream(Inflate, input, false)
    case 'Gzip': return runStream(Gzip, input, true)
    case 'Gunzip': return runStream(Gunzip, input, false)
    case 'Zlib': return runStream(Zlib, input, true)
    case 'Unzlib': return runStream(Unzlib, input, false)
    case 'Decompress': return runStream(Decompress, input, false)
    default: throw new TypeError(`unknown operation ${input.op}`)
  }
}

export function run(input) {
  if (!input.expectErrorCode) return execute(input)
  try {
    execute(input)
  } catch (error) {
    if (error && typeof error.code === 'number') return new Uint8Array([0xff, error.code])
    throw error
  }
  throw new Error(`expected FlateError for op ${input.op}, but call succeeded`)
}
