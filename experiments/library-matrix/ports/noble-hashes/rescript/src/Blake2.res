// noble's blake2.ts and _blake.ts: BLAKE2b and BLAKE2s on noble's shared _BLAKE2 base.
//
// BLAKE2b keeps noble's G1b/G2b on 64-bit words as low and high halves in a 32-word work vector
// (noble's BBUF); BLAKE2s keeps G1s/G2s on 32-bit words in a 16-word work vector, where noble
// passes and returns them as locals. The two do not share a mix function.
open U64

// Message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and 11 repeat 0 and 1.
let bsigma = [
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
  14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
  11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4,
  7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
  9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13,
  2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
  12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11,
  13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
  6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5,
  10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
  14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
]

// noble's Blake2Opts: the options reference.mjs passes to every hash.
type opts = {
  dkLen: option<float>,
  key: option<array<int>>,
  salt: option<array<int>>,
  personalization: option<array<int>>,
}

// noble's checkBlake2Opts: keyLen bounds both the digest and the key length.
let checkBlake2Opts = (outputLen, opts, ~keyLen, ~saltLen, ~persLen) => {
  let fail = message => JsError.throwWithMessage(message)
  if outputLen <= 0 || outputLen > keyLen {
    fail(`"dkLen" must be 1..${Int.toString(keyLen)}, got ${Int.toString(outputLen)}`)
  }
  switch opts.key {
  | Some(key) if Array.length(key) < 1 || Array.length(key) > keyLen =>
    fail(`"key" expected to be undefined or of length=1..${Int.toString(keyLen)}`)
  | _ => ()
  }
  switch opts.salt {
  | Some(salt) if Array.length(salt) != saltLen => fail(`"salt" expected Uint8Array of length ${Int.toString(saltLen)}`)
  | _ => ()
  }
  switch opts.personalization {
  | Some(pers) if Array.length(pers) != persLen =>
    fail(`"personalization" expected Uint8Array of length ${Int.toString(persLen)}`)
  | _ => ()
  }
}

// noble's _BLAKE2: the final block is compressed with a flag, so a full buffer waits for more
// input. `compress` reads the counter from `length`; `get` returns the state as little-endian
// 32-bit words.
type t = {
  blockLen: int,
  outputLen: int,
  buffer: array<int>,
  mutable length: float,
  mutable pos: int,
  compress: (array<int>, int, float, bool) => unit,
  get: unit => array<int>,
}

let update = (h, data) => {
  let len = Array.length(data)
  let pos = ref(0)
  while pos.contents < len {
    if h.pos == h.blockLen {
      h.compress(h.buffer, 0, h.length, false)
      h.pos = 0
    }
    let take = Math.Int.min(h.blockLen - h.pos, len - pos.contents)
    if take == h.blockLen && pos.contents + take < len {
      // Whole blocks that are not the last of the input are compressed straight from it.
      while pos.contents + h.blockLen < len {
        h.length = h.length +. Int.toFloat(h.blockLen)
        h.compress(data, pos.contents, h.length, false)
        pos := pos.contents + h.blockLen
      }
    } else {
      for i in 0 to take - 1 {
        h.buffer->Array.setUnsafe(h.pos + i, data->Array.getUnsafe(pos.contents + i))
      }
      h.pos = h.pos + take
      h.length = h.length +. Int.toFloat(take)
      pos := pos.contents + take
    }
  }
}

let digest = h => {
  h.buffer->Array.fill(0, ~start=h.pos)
  h.compress(h.buffer, 0, h.length, true)
  Utils.wordsToBytesLE(h.get(), h.outputLen)
}

// The base's constructor; noble's subclass constructors end by absorbing the zero-padded key as
// the first block.
let make = (~blockLen, ~outputLen, ~key, ~compress, ~get): Hash.t => {
  let h = {
    blockLen,
    outputLen,
    buffer: Array.make(~length=blockLen, 0),
    length: 0.,
    pos: 0,
    compress,
    get,
  }
  switch key {
  | Some(key) =>
    let block = Array.make(~length=blockLen, 0)
    key->Array.forEachWithIndex((b, i) => block->Array.setUnsafe(i, b))
    update(h, block)
  | None => ()
  }
  {blockLen, outputLen, update: data => update(h, data), digest: () => digest(h)}
}

// The parameter block's first word, RFC 7693 §2.5: p[0] = 0x0101kknn.
let paramWord = (outputLen, key) => {
  let keyLength = key->Option.mapOr(0, Array.length)
  outputLen ||| keyLength << 8 ||| 0x01 << 16 ||| 0x01 << 24
}

// XORs the little-endian words of `bytes` into `state` from `at`.
let xorWords = (state, at, bytes) =>
  switch bytes {
  | Some(bytes) =>
    for i in 0 to Array.length(bytes) / 4 - 1 {
      state->Array.setUnsafe(at + i, state->Array.getUnsafe(at + i) ^^^ Utils.getUint32LE(bytes, 4 * i))
    }
  | None => ()
  }

