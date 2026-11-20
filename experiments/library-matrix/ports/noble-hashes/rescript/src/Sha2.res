// noble's sha2.ts: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B), with the IVs from _md.ts.
open U64

let sha256IV = [
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
]

let sha256K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]

// noble's SHA2_32B state: A ... H, and the message schedule (noble shares one SHA256_W).
type sha256 = {
  mutable a: int,
  mutable b: int,
  mutable c: int,
  mutable d: int,
  mutable e: int,
  mutable f: int,
  mutable g: int,
  mutable h: int,
  w: array<int>,
}

let process256 = (s, data, offset) => {
  let w = s.w
  for i in 0 to 15 {
    w->Array.setUnsafe(i, Utils.getUint32BE(data, offset + 4 * i))
  }
  for i in 16 to 63 {
    let w15 = w->Array.getUnsafe(i - 15)
    let w2 = w->Array.getUnsafe(i - 2)
    let s0 = Utils.rotr(w15, 7) ^^^ Utils.rotr(w15, 18) ^^^ w15 >>> 3
    let s1 = Utils.rotr(w2, 17) ^^^ Utils.rotr(w2, 19) ^^^ w2 >>> 10
    w->Array.setUnsafe(i, s1 + w->Array.getUnsafe(i - 7) + s0 + w->Array.getUnsafe(i - 16))
  }
  let a = ref(s.a)
  let b = ref(s.b)
  let c = ref(s.c)
  let d = ref(s.d)
  let e = ref(s.e)
  let f = ref(s.f)
  let g = ref(s.g)
  let h = ref(s.h)
  for i in 0 to 63 {
    let sigma1 = Utils.rotr(e.contents, 6) ^^^ Utils.rotr(e.contents, 11) ^^^ Utils.rotr(e.contents, 25)
    let t1 =
      h.contents + sigma1 + Md.chi(e.contents, f.contents, g.contents) + sha256K->Array.getUnsafe(i) +
        w->Array.getUnsafe(i)
    let sigma0 = Utils.rotr(a.contents, 2) ^^^ Utils.rotr(a.contents, 13) ^^^ Utils.rotr(a.contents, 22)
    let t2 = sigma0 + Md.maj(a.contents, b.contents, c.contents)
    h := g.contents
    g := f.contents
    f := e.contents
    e := d.contents + t1
    d := c.contents
    c := b.contents
    b := a.contents
    a := t1 + t2
  }
  s.a = s.a + a.contents
  s.b = s.b + b.contents
  s.c = s.c + c.contents
  s.d = s.d + d.contents
  s.e = s.e + e.contents
  s.f = s.f + f.contents
  s.g = s.g + g.contents
  s.h = s.h + h.contents
}

let sha256 = (): Hash.t => {
  let iv = i => sha256IV->Array.getUnsafe(i)
  let s = {
    a: iv(0),
    b: iv(1),
    c: iv(2),
    d: iv(3),
    e: iv(4),
    f: iv(5),
    g: iv(6),
    h: iv(7),
    w: Array.make(~length=64, 0),
  }
  Md.make(
    ~blockLen=64,
    ~outputLen=32,
    ~padOffset=8,
    ~process=(data, offset) => process256(s, data, offset),
    ~get=() => [s.a, s.b, s.c, s.d, s.e, s.f, s.g, s.h],
  )->Md.toHash
}

let sha512IV = [
  0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b, 0xa54ff53a, 0x5f1d36f1,
  0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f, 0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
]

// The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is.
let sha512Kh = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  0xca273ece, 0xd186b8c7, 0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
  0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c, 0x5fcb6fab, 0x6c44198c,
]

let sha512Kl = [
  0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019, 0xaf194f9b, 0xda6d8118,
  0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2, 0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694,
  0x9ef14ad2, 0x384f25e3, 0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
  0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725, 0xe003826f, 0x0a0e6e70,
  0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df, 0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b,
  0x4cf10364, 0xbc423001, 0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
  0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb, 0x7763e373, 0xd6b2b8a3,
  0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec, 0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b,
  0xea26619c, 0x21c0c207, 0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
  0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a, 0x3ad6faec, 0x4a475817,
]

// noble's SHA2_64B state: Ah, Al, ..., Hh, Hl, and the halves of the message schedule (noble's
// SHA512_W_H and SHA512_W_L).
type sha512 = {state: array<int>, wh: array<int>, wl: array<int>}

