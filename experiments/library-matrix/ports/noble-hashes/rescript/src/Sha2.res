// SHA-256 and SHA-512 as in @noble/hashes 2.4.0. A 64-bit word consists of two 32-bit lanes.
let iv256 = [
  1779033703, -1150833019, 1013904242, -1521486534, 1359893119, -1694144372, 528734635, 1541459225,
]

let iv512 = [
  1779033703, -205731576, -1150833019, -2067093701, 1013904242, -23791573, -1521486534, 1595750129,
  1359893119, -1377402159, -1694144372, 725511199, 528734635, -79577749, 1541459225, 327033209,
]

let k256 = [
  1116352408, 1899447441, -1245643825, -373957723, 961987163, 1508970993, -1841331548, -1424204075,
  -670586216, 310598401, 607225278, 1426881987, 1925078388, -2132889090, -1680079193, -1046744716,
  -459576895, -272742522, 264347078, 604807628, 770255983, 1249150122, 1555081692, 1996064986,
  -1740746414, -1473132947, -1341970488, -1084653625, -958395405, -710438585, 113926993, 338241895,
  666307205, 773529912, 1294757372, 1396182291, 1695183700, 1986661051, -2117940946, -1838011259,
  -1564481375, -1474664885, -1035236496, -949202525, -778901479, -694614492, -200395387, 275423344,
  430227734, 506948616, 659060556, 883997877, 958139571, 1322822218, 1537002063, 1747873779,
  1955562222, 2024104815, -2067236844, -1933114872, -1866530822, -1538233109, -1090935817, -965641998,
]

let k512h = [
  1116352408, 1899447441, -1245643825, -373957723, 961987163, 1508970993, -1841331548, -1424204075,
  -670586216, 310598401, 607225278, 1426881987, 1925078388, -2132889090, -1680079193, -1046744716,
  -459576895, -272742522, 264347078, 604807628, 770255983, 1249150122, 1555081692, 1996064986,
  -1740746414, -1473132947, -1341970488, -1084653625, -958395405, -710438585, 113926993, 338241895,
  666307205, 773529912, 1294757372, 1396182291, 1695183700, 1986661051, -2117940946, -1838011259,
  -1564481375, -1474664885, -1035236496, -949202525, -778901479, -694614492, -200395387, 275423344,
  430227734, 506948616, 659060556, 883997877, 958139571, 1322822218, 1537002063, 1747873779,
  1955562222, 2024104815, -2067236844, -1933114872, -1866530822, -1538233109, -1090935817, -965641998,
  -903397682, -779700025, -354779690, -176337025, 116418474, 174292421, 289380356, 460393269,
  685471733, 852142971, 1017036298, 1126000580, 1288033470, 1501505948, 1607167915, 1816402316,
]

let k512l = [
  -685199838, 602891725, -330482897, -2121671748, -213338824, -1241133031, -1357295717, -630357736,
  -1560083902, 1164996542, 1323610764, -704662302, -226784913, 991336113, 633803317, -815192428,
  -1628353838, 944711139, -1953704523, 2007800933, 1495990901, 1856431235, -1119749164, -2096016459,
  -295247957, 766784016, -1728372417, -1091629340, 1034457026, -1828018395, -536640913, 168717936,
  1188179964, 1546045734, 1522805485, -1651133473, -1951439906, 1014477480, 1206759142, 344077627,
  1290863460, -1136513023, -789014639, 106217008, -688958952, 1432725776, 1467031594, 851169720,
  -1194143544, 1363258195, -544281703, -509917016, -976659869, -482243893, 2003034995, -692930397,
  1575990012, 1125592928, -1578062990, 442776044, 593698344, -561857047, -1295615723, -479046869,
  -366583396, 566280711, -840897762, -294727304, 1914138554, -1563912026, -1090974290, 320620315,
  587496836, 1086792851, 365543100, -1676669620, -885112138, -60457430, 987167468, 1246189591,
]