// The SHA-512 IV as little-endian low and high halves (noble's B2B_IV).
let b2bIV = [
  0xf3bcc908, 0x6a09e667, 0x84caa73b, 0xbb67ae85, 0xfe94f82b, 0x3c6ef372, 0x5f1d36f1, 0xa54ff53a,
  0xade682d1, 0x510e527f, 0x2b3e6c1f, 0x9b05688c, 0xfb41bd6b, 0x1f83d9ab, 0x137e2179, 0x5be0cd19,
]

// G's first half for BLAKE2b on the work vector v and message words m:
// a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24).
let g1b = (v, m, a, b, c, d, x) => {
  let get = i => v->Array.getUnsafe(i)
  let (ah, al) = add3(get(2 * a + 1), get(2 * a), get(2 * b + 1), get(2 * b), m->Array.getUnsafe(x + 1), m->Array.getUnsafe(x))
  let th = get(2 * d + 1) ^^^ ah
  let tl = get(2 * d) ^^^ al
  let dh = rotr32H(th, tl)
  let dl = rotr32L(th, tl)
  let (ch, cl) = add(get(2 * c + 1), get(2 * c), dh, dl)
  let th = get(2 * b + 1) ^^^ ch
  let tl = get(2 * b) ^^^ cl
  v->Array.setUnsafe(2 * a, al)
  v->Array.setUnsafe(2 * a + 1, ah)
  v->Array.setUnsafe(2 * b, rotrSL(th, tl, 24))
  v->Array.setUnsafe(2 * b + 1, rotrSH(th, tl, 24))
  v->Array.setUnsafe(2 * c, cl)
  v->Array.setUnsafe(2 * c + 1, ch)
  v->Array.setUnsafe(2 * d, dl)
  v->Array.setUnsafe(2 * d + 1, dh)
}

// G's second half for BLAKE2b: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63).
let g2b = (v, m, a, b, c, d, x) => {
  let get = i => v->Array.getUnsafe(i)
  let (ah, al) = add3(get(2 * a + 1), get(2 * a), get(2 * b + 1), get(2 * b), m->Array.getUnsafe(x + 1), m->Array.getUnsafe(x))
  let th = get(2 * d + 1) ^^^ ah
  let tl = get(2 * d) ^^^ al
  let dh = rotrSH(th, tl, 16)
  let dl = rotrSL(th, tl, 16)
  let (ch, cl) = add(get(2 * c + 1), get(2 * c), dh, dl)
  let th = get(2 * b + 1) ^^^ ch
  let tl = get(2 * b) ^^^ cl
  v->Array.setUnsafe(2 * a, al)
  v->Array.setUnsafe(2 * a + 1, ah)
  v->Array.setUnsafe(2 * b, rotrBL(th, tl, 63))
  v->Array.setUnsafe(2 * b + 1, rotrBH(th, tl, 63))
  v->Array.setUnsafe(2 * c, cl)
  v->Array.setUnsafe(2 * c + 1, ch)
  v->Array.setUnsafe(2 * d, dl)
  v->Array.setUnsafe(2 * d + 1, dh)
}

// noble's _BLAKE2b.
let blake2b = (opts): Hash.t => {
  let outputLen = Utils.anumber(opts.dkLen->Option.getOr(64.), "outputLen")
  checkBlake2Opts(outputLen, opts, ~keyLen=64, ~saltLen=16, ~persLen=16)
  // v0l, v0h, ..., v7l, v7h; the parameter word goes into the low half of h[0].
  let state = b2bIV->Array.copy
  state->Array.setUnsafe(0, state->Array.getUnsafe(0) ^^^ paramWord(outputLen, opts.key))
  xorWords(state, 8, opts.salt)
  xorWords(state, 12, opts.personalization)
  let v = Array.make(~length=32, 0)
  let m = Array.make(~length=32, 0)
  let compress = (data, offset, length, isLast) => {
    for i in 0 to 31 {
      m->Array.setUnsafe(i, Utils.getUint32LE(data, offset + 4 * i))
    }
    for i in 0 to 15 {
      v->Array.setUnsafe(i, state->Array.getUnsafe(i))
      v->Array.setUnsafe(i + 16, b2bIV->Array.getUnsafe(i))
    }
    // The byte counter goes into v12; v14 is inverted on the last block.
    v->Array.setUnsafe(24, b2bIV->Array.getUnsafe(8) ^^^ fromNumL(length))
    v->Array.setUnsafe(25, b2bIV->Array.getUnsafe(9) ^^^ fromNumH(length))
    if isLast {
      v->Array.setUnsafe(28, ~~~(v->Array.getUnsafe(28)))
      v->Array.setUnsafe(29, ~~~(v->Array.getUnsafe(29)))
    }
    // bsigma picks 64-bit message words; each spans two entries of m.
    for round in 0 to 11 {
      let s = i => 2 * bsigma->Array.getUnsafe(16 * round + i)
      g1b(v, m, 0, 4, 8, 12, s(0))
      g2b(v, m, 0, 4, 8, 12, s(1))
      g1b(v, m, 1, 5, 9, 13, s(2))
      g2b(v, m, 1, 5, 9, 13, s(3))
      g1b(v, m, 2, 6, 10, 14, s(4))
      g2b(v, m, 2, 6, 10, 14, s(5))
      g1b(v, m, 3, 7, 11, 15, s(6))
      g2b(v, m, 3, 7, 11, 15, s(7))
      g1b(v, m, 0, 5, 10, 15, s(8))
      g2b(v, m, 0, 5, 10, 15, s(9))
      g1b(v, m, 1, 6, 11, 12, s(10))
      g2b(v, m, 1, 6, 11, 12, s(11))
      g1b(v, m, 2, 7, 8, 13, s(12))
      g2b(v, m, 2, 7, 8, 13, s(13))
      g1b(v, m, 3, 4, 9, 14, s(14))
      g2b(v, m, 3, 4, 9, 14, s(15))
    }
    for i in 0 to 15 {
      let word = state->Array.getUnsafe(i) ^^^ v->Array.getUnsafe(i) ^^^ v->Array.getUnsafe(i + 16)
      state->Array.setUnsafe(i, word)
    }
  }
  make(~blockLen=128, ~outputLen, ~key=opts.key, ~compress, ~get=() => state)
}

