// pilot-wordfreq 1.0.0: a small handwritten JavaScript module that stands in for an npm package,
// so the library-matrix harness can be proven end to end on every toolchain before real library
// ports exist. It is not published anywhere; ../reference.mjs is its adapter, exactly as an npm
// library's adapter imports the package's published ESM entry. The behaviour is specified in
// ../CONTRACT.md, and the ports in ports/pilot/<language>/ reproduce it.
//
// analyse(text, { top }) returns one canonical string:
//
//   tokens <number of tokens>
//   distinct <number of distinct tokens>
//   mean <mean token length in code points, toFixed(3)>
//   fnv <FNV-1a 32 of the UTF-8 bytes of text, 8 lowercase hex digits>
//   <word> <count> <FNV-1a 32 of the word's UTF-8 bytes> <count / tokens, toFixed(3)>   (top rows)

const FNV_OFFSET = 0x811c9dc5
const FNV_PRIME = 0x01000193

// A word character is an ASCII letter or digit, or any code point at or above U+0080.
function isWordCodePoint(cp) {
  return cp >= 0x80 || (cp >= 0x30 && cp <= 0x39) || (cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a)
}

// FNV-1a over the UTF-8 encoding of `text`, with JavaScript's `>>> 0` unsigned 32-bit wrap.
export function fnv1a(text) {
  let hash = FNV_OFFSET
  const add = (byte) => {
    hash = Math.imul(hash ^ byte, FNV_PRIME) >>> 0
  }
  for (const char of text) {
    const cp = char.codePointAt(0)
    if (cp < 0x80) {
      add(cp)
    } else if (cp < 0x800) {
      add(0xc0 | (cp >> 6))
      add(0x80 | (cp & 0x3f))
    } else if (cp < 0x10000) {
      add(0xe0 | (cp >> 12))
      add(0x80 | ((cp >> 6) & 0x3f))
      add(0x80 | (cp & 0x3f))
    } else {
      add(0xf0 | (cp >> 18))
      add(0x80 | ((cp >> 12) & 0x3f))
      add(0x80 | ((cp >> 6) & 0x3f))
      add(0x80 | (cp & 0x3f))
    }
  }
  return hash
}

const hex8 = (value) => value.toString(16).padStart(8, '0')

export function analyse(text, { top }) {
  const entries = new Map()
  let tokens = 0
  let codePoints = 0
  let word = ''
  let length = 0
  const flush = () => {
    if (length === 0) return
    tokens++
    codePoints += length
    const entry = entries.get(word)
    if (entry) entry.count++
    else entries.set(word, { word, count: 1, length })
    word = ''
    length = 0
  }
  for (const char of text) {
    const cp = char.codePointAt(0)
    if (!isWordCodePoint(cp)) {
      flush()
      continue
    }
    // ASCII-only case folding: A-Z become a-z, every other code point is kept as it is.
    word += cp >= 0x41 && cp <= 0x5a ? String.fromCharCode(cp + 0x20) : char
    length++
  }
  flush()

  // Map iteration is first-occurrence order and Array.prototype.sort is stable, so equal
  // (count, length) keys keep the order in which their words first appeared.
  const ranked = [...entries.values()].sort((a, b) => b.count - a.count || b.length - a.length)
  const mean = tokens === 0 ? 0 : codePoints / tokens
  const lines = [
    `tokens ${tokens}`,
    `distinct ${entries.size}`,
    `mean ${mean.toFixed(3)}`,
    `fnv ${hex8(fnv1a(text))}`,
  ]
  for (const entry of ranked.slice(0, top)) {
    lines.push(`${entry.word} ${entry.count} ${hex8(fnv1a(entry.word))} ${(entry.count / tokens).toFixed(3)}`)
  }
  return lines.join('\n')
}
