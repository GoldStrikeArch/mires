open Bindings
open Data

type x64 = {
  mutable high: int,
  mutable low: int,
}

let x64 = (high, low) => {high, low}

let x64Add = (m: x64, n: x64) => {
  let m0 = lsr(m.high, 16)
  let m1 = land(m.high, 0xffff)
  let m2 = lsr(m.low, 16)
  let m3 = land(m.low, 0xffff)
  let n0 = lsr(n.high, 16)
  let n1 = land(n.high, 0xffff)
  let n2 = lsr(n.low, 16)
  let n3 = land(n.low, 0xffff)
  let o3 = m3 + n3
  let o2 = lsr(o3, 16) + m2 + n2
  let o3 = land(o3, 0xffff)
  let o1 = lsr(o2, 16) + m1 + n1
  let o2 = land(o2, 0xffff)
  let o0 = lsr(o1, 16) + m0 + n0
  let o1 = land(o1, 0xffff)
  let o0 = land(o0, 0xffff)
  m.high = lor(lsl(o0, 16), o1)
  m.low = lor(lsl(o2, 16), o3)
}

let x64Multiply = (m: x64, n: x64) => {
  let m0 = lsr(m.high, 16)
  let m1 = land(m.high, 0xffff)
  let m2 = lsr(m.low, 16)
  let m3 = land(m.low, 0xffff)
  let n0 = lsr(n.high, 16)
  let n1 = land(n.high, 0xffff)
  let n2 = lsr(n.low, 16)
  let n3 = land(n.low, 0xffff)
  let o3 = m3 * n3
  let o2 = lsr(o3, 16) + m2 * n3
  let o3 = land(o3, 0xffff)
  let o1 = lsr(o2, 16)
  let o2 = land(o2, 0xffff) + m3 * n2
  let o1 = o1 + lsr(o2, 16)
  let o2 = land(o2, 0xffff)
  let o1 = o1 + m1 * n3
  let o0 = lsr(o1, 16)
  let o1 = land(o1, 0xffff) + m2 * n2
  let o0 = o0 + lsr(o1, 16)
  let o1 = land(o1, 0xffff) + m3 * n1
  let o0 = o0 + lsr(o1, 16)
  let o1 = land(o1, 0xffff)
  let o0 = land(o0 + m0 * n3 + m1 * n2 + m2 * n1 + m3 * n0, 0xffff)
  m.high = lor(lsl(o0, 16), o1)
  m.low = lor(lsl(o2, 16), o3)
}

let x64Rotl = (m: x64, bits: int) => {
  let m0 = m.high
  let bits = mod(bits, 64)
  if bits == 32 {
    m.high = m.low
    m.low = m0
  } else if bits < 32 {
    m.high = lor(lsl(m0, bits), lsr(m.low, 32 - bits))
    m.low = lor(lsl(m.low, bits), lsr(m0, 32 - bits))
  } else {
    let bits = bits - 32
    m.high = lor(lsl(m.low, bits), lsr(m0, 32 - bits))
    m.low = lor(lsl(m0, bits), lsr(m.low, 32 - bits))
  }
}

let x64LeftShift = (m: x64, bits: int) => {
  let bits = mod(bits, 64)
  if bits == 0 {
    ()
  } else if bits < 32 {
    m.high = lsr(m.low, 32 - bits)
    m.low = lsl(m.low, bits)
  } else {
    m.high = lsl(m.low, bits - 32)
    m.low = 0
  }
}

let x64Xor = (m: x64, n: x64) => {
  m.high = lxor(m.high, n.high)
  m.low = lxor(m.low, n.low)
}

let f1 = x64(0xff51afd7, 0xed558ccd)
let f2 = x64(0xc4ceb9fe, 0x1a85ec53)

let x64Fmix = (h: x64) => {
  let shifted = x64(0, lsr(h.high, 1))
  x64Xor(h, shifted)
  x64Multiply(h, f1)
  shifted.low = lsr(h.high, 1)
  x64Xor(h, shifted)
  x64Multiply(h, f2)
  shifted.low = lsr(h.high, 1)
  x64Xor(h, shifted)
}

let c1 = x64(0x87c37b91, 0x114253d5)
let c2 = x64(0x4cf5ad43, 0x2745937f)
let mConst = x64(0, 5)
let n1 = x64(0, 0x52dce729)
let n2 = x64(0, 0x38495ab5)

let hex8 = (value: int) => {
  let digits = "0123456789abcdef"
  let rec loop = (shift, acc) =>
    if shift < 0 {
      acc
    } else {
      let idx = land(lsr(value, shift), 0xf)
      loop(shift - 4, acc ++ sliceString(digits, idx, idx + 1))
    }
  loop(28, "")
}