// G's first half for BLAKE2s: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12).
let g1s = (v, a, b, c, d, x) => {
  let get = i => v->Array.getUnsafe(i)
  v->Array.setUnsafe(a, get(a) + get(b) + x)
  v->Array.setUnsafe(d, Utils.rotr(get(d) ^^^ get(a), 16))
  v->Array.setUnsafe(c, get(c) + get(d))
  v->Array.setUnsafe(b, Utils.rotr(get(b) ^^^ get(c), 12))
}

// G's second half for BLAKE2s: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7).
let g2s = (v, a, b, c, d, x) => {
  let get = i => v->Array.getUnsafe(i)
  v->Array.setUnsafe(a, get(a) + get(b) + x)
  v->Array.setUnsafe(d, Utils.rotr(get(d) ^^^ get(a), 8))
  v->Array.setUnsafe(c, get(c) + get(d))
  v->Array.setUnsafe(b, Utils.rotr(get(b) ^^^ get(c), 7))
}

// noble's _BLAKE2s, whose IV is SHA-256's.
let blake2s = (opts): Hash.t => {
  let outputLen = Utils.anumber(opts.dkLen->Option.getOr(32.), "outputLen")
  checkBlake2Opts(outputLen, opts, ~keyLen=32, ~saltLen=8, ~persLen=8)
  // v0 ... v7.
  let iv = Sha2.sha256IV
  let state = iv->Array.copy
  state->Array.setUnsafe(0, state->Array.getUnsafe(0) ^^^ paramWord(outputLen, opts.key))
  xorWords(state, 4, opts.salt)
  xorWords(state, 6, opts.personalization)
  let v = Array.make(~length=16, 0)
  let m = Array.make(~length=16, 0)
  let compress = (data, offset, length, isLast) => {
    for i in 0 to 15 {
      m->Array.setUnsafe(i, Utils.getUint32LE(data, offset + 4 * i))
    }
    // v8..v15 come from the IV, with the byte counter in v12 and v13 and v14 inverted last.
    for i in 0 to 7 {
      v->Array.setUnsafe(i, state->Array.getUnsafe(i))
      v->Array.setUnsafe(i + 8, iv->Array.getUnsafe(i))
    }
    v->Array.setUnsafe(12, v->Array.getUnsafe(12) ^^^ fromNumL(length))
    v->Array.setUnsafe(13, v->Array.getUnsafe(13) ^^^ fromNumH(length))
    if isLast {
      v->Array.setUnsafe(14, ~~~(v->Array.getUnsafe(14)))
    }
    for round in 0 to 9 {
      let x = i => m->Array.getUnsafe(bsigma->Array.getUnsafe(16 * round + i))
      g1s(v, 0, 4, 8, 12, x(0))
      g2s(v, 0, 4, 8, 12, x(1))
      g1s(v, 1, 5, 9, 13, x(2))
      g2s(v, 1, 5, 9, 13, x(3))
      g1s(v, 2, 6, 10, 14, x(4))
      g2s(v, 2, 6, 10, 14, x(5))
      g1s(v, 3, 7, 11, 15, x(6))
      g2s(v, 3, 7, 11, 15, x(7))
      g1s(v, 0, 5, 10, 15, x(8))
      g2s(v, 0, 5, 10, 15, x(9))
      g1s(v, 1, 6, 11, 12, x(10))
      g2s(v, 1, 6, 11, 12, x(11))
      g1s(v, 2, 7, 8, 13, x(12))
      g2s(v, 2, 7, 8, 13, x(13))
      g1s(v, 3, 4, 9, 14, x(14))
      g2s(v, 3, 4, 9, 14, x(15))
    }
    for i in 0 to 7 {
      let word = state->Array.getUnsafe(i) ^^^ v->Array.getUnsafe(i) ^^^ v->Array.getUnsafe(i + 8)
      state->Array.setUnsafe(i, word)
    }
  }
  make(~blockLen=64, ~outputLen, ~key=opts.key, ~compress, ~get=() => state)
}
