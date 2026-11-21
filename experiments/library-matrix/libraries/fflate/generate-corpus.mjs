#!/usr/bin/env node
// Writes corpus.json for the fflate library: every case is built deterministically (no clock, no
// Math.random: pseudo-random data uses mulberry32 with fixed seeds, and the 1 MiB word-salad text a
// fixed-seed LCG), so corpus.json is a pure function of this script. CONTRACT.md documents every
// case.
//
//   node libraries/fflate/generate-corpus.mjs            # writes corpus.json
//   node libraries/fflate/generate-corpus.mjs --verify   # also runs reference.mjs on every case,
//                                                        # validates all invariants & mutant killers,
//                                                        # bench-variant, and prints bench timings.

import { createHash } from 'node:crypto'
import { writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  deflateSync,
  gzipSync,
  inflateSync,
  strFromU8,
  strToU8,
  zlibSync,
} from '../../../../node_modules/fflate/esm/browser.js'

const here = dirname(fileURLToPath(import.meta.url))

// ---------------------------------------------------------------------------------------------
// Deterministic PRNG and helpers.

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

function concatBytes(...arrays) {
  let total = 0
  for (const a of arrays) total += a.length
  const out = new Uint8Array(total)
  let off = 0
  for (const a of arrays) {
    out.set(a, off)
    off += a.length
  }
  return out
}

// LSB-first bit writer for hand-crafted RFC 1951 test streams.
function bitWriter() {
  const bytes = []
  let cur = 0
  let bits = 0
  return {
    write(val, count) {
      for (let i = 0; i < count; i++) {
        cur |= ((val >>> i) & 1) << bits
        bits++
        if (bits === 8) {
          bytes.push(cur)
          cur = 0
          bits = 0
        }
      }
    },
    writeRev(val, count) {
      // Write Huffman code MSB-first into LSB-first stream.
      for (let i = count - 1; i >= 0; i--) {
        cur |= ((val >>> i) & 1) << bits
        bits++
        if (bits === 8) {
          bytes.push(cur)
          cur = 0
          bits = 0
        }
      }
    },
    align() {
      if (bits > 0) {
        bytes.push(cur)
        cur = 0
        bits = 0
      }
    },
    writeBytes(arr) {
      this.align()
      for (const b of arr) bytes.push(b & 0xff)
    },
    finish() {
      this.align()
      return new Uint8Array(bytes)
    },
  }
}

function latin1Payload(u8) {
  return { text: strFromU8(u8, true), latin1: true }
}

function bytesPayload(u8) {
  return { bytes: Array.from(u8) }
}

function splitChunks(u8, sizes) {
  const chunks = []
  let off = 0
  let si = 0
  while (off < u8.length) {
    const sz = sizes[si % sizes.length]
    si++
    chunks.push(u8.subarray(off, Math.min(u8.length, off + sz)))
    off += sz
  }
  return chunks
}

// Realistic mixed-entropy text (JSON API logs, HTML fragments, structured server logs, prose,
// and occasional non-ASCII UTF-8) yielding ~3.0x compression ratio at level 6.
function makeBenchText(seed, targetBytes) {
  const rnd = mulberry32(seed)
  const routes = [
    '/api/v2/users', '/api/v2/orders', '/api/v2/events', '/api/v2/invoices',
    '/assets/app.bundle.js', '/docs/guide/compression', '/healthz', '/search?q=deflate',
    '/static/theme.css', '/v1/telemetry/ingest', '/v1/catalog/items', '/graphql',
  ]
  const methods = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD']
  const statuses = [200, 200, 200, 201, 204, 301, 304, 400, 401, 404, 429, 502, 503]
  const hosts = ['edge-fra-01', 'edge-iad-04', 'edge-nrt-02', 'edge-sjc-07', 'edge-lhr-03', 'edge-sin-05']
  const tags = [
    'alpha', 'beta', 'gamma', 'delta', 'epsilon', 'zeta', 'eta', 'theta', 'iota', 'kappa',
    'lambda', 'sigma', 'omega', 'cache', 'worker', 'stream', 'buffer', 'codec', 'header', 'footer',
    'canary', 'primary', 'replica', 'archive', 'ingest', 'egress', 'metrics', 'tracing',
  ]
  const words = [
    'compression', 'stream', 'produces', 'deterministic', 'output', 'blocks', 'using', 'sliding',
    'window', 'Huffman', 'tables', 'checksum', 'payload', 'benchmark', 'latency', 'throughput',
    'memory', 'allocation', 'parser', 'serializer', 'document', 'record', 'metadata', 'timestamp',
    'encoding', 'decoding', 'fragment', 'sequence', 'boundary', 'protocol', 'network', 'storage',
    'browser', 'runtime', 'compiler', 'pipeline', 'module', 'package', 'manifest', 'dictionary',
    'histogram', 'frequency', 'backreference', 'literal', 'distance', 'symbol', 'canonical', 'prefix',
    'entropy', 'variance', 'cluster', 'shard', 'partition', 'cursor', 'offset', 'length', 'trailer',
    'café', 'naïve', 'résumé', 'Zürich', '東京', 'Straße',
  ]
  const pick = (a) => a[Math.floor(rnd() * a.length)]
  const token = (n) => {
    const alphabet = '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_-'
    let s = ''
    for (let i = 0; i < n; i++) s += alphabet[Math.floor(rnd() * alphabet.length)]
    return s
  }
  const parts = []
  let byteLen = 0
  let id = 1000
  while (byteLen < targetBytes) {
    id++
    const kind = id % 4
    let chunk
    if (kind === 0) {
      chunk = `${JSON.stringify({
        id: `evt_${id}_${token(10)}`,
        trace: token(16),
        ts: 1700000000 + id * 7 + Math.floor(rnd() * 5),
        host: pick(hosts),
        method: pick(methods),
        path: `${pick(routes)}/${token(6)}`,
        status: pick(statuses),
        ms: Math.floor(rnd() * 950) + 1,
        bytes: Math.floor(rnd() * 131072) + 64,
        tags: [pick(tags), pick(tags), pick(tags)],
        note: `${pick(words)} ${pick(words)} ${token(8)} ${pick(words)}`,
      })}\n`
    } else if (kind === 1) {
      chunk = `<article data-id="${id}" data-sig="${token(12)}" class="entry ${pick(tags)}"><h3>${pick(words)} ${pick(words)} #${id}</h3><p>${pick(words)} ${pick(words)} ${pick(words)} ${pick(words)} (${token(10)}) — ${pick(words)} ${pick(words)} ${pick(words)}.</p></article>\n`
    } else if (kind === 2) {
      chunk = `2026-11-19T09:${String(id % 60).padStart(2, '0')}:${String((id * 13) % 60).padStart(2, '0')}.${String(Math.floor(rnd() * 1000)).padStart(3, '0')}Z INFO [${pick(hosts)}] req=${token(14)} ${pick(methods)} ${pick(routes)}/${token(4)} status=${pick(statuses)} dur=${(rnd() * 120).toFixed(2)}ms msg="${pick(words)} ${pick(words)} ${pick(words)} ${token(6)}"\n`
    } else {
      const n = 12 + Math.floor(rnd() * 12)
      const ws = []
      for (let i = 0; i < n; i++) ws.push(rnd() < 0.22 ? token(8) : pick(words))
      chunk = `${ws.join(' ')}.\n`
    }
    parts.push(chunk)
    byteLen += strToU8(chunk).length
  }
  let joined = parts.join('')
  while (strToU8(joined).length > targetBytes) {
    joined = joined.slice(0, joined.length - Math.max(1, Math.floor((strToU8(joined).length - targetBytes) / 2)))
  }
  return joined
}

// ~15 KB payload where EVERY compression level 0..9 yields a strictly distinct output,
// exercising every (nice, chain) pair of fflate's deo table — including chain depth 2500 (> 2048)
// so level 9 (chain = 4096) is distinct from chain = 2048 as well as level 8 (chain = 1024):
// L1(8,4), L2(16,8), L3(16,16), L4(16,32), L5(32,32), L6(128,128), L7(128,256), L8(258,1024), L9(258,4096).
function makeLevelText() {
  const parts = []
  const probe = (pfx, depth, targetTail) => {
    parts.push(`{${pfx}${targetTail}}`)
    for (let i = 0; i < depth; i++) {
      const c1 = String.fromCharCode(0x21 + (i % 80))
      const c2 = String.fromCharCode(0x21 + Math.floor(i / 80))
      parts.push(pfx + (c1 === targetTail[0] ? '~' : c1) + c2)
    }
    parts.push(`|${pfx}${targetTail}^`)
  }
  const tail = (len, seed) => {
    let s = ''
    for (let i = 0; i < len; i++) s += String.fromCharCode(0x61 + ((i * 13 + seed * 7 + i * i) % 26))
    return s
  }
  probe('X1Y', 2500, tail(40, 1)) // L8 (1024) & mutant 2048 miss; L9 (4096) hits
  probe('X2Y', 180, tail(40, 2))  // L6 (128) misses, L7 (256) hits
  probe('X3Y', 22, tail(40, 3))   // L3 (16) misses, L4 (32) hits
  probe('X4Y', 11, tail(40, 4))   // L2 (8) misses, L3 (16) hits
  probe('X5Y', 5, tail(40, 5))    // L1 (4) misses, L2 (8) hits
  const niceProbe = (pfx, shortLen, longLen, seed) => {
    const t = tail(longLen, seed)
    parts.push(`{${pfx}${t}}`)
    parts.push(`[${pfx}${t.slice(0, shortLen)}]`)
    parts.push(`|${pfx}${t}^`)
  }
  niceProbe('F6G', 20, 60, 6)   // L4 (nice=16) stops at 20, L5 (nice=32) finds 60
  niceProbe('G7H', 40, 160, 7)  // L5 (nice=32) stops at 40, L6 (nice=128) finds 160
  niceProbe('H8J', 140, 250, 8) // L7 (nice=128) stops at 140, L8 (nice=258) finds 250
  return parts.join('')
}