external band: (int, int) => int = "%andint"
external bor: (int, int) => int = "%orint"
external bxor: (int, int) => int = "%xorint"
external shl: (int, int) => int = "%lslint"
external shr: (int, int) => int = "%lsrint"
external inv: int => int = "%bitnot_int"
let ror = (x, n) => bor(shr(x, n), shl(x, 32 - n))
let choice = (a, b, c) => bxor(band(a, b), band(inv(a), c))
let majority = (a, b, c) => bxor(bxor(band(a, b), band(a, c)), band(b, c))
let carry = (a, b) => shr(bor(band(a, b), band(bor(a, b), inv((a + b)))), 31)
let readBe = (data, at) => bor(bor(shl(Array.getUnsafe(data, at), 24), shl(Array.getUnsafe(data, at + 1), 16)), bor(shl(Array.getUnsafe(data, at + 2), 8), Array.getUnsafe(data, at + 3)))
let compress256 = (state, data, at, wh) => {
  for i in 0 to 15 {
    Array.setUnsafe(wh, i, readBe(data, at + i * 4))
  }
  for i in 16 to 63 {
    let x = Array.getUnsafe(wh, i - 15)
    let y = Array.getUnsafe(wh, i - 2)
    let s0 = bxor(bxor(ror(x, 7), ror(x, 18)), shr(x, 3))
    let s1 = bxor(bxor(ror(y, 17), ror(y, 19)), shr(y, 10))
    Array.setUnsafe(wh, i, (s1 + Array.getUnsafe(wh, i - 7) + s0 + Array.getUnsafe(wh, i - 16)))
  }
  let a = ref(Array.getUnsafe(state, 0))
  let b = ref(Array.getUnsafe(state, 1))
  let c = ref(Array.getUnsafe(state, 2))
  let d = ref(Array.getUnsafe(state, 3))
  let e = ref(Array.getUnsafe(state, 4))
  let f = ref(Array.getUnsafe(state, 5))
  let g = ref(Array.getUnsafe(state, 6))
  let h = ref(Array.getUnsafe(state, 7))
  for i in 0 to 63 {
    let s1 = bxor(bxor(ror(e.contents, 6), ror(e.contents, 11)), ror(e.contents, 25))
    let t1 = (h.contents + s1 + choice(e.contents, f.contents, g.contents) + Array.getUnsafe(k256, i) + Array.getUnsafe(wh, i))
    let s0 = bxor(bxor(ror(a.contents, 2), ror(a.contents, 13)), ror(a.contents, 22))
    let newA = (t1 + s0 + majority(a.contents, b.contents, c.contents))
    let newE = (d.contents + t1)
    h := g.contents
    g := f.contents
    f := e.contents
    e := newE
    d := c.contents
    c := b.contents
    b := a.contents
    a := newA
  }
  Array.setUnsafe(state, 0, (Array.getUnsafe(state, 0) + a.contents))
  Array.setUnsafe(state, 1, (Array.getUnsafe(state, 1) + b.contents))
  Array.setUnsafe(state, 2, (Array.getUnsafe(state, 2) + c.contents))
  Array.setUnsafe(state, 3, (Array.getUnsafe(state, 3) + d.contents))
  Array.setUnsafe(state, 4, (Array.getUnsafe(state, 4) + e.contents))
  Array.setUnsafe(state, 5, (Array.getUnsafe(state, 5) + f.contents))
  Array.setUnsafe(state, 6, (Array.getUnsafe(state, 6) + g.contents))
  Array.setUnsafe(state, 7, (Array.getUnsafe(state, 7) + h.contents))
}

