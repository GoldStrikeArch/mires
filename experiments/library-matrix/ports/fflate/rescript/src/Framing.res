// Checksums (CRC-32, Adler-32), GZIP/Zlib framing, format auto-detection (`decompressSync`), and
// string codecs (`strToU8`, `strFromU8`) matching `fflate@0.8.3`.

open Tables

// CRC-32 accumulator (`crc`).
type crcState = {mutable c: int}

let crcMake = (): crcState => {c: -1}

let crcPush = (st: crcState, d: Uint8Array.t): unit => {
  let len = d->TypedArray.length
  let cr = ref(st.c)
  for i in 0 to len - 1 {
    cr := bxor(getI32(crct, bxor(band(cr.contents, 255), getU8(d, i))), ushr(cr.contents, 8))
  }
  st.c = cr.contents
}

let crcDigest = (st: crcState): int => bnot(st.c)

// Adler-32 accumulator (`adler`).
type adlerState = {
  mutable a: int,
  mutable b: int,
}

let adlerMake = (): adlerState => {a: 1, b: 0}

let adlerPush = (st: adlerState, d: Uint8Array.t): unit => {
  let n = ref(st.a)
  let m = ref(st.b)
  let l = d->TypedArray.length
  let i = ref(0)
  while i.contents != l {
    let e = Math.Int.min(i.contents + 2655, l)
    while i.contents < e {
      n := n.contents + getU8(d, i.contents)
      m := m.contents + n.contents
      i := i.contents + 1
    }
    n := band(n.contents, 65535) + 15 * shr(n.contents, 16)
    m := band(m.contents, 65535) + 15 * shr(m.contents, 16)
  }
  st.a = n.contents
  st.b = m.contents
}

let adlerDigest = (st: adlerState): int => {
  let a = Int.mod(st.a, 65521)
  let b = Int.mod(st.b, 65521)
  st.a = a
  st.b = b
  bor(bor(bor(shl(band(a, 255), 24), shl(band(a, 0xFF00), 8)), shl(band(b, 255), 8)), shr(b, 8))
}

// GZIP header helpers (`gzhl`, `gzh`, `gzs`, `gzl`).
let gzhl = (opts: Deflate.compressOptions): int =>
  switch opts.filename {
  | Some(fn) if fn != "" => 11 + fn->String.length
  | _ => 10
  }

let gzh = (c: Uint8Array.t, opts: Deflate.compressOptions): unit => {
  setU8(c, 0, 31)
  setU8(c, 1, 139)
  setU8(c, 2, 8)
  let xfl = switch opts.level {
  | Some(lv) if lv < 2 => 4
  | Some(9) => 2
  | _ => 0
  }
  setU8(c, 8, xfl)
  setU8(c, 9, 3)
  switch opts.mtime {
  | Some(0.) => ()
  | Some(mtime) => wbytes(c, 4, Math.floor(mtime /. 1000.)->Int.fromFloat)
  | None => wbytes(c, 4, Math.floor(Date.now() /. 1000.)->Int.fromFloat)
  }
  switch opts.filename {
  | Some(fn) if fn != "" =>
    setU8(c, 3, 8)
    let fnLen = fn->String.length
    for i in 0 to fnLen - 1 {
      setU8(c, i + 10, fn->String.charCodeAtUnsafe(i))
    }
    setU8(c, fnLen + 10, 0)
  | _ => ()
  }
}

let gzs = (d: Uint8Array.t): int => {
  let len = d->TypedArray.length
  if len < 3 || getU8(d, 0) != 31 || getU8(d, 1) != 139 || getU8(d, 2) != 8 {
    fail(6, ~msg="invalid gzip data")
  }
  let flg = len > 3 ? getU8(d, 3) : 0
  let st = ref(10)
  if band(flg, 4) != 0 {
    st := st.contents + bor(getU8(d, 10), shl(getU8(d, 11), 8)) + 2
  }
  let zs = ref(band(shr(flg, 3), 1) + band(shr(flg, 4), 1))
  while zs.contents > 0 {
    let idx = st.contents
    st := idx + 1
    if idx >= len || getU8(d, idx) == 0 {
      zs := zs.contents - 1
    }
  }
  st.contents + band(flg, 2)
}

// GZIP ISIZE as an exact unsigned 32-bit float (ReScript strips `>>> 0` on 32-bit signed `int`).
let gzl = (d: Uint8Array.t): float => {
  let l = d->TypedArray.length
  let lo24 = bor(bor(getU8(d, l - 4), shl(getU8(d, l - 3), 8)), shl(getU8(d, l - 2), 16))
  Int.toFloat(lo24) +. Int.toFloat(getU8(d, l - 1)) *. 16777216.
}

let gzipSync = (data: Uint8Array.t, ~opts: Deflate.compressOptions={}): Uint8Array.t => {
  let c = crcMake()
  let l = data->TypedArray.length
  crcPush(c, data)
  let d = Deflate.dopt(data, opts, gzhl(opts), 8)
  let s = d->TypedArray.length
  gzh(d, opts)
  wbytes(d, s - 8, crcDigest(c))
  wbytes(d, s - 4, l)
  d
}