function makeRandomBytes(seed, len) {
  const rnd = mulberry32(seed)
  const u8 = new Uint8Array(len)
  for (let i = 0; i < len; i++) u8[i] = Math.floor(rnd() * 256)
  return u8
}

// 22,770-byte (< 24,576!) payload built from 90 3-byte seeds with 90 unique first bytes,
// avoiding repeated pairs so every match has length 3. Thus lc_1 exceeds 7,000 while li stays
// well below 24,576, forcing a multi-block split purely via `lc_1 > 7000`.
function makeSplitLcText() {
  const seeds = []
  for (let i = 0; i < 90; i++) {
    seeds.push(String.fromCharCode(
      0x21 + i,
      0x21 + ((i * 17 + 5) % 90),
      0x21 + ((i * 31 + 11) % 90),
    ))
  }
  const rnd = mulberry32(0x11223344)
  const usedPair = new Set()
  const parts = [...seeds]
  let prev = seeds.length - 1
  for (let step = 0; step < 7500; step++) {
    let pick = Math.floor(rnd() * seeds.length)
    for (let tries = 0; tries < 90; tries++) {
      const cand = (pick + tries) % seeds.length
      const key = prev * 100 + cand
      if (!usedPair.has(key)) {
        usedPair.add(key)
        pick = cand
        break
      }
    }
    parts.push(seeds[pick])
    prev = pick
  }
  return parts.join('')
}

// Zero-trigram-repeat 24,345-byte block: 13 Fibonacci symbols [1, 2, 3, ..., 377] (plus EOB=1
// => 14 levels) underneath 64 top symbols of count 365 (+6 levels => unclamped Huffman depth 19),
// forcing hTree(lf, 15) length clamping + debt redistribution AND producing an empty distance tree
// (df all zeros => lc(et) => { c: [0], n: 1 }).
function makeHuffmanClampText() {
  const rnd = mulberry32(0x42424242)
  const counts = [1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377]
  for (let i = 0; i < 64; i++) counts.push(365)
  const rem = counts.slice()
  const total = rem.reduce((a, b) => a + b, 0)
  const out = new Uint8Array(total)
  const seen = new Set()
  for (let i = 0; i < total; i++) {
    let best = -1
    let bestScore = -1
    for (let s = 0; s < rem.length; s++) {
      if (rem[s] === 0) continue
      const score = (rem[s] / counts[s]) + rnd() * 0.25
      if (score <= bestScore) continue
      if (i >= 2) {
        const tri = (out[i - 2] << 16) | (out[i - 1] << 8) | (0x21 + s)
        if (seen.has(tri)) continue
      }
      best = s
      bestScore = score
    }
    if (best === -1) {
      for (let s = 0; s < rem.length; s++) if (rem[s] > 0) { best = s; break }
    }
    out[i] = 0x21 + best
    rem[best]--
    if (i >= 2) seen.add((out[i - 2] << 16) | (out[i - 1] << 8) | out[i])
  }
  return strFromU8(out, true)
}

function makeAllFlagsGzip() {
  const plain = strToU8('All GZIP header flags (FTEXT, FHCRC, FEXTRA, FNAME, FCOMMENT) parsed cleanly!')
  const baseGz = gzipSync(plain, { level: 6, mtime: 0 })
  const bodyAndTrailer = baseGz.subarray(10)
  const extra = new Uint8Array([0x06, 0x00, 0x41, 0x42, 0x02, 0x00, 0xca, 0xfe])
  const fname = strToU8('archive-entry.txt\0')
  const fcomment = strToU8('all optional flags present\0')
  const fhcrc = new Uint8Array([0x12, 0x34])
  const head = new Uint8Array([31, 139, 8, 0x1f, 0x10, 0x20, 0x30, 0x40, 0, 3])
  return concatBytes(head, extra, fname, fcomment, fhcrc, bodyAndTrailer)
}

function makeAmbiguous0x78Deflate() {
  const msg = strToU8('raw-deflate-starting-with-0x78!')
  const len = msg.length
  const nlen = len ^ 0xffff
  return concatBytes(
    new Uint8Array([0x78, len & 0xff, len >>> 8, nlen & 0xff, nlen >>> 8]),
    msg,
    new Uint8Array([0x03, 0x00]),
  )
}

function makeDist32768Deflate() {
  const bw = bitWriter()
  bw.write(0, 1) // BFINAL = 0
  bw.write(0, 2) // BTYPE = 00 (stored)
  bw.align()
  const len = 32768
  const nlen = len ^ 0xffff
  bw.writeBytes([len & 0xff, len >>> 8, nlen & 0xff, nlen >>> 8])
  const payload = new Uint8Array(len)
  payload.fill(0x61)
  payload[0] = 0x5a // 'Z'
  payload[1] = 0x59 // 'Y'
  payload[2] = 0x58 // 'X'
  bw.writeBytes(payload)
  bw.write(1, 1) // BFINAL = 1
  bw.write(1, 2) // BTYPE = 01 (fixed)
  bw.writeRev(1, 7) // sym 257 (length 3)
  bw.writeRev(29, 5) // dsym 29 (base 24577)
  bw.write(8191, 13) // + 8191 = 32768
  bw.writeRev(0, 7) // sym 256 (EOB)
  return bw.finish()
}

// Multi-block raw DEFLATE stream combining stored (BTYPE=0), fixed Huffman (BTYPE=1),
// and a genuine dynamic Huffman block (BTYPE=2, asserted in --verify).
function makeMixedMultiblockDeflate() {
  const bw = bitWriter()
  const p1 = strToU8('STORED-PART|')
  bw.write(0, 1)
  bw.write(0, 2)
  bw.writeBytes([p1.length & 0xff, p1.length >>> 8, (p1.length ^ 0xff) & 0xff, ((p1.length >>> 8) ^ 0xff) & 0xff])
  bw.writeBytes(p1)
  bw.write(0, 1)
  bw.write(1, 2)
  for (const ch of strToU8('FIXED|')) bw.writeRev(0x30 + ch, 8)
  bw.writeRev(0, 7) // EOB
  bw.write(0, 1)
  bw.write(0, 2)
  bw.writeBytes([0, 0, 255, 255])
  const dynSrc = strToU8(makeBenchText(0x31415926, 800))
  const dyn = deflateSync(dynSrc, { level: 6 })
  return { stream: concatBytes(bw.finish(), dyn), dynBlock: dyn }
}

function makeInvalidLitLenStream() {
  const bw = bitWriter()
  bw.write(1, 1) // BFINAL = 1
  bw.write(2, 2) // BTYPE = 10 (dynamic)
  bw.write(0, 5) // HLIT = 0 (257 codes)
  bw.write(0, 5) // HDIST = 0 (1 code)
  bw.write(14, 4) // HCLEN = 14 (18 code-length entries; clim[2]=18, clim[17]=1)
  for (let i = 0; i < 18; i++) bw.write(i === 2 || i === 17 ? 1 : 0, 3)
  bw.writeRev(1, 1); bw.write(127, 7) // sym 18: 138 zeros
  bw.writeRev(1, 1); bw.write(107, 7) // sym 18: 118 zeros (256 zeros)
  bw.writeRev(0, 1) // sym 1 -> lit/len[256] = 1 (code 0); code 1 unmapped
  bw.writeRev(0, 1) // sym 1 -> dist[0] = 1
  bw.write(1, 1) // unmapped lit/len code 1 => InvalidLengthLiteral (2)
  bw.write(0, 8)
  return bw.finish()
}

function makeInvalidDistCodeStream() {
  const bw = bitWriter()
  bw.write(1, 1)
  bw.write(2, 2)
  bw.write(1, 5) // HLIT = 1 (258 codes: 0..257)
  bw.write(0, 5) // HDIST = 0 (1 dist code)
  bw.write(14, 4)
  for (let i = 0; i < 18; i++) bw.write(i === 2 || i === 17 ? 1 : 0, 3)
  bw.writeRev(1, 1); bw.write(127, 7)
  bw.writeRev(1, 1); bw.write(107, 7)
  bw.writeRev(0, 1) // lit/len[256] = 1 (code 0)
  bw.writeRev(0, 1) // lit/len[257] = 1 (code 1)
  bw.writeRev(0, 1) // dist[0] = 1 (code 0); dist code 1 unmapped
  bw.write(1, 1) // lit/len code 1 => sym 257 (length 3)
  bw.write(1, 1) // unmapped dist code 1 => InvalidDistance (3)
  bw.write(0, 8)
  return bw.finish()
}

