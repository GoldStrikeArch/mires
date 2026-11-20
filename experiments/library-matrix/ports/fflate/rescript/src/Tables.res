// Fixed tables, typed-array primitives, bit I/O helpers, canonical Huffman map construction
// (`hMap`), and `FlateError` matching `fflate@0.8.3` (`node_modules/fflate/esm/browser.js`).

// Bitwise primitives (compile directly to JS `&`, `|`, `^`, `<<`, `>>`, `>>>`, `~`).
external band: (int, int) => int = "%andint"
external bor: (int, int) => int = "%orint"
external bxor: (int, int) => int = "%xorint"
external shl: (int, int) => int = "%lslint"
external shr: (int, int) => int = "%asrint"
external ushr: (int, int) => int = "%lsrint"
external bnot: int => int = "%bitnot_int"

// Direct indexed access on JS typed arrays (compiles to `arr[i]` / `arr[i] = v`).
@get_index external getU8: (Uint8Array.t, int) => int = ""
@set_index external setU8: (Uint8Array.t, int, int) => unit = ""
@get_index external getU16: (Uint16Array.t, int) => int = ""
@set_index external setU16: (Uint16Array.t, int, int) => unit = ""
@get_index external getI32: (Int32Array.t, int) => int = ""
@set_index external setI32: (Int32Array.t, int, int) => unit = ""

@new external copyU8FromU8: Uint8Array.t => Uint8Array.t = "Uint8Array"
@new external copyU8FromU16: Uint16Array.t => Uint8Array.t = "Uint8Array"
@new external u8FromFloatLength: float => Uint8Array.t = "Uint8Array"

@send external setU8Slice: (Uint8Array.t, Uint8Array.t) => unit = "set"
@send external setU8SliceAt: (Uint8Array.t, Uint8Array.t, int) => unit = "set"

// Error codes (`FlateErrorCode`) and exception.
exception FlateError(int, string)

let errorMessages = [
  "unexpected EOF",
  "invalid block type",
  "invalid length/literal",
  "invalid distance",
  "stream finished",
  "no stream handler",
  "",
  "no callback",
  "invalid UTF-8 data",
]

let fail = (code: int, ~msg: string=""): 'a => {
  let message = msg != "" ? msg : errorMessages->Array.getUnsafe(code)
  throw(FlateError(code, message))
}

// Empty immutable Uint8Array sentinel (`et`).
let et = Uint8Array.fromLength(0)

// Bit I/O helpers.
let shft = (p: int): int => shr(p + 7, 3)

let bits = (d: Uint8Array.t, p: int, m: int): int => {
  let o = shr(p, 3)
  band(shr(bor(getU8(d, o), shl(getU8(d, o + 1), 8)), band(p, 7)), m)
}

let bits16 = (d: Uint8Array.t, p: int): int => {
  let o = shr(p, 3)
  shr(bor(bor(getU8(d, o), shl(getU8(d, o + 1), 8)), shl(getU8(d, o + 2), 16)), band(p, 7))
}

let wbits = (d: Uint8Array.t, p: int, v: int): unit => {
  let v = shl(v, band(p, 7))
  let o = shr(p, 3)
  setU8(d, o, bor(getU8(d, o), v))
  setU8(d, o + 1, bor(getU8(d, o + 1), shr(v, 8)))
}

let wbits16 = (d: Uint8Array.t, p: int, v: int): unit => {
  let v = shl(v, band(p, 7))
  let o = shr(p, 3)
  setU8(d, o, bor(getU8(d, o), v))
  setU8(d, o + 1, bor(getU8(d, o + 1), shr(v, 8)))
  setU8(d, o + 2, bor(getU8(d, o + 2), shr(v, 16)))
}

let wbytes = (d: Uint8Array.t, b: int, v: int): unit => {
  let b = ref(b)
  let v = ref(v)
  while v.contents != 0 {
    setU8(d, b.contents, v.contents)
    b := b.contents + 1
    v := ushr(v.contents, 8)
  }
}

// Fresh copy slice with clamped bounds (`slc`).
let slc = (v: Uint8Array.t, s: int, e: int): Uint8Array.t => {
  let len = v->TypedArray.length
  let s = s < 0 ? 0 : s
  let e = e > len ? len : e
  copyU8FromU8(v->TypedArray.subarray(~start=s, ~end=e))
}

let slcFrom = (v: Uint8Array.t, s: int): Uint8Array.t => {
  let s = s < 0 ? 0 : s
  copyU8FromU8(v->TypedArray.subarray(~start=s))
}

// Max element of a non-empty Uint8Array (`max`).
let maxU8 = (a: Uint8Array.t): int => {
  let len = a->TypedArray.length
  let m = ref(getU8(a, 0))
  for i in 1 to len - 1 {
    let v = getU8(a, i)
    if v > m.contents {
      m := v
    }
  }
  m.contents
}

// Fixed length extra bits (`fleb`).
let fleb = Uint8Array.fromArray([
  0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 0, 0, 0,
])

