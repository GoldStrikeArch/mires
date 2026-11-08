// MurmurHash3 x64 128, FingerprintJS 5.2.0. Four 16-bit lanes + UTF-8 walk.

const C1 = [0x87c37b91, 0x114253d5]
const C2 = [0x4cf5ad43, 0x2745937f]
const F1 = [0xff51afd7, 0xed558ccd]
const F2 = [0xc4ceb9fe, 0x1a85ec53]
const FIVE = [0, 5]
const N1 = [0, 0x52dce729]
const N2 = [0, 0x38495ab5]

function add64(m, n) {
  const a = [m[0] >>> 16, m[0] & 0xffff, m[1] >>> 16, m[1] & 0xffff]
  const b = [n[0] >>> 16, n[0] & 0xffff, n[1] >>> 16, n[1] & 0xffff]
  const o = [0, 0, 0, 0]
  o[3] += a[3] + b[3]
  o[2] += o[3] >>> 16
  o[3] &= 0xffff
  o[2] += a[2] + b[2]
  o[1] += o[2] >>> 16
  o[2] &= 0xffff
  o[1] += a[1] + b[1]
  o[0] += o[1] >>> 16
  o[1] &= 0xffff
  o[0] += a[0] + b[0]
  o[0] &= 0xffff
  return [(o[0] << 16) | o[1], (o[2] << 16) | o[3]]
}

function mul64(m, n) {
  const a = [m[0] >>> 16, m[0] & 0xffff, m[1] >>> 16, m[1] & 0xffff]
  const b = [n[0] >>> 16, n[0] & 0xffff, n[1] >>> 16, n[1] & 0xffff]
  const o = [0, 0, 0, 0]
  o[3] += a[3] * b[3]
  o[2] += o[3] >>> 16
  o[3] &= 0xffff
  o[2] += a[2] * b[3]
  o[1] += o[2] >>> 16
  o[2] &= 0xffff
  o[2] += a[3] * b[2]
  o[1] += o[2] >>> 16
  o[2] &= 0xffff
  o[1] += a[1] * b[3]
  o[0] += o[1] >>> 16
  o[1] &= 0xffff
  o[1] += a[2] * b[2]
  o[0] += o[1] >>> 16
  o[1] &= 0xffff
  o[1] += a[3] * b[1]
  o[0] += o[1] >>> 16
  o[1] &= 0xffff
  o[0] += a[0] * b[3] + a[1] * b[2] + a[2] * b[1] + a[3] * b[0]
  o[0] &= 0xffff
  return [(o[0] << 16) | o[1], (o[2] << 16) | o[3]]
}

function rotl64(m, bits) {
  bits %= 64
  if (bits === 32) return [m[1], m[0]]
  if (bits < 32) {
    return [(m[0] << bits) | (m[1] >>> (32 - bits)), (m[1] << bits) | (m[0] >>> (32 - bits))]
  }
  bits -= 32
  return [(m[1] << bits) | (m[0] >>> (32 - bits)), (m[0] << bits) | (m[1] >>> (32 - bits))]
}

function xor64(m, n) {
  return [m[0] ^ n[0], m[1] ^ n[1]]
}

function shiftl64(m, bits) {
  bits %= 64
  if (bits === 0) return [m[0], m[1]]
  if (bits < 32) return [(m[0] << bits) | (m[1] >>> (32 - bits)), m[1] << bits]
  return [m[1] << (bits - 32), 0]
}

function fmix64(h) {
  h = xor64(h, [0, h[0] >>> 1])
  h = mul64(h, F1)
  h = xor64(h, [0, h[0] >>> 1])
  h = mul64(h, F2)
  h = xor64(h, [0, h[0] >>> 1])
  return h
}

function utf8Walk(input) {
  const bytes = []
  const text = input == null ? '' : String(input)
  for (let i = 0; i < text.length; i++) {
    let code = text.charCodeAt(i)
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = i + 1 < text.length ? text.charCodeAt(i + 1) : 0
      if (next >= 0xdc00 && next <= 0xdfff) {
        code = 0x10000 + ((code - 0xd800) << 10) + (next - 0xdc00)
        i++
      } else {
        code = 0xfffd
      }
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      code = 0xfffd
    }
    if (code < 0x80) {
      bytes.push(code)
    } else if (code < 0x800) {
      bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f))
    } else if (code < 0x10000) {
      bytes.push(
        0xe0 | (code >> 12),
        0x80 | ((code >> 6) & 0x3f),
        0x80 | (code & 0x3f),
      )
    } else {
      bytes.push(
        0xf0 | (code >> 18),
        0x80 | ((code >> 12) & 0x3f),
        0x80 | ((code >> 6) & 0x3f),
        0x80 | (code & 0x3f),
      )
    }
  }
  return bytes
}