function makeInvalidDistBackrefStream() {
  const bw = bitWriter()
  bw.write(1, 1)
  bw.write(1, 2)
  bw.writeRev(1, 7) // sym 257 (length 3)
  bw.writeRev(0, 5) // dsym 0 (distance 1 > bt=0)
  bw.writeRev(0, 7) // EOB
  return bw.finish()
}

// One final dynamic block holding `n` literal 'a' (97) as 1-bit codes, so the run is far longer
// than fflate's output buffer: `inflt` grows the buffer (`cbuf(bt + 131072)`) only at block start
// and before a back-reference, and drops every literal stored past its end (`buf[bt++] = sym` on a
// Uint8Array). Without `backref`: lit/len lengths {97: 1, 256: 1} via a code-length tree {1: 1,
// 18: 1} (the final 18 writes 11 zero lengths from index 257, overrunning `tl = 258`, which `inflt`
// ignores), then n zero bits and EOB. With `backref`: lit/len {97: 1, 256: 2, 257: 2} and one
// distance code (symbol 0, distance 1) via a code-length tree {0, 1, 2, 18: 2 bits each}; the run
// is followed by one length-3 copy at distance 1 that grows the buffer past `bt`, so the dropped
// positions read back as the new buffer's zeros.
function makeLiteralRunDeflate(n, backref) {
  const bw = bitWriter()
  bw.write(1, 1) // BFINAL
  bw.write(2, 2) // BTYPE = 2 (dynamic)
  bw.write(backref ? 1 : 0, 5) // HLIT: 257 or 258 lit/len lengths
  bw.write(0, 5) // HDIST: 1 distance length
  bw.write(14, 4) // HCLEN: 18 code-length lengths (clim[0..17])
  const clim = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]
  const cl = backref ? { 0: 2, 1: 2, 2: 2, 18: 2 } : { 1: 1, 18: 1 }
  for (let k = 0; k < 18; k++) bw.write(cl[clim[k]] || 0, 3)
  if (backref) {
    // canonical code-length codes: 0 -> 00, 1 -> 01, 2 -> 10, 18 -> 11
    bw.writeRev(3, 2); bw.write(86, 7) // 97 zeros (0..96)
    bw.writeRev(1, 2) // 97 ('a'): 1
    bw.writeRev(3, 2); bw.write(127, 7) // 138 zeros (98..235)
    bw.writeRev(3, 2); bw.write(9, 7) // 20 zeros (236..255)
    bw.writeRev(2, 2) // 256: 2
    bw.writeRev(2, 2) // 257: 2
    bw.writeRev(1, 2) // distance symbol 0: 1
    // lit/len codes: 'a' -> 0, 256 -> 10, 257 -> 11; distance symbol 0 -> 0
    for (let i = 0; i < n; i++) bw.writeRev(0, 1)
    bw.writeRev(3, 2) // 257: length 3
    bw.writeRev(0, 1) // distance 1
    bw.writeRev(2, 2) // EOB
  } else {
    // canonical code-length codes: 1 -> 0, 18 -> 1
    bw.writeRev(1, 1); bw.write(86, 7) // 97 zeros (0..96)
    bw.writeRev(0, 1) // 97 ('a'): 1
    bw.writeRev(1, 1); bw.write(127, 7) // 138 zeros (98..235)
    bw.writeRev(1, 1); bw.write(9, 7) // 20 zeros (236..255)
    bw.writeRev(0, 1) // 256: 1
    bw.writeRev(1, 1); bw.write(0, 7) // 11 zeros from 257 (distance lengths; overrun ignored)
    // lit/len codes: 'a' -> 0, 256 -> 1
    for (let i = 0; i < n; i++) bw.writeRev(0, 1)
    bw.writeRev(1, 1) // EOB
  }
  return bw.finish()
}

// Concatenation of `n` one-line GZIP members ('m0', 'm1', ...): `Gunzip.push` recurses once per
// member, so a single final push of this input nests `n` calls.
function makeMultiMemberGzip(n) {
  const parts = []
  for (let i = 0; i < n; i++) parts.push(gzipSync(strToU8(`m${i}`), { mtime: 0 }))
  return concatBytes(...parts)
}

// Word-salad text from a 32-bit LCG (x = x * 1103515245 + 12345) over a fixed word list with
// ASCII, 2-, 3- and 4-byte UTF-8 words: the 2026-11-20 review's large-input probe text, kept
// byte-identical here so `zlib-1mb-l1-mem12` equals the probe that every port passed.
function makeWordSaladText(n, seed) {
  const words = ['alpha', 'beta', 'gamma', 'delta', '{"id":', '"event":"click"', '<div class="x">', '</div>', 'ERROR', 'info', 'été', '€', '\n', ' ', 'the', 'and', '12345', '\u{1F600}']
  let x = seed >>> 0
  let s = ''
  while (s.length < n) {
    x = (Math.imul(x, 1103515245) + 12345) >>> 0
    s += words[(x >>> 16) % words.length]
    if ((x & 0xff) < 9) s += String.fromCharCode(33 + ((x >>> 8) % 90))
  }
  return s.slice(0, n)
}

// ---------------------------------------------------------------------------------------------
// Assemble corpus cases.

const cases = []
function add(id, note, input) {
  cases.push({ id, note, input })
}

const benchCompressText = makeBenchText(0x5f3759df, 65536) // ~64 KB realistic mixed-entropy text

// 1. Empty & tiny inputs (5)
add('deflate-empty', 'deflateSync on empty input (single fixed-Huffman EOB block)', {
  op: 'deflateSync',
  data: { text: '' },
})
add('deflate-empty-level0', 'deflateSync level 0 on empty input (empty stored block 01 00 00 FF FF)', {
  op: 'deflateSync',
  data: { text: '' },
  options: { level: 0 },
})
add('deflate-1byte', 'deflateSync on 1-byte input (skips main LZ77 loop)', {
  op: 'deflateSync',
  data: { text: 'A' },
})
add('gzip-empty', 'gzipSync on empty input with mtime 0', {
  op: 'gzipSync',
  data: { text: '' },
  options: { mtime: 0 },
})
add('zlib-empty', 'zlibSync on empty input (header 78 9C, Adler-32 = 1)', {
  op: 'zlibSync',
  data: { text: '' },
})

// 2. Compression levels 0..9 and mem option (12)
const levelText = makeLevelText()
for (let lvl = 0; lvl <= 9; lvl++) {
  add(`deflate-level-${lvl}`, `deflateSync at level ${lvl} exercising deo nice/chain table (all 10 levels strictly distinct; L9 chain > 2048)`, {
    op: 'deflateSync',
    data: { text: levelText },
    options: { level: lvl },
  })
}
// 13.9 KB payload (default plvl = 15: bs1=5, bs2=10, msk=32767) designed so mem:0 (plvl=12),
// default mem (plvl=15), and mem:8 (plvl=20) all produce strictly distinct outputs at level 1:
// "PQA" collides with "PQ!" under plvl=15 (0x41 & 31 === 0x21 & 31) but NOT under plvl=20,
// while the random-mixed backdrop causes heavy 4096-bucket collisions under mem:0.
function makeMemText() {
  const rnd = mulberry32(0x51515151)
  const tail = 'abcdefghij0123456789klmnopqrstuvwxzyABCD'
  const parts = [`{PQ!${tail}}`]
  for (let i = 0; i < 8; i++) parts.push(`PQA${String.fromCharCode(0x30 + i)}${String.fromCharCode(0x61 + i)}`)
  parts.push(`|PQ!${tail}^`)
  const pad = new Uint8Array(13800)
  for (let i = 0; i < pad.length; i++) pad[i] = 0x20 + Math.floor(rnd() * 95)
  for (let k = 0; k < 120; k++) {
    const p1 = k * 30
    const p2 = 9000 + k * 30
    for (let j = 0; j < 12; j++) pad[p2 + j] = pad[p1 + j]
  }
  parts.push(strFromU8(pad, true))
  return parts.join('')
}

const memText = makeMemText()
add('deflate-mem-0', 'deflateSync level 1 with mem 0 (plvl 12, 4096-entry hash table; strictly distinct from default mem and mem 8)', {
  op: 'deflateSync',
  data: { text: memText },
  options: { level: 1, mem: 0 },
})
add('deflate-mem-8', 'deflateSync level 1 with mem 8 (plvl 20, bs1 = 7, bs2 = 14; strictly distinct from default mem and mem 0)', {
  op: 'deflateSync',
  data: { text: memText },
  options: { level: 1, mem: 8 },
})