let process512 = ({state, wh, wl}, data, offset) => {
  for i in 0 to 15 {
    wh->Array.setUnsafe(i, Utils.getUint32BE(data, offset + 8 * i))
    wl->Array.setUnsafe(i, Utils.getUint32BE(data, offset + 8 * i + 4))
  }
  for i in 16 to 79 {
    // s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7)
    let w15h = wh->Array.getUnsafe(i - 15)
    let w15l = wl->Array.getUnsafe(i - 15)
    let s0h = rotrSH(w15h, w15l, 1) ^^^ rotrSH(w15h, w15l, 8) ^^^ shrSH(w15h, w15l, 7)
    let s0l = rotrSL(w15h, w15l, 1) ^^^ rotrSL(w15h, w15l, 8) ^^^ shrSL(w15h, w15l, 7)
    // s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6)
    let w2h = wh->Array.getUnsafe(i - 2)
    let w2l = wl->Array.getUnsafe(i - 2)
    let s1h = rotrSH(w2h, w2l, 19) ^^^ rotrBH(w2h, w2l, 61) ^^^ shrSH(w2h, w2l, 6)
    let s1l = rotrSL(w2h, w2l, 19) ^^^ rotrBL(w2h, w2l, 61) ^^^ shrSL(w2h, w2l, 6)
    let (sumH, sumL) = add4(
      s0h,
      s0l,
      s1h,
      s1l,
      wh->Array.getUnsafe(i - 7),
      wl->Array.getUnsafe(i - 7),
      wh->Array.getUnsafe(i - 16),
      wl->Array.getUnsafe(i - 16),
    )
    wh->Array.setUnsafe(i, sumH)
    wl->Array.setUnsafe(i, sumL)
  }
  let get = i => state->Array.getUnsafe(i)
  let ah = ref(get(0))
  let al = ref(get(1))
  let bh = ref(get(2))
  let bl = ref(get(3))
  let ch = ref(get(4))
  let cl = ref(get(5))
  let dh = ref(get(6))
  let dl = ref(get(7))
  let eh = ref(get(8))
  let el = ref(get(9))
  let fh = ref(get(10))
  let fl = ref(get(11))
  let gh = ref(get(12))
  let gl = ref(get(13))
  let hh = ref(get(14))
  let hl = ref(get(15))
  for i in 0 to 79 {
    let (e1, e2) = (eh.contents, el.contents)
    let (a1, a2) = (ah.contents, al.contents)
    // S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
    let sigma1h = rotrSH(e1, e2, 14) ^^^ rotrSH(e1, e2, 18) ^^^ rotrBH(e1, e2, 41)
    let sigma1l = rotrSL(e1, e2, 14) ^^^ rotrSL(e1, e2, 18) ^^^ rotrBL(e1, e2, 41)
    // T1 = H + S1 + Chi(E, F, G) + K[i] + W[i]
    let (t1h, t1l) = add5(
      hh.contents,
      hl.contents,
      sigma1h,
      sigma1l,
      Md.chi(e1, fh.contents, gh.contents),
      Md.chi(e2, fl.contents, gl.contents),
      sha512Kh->Array.getUnsafe(i),
      sha512Kl->Array.getUnsafe(i),
      wh->Array.getUnsafe(i),
      wl->Array.getUnsafe(i),
    )
    // S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
    let sigma0h = rotrSH(a1, a2, 28) ^^^ rotrBH(a1, a2, 34) ^^^ rotrBH(a1, a2, 39)
    let sigma0l = rotrSL(a1, a2, 28) ^^^ rotrBL(a1, a2, 34) ^^^ rotrBL(a1, a2, 39)
    let majH = Md.maj(a1, bh.contents, ch.contents)
    let majL = Md.maj(a2, bl.contents, cl.contents)
    hh := gh.contents
    hl := gl.contents
    gh := fh.contents
    gl := fl.contents
    fh := e1
    fl := e2
    let (nextEh, nextEl) = add(dh.contents, dl.contents, t1h, t1l)
    eh := nextEh
    el := nextEl
    dh := ch.contents
    dl := cl.contents
    ch := bh.contents
    cl := bl.contents
    bh := a1
    bl := a2
    let (nextAh, nextAl) = add3(t1h, t1l, sigma0h, sigma0l, majH, majL)
    ah := nextAh
    al := nextAl
  }
  // Add the compressed block to the state.
  let working = [
    ah.contents,
    al.contents,
    bh.contents,
    bl.contents,
    ch.contents,
    cl.contents,
    dh.contents,
    dl.contents,
    eh.contents,
    el.contents,
    fh.contents,
    fl.contents,
    gh.contents,
    gl.contents,
    hh.contents,
    hl.contents,
  ]
  for word in 0 to 7 {
    let i = 2 * word
    let (h, l) = add(get(i), get(i + 1), working->Array.getUnsafe(i), working->Array.getUnsafe(i + 1))
    state->Array.setUnsafe(i, h)
    state->Array.setUnsafe(i + 1, l)
  }
}

let sha512 = (): Hash.t => {
  let s = {
    state: sha512IV->Array.copy,
    wh: Array.make(~length=80, 0),
    wl: Array.make(~length=80, 0),
  }
  Md.make(
    ~blockLen=128,
    ~outputLen=64,
    ~padOffset=16,
    ~process=(data, offset) => process512(s, data, offset),
    ~get=() => s.state,
  )->Md.toHash
}