let compress512 = (state, data, at, wh, wl) => {
  for i in 0 to 15 {
    Array.setUnsafe(wh, i, readBe(data, at + i * 8))
    Array.setUnsafe(wl, i, readBe(data, at + i * 8 + 4))
  }
  for i in 16 to 79 {
    let xh = Array.getUnsafe(wh, i - 15)
    let xl = Array.getUnsafe(wl, i - 15)
    let yh = Array.getUnsafe(wh, i - 2)
    let yl = Array.getUnsafe(wl, i - 2)
    let s0h = bxor(bxor(bor(shr(xh, 1), shl(xl, 31)), bor(shr(xh, 8), shl(xl, 24))), shr(xh, 7))
    let s0l = bxor(bxor(bor(shr(xl, 1), shl(xh, 31)), bor(shr(xl, 8), shl(xh, 24))), bor(shr(xl, 7), shl(xh, 25)))
    let s1h = bxor(bxor(bor(shr(yh, 19), shl(yl, 13)), bor(shr(yl, 29), shl(yh, 3))), shr(yh, 6))
    let s1l = bxor(bxor(bor(shr(yl, 19), shl(yh, 13)), bor(shr(yh, 29), shl(yl, 3))), bor(shr(yl, 6), shl(yh, 26)))
    let nextl2 = (s0l + s1l)
    let nextl3 = (nextl2 + Array.getUnsafe(wl, i - 7))
    let nextl = (nextl3 + Array.getUnsafe(wl, i - 16))
    let nexth = (s0h + s1h + Array.getUnsafe(wh, i - 7) + Array.getUnsafe(wh, i - 16) + carry(s0l, s1l) + carry(nextl2, Array.getUnsafe(wl, i - 7)) + carry(nextl3, Array.getUnsafe(wl, i - 16)))
    Array.setUnsafe(wh, i, nexth)
    Array.setUnsafe(wl, i, nextl)
  }
  let ah = ref(Array.getUnsafe(state, 0))
  let al = ref(Array.getUnsafe(state, 1))
  let bh = ref(Array.getUnsafe(state, 2))
  let bl = ref(Array.getUnsafe(state, 3))
  let ch = ref(Array.getUnsafe(state, 4))
  let cl = ref(Array.getUnsafe(state, 5))
  let dh = ref(Array.getUnsafe(state, 6))
  let dl = ref(Array.getUnsafe(state, 7))
  let eh = ref(Array.getUnsafe(state, 8))
  let el = ref(Array.getUnsafe(state, 9))
  let fh = ref(Array.getUnsafe(state, 10))
  let fl = ref(Array.getUnsafe(state, 11))
  let gh = ref(Array.getUnsafe(state, 12))
  let gl = ref(Array.getUnsafe(state, 13))
  let hh = ref(Array.getUnsafe(state, 14))
  let hl = ref(Array.getUnsafe(state, 15))
  for i in 0 to 79 {
    let s1h = bxor(bxor(bor(shr(eh.contents, 14), shl(el.contents, 18)), bor(shr(eh.contents, 18), shl(el.contents, 14))), bor(shr(el.contents, 9), shl(eh.contents, 23)))
    let s1l = bxor(bxor(bor(shr(el.contents, 14), shl(eh.contents, 18)), bor(shr(el.contents, 18), shl(eh.contents, 14))), bor(shr(eh.contents, 9), shl(el.contents, 23)))
    let chh = choice(eh.contents, fh.contents, gh.contents)
    let chl = choice(el.contents, fl.contents, gl.contents)
    let t1l2 = (hl.contents + s1l)
    let t1l3 = (t1l2 + chl)
    let t1l4 = (t1l3 + Array.getUnsafe(k512l, i))
    let t1l = (t1l4 + Array.getUnsafe(wl, i))
    let t1h = (hh.contents + s1h + chh + Array.getUnsafe(k512h, i) + Array.getUnsafe(wh, i) + carry(hl.contents, s1l) + carry(t1l2, chl) + carry(t1l3, Array.getUnsafe(k512l, i)) + carry(t1l4, Array.getUnsafe(wl, i)))
    let s0h = bxor(bxor(bor(shr(ah.contents, 28), shl(al.contents, 4)), bor(shr(al.contents, 2), shl(ah.contents, 30))), bor(shr(al.contents, 7), shl(ah.contents, 25)))
    let s0l = bxor(bxor(bor(shr(al.contents, 28), shl(ah.contents, 4)), bor(shr(ah.contents, 2), shl(al.contents, 30))), bor(shr(ah.contents, 7), shl(al.contents, 25)))
    let majh = majority(ah.contents, bh.contents, ch.contents)
    let majl = majority(al.contents, bl.contents, cl.contents)
    let newAl2 = (t1l + s0l)
    let newAl = (newAl2 + majl)
    let newAh = (t1h + s0h + majh + carry(t1l, s0l) + carry(newAl2, majl))
    let newEl = (dl.contents + t1l)
    let newEh = (dh.contents + t1h + carry(dl.contents, t1l))
    hh := gh.contents
    hl := gl.contents
    gh := fh.contents
    gl := fl.contents
    fh := eh.contents
    fl := el.contents
    eh := newEh
    el := newEl
    dh := ch.contents
    dl := cl.contents
    ch := bh.contents
    cl := bl.contents
    bh := ah.contents
    bl := al.contents
    ah := newAh
    al := newAl
  }
  Array.setUnsafe(state, 0, (Array.getUnsafe(state, 0) + ah.contents + carry(Array.getUnsafe(state, 1), al.contents)))
  Array.setUnsafe(state, 1, (Array.getUnsafe(state, 1) + al.contents))
  Array.setUnsafe(state, 2, (Array.getUnsafe(state, 2) + bh.contents + carry(Array.getUnsafe(state, 3), bl.contents)))
  Array.setUnsafe(state, 3, (Array.getUnsafe(state, 3) + bl.contents))
  Array.setUnsafe(state, 4, (Array.getUnsafe(state, 4) + ch.contents + carry(Array.getUnsafe(state, 5), cl.contents)))
  Array.setUnsafe(state, 5, (Array.getUnsafe(state, 5) + cl.contents))
  Array.setUnsafe(state, 6, (Array.getUnsafe(state, 6) + dh.contents + carry(Array.getUnsafe(state, 7), dl.contents)))
  Array.setUnsafe(state, 7, (Array.getUnsafe(state, 7) + dl.contents))
  Array.setUnsafe(state, 8, (Array.getUnsafe(state, 8) + eh.contents + carry(Array.getUnsafe(state, 9), el.contents)))
  Array.setUnsafe(state, 9, (Array.getUnsafe(state, 9) + el.contents))
  Array.setUnsafe(state, 10, (Array.getUnsafe(state, 10) + fh.contents + carry(Array.getUnsafe(state, 11), fl.contents)))
  Array.setUnsafe(state, 11, (Array.getUnsafe(state, 11) + fl.contents))
  Array.setUnsafe(state, 12, (Array.getUnsafe(state, 12) + gh.contents + carry(Array.getUnsafe(state, 13), gl.contents)))
  Array.setUnsafe(state, 13, (Array.getUnsafe(state, 13) + gl.contents))
  Array.setUnsafe(state, 14, (Array.getUnsafe(state, 14) + hh.contents + carry(Array.getUnsafe(state, 15), hl.contents)))
  Array.setUnsafe(state, 15, (Array.getUnsafe(state, 15) + hl.contents))
}