// 3. Block types & LZ77 / Huffman stress (12)
const incompBytes = makeRandomBytes(0xdeadbeef, 1500)
add('deflate-stored-fallback', 'deflateSync level 6 on high-entropy incompressible data triggers wblk stored-block fallback', {
  op: 'deflateSync',
  data: latin1Payload(incompBytes),
  options: { level: 6 },
})
add('deflate-fixed-huffman', 'deflateSync on short text choosing fixed Huffman (BTYPE=1, ftlen <= dtlen)', {
  op: 'deflateSync',
  data: { text: 'hello world! 12345' },
})
const rleText = 'A'.repeat(400) + 'AB'.repeat(300) + 'ABC'.repeat(250) + 'ABCD'.repeat(200)
add('deflate-rle-overlap', 'deflateSync on overlapping RLE runs hitting max length 258 (fl[28] = 258) and dist 1..4', {
  op: 'deflateSync',
  data: { text: rleText },
})
add('inflate-rle-overlap', 'inflateSync on overlapping back-references (dist < len, length 258)', {
  op: 'inflateSync',
  data: latin1Payload(deflateSync(strToU8(rleText))),
})
const marker = 'MARKER_DISTANCE_BOUNDARY_CHECK_1234567890!'
const filler = makeRandomBytes(0x77771111, 32767 - marker.length)
for (let i = 0; i < filler.length; i++) filler[i] = 0x21 + (filler[i] % 94)
const maxDistText = marker + strFromU8(filler, true) + marker + '!' + marker
add('deflate-max-distance-32767', 'deflateSync with match at exact maxd = 32767 boundary (fd[29] + 13 extra bits)', {
  op: 'deflateSync',
  data: { text: maxDistText },
  options: { level: 6 },
})
add('inflate-max-distance-32768', 'inflateSync on hand-crafted stream with exact distance-32768 match (fd[29] + 8191)', {
  op: 'inflateSync',
  data: latin1Payload(makeDist32768Deflate()),
})
const clampText = makeHuffmanClampText()
add('deflate-huffman-15bit-clamp', 'deflateSync on zero-repeat Fibonacci literals forcing hTree 15-bit clamping + debt rollback + empty distance tree', {
  op: 'deflateSync',
  data: { text: clampText },
  options: { level: 6 },
})
const splitLcText = makeSplitLcText()
add('deflate-split-lc', 'deflateSync on 22.7 KB (< 24,576) 3-byte-match payload splitting blocks purely via lc > 7000', {
  op: 'deflateSync',
  data: { text: splitLcText },
  options: { level: 6 },
})
const largeMultiText = makeBenchText(0x2468ace0, 70000)
add('deflate-multiblock-large', 'deflateSync on 70 KB payload (>64 KB) splitting across multiple blocks (lc > 7000 / li > 24576)', {
  op: 'deflateSync',
  data: { text: largeMultiText },
  options: { level: 6 },
})
const stored65536 = new Uint8Array(65536)
for (let i = 0; i < 65536; i++) stored65536[i] = 0x30 + (i % 64)
add('deflate-level0-65536', 'deflateSync level 0 on 65536 bytes splitting into 65535 + 1 stored blocks', {
  op: 'deflateSync',
  data: { text: strFromU8(stored65536, true) },
  options: { level: 0 },
})
add('deflate-level0-65535', 'deflateSync level 0 on exact 65535 bytes (exercises fflate i < s + lst trailing empty stored block)', {
  op: 'deflateSync',
  data: { text: strFromU8(stored65536.subarray(0, 65535), true) },
  options: { level: 0 },
})
const mixedMulti = makeMixedMultiblockDeflate()
add('inflate-multiblock-mixed', 'inflateSync on multi-block stream mixing stored (0), fixed-Huffman (1), and genuine dynamic-Huffman (2) blocks', {
  op: 'inflateSync',
  data: latin1Payload(mixedMulti.stream),
})

// 4. Custom dictionary (6)
// Place disjoint oldDictMarker BEFORE the -32768 cutoff (must NOT match) and insideDictMarker
// just INSIDE the -32768 cutoff (MUST match; also outside -16384 to verify exact 32768 boundary).
const oldDictMarker = 'OLD_ALPHA_OUTSIDE_WINDOW_QWERTYUIOP_7777' // 40 bytes
const insideDictMarker = 'NEW_OMEGA_INSIDE_CUTOFF_ZXCVBNMLKJH_8888' // 40 bytes
const dictStr = 'COMMON_HEADER_PREFIX_JSON_SCHEMA_VERSION_2026: {"status":"ok","service":"fflate-matrix","items":['
const dictTargetStr = `${oldDictMarker}|${insideDictMarker}|${dictStr}"one","two","three"]}`
// Total length = 34000; cutoff -32768 is at offset 1232.
// oldDictMarker sits at [100..140] (< 1232 -> truncated away);
// insideDictMarker sits at [1300..1340] (>= 1232 -> kept, and 34000 - 1300 = 32700 > 16384!).
const longDictStr = 'x'.repeat(100)
  + oldDictMarker
  + 'y'.repeat(1160)
  + insideDictMarker
  + 'z'.repeat(34000 - 100 - oldDictMarker.length - 1160 - insideDictMarker.length - dictStr.length)
  + dictStr
add('deflate-with-dictionary', 'deflateSync with 34 KB dictionary verifying exact -32768 truncation (old marker dropped, inside-32K marker matched)', {
  op: 'deflateSync',
  data: { text: dictTargetStr },
  options: { dictionary: { text: longDictStr } },
})
add('inflate-with-dictionary', 'inflateSync with matching custom dictionary (back-references into dict via shift = dl - dt)', {
  op: 'inflateSync',
  data: latin1Payload(deflateSync(strToU8(dictTargetStr), { dictionary: strToU8(dictStr) })),
  options: { dictionary: { text: dictStr } },
})
add('zlib-with-dictionary', 'zlibSync level 0 with 34 KB dictionary (FDICT=1, FCHECK=31 -> 78 3F, and Adler-32 DICTID over full untruncated dictionary)', {
  op: 'zlibSync',
  data: { text: dictTargetStr },
  options: { level: 0, dictionary: { text: longDictStr } },
})
add('unzlib-with-dictionary', 'unzlibSync with matching custom dictionary', {
  op: 'unzlibSync',
  data: latin1Payload(zlibSync(strToU8(dictTargetStr), { level: 6, dictionary: strToU8(dictStr) })),
  options: { dictionary: { text: dictStr } },
})
add('gzip-with-dictionary', 'gzipSync with custom dictionary and mtime 0', {
  op: 'gzipSync',
  data: { text: dictTargetStr },
  options: { mtime: 0, dictionary: { text: dictStr } },
})
add('gunzip-with-dictionary', 'gunzipSync with matching custom dictionary', {
  op: 'gunzipSync',
  data: latin1Payload(gzipSync(strToU8(dictTargetStr), { mtime: 0, dictionary: strToU8(dictStr) })),
  options: { dictionary: { text: dictStr } },
})

// 5. GZIP & Zlib framing (5)
add('gzip-mtime-filename-l9', 'gzipSync with explicit mtime, filename (FNAME flag), and level 9 (XFL = 2)', {
  op: 'gzipSync',
  data: { text: 'GZIP metadata check: explicit mtime and filename at level 9! '.repeat(12) },
  options: { level: 9, mtime: 1700000000987, filename: 'report-2026.log' },
})
add('gzip-level1-empty-filename', 'gzipSync with level 1 (XFL = 4) and empty filename (falsy -> no FNAME flag)', {
  op: 'gzipSync',
  data: { text: 'Empty filename omits FNAME bit in GZIP header. '.repeat(8) },
  options: { level: 1, mtime: 1609459200000, filename: '' },
})
add('gunzip-all-header-flags', 'gunzipSync on valid GZIP with FTEXT|FHCRC|FEXTRA|FNAME|FCOMMENT flags all set', {
  op: 'gunzipSync',
  data: latin1Payload(makeAllFlagsGzip()),
})
add('zlib-level-9-adler-wrap', 'zlibSync level 9 (FLEVEL=3 -> 78 DA) on 6000 bytes of 0xFF stressing Adler-32 chunk reduction', {
  op: 'zlibSync',
  data: latin1Payload(new Uint8Array(6000).fill(0xff)),
  options: { level: 9 },
})
// 256 bytes of 0xFF + 1 byte of 0xF0 => a = 1 + 256*255 + 240 = 65521 (65521 % 65521 === 0 vs % 65536 === 65521).
const adlerModBytes = new Uint8Array(257)
adlerModBytes.fill(0xff)
adlerModBytes[256] = 0xf0
add('zlib-adler-mod-65521', 'zlibSync where final Adler-32 a = 65521 (kills % 65536 vs % 65521 mutant)', {
  op: 'zlibSync',
  data: latin1Payload(adlerModBytes),
})

// 6. decompressSync auto-detection (4)
const autoPlain = strToU8('Auto-detection across gzip, zlib, and raw deflate formats! '.repeat(10))
add('decompress-gzip', 'decompressSync auto-detects GZIP magic 1F 8B 08', {
  op: 'decompressSync',
  data: latin1Payload(gzipSync(autoPlain, { mtime: 0 })),
})
add('decompress-zlib', 'decompressSync auto-detects Zlib header (level 1: 78 5E, FCHECK % 31 == 0)', {
  op: 'decompressSync',
  data: latin1Payload(zlibSync(autoPlain, { level: 1 })),
})
add('decompress-raw-deflate', 'decompressSync auto-detects raw DEFLATE stream', {
  op: 'decompressSync',
  data: latin1Payload(deflateSync(autoPlain)),
})
add('decompress-raw-0x78-ambiguous', 'decompressSync falls through to inflateSync when first byte is 0x78 but (b0<<8|b1)%31 != 0', {
  op: 'decompressSync',
  data: bytesPayload(makeAmbiguous0x78Deflate()),
})

