// noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.
//
// The state is noble's state32: 25 lanes of two 32-bit words, the low word first. noble derives
// the pi, rho and iota tables at load time; they are written out here.

// Pi: the word index each rho step moves its lane to (noble's SHA3_PI).
let sha3Pi = [20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2]

// Rho: the left rotation of each step (noble's SHA3_ROTL).
let sha3Rotl = [1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44]

// Iota: the low and high words of each round constant (noble's SHA3_IOTA_H and SHA3_IOTA_L).
let sha3IotaH = [
  0x00000001, 0x00008082, 0x0000808a, 0x80008000, 0x0000808b, 0x80000001, 0x80008081, 0x00008009,
  0x0000008a, 0x00000088, 0x80008009, 0x8000000a, 0x8000808b, 0x0000008b, 0x00008089, 0x00008003,
  0x00008002, 0x00000080, 0x0000800a, 0x8000000a, 0x80008081, 0x00008080, 0x80000001, 0x80008008,
]
let sha3IotaL = [
  0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000,
  0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000,
  0x80000000, 0x80000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000, 0x00000000, 0x80000000,
]

// Left rotations of a lane by s in 1..31 (S) or 33..63 (B). As in noble, h is the lane's first
// (low) word and l its second (high) word.
let rotlSH = (h, l, s) => h << s ||| l >>> 32 - s
let rotlSL = (h, l, s) => l << s ||| h >>> 32 - s
let rotlBH = (h, l, s) => l << s - 32 ||| h >>> 64 - s
let rotlBL = (h, l, s) => h << s - 32 ||| l >>> 64 - s
let rotlH = (h, l, s) => s > 32 ? rotlBH(h, l, s) : rotlSH(h, l, s)
let rotlL = (h, l, s) => s > 32 ? rotlBL(h, l, s) : rotlSL(h, l, s)

// noble's keccakP: the 24 rounds of Keccak-f[1600] on s, with b as the theta scratch.
let keccakP = (s, b) => {
  let get = i => s->Array.getUnsafe(i)
  let xor = (i, v) => s->Array.setUnsafe(i, get(i) ^^^ v)
  for round in 0 to 23 {
    // Theta
    for x in 0 to 9 {
      b->Array.setUnsafe(x, get(x) ^^^ get(x + 10) ^^^ get(x + 20) ^^^ get(x + 30) ^^^ get(x + 40))
    }
    for column in 0 to 4 {
      let x = 2 * column
      let idx1 = mod(x + 8, 10)
      let idx0 = mod(x + 2, 10)
      let b0 = b->Array.getUnsafe(idx0)
      let b1 = b->Array.getUnsafe(idx0 + 1)
      let th = rotlH(b0, b1, 1) ^^^ b->Array.getUnsafe(idx1)
      let tl = rotlL(b0, b1, 1) ^^^ b->Array.getUnsafe(idx1 + 1)
      for row in 0 to 4 {
        xor(x + 10 * row, th)
        xor(x + 10 * row + 1, tl)
      }
    }
    // Rho and pi
    let curH = ref(get(2))
    let curL = ref(get(3))
    for t in 0 to 23 {
      let shift = sha3Rotl->Array.getUnsafe(t)
      let th = rotlH(curH.contents, curL.contents, shift)
      let tl = rotlL(curH.contents, curL.contents, shift)
      let pi = sha3Pi->Array.getUnsafe(t)
      curH := get(pi)
      curL := get(pi + 1)
      s->Array.setUnsafe(pi, th)
      s->Array.setUnsafe(pi + 1, tl)
    }
    // Chi
    for row in 0 to 4 {
      let y = 10 * row
      let b0 = get(y)
      let b1 = get(y + 1)
      let b2 = get(y + 2)
      let b3 = get(y + 3)
      for x in 0 to 5 {
        xor(y + x, ~~~get(y + x + 2) &&& get(y + x + 4))
      }
      xor(y + 6, ~~~get(y + 8) &&& b0)
      xor(y + 7, ~~~get(y + 9) &&& b1)
      xor(y + 8, ~~~b0 &&& b2)
      xor(y + 9, ~~~b1 &&& b3)
    }
    // Iota
    xor(0, sha3IotaH->Array.getUnsafe(round))
    xor(1, sha3IotaL->Array.getUnsafe(round))
  }
}

// noble's Keccak sponge with a byte-length rate `blockLen` and domain `suffix`.
type t = {
  blockLen: int,
  suffix: int,
  outputLen: int,
  state32: array<int>,
  theta: array<int>,
  mutable pos: int,
  mutable posOut: int,
}

// XORs `byte` into state byte `index`, as noble does through its byte view of state32.
let xorByte = (k, index, byte) => {
  let word = index >>> 2
  k.state32->Array.setUnsafe(word, k.state32->Array.getUnsafe(word) ^^^ byte << 8 * (index &&& 3))
}

// The state byte at `index`.
let stateByte = (k, index) => k.state32->Array.getUnsafe(index >>> 2) >>> 8 * (index &&& 3) &&& 255

let keccak = k => {
  keccakP(k.state32, k.theta)
  k.posOut = 0
  k.pos = 0
}

let update = (k, data) => {
  let len = Array.length(data)
  let pos = ref(0)
  while pos.contents < len {
    if k.pos == 0 && len - pos.contents >= k.blockLen {
      // Whole blocks with nothing buffered are absorbed a word at a time.
      for i in 0 to k.blockLen / 4 - 1 {
        let word = k.state32->Array.getUnsafe(i) ^^^ Utils.getUint32LE(data, pos.contents + 4 * i)
        k.state32->Array.setUnsafe(i, word)
      }
      pos := pos.contents + k.blockLen
      k.pos = k.blockLen
      keccak(k)
    } else {
      let take = Math.Int.min(k.blockLen - k.pos, len - pos.contents)
      for _ in 1 to take {
        xorByte(k, k.pos, data->Array.getUnsafe(pos.contents))
        k.pos = k.pos + 1
        pos := pos.contents + 1
      }
      if k.pos == k.blockLen {
        keccak(k)
      }
    }
  }
}

let finish = k => {
  // The suffix holds the domain bits and the first padding bit; 0x80 closes pad10*1.
  xorByte(k, k.pos, k.suffix)
  if (k.suffix &&& 0x80) != 0 && k.pos == k.blockLen - 1 {
    keccak(k)
  }
  xorByte(k, k.blockLen - 1, 0x80)
  keccak(k)
}

let digest = k => {
  finish(k)
  let out = Array.make(~length=k.outputLen, 0)
  let pos = ref(0)
  while pos.contents < k.outputLen {
    if k.posOut >= k.blockLen {
      keccak(k)
    }
    let take = Math.Int.min(k.blockLen - k.posOut, k.outputLen - pos.contents)
    for _ in 1 to take {
      out->Array.setUnsafe(pos.contents, stateByte(k, k.posOut))
      k.posOut = k.posOut + 1
      pos := pos.contents + 1
    }
  }
  out
}

let make = (~blockLen, ~suffix, ~outputLen): Hash.t => {
  let k = {
    blockLen,
    suffix,
    outputLen,
    state32: Array.make(~length=50, 0),
    theta: Array.make(~length=10, 0),
    pos: 0,
    posOut: 0,
  }
  {blockLen, outputLen, update: data => update(k, data), digest: () => digest(k)}
}