let hash = (wide, chunks) => {
  let block = wide ? 128 : 64
  let state = Array.copy(wide ? iv512 : iv256)
  let wh = Array.make(~length=80, 0)
  let wl = Array.make(~length=80, 0)
  let tail = Array.make(~length=block, 0)
  let process = (data, at) => {
    if wide { compress512(state, data, at, wh, wl) } else { compress256(state, data, at, wh) }
  }
  let pos = ref(0)
  let length = ref(0.0)
  chunks->Array.forEach(chunk => {
    let len = Array.length(chunk)
    let at = ref(0)
    length := length.contents +. Int.toFloat(len)
    while at.contents < len {
      if pos.contents == 0 && len - at.contents >= block {
        process(chunk, at.contents)
        at := at.contents + block
      } else {
        let take = Math.Int.min(block - pos.contents, len - at.contents)
        for i in 0 to take - 1 {
          Array.setUnsafe(tail, pos.contents + i, Array.getUnsafe(chunk, at.contents + i))
        }
        pos := pos.contents + take
        at := at.contents + take
        if pos.contents == block { process(tail, 0); pos := 0 }
      }
    }
  })
  Array.setUnsafe(tail, pos.contents, 128)
  for i in pos.contents + 1 to block - 1 { Array.setUnsafe(tail, i, 0) }
  if pos.contents + 1 > block - block / 8 {
    process(tail, 0)
    for i in 0 to block - 1 { Array.setUnsafe(tail, i, 0) }
  }
  let bits = ref(length.contents *. 8.0)
  for i in 0 to 7 {
    Array.setUnsafe(tail, block - 1 - i, band(Int.fromFloat(bits.contents), 255))
    bits := Math.floor(bits.contents /. 256.0)
  }
  process(tail, 0)
  Array.fromInitializer(~length=Array.length(state)*4, i =>
    band(shr(Array.getUnsafe(state, i / 4), 24 - mod(i, 4)*8), 255)
  )
}

let digest = (mode, chunks, key) => {
  if mode != 2 { hash(mode == 1, chunks) } else {
    let k = Array.length(key) > 64 ? hash(false,[key]) : key
    let pad = mask => Array.fromInitializer(~length=64, i =>
      bxor(i < Array.length(k) ? Array.getUnsafe(k,i) : 0, mask))
    let inner = Array.concat([pad(0x36)],chunks)
    hash(false,[pad(0x5c), hash(false,inner)])
  }
}