// 7. outSize option (5)
const outTarget = strToU8('Pre-allocated output buffer sizing check! '.repeat(8))
const outDeflated = deflateSync(outTarget)
add('inflate-empty-outsize', 'inflateSync on empty byte array with outSize returns full zeroed out buffer', {
  op: 'inflateSync',
  data: { bytes: [] },
  options: { outSize: 8 },
})
add('inflate-outsize-exact', 'inflateSync with exact outSize', {
  op: 'inflateSync',
  data: latin1Payload(outDeflated),
  options: { outSize: outTarget.length },
})
add('inflate-outsize-larger', 'inflateSync with outSize larger than output returns trimmed subarray', {
  op: 'inflateSync',
  data: latin1Payload(outDeflated),
  options: { outSize: outTarget.length + 128 },
})
add('inflate-outsize-smaller', 'inflateSync on Huffman stream with outSize smaller than output truncates cleanly', {
  op: 'inflateSync',
  data: latin1Payload(outDeflated),
  options: { outSize: 24 },
})
add('inflate-outsize-smaller-stored-throws', 'inflateSync on stored-block stream with too-small outSize throws (both-threw)', {
  op: 'inflateSync',
  data: latin1Payload(deflateSync(strToU8('stored block too large for outSize'), { level: 0 })),
  options: { outSize: 10 },
})

// 8. String codec: strToU8 & strFromU8 (5)
add('str-to-u8-utf8', 'strToU8 UTF-8 across ASCII, NUL, 2-byte, 3-byte, and 4-byte astral code points', {
  op: 'strToU8',
  text: 'ASCII\u0000éßΩ中—€🚀😀𝕳end',
})
const all256 = new Uint8Array(256)
for (let i = 0; i < 256; i++) all256[i] = i
add('str-to-u8-latin1', 'strToU8 latin1 across all 0x00..0xFF code units plus >0xFF low-byte truncation', {
  op: 'strToU8',
  text: `${strFromU8(all256, true)}\u0141\u1234`,
  latin1: true,
})
add('str-from-u8-utf8', 'strFromU8 UTF-8 with leading BOM stripped, embedded BOM kept, 2/3/4-byte astral pairs', {
  op: 'strFromU8',
  data: bytesPayload(concatBytes(
    new Uint8Array([0xef, 0xbb, 0xbf]),
    strToU8('Hi éßΩ中—€\ufeff🚀😀𝕳!'),
  )),
})
add('str-from-u8-utf8-invalid', 'strFromU8 UTF-8 on ill-formed byte sequences (WHATWG U+FFFD maximal-subpart replacement)', {
  op: 'strFromU8',
  data: {
    bytes: [
      0x41, 0xff, 0x80, 0xc0, 0xaf, 0xc2, 0xe0, 0x80, 0xaf,
      0xe0, 0xa0, 0xed, 0xa0, 0x80, 0xf0, 0x90, 0x80, 0xf4, 0x90, 0x80, 0x80, 0x42,
    ],
  },
})
add('str-from-u8-latin1', 'strFromU8 latin1 across all 256 byte values 0x00..0xFF', {
  op: 'strFromU8',
  data: bytesPayload(all256),
  latin1: true,
})

// 9. Streaming classes (12)
const bigStreamChunk = makeBenchText(0x55aa55aa, 12000)
// 136 KB chunk pushed after bigStreamChunk -> endLen (~181 KB) > 2 * 98304 - 32768 (163840),
// exercising Deflate's internal buffer growth (`new u8(endLen & -32768)`) as well as window wrap!
const growStreamChunk = makeBenchText(0xaa55aa55, 136000)
add('stream-deflate-flush-and-wrap', 'Deflate streaming: small pushes, flush(), flush(true), >8 KB auto-emit, >164 KB buffer grow + window wrap, final', {
  op: 'Deflate',
  options: { level: 6 },
  steps: [
    { action: 'push', data: { text: 'first small chunk ' } },
    { action: 'flush' },
    { action: 'push', data: { text: 'second chunk before sync flush! ' } },
    { action: 'flush', sync: true },
    { action: 'push', data: { text: bigStreamChunk } },
    { action: 'push', data: { text: growStreamChunk } },
    { action: 'push', data: { text: 'final tail' }, final: true },
  ],
})
// Small incompressible first chunk in streaming Deflate: bs starts at 0 (not wi=32768), so flen >> ftlen
// and fflate emits Huffman instead of stored block (kills dflt-bs-starts-at-wi mutant).
add('stream-deflate-incomp-first', 'Deflate streaming on 200-byte incompressible first chunk + flush (verifies bs = 0 start quirk)', {
  op: 'Deflate',
  options: { level: 6 },
  steps: [
    { action: 'push', data: latin1Payload(makeRandomBytes(0x99887766, 200)) },
    { action: 'flush' },
    { action: 'push', data: { text: 'tail' }, final: true },
  ],
})
add('stream-deflate-level0', 'Deflate streaming at level 0 across chunks and flush', {
  op: 'Deflate',
  options: { level: 0 },
  steps: [
    { action: 'push', data: { text: 'stored chunk one; ' } },
    { action: 'flush' },
    { action: 'push', data: { text: 'stored chunk two.' }, final: true },
  ],
})
add('stream-inflate-1byte-chunks', 'Inflate fed mixed stored/fixed/dynamic (BTYPE=2) stream 1 byte at a time plus empty chunk (splits dynamic header!)', {
  op: 'Inflate',
  steps: [
    { action: 'push', data: { bytes: [] } },
    ...Array.from(mixedMulti.stream, (b, idx) => ({
      action: 'push',
      data: { bytes: [b] },
      ...(idx === mixedMulti.stream.length - 1 ? { final: true } : {}),
    })),
  ],
})
const dynSplitSrc = deflateSync(strToU8(makeBenchText(0x76543210, 2400)), { level: 6 })
const dynSplitChunks = splitChunks(dynSplitSrc, [2, 3, 1, 5, 2, 7, 3])
add('stream-inflate-dynamic-splits', 'Inflate fed dynamic-Huffman stream in awkward 2/3/1/5/7-byte chunks splitting dynamic header and symbols', {
  op: 'Inflate',
  steps: dynSplitChunks.map((c, idx) => ({
    action: 'push',
    data: bytesPayload(c),
    ...(idx === dynSplitChunks.length - 1 ? { final: true } : {}),
  })),
})
add('stream-gzip-flush', 'Gzip streaming with mtime, filename, flush(), flush(true), and >8 KB chunk', {
  op: 'Gzip',
  options: { level: 6, mtime: 1700000000000, filename: 'stream.dat' },
  steps: [
    { action: 'push', data: { text: 'gzip stream part 1; ' } },
    { action: 'flush' },
    { action: 'push', data: { text: 'gzip stream part 2; ' } },
    { action: 'flush', sync: true },
    { action: 'push', data: { text: bigStreamChunk } },
    { action: 'push', data: { text: 'gzip stream end.' }, final: true },
  ],
})
// Use ~700 B mixed-entropy text for member 1 so Gunzip also splits a genuine BTYPE=2 dynamic header across chunks!
const gzMember1 = gzipSync(strToU8(makeBenchText(0x27182818, 700)), { mtime: 0, filename: 'part1.txt' })
const gzMember2 = gzipSync(strToU8('Second GZIP member payload (level 0)!'), { level: 0, mtime: 1700000000000 })
const multiGz = concatBytes(gzMember1, gzMember2)
const multiGzChunks = splitChunks(multiGz, [2, 5, 3, 1, 7, 4, 2, 11])
add('stream-gunzip-multimember', 'Gunzip streaming two concatenated GZIP members (BTYPE=2 dynamic + level-0 stored) split across headers, FNAME, dynamic trees, and trailers', {
  op: 'Gunzip',
  steps: multiGzChunks.map((c, idx) => ({
    action: 'push',
    data: bytesPayload(c),
    ...(idx === multiGzChunks.length - 1 ? { final: true } : {}),
  })),
})
add('stream-zlib-dict-flush', 'Zlib streaming with dictionary (exercises Deflate dictionary) and flush(true)', {
  op: 'Zlib',
  options: { level: 6, dictionary: { text: dictStr } },
  steps: [
    { action: 'push', data: { text: dictTargetStr.slice(0, 60) } },
    { action: 'flush', sync: true },
    { action: 'push', data: { text: dictTargetStr.slice(60) }, final: true },
  ],
})
const zlibDictBytes = zlibSync(strToU8(dictTargetStr), { level: 6, dictionary: strToU8(dictStr) })
const unzlibChunks = [
  zlibDictBytes.subarray(0, 3),
  zlibDictBytes.subarray(3, 10),
  zlibDictBytes.subarray(10, zlibDictBytes.length - 2),
  zlibDictBytes.subarray(zlibDictBytes.length - 2),
]
add('stream-unzlib-dict-chunks', 'Unzlib streaming with dictionary (exercises Inflate dictionary), <6-byte initial chunk, and split Adler-32 trailer', {
  op: 'Unzlib',
  options: { dictionary: { text: dictStr } },
  steps: unzlibChunks.map((c, idx) => ({
    action: 'push',
    data: bytesPayload(c),
    ...(idx === unzlibChunks.length - 1 ? { final: true } : {}),
  })),
})
const decGz = gzipSync(strToU8(makeBenchText(0xabcdef01, 700)), { mtime: 0 })
add('stream-decompress-gzip', 'Decompress streaming auto-detects GZIP (BTYPE=2 dynamic body) after leading empty chunk', {
  op: 'Decompress',
  steps: [
    { action: 'push', data: { bytes: [] } },
    { action: 'push', data: bytesPayload(decGz.subarray(0, 5)) },
    { action: 'push', data: bytesPayload(decGz.subarray(5, 20)) },
    { action: 'push', data: bytesPayload(decGz.subarray(20)), final: true },
  ],
})
const decZl = zlibSync(strToU8('Decompress streaming zlib test! '.repeat(6)))
add('stream-decompress-zlib', 'Decompress streaming auto-detects Zlib', {
  op: 'Decompress',
  steps: [
    { action: 'push', data: bytesPayload(decZl.subarray(0, 4)) },
    { action: 'push', data: bytesPayload(decZl.subarray(4)), final: true },
  ],
})
const decRaw = deflateSync(strToU8('Decompress streaming raw deflate test! '.repeat(6)))
add('stream-decompress-raw', 'Decompress streaming auto-detects raw DEFLATE', {
  op: 'Decompress',
  steps: [
    { action: 'push', data: bytesPayload(decRaw.subarray(0, 4)) },
    { action: 'push', data: bytesPayload(decRaw.subarray(4)), final: true },
  ],
})