let x64hash128 = (input: string, seed: any) => {
  let key = getUTF8Bytes(input)
  let seed = if isFalsy(seed) {
    0
  } else {
    asInt(seed)
  }
  let length = x64(0, u8Length(key))
  let remainder = mod(length.low, 16)
  let bytes = length.low - remainder
  let h1 = x64(0, seed)
  let h2 = x64(0, seed)
  let k1 = x64(0, 0)
  let k2 = x64(0, 0)
  let rec loop = i =>
    if i >= bytes {
      i
    } else {
      k1.high =
        lor(
          lor(lor(asInt(jsIndex(key, i + 4)), lsl(asInt(jsIndex(key, i + 5)), 8)), lsl(asInt(jsIndex(key, i + 6)), 16)),
          lsl(asInt(jsIndex(key, i + 7)), 24),
        )
      k1.low =
        lor(
          lor(lor(asInt(jsIndex(key, i)), lsl(asInt(jsIndex(key, i + 1)), 8)), lsl(asInt(jsIndex(key, i + 2)), 16)),
          lsl(asInt(jsIndex(key, i + 3)), 24),
        )
      k2.high =
        lor(
          lor(
            lor(asInt(jsIndex(key, i + 12)), lsl(asInt(jsIndex(key, i + 13)), 8)),
            lsl(asInt(jsIndex(key, i + 14)), 16),
          ),
          lsl(asInt(jsIndex(key, i + 15)), 24),
        )
      k2.low =
        lor(
          lor(lor(asInt(jsIndex(key, i + 8)), lsl(asInt(jsIndex(key, i + 9)), 8)), lsl(asInt(jsIndex(key, i + 10)), 16)),
          lsl(asInt(jsIndex(key, i + 11)), 24),
        )
      x64Multiply(k1, c1)
      x64Rotl(k1, 31)
      x64Multiply(k1, c2)
      x64Xor(h1, k1)
      x64Rotl(h1, 27)
      x64Add(h1, h2)
      x64Multiply(h1, mConst)
      x64Add(h1, n1)
      x64Multiply(k2, c2)
      x64Rotl(k2, 33)
      x64Multiply(k2, c1)
      x64Xor(h2, k2)
      x64Rotl(h2, 31)
      x64Add(h2, h1)
      x64Multiply(h2, mConst)
      x64Add(h2, n2)
      loop(i + 16)
    }
  let i = loop(0)
  k1.high = 0
  k1.low = 0
  k2.high = 0
  k2.low = 0
  let val_ = x64(0, 0)
  let xorShift = (target, byteIndex, shift) => {
    val_.high = 0
    val_.low = asInt(jsIndex(key, byteIndex))
    if shift != 0 {
      x64LeftShift(val_, shift)
    }
    x64Xor(target, val_)
  }
  if remainder >= 15 {
    xorShift(k2, i + 14, 48)
  }
  if remainder >= 14 {
    xorShift(k2, i + 13, 40)
  }
  if remainder >= 13 {
    xorShift(k2, i + 12, 32)
  }
  if remainder >= 12 {
    xorShift(k2, i + 11, 24)
  }
  if remainder >= 11 {
    xorShift(k2, i + 10, 16)
  }
  if remainder >= 10 {
    xorShift(k2, i + 9, 8)
  }
  if remainder >= 9 {
    xorShift(k2, i + 8, 0)
    x64Multiply(k2, c2)
    x64Rotl(k2, 33)
    x64Multiply(k2, c1)
    x64Xor(h2, k2)
  }
  if remainder >= 8 {
    xorShift(k1, i + 7, 56)
  }
  if remainder >= 7 {
    xorShift(k1, i + 6, 48)
  }
  if remainder >= 6 {
    xorShift(k1, i + 5, 40)
  }
  if remainder >= 5 {
    xorShift(k1, i + 4, 32)
  }
  if remainder >= 4 {
    xorShift(k1, i + 3, 24)
  }
  if remainder >= 3 {
    xorShift(k1, i + 2, 16)
  }
  if remainder >= 2 {
    xorShift(k1, i + 1, 8)
  }
  if remainder >= 1 {
    xorShift(k1, i, 0)
    x64Multiply(k1, c1)
    x64Rotl(k1, 31)
    x64Multiply(k1, c2)
    x64Xor(h1, k1)
  }
  x64Xor(h1, length)
  x64Xor(h2, length)
  x64Add(h1, h2)
  x64Add(h2, h1)
  x64Fmix(h1)
  x64Fmix(h2)
  x64Add(h1, h2)
  x64Add(h2, h1)
  hex8(h1.high) ++ hex8(h1.low) ++ hex8(h2.high) ++ hex8(h2.low)
}

let murmurX64Hash128 = x64hash128