function hex8(value) {
  return (value >>> 0).toString(16).padStart(8, '0')
}

export function murmurX64Hash128(input, seed) {
  const key = utf8Walk(input)
  seed = seed == null ? 0 : seed >>> 0
  const remainder = key.length % 16
  const bytes = key.length - remainder

  let h1 = [0, seed]
  let h2 = [0, seed]

  for (let i = 0; i < bytes; i += 16) {
    let k1 = [
      (key[i + 4] | (key[i + 5] << 8) | (key[i + 6] << 16) | (key[i + 7] << 24)) >>> 0,
      (key[i] | (key[i + 1] << 8) | (key[i + 2] << 16) | (key[i + 3] << 24)) >>> 0,
    ]
    let k2 = [
      (key[i + 12] | (key[i + 13] << 8) | (key[i + 14] << 16) | (key[i + 15] << 24)) >>> 0,
      (key[i + 8] | (key[i + 9] << 8) | (key[i + 10] << 16) | (key[i + 11] << 24)) >>> 0,
    ]

    k1 = mul64(k1, C1)
    k1 = rotl64(k1, 31)
    k1 = mul64(k1, C2)
    h1 = xor64(h1, k1)
    h1 = rotl64(h1, 27)
    h1 = add64(h1, h2)
    h1 = mul64(h1, FIVE)
    h1 = add64(h1, N1)

    k2 = mul64(k2, C2)
    k2 = rotl64(k2, 33)
    k2 = mul64(k2, C1)
    h2 = xor64(h2, k2)
    h2 = rotl64(h2, 31)
    h2 = add64(h2, h1)
    h2 = mul64(h2, FIVE)
    h2 = add64(h2, N2)
  }

  let k1 = [0, 0]
  let k2 = [0, 0]
  switch (remainder) {
    case 15:
      k2 = xor64(k2, shiftl64([0, key[bytes + 14]], 48))
    case 14:
      k2 = xor64(k2, shiftl64([0, key[bytes + 13]], 40))
    case 13:
      k2 = xor64(k2, shiftl64([0, key[bytes + 12]], 32))
    case 12:
      k2 = xor64(k2, shiftl64([0, key[bytes + 11]], 24))
    case 11:
      k2 = xor64(k2, shiftl64([0, key[bytes + 10]], 16))
    case 10:
      k2 = xor64(k2, shiftl64([0, key[bytes + 9]], 8))
    case 9:
      k2 = xor64(k2, [0, key[bytes + 8]])
      k2 = mul64(k2, C2)
      k2 = rotl64(k2, 33)
      k2 = mul64(k2, C1)
      h2 = xor64(h2, k2)
    case 8:
      k1 = xor64(k1, shiftl64([0, key[bytes + 7]], 56))
    case 7:
      k1 = xor64(k1, shiftl64([0, key[bytes + 6]], 48))
    case 6:
      k1 = xor64(k1, shiftl64([0, key[bytes + 5]], 40))
    case 5:
      k1 = xor64(k1, shiftl64([0, key[bytes + 4]], 32))
    case 4:
      k1 = xor64(k1, shiftl64([0, key[bytes + 3]], 24))
    case 3:
      k1 = xor64(k1, shiftl64([0, key[bytes + 2]], 16))
    case 2:
      k1 = xor64(k1, shiftl64([0, key[bytes + 1]], 8))
    case 1:
      k1 = xor64(k1, [0, key[bytes]])
      k1 = mul64(k1, C1)
      k1 = rotl64(k1, 31)
      k1 = mul64(k1, C2)
      h1 = xor64(h1, k1)
  }

  h1 = xor64(h1, [0, key.length])
  h2 = xor64(h2, [0, key.length])
  h1 = add64(h1, h2)
  h2 = add64(h2, h1)
  h1 = fmix64(h1)
  h2 = fmix64(h2)
  h1 = add64(h1, h2)
  h2 = add64(h2, h1)

  return hex8(h1[0]) + hex8(h1[1]) + hex8(h2[0]) + hex8(h2[1])
}