// 10. Error codes 0..6 (expectErrorCode: true) + uncaught throws (15)
const truncDeflate = deflateSync(strToU8('truncated deflate stream payload for EOF check! '.repeat(4)))
add('err-code-0-unexpected-eof', 'FlateErrorCode.UnexpectedEOF (0) on truncated Huffman deflate stream', {
  op: 'inflateSync',
  data: bytesPayload(truncDeflate.subarray(0, truncDeflate.length - 5)),
  expectErrorCode: true,
})
add('err-code-0-stored-eof', 'FlateErrorCode.UnexpectedEOF (0) on truncated stored block', {
  op: 'inflateSync',
  data: { bytes: [0x01, 0x0a, 0x00, 0xf5, 0xff, 0x41, 0x42] },
  expectErrorCode: true,
})
add('err-code-1-invalid-block-type', 'FlateErrorCode.InvalidBlockType (1) on reserved block type 3 (0b11)', {
  op: 'inflateSync',
  data: { bytes: [0x07] },
  expectErrorCode: true,
})
add('err-code-2-invalid-litlen', 'FlateErrorCode.InvalidLengthLiteral (2) on unmapped dynamic lit/len Huffman code', {
  op: 'inflateSync',
  data: bytesPayload(makeInvalidLitLenStream()),
  expectErrorCode: true,
})
add('err-code-3-invalid-dist-code', 'FlateErrorCode.InvalidDistance (3) on unmapped dynamic distance Huffman code', {
  op: 'inflateSync',
  data: bytesPayload(makeInvalidDistCodeStream()),
  expectErrorCode: true,
})
add('err-code-3-invalid-dist-backref', 'FlateErrorCode.InvalidDistance (3) on back-reference distance exceeding output + dict', {
  op: 'inflateSync',
  data: bytesPayload(makeInvalidDistBackrefStream()),
  expectErrorCode: true,
})
add('err-code-4-stream-finished', 'FlateErrorCode.StreamFinished (4) on Inflate push after final=true', {
  op: 'Inflate',
  steps: [
    { action: 'push', data: bytesPayload(deflateSync(strToU8('done'))), final: true },
    { action: 'push', data: { bytes: [0] }, final: true },
  ],
  expectErrorCode: true,
})
add('err-code-5-no-stream-handler', 'FlateErrorCode.NoStreamHandler (5) on Deflate without ondata handler', {
  op: 'Deflate',
  noHandler: true,
  steps: [
    { action: 'push', data: { text: 'no callback' }, final: true },
  ],
  expectErrorCode: true,
})
add('err-code-6-invalid-gzip', 'FlateErrorCode.InvalidHeader (6) on gunzipSync with invalid magic bytes', {
  op: 'gunzipSync',
  data: { bytes: [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11] },
  expectErrorCode: true,
})
add('err-code-6-invalid-zlib', 'FlateErrorCode.InvalidHeader (6) on unzlibSync with failing FCHECK modulo 31', {
  op: 'unzlibSync',
  data: { bytes: [0x78, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01] },
  expectErrorCode: true,
})
add('err-code-6-zlib-need-dict', 'FlateErrorCode.InvalidHeader (6) on unzlibSync when FDICT=1 without dictionary option', {
  op: 'unzlibSync',
  data: bytesPayload(zlibSync(strToU8('needs dict'), { dictionary: strToU8('my-dict') })),
  expectErrorCode: true,
})
add('err-code-6-zlib-unexpected-dict', 'FlateErrorCode.InvalidHeader (6) on unzlibSync when FDICT=0 with dictionary option', {
  op: 'unzlibSync',
  data: bytesPayload(zlibSync(strToU8('no dict'))),
  options: { dictionary: { text: 'unexpected' } },
  expectErrorCode: true,
})
add('err-code-6-unzlib-short-trailer', 'FlateErrorCode.InvalidHeader (6) on Unzlib final push with fewer than 4 trailer bytes', {
  op: 'Unzlib',
  steps: [
    { action: 'push', data: { bytes: [0x78, 0x9c, 0x03, 0x00, 0x01] }, final: true },
  ],
  expectErrorCode: true,
})
add('err-uncaught-invalid-block', 'Uncaught FlateError on invalid block type (exercises harness both-threw parity)', {
  op: 'inflateSync',
  data: { bytes: [0x07] },
})
add('err-uncaught-invalid-gzip', 'Uncaught FlateError on invalid gzip data (exercises harness both-threw parity)', {
  op: 'gunzipSync',
  data: { bytes: [0xde, 0xad, 0xbe, 0xef] },
})

// 11. Output-buffer growth, GZIP member count, truncated FDICT Zlib, and a 1 MiB input (9)
const literalRun = makeLiteralRunDeflate(200000, false)
const literalRunBackref = makeLiteralRunDeflate(200000, true)
add('inflate-literal-run-200k', 'inflateSync on one dynamic block of 200,000 literals: buffer sl*3 grows once at block start, literals past its end are dropped (150,084 bytes)', {
  op: 'inflateSync',
  data: latin1Payload(literalRun),
})
add('decompress-literal-run-200k', 'decompressSync (raw DEFLATE path) on the same 200,000-literal block', {
  op: 'decompressSync',
  data: latin1Payload(literalRun),
})
// Split so the first push already runs past the 131,072-byte stream buffer: st.b overshoots the
// buffer, the emitted chunk and the kept 32 KiB window are clamped to it, and decoding resumes.
add('stream-inflate-literal-run-200k', 'Inflate on the 200,000-literal block in two pushes; the first runs past the 131,072-byte buffer mid-block', {
  op: 'Inflate',
  steps: [
    { action: 'push', data: latin1Payload(literalRun.subarray(0, 18000)) },
    { action: 'push', data: latin1Payload(literalRun.subarray(18000)), final: true },
  ],
})
add('inflate-literal-run-200k-backref', 'inflateSync on 200,000 literals then one length-3 distance-1 copy: the copy grows the buffer past bt, dropped literals read as zeros', {
  op: 'inflateSync',
  data: latin1Payload(literalRunBackref),
})
const gzip1000 = makeMultiMemberGzip(1000)
add('stream-gunzip-1000-members', 'Gunzip on 1,000 concatenated GZIP members in one final push (1,000 nested push calls; the parity-domain bound)', {
  op: 'Gunzip',
  steps: [{ action: 'push', data: latin1Payload(gzip1000), final: true }],
})
const salad1m = makeWordSaladText(1 << 20, 3)
add('zlib-1mb-l1-mem12', 'zlibSync level 1, mem 12 (plvl 24) on 1 MiB of word-salad text (1,119,186 UTF-8 bytes): many blocks, window wraps, Adler-32 over 1 MB', {
  op: 'zlibSync',
  data: { text: salad1m },
  options: { level: 1, mem: 12 },
})
add('unzlib-truncated-2byte-fdict', 'unzlibSync on truncated 2-byte FDICT=1 Zlib header [0x78, 0xBB] with dictionary (zls returns st=6 > len; subarray(6, -4) is empty)', {
  op: 'unzlibSync',
  data: { bytes: [0x78, 0xbb] },
  options: { dictionary: { text: 'a' } },
})
add('unzlib-truncated-2byte-fdict-outsize', 'unzlibSync on truncated 2-byte FDICT=1 Zlib header [0x78, 0xBB] with dictionary and outSize: 5 returns 5 zero bytes', {
  op: 'unzlibSync',
  data: { bytes: [0x78, 0xbb] },
  options: { dictionary: { text: 'a' }, outSize: 5 },
})
add('decompress-truncated-2byte-fdict', 'decompressSync on truncated 2-byte FDICT=1 Zlib header [0x78, 0xBB] with dictionary dispatches to unzlibSync and returns empty', {
  op: 'decompressSync',
  data: { bytes: [0x78, 0xbb] },
  options: { dictionary: { text: 'a' } },
})

