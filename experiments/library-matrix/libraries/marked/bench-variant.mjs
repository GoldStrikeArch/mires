// Bench variant for marked (registry `bench.variant`; README "Adding a library", step 5).
// Browser-safe: zero imports, deterministic pure function of (input, k).
//
// The k-th variant (k >= 1) applies a k-seeded case-preserving bijection on ASCII letters
// (`a`-`z` and `A`-`Z`, same permutation for both cases) across `input.markdown`, while keeping
// syntactic keywords untouched so the parse tree, token counts, HTML tags, URL schemes, task
// checkboxes, and entities remain structurally identical:
//
// - HTML tags / comments / PIs / declarations / CDATA / angle-bracket autolinks: `<...>` spans
//   (when `<` is not followed by space or newline, up to matching `>`).
// - HTML entity & numeric character references: `&[A-Za-z0-9#]+;`.
// - GFM task list checkboxes: `[x]` and `[X]`.
// - GFM autolink scheme prefixes: `http://`, `https://`, `ftp://` (case-insensitive), `www.`,
//   `mailto:`, and `xmpp:` (the domain, path, query, and email address after the prefix ARE
//   permuted, preserving valid URL/email syntax and exact byte length).
//
// Because the bijection is case-preserving and global across the entire document, link reference
// definitions `[label]: url` and all references `[text][LABEL]` / `[label]` map to the exact same
// folded key and continue to resolve identically.
//
// Only `parse` and `parseInline` with string `markdown` are accepted; anything else throws.

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

function matchSchemeAt(text, i) {
  const c0 = text.charCodeAt(i)
  const l0 = c0 | 32
  if (l0 === 104) { // h -> http:// or https://
    const sub = text.slice(i, i + 8).toLowerCase()
    if (sub === 'https://') return 8
    if (sub.startsWith('http://')) return 7
  } else if (l0 === 102) { // f -> ftp://
    if (text.slice(i, i + 6).toLowerCase() === 'ftp://') return 6
  } else if (c0 === 119) { // w -> www.
    if (text.startsWith('www.', i)) return 4
  } else if (c0 === 109) { // m -> mailto:
    if (text.startsWith('mailto:', i)) return 7
  } else if (c0 === 120) { // x -> xmpp:
    if (text.startsWith('xmpp:', i)) return 5
  }
  return 0
}

function isEntityChar(c) {
  return (c >= 48 && c <= 57) // 0-9
    || (c >= 65 && c <= 90)   // A-Z
    || (c >= 97 && c <= 122)  // a-z
    || c === 35               // #
    || c === 95               // _
}

function permuteMarkdown(text, table) {
  const len = text.length
  const codes = new Uint16Array(len)
  let i = 0
  while (i < len) {
    const c = text.charCodeAt(i)

    // Preserve HTML tags / comments / PIs / declarations / CDATA / angle-bracket autolinks: `<...>`
    if (c === 60 && i + 1 < len) { // '<'
      const next = text.charCodeAt(i + 1)
      if (next !== 32 && next !== 9 && next !== 10 && next !== 13) {
        let end = -1
        if (text.startsWith('<!--', i)) {
          const close = text.indexOf('-->', i + 4)
          end = close !== -1 ? close + 3 : -1
        } else if (text.startsWith('<![CDATA[', i)) {
          const close = text.indexOf(']]>', i + 9)
          end = close !== -1 ? close + 3 : -1
        } else {
          const close = text.indexOf('>', i + 1)
          const nl = text.indexOf('\n', i + 1)
          if (close !== -1 && (nl === -1 || close < nl)) end = close + 1
        }
        if (end !== -1) {
          for (let j = i; j < end; j++) codes[j] = text.charCodeAt(j)
          i = end
          continue
        }
      }
    }

    // Preserve HTML entities and numeric character references: `&...;`
    if (c === 38 && i + 2 < len) { // '&'
      let j = i + 1
      while (j < len && j - i <= 32 && isEntityChar(text.charCodeAt(j))) j++
      if (j > i + 1 && j < len && text.charCodeAt(j) === 59) { // ';'
        for (let m = i; m <= j; m++) codes[m] = text.charCodeAt(m)
        i = j + 1
        continue
      }
    }

    // Preserve GFM task list checkboxes: `[x]` or `[X]`
    if (c === 91 && i + 2 < len && text.charCodeAt(i + 2) === 93) { // '[' ... ']'
      const mid = text.charCodeAt(i + 1)
      if (mid === 120 || mid === 88) { // 'x' or 'X'
        codes[i] = 91
        codes[i + 1] = mid
        codes[i + 2] = 93
        i += 3
        continue
      }
    }

    // Preserve URL / email autolink scheme prefixes (only when not preceded by ASCII alphanumeric)
    const prev = i > 0 ? text.charCodeAt(i - 1) : 0
    const prevAlnum = (prev >= 48 && prev <= 57) || (prev >= 65 && prev <= 90) || (prev >= 97 && prev <= 122)
    if (!prevAlnum) {
      const schemeLen = matchSchemeAt(text, i)
      if (schemeLen > 0) {
        for (let j = 0; j < schemeLen; j++) codes[i + j] = text.charCodeAt(i + j)
        i += schemeLen
        continue
      }
    }

    codes[i] = c < 128 ? table[c] : c
    i++
  }

  let out = ''
  for (let off = 0; off < len; off += 16384) {
    out += String.fromCharCode.apply(null, codes.subarray(off, Math.min(len, off + 16384)))
  }
  return out
}

export function variant(input, k) {
  if (!Number.isInteger(k) || k < 1) {
    throw new Error(`bench variant: k must be an integer >= 1 (got ${k})`)
  }
  if (!input || (input.op !== 'parse' && input.op !== 'parseInline') || typeof input.markdown !== 'string') {
    throw new Error(`bench variant: unexpected input op=${input?.op}`)
  }
  const table = letterTable(k)
  return {
    ...input,
    markdown: permuteMarkdown(input.markdown, table),
  }
}