let gunzipSync = (data: Uint8Array.t, ~opts: Inflate.decompressOptions={}): Uint8Array.t => {
  let st = gzs(data)
  if st + 8 > data->TypedArray.length {
    fail(6, ~msg="invalid gzip data")
  }
  let outBuf = switch opts.out {
  | Some(b) => b
  | None => u8FromFloatLength(gzl(data))
  }
  Inflate.inflt(
    data->TypedArray.subarray(~start=st, ~end=-8),
    Inflate.makeInflateState(~i=2),
    ~buf=outBuf,
    ~dict=?opts.dictionary,
  )
}

// Zlib header helpers (`zlh`, `zls`).
let zlh = (c: Uint8Array.t, opts: Deflate.compressOptions): unit => {
  let fl = switch opts.level {
  | Some(0) => 0
  | Some(lv) if lv < 6 => 1
  | Some(9) => 3
  | _ => 2
  }
  let hasDict = opts.dictionary !== None
  let c1 = bor(shl(fl, 6), hasDict ? 32 : 0)
  let c1 = bor(c1, 31 - Int.mod(bor(shl(120, 8), c1), 31))
  setU8(c, 0, 120)
  setU8(c, 1, c1)
  switch opts.dictionary {
  | Some(dict) =>
    let h = adlerMake()
    adlerPush(h, dict)
    wbytes(c, 2, adlerDigest(h))
  | None => ()
  }
}

let zls = (d: Uint8Array.t, hasDict: bool): int => {
  let d0 = getU8(d, 0)
  let d1 = getU8(d, 1)
  if band(d0, 15) != 8 || shr(d0, 4) > 7 || Int.mod(bor(shl(d0, 8), d1), 31) != 0 {
    fail(6, ~msg="invalid zlib data")
  }
  let fdict = band(shr(d1, 5), 1)
  if (fdict != 0) != hasDict {
    fail(6, ~msg="invalid zlib data: " ++ (fdict != 0 ? "need" : "unexpected") ++ " dictionary")
  }
  band(shr(d1, 3), 4) + 2
}

let zlibSync = (data: Uint8Array.t, ~opts: Deflate.compressOptions={}): Uint8Array.t => {
  let a = adlerMake()
  adlerPush(a, data)
  let pre = opts.dictionary !== None ? 6 : 2
  let d = Deflate.dopt(data, opts, pre, 4)
  zlh(d, opts)
  wbytes(d, d->TypedArray.length - 4, adlerDigest(a))
  d
}

let unzlibSync = (data: Uint8Array.t, ~opts: Inflate.decompressOptions={}): Uint8Array.t => {
  let start = zls(data, opts.dictionary !== None)
  Inflate.inflt(
    data->TypedArray.subarray(~start, ~end=-4),
    Inflate.makeInflateState(~i=2),
    ~buf=?opts.out,
    ~dict=?opts.dictionary,
  )
}

let decompressSync = (data: Uint8Array.t, ~opts: Inflate.decompressOptions={}): Uint8Array.t => {
  let len = data->TypedArray.length
  if len >= 3 && getU8(data, 0) == 31 && getU8(data, 1) == 139 && getU8(data, 2) == 8 {
    gunzipSync(data, ~opts)
  } else {
    let d0 = getU8(data, 0)
    let d1 = getU8(data, 1)
    if band(d0, 15) != 8 || shr(d0, 4) > 7 || Int.mod(bor(shl(d0, 8), d1), 31) != 0 {
      Inflate.inflateSync(data, ~opts)
    } else {
      unzlibSync(data, ~opts)
    }
  }
}

// Host TextEncoder / TextDecoder bindings (stateless immutable instances, matching `fflate`).
type textEncoder
@new external makeTextEncoder: unit => textEncoder = "TextEncoder"
@send external encodeText: (textEncoder, string) => Uint8Array.t = "encode"

type textDecoder
@new external makeTextDecoder: unit => textDecoder = "TextDecoder"
@send external decodeText: (textDecoder, Uint8Array.t) => string = "decode"

@val @scope(("String", "fromCharCode"))
external fromCharCodeApply: (Nullable.t<unit>, Uint8Array.t) => string = "apply"

let textEncoder = makeTextEncoder()
let textDecoder = makeTextDecoder()

let strToU8 = (str: string, ~latin1=false): Uint8Array.t =>
  if latin1 {
    let len = str->String.length
    let ar = Uint8Array.fromLength(len)
    for i in 0 to len - 1 {
      setU8(ar, i, str->String.charCodeAtUnsafe(i))
    }
    ar
  } else {
    textEncoder->encodeText(str)
  }

let strFromU8 = (dat: Uint8Array.t, ~latin1=false): string =>
  if latin1 {
    let len = dat->TypedArray.length
    let r = ref("")
    let i = ref(0)
    while i.contents < len {
      let chunk = dat->TypedArray.subarray(~start=i.contents, ~end=i.contents + 16384)
      r := r.contents ++ fromCharCodeApply(Nullable.null, chunk)
      i := i.contents + 16384
    }
    r.contents
  } else {
    textDecoder->decodeText(dat)
  }