// 12. Bench cases (2)
add('bench-compress', 'Benchmark: gzipSync (level 6, mtime 0) on ~64 KB realistic mixed-entropy JSON/HTML/log/prose text', {
  op: 'gzipSync',
  data: { text: benchCompressText },
  options: { level: 6, mtime: 0 },
})
const benchDecompressPlain = makeBenchText(0x9e3779b9, 131072) // ~128 KB realistic mixed-entropy text
const benchDecompressGz = gzipSync(strToU8(benchDecompressPlain), { level: 6, mtime: 0 })
add('bench-decompress', 'Benchmark: gunzipSync on ~128 KB realistic mixed-entropy text compressed at level 6', {
  op: 'gunzipSync',
  data: latin1Payload(benchDecompressGz),
})

// Write corpus.json
const corpus = { schema: 1, library: 'fflate', cases }
const outPath = resolve(here, 'corpus.json')
writeFileSync(outPath, `${JSON.stringify(corpus, null, 2)}\n`)
console.log(`Wrote ${cases.length} cases to ${outPath}`)

// ---------------------------------------------------------------------------------------------
// Optional --verify mode
if (process.argv.includes('--verify')) {
  const { run } = await import('./reference.mjs')
  const { variant } = await import('./bench-variant.mjs')

  let errors = 0
  let threwCount = 0
  const levelOutputs = new Map()
  const sha = (u8) => createHash('sha256').update(u8).digest('hex').slice(0, 12)

  for (const c of cases) {
    const inp = structuredClone(c.input)
    const before = JSON.stringify(inp)
    try {
      const out = run(inp)
      if (!(out instanceof Uint8Array)) {
        console.error(`FAIL ${c.id}: expected Uint8Array`)
        errors++
      }
      if (JSON.stringify(inp) !== before) {
        console.error(`FAIL ${c.id}: input was mutated!`)
        errors++
      }
      if (c.input.expectErrorCode) {
        const expectedCode = Number(c.id.match(/^err-code-(\d+)-/)?.[1])
        if (out.length !== 2 || out[0] !== 0xff || out[1] !== expectedCode) {
          console.error(`FAIL ${c.id}: expected [0xff, ${expectedCode}], got ${Array.from(out)}`)
          errors++
        } else {
          console.log(`  ok ${c.id} -> error code ${out[1]}`)
        }
      }
      if (c.id.startsWith('deflate-level-')) levelOutputs.set(c.id, out)
    } catch (e) {
      if (c.id.includes('throws') || c.id.startsWith('err-uncaught-')) {
        threwCount++
        console.log(`  ok ${c.id} -> threw (${e.name}: ${e.message})`)
      } else {
        console.error(`FAIL ${c.id}: unexpected throw: ${e.stack || e}`)
        errors++
      }
    }
  }

  // 1. Verify ALL 10 levels 0..9 produce strictly distinct outputs
  const levelHashes = new Set([...levelOutputs.values()].map(sha))
  const levelSummary = [...levelOutputs.entries()].map(([id, out]) => `${id.slice(-1)}:${out.length}`).join(' ')
  console.log(`Levels (${levelHashes.size}/10 distinct): ${levelSummary}`)
  if (levelHashes.size !== 10) { console.error('FAIL: not all 10 compression levels produced distinct outputs'); errors++ }

  // 2. Verify mem:0, default mem, and mem:8 are ALL strictly distinct
  const memBytes = strToU8(memText)
  const m0 = run(cases.find((c) => c.id === 'deflate-mem-0').input)
  const mDef = deflateSync(memBytes, { level: 1 })
  const m8 = run(cases.find((c) => c.id === 'deflate-mem-8').input)
  const memHashes = new Set([sha(m0), sha(mDef), sha(m8)])
  console.log(`mem variants (${memHashes.size}/3 distinct): mem0=${m0.length} def=${mDef.length} mem8=${m8.length}`)
  if (memHashes.size !== 3) { console.error('FAIL: mem:0, default, and mem:8 did not all differ'); errors++ }

  // 3. Verify block types: stored fallback, fixed Huffman, genuine BTYPE=2 dynamic in mixed & streaming cases
  const sfOut = run(cases.find((c) => c.id === 'deflate-stored-fallback').input)
  const fhOut = run(cases.find((c) => c.id === 'deflate-fixed-huffman').input)
  if (((sfOut[0] >> 1) & 3) !== 0) { console.error('FAIL: deflate-stored-fallback not BTYPE=0'); errors++ }
  if (((fhOut[0] >> 1) & 3) !== 1) { console.error('FAIL: deflate-fixed-huffman not BTYPE=1'); errors++ }
  if (((mixedMulti.dynBlock[0] >> 1) & 3) !== 2) { console.error('FAIL: makeMixedMultiblockDeflate dynBlock not BTYPE=2'); errors++ }
  if (((dynSplitSrc[0] >> 1) & 3) !== 2) { console.error('FAIL: stream-inflate-dynamic-splits not BTYPE=2'); errors++ }
  if (((gzMember1[10 + 'part1.txt'.length + 1] >> 1) & 3) !== 2) { console.error('FAIL: stream-gunzip-multimember member 1 not BTYPE=2'); errors++ }
  console.log(`dynamic BTYPE=2 verified: mixedMulti=2 dynSplit=2 gunzipM1=2`)

  // 4. Verify deflate-split-lc splits purely via lc > 7000 (input < 24576 and BFINAL_0 === 0)
  const lcOut = run(cases.find((c) => c.id === 'deflate-split-lc').input)
  if (strToU8(splitLcText).length >= 24576 || (lcOut[0] & 1) !== 0) {
    console.error(`FAIL: deflate-split-lc did not split via lc > 7000 (inLen=${strToU8(splitLcText).length}, BFINAL=${lcOut[0] & 1})`)
    errors++
  } else {
    console.log(`deflate-split-lc: inLen=${strToU8(splitLcText).length} (<24576), BFINAL_block0=${lcOut[0] & 1}, outLen=${lcOut.length}`)
  }

  // 5. Verify >32 KiB dictionary truncation (32768 vs 16384 vs un-truncated) AND full-dict Zlib Adler-32
  {
    const fullDict = strToU8(longDictStr)
    const target = strToU8(dictTargetStr)
    const d32k = run(cases.find((c) => c.id === 'deflate-with-dictionary').input)
    const d16k = deflateSync(target, { dictionary: fullDict.subarray(-16384) })
    // Place oldDictMarker inside the 32 KiB window (replacing filler at offset 1240, keeping insideDictMarker intact):
    // if oldDictMarker had NOT been truncated away, output would be strictly shorter!
    const withOldInside = new Uint8Array(fullDict.subarray(-32768))
    withOldInside.set(strToU8(oldDictMarker), 10)
    const dWithOld = deflateSync(target, { dictionary: withOldInside })
    if (sha(d32k) === sha(d16k)) { console.error('FAIL: deflate-with-dictionary does not distinguish -32768 from -16384'); errors++ }
    if (sha(d32k) === sha(dWithOld) || dWithOld.length >= d32k.length) { console.error('FAIL: deflate-with-dictionary does not prove old marker was truncated away'); errors++ }
    const zlFull = run(cases.find((c) => c.id === 'zlib-with-dictionary').input)
    const zlTrunc = zlibSync(target, { level: 0, dictionary: fullDict.subarray(-32768) })
    const dictIdEq = [2, 3, 4, 5].every((idx) => zlFull[idx] === zlTrunc[idx])
    if (dictIdEq) { console.error('FAIL: zlib-with-dictionary DICTID does not distinguish full vs -32768 dictionary'); errors++ }
    console.log(`dictionary >32K verified: d32k=${d32k.length}B vs d16k=${d16k.length}B vs withOldInside=${dWithOld.length}B; zlib DICTID full!=trunc`)
  }

  // 6. Verify zlib-adler-mod-65521 hits a = 65521 -> trailer ends with 00 00 (not FF F1)
  {
    const zMod = run(cases.find((c) => c.id === 'zlib-adler-mod-65521').input)
    const t = Array.from(zMod.subarray(-4))
    if (t[2] !== 0 || t[3] !== 0) { console.error(`FAIL: zlib-adler-mod-65521 trailer=${t}`); errors++ }
    else console.log(`zlib-adler-mod-65521: trailer=${t.map((b) => b.toString(16).padStart(2, '0')).join(' ')}`)
  }

  // 7. Verify stream-deflate-incomp-first emits non-stored block on first 200B random chunk (bs = 0 quirk)
  {
    const incFramed = run(cases.find((c) => c.id === 'stream-deflate-incomp-first').input)
    const firstBtype = (incFramed[5] >> 1) & 3
    if (firstBtype === 0) { console.error('FAIL: stream-deflate-incomp-first emitted stored block'); errors++ }
    else console.log(`stream-deflate-incomp-first: first chunk BTYPE=${firstBtype} (non-stored)`)
  }

  // 8. Verify stream-deflate-flush-and-wrap triggers endLen > 2 * 98304 - 32768 (163840)
  {
    const growBytes = strToU8(growStreamChunk).length
    if (32768 + strToU8(bigStreamChunk).length + growBytes <= 2 * 98304 - 32768) {
      console.error('FAIL: stream-deflate-flush-and-wrap does not exceed 2*98304 - 32768')
      errors++
    } else {
      console.log(`stream-deflate-flush-and-wrap: grow chunk=${growBytes}B triggers buffer growth (>163840)`)
    }
  }

  // 9. Verify deflate-huffman-15bit-clamp genuinely hits 15-bit code length in emitted dynamic header
  {
    const comp = run(cases.find((c) => c.id === 'deflate-huffman-15bit-clamp').input)
    const bits = (p, m) => ((comp[p >> 3] | (comp[(p >> 3) + 1] << 8)) >> (p & 7)) & m
    let pos = 3
    const hLit = bits(pos, 31) + 257, hDist = bits(pos + 5, 31) + 1, hcLen = bits(pos + 10, 15) + 4
    pos += 14
    const clim = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15]
    const clt = new Uint8Array(19)
    for (let i = 0; i < hcLen; i++) clt[clim[i]] = bits(pos + i * 3, 7)
    pos += hcLen * 3
    const clb = Math.max(...clt)
    const l = new Uint16Array(clb + 1), le = new Uint16Array(clb + 1)
    for (const v of clt) if (v) l[v]++
    for (let i = 1; i <= clb; i++) le[i] = (le[i - 1] + l[i - 1]) << 1
    const dec = new Map()
    for (let i = 0; i < 19; i++) if (clt[i]) dec.set((clt[i] << 8) | le[clt[i]]++, i)
    const readCl = () => {
      let code = 0
      for (let len = 1; len <= clb; len++) {
        code = (code << 1) | bits(pos++, 1)
        const sym = dec.get((len << 8) | code)
        if (sym !== undefined) return sym
      }
      throw new Error('bad cl')
    }
    const ldt = new Uint8Array(hLit + hDist)
    for (let i = 0; i < ldt.length;) {
      const s = readCl()
      if (s < 16) ldt[i++] = s
      else {
        let c = 0, n = 0
        if (s === 16) { n = 3 + bits(pos, 3); pos += 2; c = ldt[i - 1] }
        else if (s === 17) { n = 3 + bits(pos, 7); pos += 3 }
        else { n = 11 + bits(pos, 127); pos += 7 }
        while (n--) ldt[i++] = c
      }
    }
    const lt = ldt.subarray(0, hLit), dt = ldt.subarray(hLit)
    const maxLit = Math.max(...lt), count15 = lt.filter((x) => x === 15).length, maxDist = Math.max(...dt)
    console.log(`deflate-huffman-15bit-clamp: compLen=${comp.length} maxLitLen=${maxLit} count15=${count15} hDist=${hDist} maxDistLen=${maxDist}`)
    if (maxLit !== 15 || count15 < 2) { console.error('FAIL: deflate-huffman-15bit-clamp did not clamp to 15 bits'); errors++ }
    const back = inflateSync(comp)
    const raw = strToU8(clampText)
    if (back.length !== raw.length || back.some((v, i) => v !== raw[i])) {
      console.error('FAIL: deflate-huffman-15bit-clamp roundtrip mismatch')
      errors++
    }
  }

  // 10. Verify dist-32768 roundtrip
  const d32k = run(cases.find((c) => c.id === 'inflate-max-distance-32768').input)
  if (d32k.length !== 32771 || d32k[32768] !== 0x5a || d32k[32769] !== 0x59 || d32k[32770] !== 0x58) {
    console.error('FAIL: inflate-max-distance-32768 wrong output')
    errors++
  }

  // 11. Verify the buffer-growth cases drop literals, and the shape of the other section-11 cases
  {
    const out = (id) => run(cases.find((c) => c.id === id).input)
    const frames = (u8) => {
      const fs = []
      for (let o = 0; o < u8.length; o += 5 + fs[fs.length - 1].len) {
        fs.push({ len: (u8[o] | (u8[o + 1] << 8) | (u8[o + 2] << 16) | (u8[o + 3] << 24)) >>> 0, final: u8[o + 4] })
      }
      return fs
    }
    // inflateSync: buf = sl * 3, grown at block start to max(2 * sl * 3, bt + 131072)
    const lit = out('inflate-literal-run-200k')
    const litCap = Math.max(6 * literalRun.length, 131072)
    if (lit.length !== litCap || litCap >= 200000 || !lit.every((b) => b === 97)) {
      console.error(`FAIL: inflate-literal-run-200k length ${lit.length}, expected ${litCap} 'a' bytes (< 200000)`)
      errors++
    }
    if (sha(out('decompress-literal-run-200k')) !== sha(lit)) { console.error('FAIL: decompress-literal-run-200k differs from inflateSync'); errors++ }
    const br = out('inflate-literal-run-200k-backref')
    const brCap = Math.max(6 * literalRunBackref.length, 131072)
    if (br.length !== 200003 || !br.subarray(0, brCap).every((b) => b === 97) || !br.subarray(brCap).every((b) => b === 0)) {
      console.error(`FAIL: inflate-literal-run-200k-backref expected ${brCap} 'a' then ${200003 - brCap} zeros, got length ${br.length}`)
      errors++
    }
    const st = frames(out('stream-inflate-literal-run-200k'))
    const stTotal = st.reduce((a, f) => a + f.len, 0)
    if (st.length !== 2 || st[0].len !== 131072 || st[0].final !== 0 || st[1].final !== 1 || stTotal >= 200000) {
      console.error(`FAIL: stream-inflate-literal-run-200k frames ${JSON.stringify(st)}`)
      errors++
    }
    const gz = frames(out('stream-gunzip-1000-members'))
    if (gz.length !== 1002 || gz.slice(0, -1).some((f) => f.final !== 0) || gz[1001].final !== 1) {
      console.error(`FAIL: stream-gunzip-1000-members emitted ${gz.length} chunks (expected 1,000 members + (empty, false) + (empty, true))`)
      errors++
    }
    const saladBytes = strToU8(salad1m).length
    if (saladBytes < 1000000 || !salad1m.isWellFormed()) { console.error(`FAIL: zlib-1mb-l1-mem12 input ${saladBytes} B, well-formed ${salad1m.isWellFormed()}`); errors++ }
    const fdict0 = out('unzlib-truncated-2byte-fdict')
    const fdict5 = out('unzlib-truncated-2byte-fdict-outsize')
    const fdictDec = out('decompress-truncated-2byte-fdict')
    if (fdict0.length !== 0 || fdictDec.length !== 0 || fdict5.length !== 5 || !fdict5.every((b) => b === 0)) {
      console.error(`FAIL: truncated 2-byte FDICT cases gave lengths ${fdict0.length}, ${fdict5.length}, ${fdictDec.length}`)
      errors++
    }
    console.log(`buffer growth: inflate ${lit.length}B (cap ${litCap}), backref ${br.length}B (${brCap} 'a'), stream frames ${st.map((f) => f.len).join('+')}; gunzip members -> ${gz.length} chunks; salad ${saladBytes}B; fdict2 -> ${fdict0.length}/${fdict5.length}/${fdictDec.length}B`)
  }

  // 12. Verify bench cases, ratios, timings, and bench-variant distinctness & equivalence
  for (const id of ['bench-compress', 'bench-decompress']) {
    const c = cases.find((x) => x.id === id)
    const baseOut = run(c.input)
    for (let i = 0; i < 5; i++) run(c.input)
    const t0 = performance.now()
    const iters = 20
    for (let i = 0; i < iters; i++) run(c.input)
    const ms = (performance.now() - t0) / iters

    const vt0 = performance.now()
    const seen = new Set([sha(baseOut)])
    for (const k of [1, 2, 3, 77]) {
      const v = variant(c.input, k)
      const vOut = run(v)
      const h = sha(vOut)
      if (seen.has(h)) { console.error(`FAIL ${id}: duplicate variant output digest for k=${k}`); errors++ }
      seen.add(h)
      if (id === 'bench-decompress' && vOut.length !== baseOut.length) {
        console.error(`FAIL bench-decompress variant ${k}: length changed ${vOut.length} vs ${baseOut.length}`)
        errors++
      }
      if (id === 'bench-compress' && Math.abs(vOut.length - baseOut.length) / baseOut.length > 0.02) {
        console.error(`FAIL bench-compress variant ${k}: size drifted ${vOut.length} vs ${baseOut.length}`)
        errors++
      }
    }
    const vMs = (performance.now() - vt0) / 4
    if (id === 'bench-compress') {
      const inBytes = strToU8(c.input.data.text).length
      console.log(`bench-compress: in=${inBytes}B out=${baseOut.length}B ratio=${(inBytes / baseOut.length).toFixed(2)}x ref=${ms.toFixed(2)}ms variantGen=${vMs.toFixed(2)}ms`)
    } else {
      const inBytes = c.input.data.text.length
      console.log(`bench-decompress: in=${inBytes}B out=${baseOut.length}B ratio=${(baseOut.length / inBytes).toFixed(2)}x ref=${ms.toFixed(2)}ms variantGen=${vMs.toFixed(2)}ms`)
    }
  }

  console.log(`Verified ${cases.length} cases (${threwCount} uncaught throws), errors=${errors}`)
  if (errors > 0) process.exit(1)
}
