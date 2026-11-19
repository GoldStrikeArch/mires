// Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
// libraries/pilot/CONTRACT.md.

type entry = {word: string, mutable count: int, length: int}

let isWordCodePoint = cp => cp >= 0x80 || (cp >= 0x30 && cp <= 0x39) || (cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a)

// FNV-1a 32 over the UTF-8 encoding of the code points of `text`. ReScript's `int` is a signed
// 32-bit value whose `*` compiles to `(a * b) | 0`, which loses bits once the exact product passes
// 2^53, so the multiply is Math.imul. The result is the signed 32-bit hash.
let fnv1a = text => {
  let hash = ref(0x811c9dc5)
  let add = byte => hash := Math.Int.imul(Int.bitwiseXor(hash.contents, byte), 0x01000193)
  let index = ref(0)
  let length = String.length(text)
  while index.contents < length {
    let cp = String.codePointAt(text, index.contents)->Option.getOr(0)
    if cp < 0x80 {
      add(cp)
    } else if cp < 0x800 {
      add(Int.bitwiseOr(0xc0, Int.shiftRight(cp, 6)))
      add(Int.bitwiseOr(0x80, Int.bitwiseAnd(cp, 0x3f)))
    } else if cp < 0x10000 {
      add(Int.bitwiseOr(0xe0, Int.shiftRight(cp, 12)))
      add(Int.bitwiseOr(0x80, Int.bitwiseAnd(Int.shiftRight(cp, 6), 0x3f)))
      add(Int.bitwiseOr(0x80, Int.bitwiseAnd(cp, 0x3f)))
    } else {
      add(Int.bitwiseOr(0xf0, Int.shiftRight(cp, 18)))
      add(Int.bitwiseOr(0x80, Int.bitwiseAnd(Int.shiftRight(cp, 12), 0x3f)))
      add(Int.bitwiseOr(0x80, Int.bitwiseAnd(Int.shiftRight(cp, 6), 0x3f)))
      add(Int.bitwiseOr(0x80, Int.bitwiseAnd(cp, 0x3f)))
    }
    index := index.contents + (cp > 0xffff ? 2 : 1)
  }
  hash.contents
}

// The unsigned (`>>> 0`) reading of a signed 32-bit hash, as 8 hex digits. `int` cannot hold it
// (the compiler folds `Int.shiftRightUnsigned(h, 0)` to `h`), so it goes through a float.
let hex8 = hash => {
  let unsigned = hash < 0 ? Int.toFloat(hash) +. 4294967296.0 : Int.toFloat(hash)
  String.padStart(Float.toString(unsigned, ~radix=16), 8, "0")
}

let analyse = (text, top) => {
  let entries: Map.t<string, entry> = Map.make()
  let tokens = ref(0)
  let codePoints = ref(0)
  let word = ref([])
  let flush = () => {
    let length = Array.length(word.contents)
    if length > 0 {
      tokens := tokens.contents + 1
      codePoints := codePoints.contents + length
      let key = String.fromCodePointMany(word.contents)
      switch Map.get(entries, key) {
      | Some(entry) => entry.count = entry.count + 1
      | None => Map.set(entries, key, {word: key, count: 1, length})
      }
      word := []
    }
  }
  let index = ref(0)
  let length = String.length(text)
  while index.contents < length {
    let cp = String.codePointAt(text, index.contents)->Option.getOr(0)
    if isWordCodePoint(cp) {
      // ASCII-only case folding.
      Array.push(word.contents, cp >= 0x41 && cp <= 0x5a ? cp + 0x20 : cp)
    } else {
      flush()
    }
    index := index.contents + (cp > 0xffff ? 2 : 1)
  }
  flush()

  // Map iteration is first-occurrence order and toSorted is stable.
  let ranked =
    Map.values(entries)
    ->Array.fromIterator
    ->Array.toSorted((a, b) =>
      b.count != a.count ? Int.toFloat(b.count - a.count) : Int.toFloat(b.length - a.length)
    )
  let mean = tokens.contents == 0 ? 0.0 : Int.toFloat(codePoints.contents) /. Int.toFloat(tokens.contents)
  let lines = [
    `tokens ${Int.toString(tokens.contents)}`,
    `distinct ${Int.toString(Map.size(entries))}`,
    `mean ${Float.toFixed(mean, ~digits=3)}`,
    `fnv ${hex8(fnv1a(text))}`,
  ]
  ranked
  ->Array.slice(~start=0, ~end=top)
  ->Array.forEach(entry => {
    let share = Int.toFloat(entry.count) /. Int.toFloat(tokens.contents)
    Array.push(
      lines,
      `${entry.word} ${Int.toString(entry.count)} ${hex8(fnv1a(entry.word))} ${Float.toFixed(share, ~digits=3)}`,
    )
  })
  Array.join(lines, "\n")
}