// Fixed distance extra bits (`fdeb`).
let fdeb = Uint8Array.fromArray([
  0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
  0, 0,
])

// Code length alphabet index order (`clim`).
let clim = Uint8Array.fromArray([16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15])

// Base and reverse lookup tables from extra bits (`freb`).
let freb = (eb: Uint8Array.t, start: int): (Uint16Array.t, Int32Array.t) => {
  let b = Uint16Array.fromLength(31)
  let acc = ref(start)
  for i in 0 to 30 {
    let shift = i == 0 ? 0 : getU8(eb, i - 1)
    acc := acc.contents + shl(1, shift)
    setU16(b, i, acc.contents)
  }
  let r = Int32Array.fromLength(getU16(b, 30))
  for i in 1 to 29 {
    let bi = getU16(b, i)
    let bnext = getU16(b, i + 1)
    for j in bi to bnext - 1 {
      setI32(r, j, bor(shl(j - bi, 5), i))
    }
  }
  (b, r)
}

let (fl, revfl) = {
  let (b, r) = freb(fleb, 2)
  setU16(b, 28, 258)
  setI32(r, 258, 28)
  (b, r)
}

let (fd, revfd) = freb(fdeb, 0)

// 15-bit reversal lookup table (`rev`, 0..32767).
let rev = {
  let t = Uint16Array.fromLength(32768)
  for i in 0 to 32767 {
    let x = bor(shr(band(i, 0xAAAA), 1), shl(band(i, 0x5555), 1))
    let x = bor(shr(band(x, 0xCCCC), 2), shl(band(x, 0x3333), 2))
    let x = bor(shr(band(x, 0xF0F0), 4), shl(band(x, 0x0F0F), 4))
    setU16(t, i, shr(bor(shr(band(x, 0xFF00), 8), shl(band(x, 0x00FF), 8)), 1))
  }
  t
}

// Canonical Huffman encode (`reverse = false`) or decode (`reverse = true`) table (`hMap`).
let hMap = (cd: Uint8Array.t, mb: int, reverse: bool): Uint16Array.t => {
  let s = cd->TypedArray.length
  let l = Uint16Array.fromLength(mb)
  for i in 0 to s - 1 {
    let ci = getU8(cd, i)
    if ci != 0 {
      setU16(l, ci - 1, getU16(l, ci - 1) + 1)
    }
  }
  let le = Uint16Array.fromLength(mb)
  for i in 1 to mb - 1 {
    setU16(le, i, shl(getU16(le, i - 1) + getU16(l, i - 1), 1))
  }
  if reverse {
    let co = Uint16Array.fromLength(shl(1, mb))
    let rvb = 15 - mb
    for i in 0 to s - 1 {
      let ci = getU8(cd, i)
      if ci != 0 {
        let sv = bor(shl(i, 4), ci)
        let r1 = mb - ci
        let cur = getU16(le, ci - 1)
        setU16(le, ci - 1, cur + 1)
        let v = shl(cur, r1)
        let m = bor(v, shl(1, r1) - 1)
        for x in v to m {
          setU16(co, shr(getU16(rev, x), rvb), sv)
        }
      }
    }
    co
  } else {
    let co = Uint16Array.fromLength(s)
    for i in 0 to s - 1 {
      let ci = getU8(cd, i)
      if ci != 0 {
        let cur = getU16(le, ci - 1)
        setU16(le, ci - 1, cur + 1)
        setU16(co, i, shr(getU16(rev, cur), 15 - ci))
      }
    }
    co
  }
}

// Fixed Huffman code length tables (`flt`, `fdt`) and maps (`flm`, `flrm`, `fdm`, `fdrm`).
let flt = {
  let t = Uint8Array.fromLength(288)
  for i in 0 to 143 {
    setU8(t, i, 8)
  }
  for i in 144 to 255 {
    setU8(t, i, 9)
  }
  for i in 256 to 279 {
    setU8(t, i, 7)
  }
  for i in 280 to 287 {
    setU8(t, i, 8)
  }
  t
}

let fdt = {
  let t = Uint8Array.fromLength(32)
  for i in 0 to 31 {
    setU8(t, i, 5)
  }
  t
}

let flm = hMap(flt, 9, false)
let flrm = hMap(flt, 9, true)
let fdm = hMap(fdt, 5, false)
let fdrm = hMap(fdt, 5, true)

// Deflate level options (`(nice << 13) | chain`, levels 1..9).
let deo = Int32Array.fromArray([
  65540, 131080, 131088, 131104, 262176, 1048704, 1048832, 2114560, 2117632,
])

// Reflected IEEE CRC-32 lookup table (`crct`).
let crct = {
  let t = Int32Array.fromLength(256)
  for i in 0 to 255 {
    let c = ref(i)
    for _ in 0 to 7 {
      c := bxor(band(c.contents, 1) != 0 ? -306674912 : 0, ushr(c.contents, 1))
    }
    setI32(t, i, c.contents)
  }
  t
}
