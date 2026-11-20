// Bench variant for fflate (registry `bench.variant`; README "Adding a library", step 5).
// Browser-safe: relative import of fflate only (no Node imports).
//
// The k-th variant (k >= 1) applies a k-seeded bijection on ASCII letters (`a`-`z` and `A`-`Z`,
// same permutation for both cases):
//
// - `bench-compress` (`gzipSync`): permutes every ASCII letter in `input.data.text`. Content
//   changes throughout, while preserving exact byte length, UTF-8 validity, match lengths and
//   distances, symbol frequencies, and Huffman tree depths.
// - `bench-decompress` (`gunzipSync`): decompresses the base input once (cached per base text
//   string inside the variant generator), applies the k-seeded ASCII letter bijection to a fresh
//   copy of the UTF-8 plaintext, re-compresses with `gzipSync(..., { level: 6, mtime: 0 })`, and
//   encodes to Latin-1 via `strFromU8(..., true)`. Every k yields a distinct valid GZIP stream
//   whose blocks, Huffman tables, literals, and CRC-32 all differ while keeping decompression work
//   equivalent (same uncompressed byte length, same symbol entropy and match structure).
//
// Only the bench cases' operations (`gzipSync` with UTF-8 text, `gunzipSync` with Latin-1 text)
// are handled; anything else throws rather than returning an unvaried input.
import { gunzipSync, gzipSync, strFromU8, strToU8 } from '../../../../node_modules/fflate/esm/browser.js'

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

function letterTable(k) {
  const random = mulberry32((0x9e3779b9 ^ Math.imul(k, 0x85ebca6b)) >>> 0)
  const lower = new Uint8Array(26)
  for (let i = 0; i < 26; i++) lower[i] = 97 + i
  for (let i = 25; i > 0; i--) {
    const j = Math.floor(random() * (i + 1))
    const tmp = lower[i]
    lower[i] = lower[j]
    lower[j] = tmp
  }
  const table = new Uint8Array(128)
  for (let i = 0; i < 128; i++) table[i] = i
  for (let i = 0; i < 26; i++) {
    table[97 + i] = lower[i]
    table[65 + i] = lower[i] - 32
  }
  return table
}

function permuteString(text, table) {
  const len = text.length
  const codes = new Uint16Array(len)
  for (let i = 0; i < len; i++) {
    const c = text.charCodeAt(i)
    codes[i] = c < 128 ? table[c] : c
  }
  let out = ''
  for (let i = 0; i < len; i += 16384) {
    out += String.fromCharCode.apply(null, codes.subarray(i, Math.min(len, i + 16384)))
  }
  return out
}

// Single-entry cache for the base plaintext of `bench-decompress` so variant generation does not
// repeat the constant gunzip of the base input on every k.
let cachedKey = null
let cachedBase = null

function basePlaintext(latin1Gzip) {
  if (cachedKey === latin1Gzip) return cachedBase
  cachedBase = gunzipSync(strToU8(latin1Gzip, true))
  cachedKey = latin1Gzip
  return cachedBase
}

export function variant(input, k) {
  if (!Number.isInteger(k) || k < 1) {
    throw new Error(`bench variant: k must be an integer >= 1 (got ${k})`)
  }
  const table = letterTable(k)
  if (input.op === 'gzipSync' && input.data && typeof input.data.text === 'string' && !input.data.latin1) {
    return {
      ...input,
      data: { text: permuteString(input.data.text, table) },
    }
  }
  if (input.op === 'gunzipSync' && input.data && typeof input.data.text === 'string' && input.data.latin1 === true) {
    const base = basePlaintext(input.data.text)
    const permuted = new Uint8Array(base.length)
    for (let i = 0; i < base.length; i++) {
      const b = base[i]
      permuted[i] = b < 128 ? table[b] : b
    }
    const gz = gzipSync(permuted, { level: 6, mtime: 0 })
    return {
      ...input,
      data: { text: strFromU8(gz, true), latin1: true },
    }
  }
  throw new Error(`bench variant: unexpected operation ${input?.op}`)
}
