// All 7 synchronous streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip`, `Unzlib`,
// `Decompress`) matching `fflate@0.8.3` emission boundaries and recursion semantics.

open Tables

type ondata = (Uint8Array.t, bool) => unit

// --- Compression streams (`Deflate`, `Gzip`, `Zlib`) ---

type compressMode =
  | RawDeflate
  | GzipStream({crc: Framing.crcState, mutable l: int, mutable v: int})
  | ZlibStream({adler: Framing.adlerState, mutable v: int})

type compressStream = {
  ondata: option<ondata>,
  mutable o: Deflate.compressOptions,
  mutable s: Deflate.deflateState,
  mutable b: Uint8Array.t,
  mode: compressMode,
}

let makeCompressStream = (
  mode: compressMode,
  ~opts: Deflate.compressOptions={},
  ~ondata: option<ondata>=?,
): compressStream => {
  let s = Deflate.makeDeflateState(~l=0, ~i=32768, ~w=32768, ~z=32768)
  let b = Uint8Array.fromLength(98304)
  switch opts.dictionary {
  | Some(d) =>
    let dict = d->TypedArray.subarray(~start=-32768)
    let dictLen = dict->TypedArray.length
    b->setU8SliceAt(dict, 32768 - dictLen)
    s.i = 32768 - dictLen
  | None => ()
  }
  {ondata, o: opts, s, b, mode}
}

let makeDeflate = (~opts: Deflate.compressOptions={}, ~ondata: option<ondata>=?) =>
  makeCompressStream(RawDeflate, ~opts, ~ondata?)

let makeGzip = (~opts: Deflate.compressOptions={}, ~ondata: option<ondata>=?) =>
  makeCompressStream(GzipStream({crc: Framing.crcMake(), l: 0, v: 1}), ~opts, ~ondata?)

let makeZlib = (~opts: Deflate.compressOptions={}, ~ondata: option<ondata>=?) =>
  makeCompressStream(ZlibStream({adler: Framing.adlerMake(), v: 1}), ~opts, ~ondata?)

let compressProcess = (self: compressStream, c: Uint8Array.t, final: bool, cb: ondata): unit =>
  switch self.mode {
  | RawDeflate => cb(Deflate.dopt(c, self.o, 0, 0, ~st=self.s), final)
  | GzipStream(g) =>
    let pre = g.v != 0 ? Framing.gzhl(self.o) : 0
    let post = final ? 8 : 0
    let raw = Deflate.dopt(c, self.o, pre, post, ~st=self.s)
    if g.v != 0 {
      Framing.gzh(raw, self.o)
      g.v = 0
    }
    if final {
      let rawLen = raw->TypedArray.length
      wbytes(raw, rawLen - 8, Framing.crcDigest(g.crc))
      wbytes(raw, rawLen - 4, g.l)
    }
    cb(raw, final)
  | ZlibStream(z) =>
    let pre = z.v != 0 ? (self.o.dictionary !== None ? 6 : 2) : 0
    let post = final ? 4 : 0
    let raw = Deflate.dopt(c, self.o, pre, post, ~st=self.s)
    if z.v != 0 {
      Framing.zlh(raw, self.o)
      z.v = 0
    }
    if final {
      wbytes(raw, raw->TypedArray.length - 4, Framing.adlerDigest(z.adler))
    }
    cb(raw, final)
  }

let compressPush = (self: compressStream, chunk: Uint8Array.t, ~final=false): unit => {
  switch self.mode {
  | RawDeflate => ()
  | GzipStream(g) =>
    Framing.crcPush(g.crc, chunk)
    g.l = g.l + chunk->TypedArray.length
  | ZlibStream(z) => Framing.adlerPush(z.adler, chunk)
  }
  let cb = switch self.ondata {
  | Some(fn) => fn
  | None => fail(5)
  }
  if self.s.l != 0 {
    fail(4)
  }
  let chunkLen = chunk->TypedArray.length
  let endLen = chunkLen + self.s.z
  if endLen > self.b->TypedArray.length {
    if endLen > 2 * self.b->TypedArray.length - 32768 {
      let newBuf = Uint8Array.fromLength(band(endLen, -32768))
      newBuf->setU8Slice(self.b->TypedArray.subarray(~start=0, ~end=self.s.z))
      self.b = newBuf
    }
    let split = self.b->TypedArray.length - self.s.z
    self.b->setU8SliceAt(chunk->TypedArray.subarray(~start=0, ~end=split), self.s.z)
    self.s.z = self.b->TypedArray.length
    compressProcess(self, self.b, false, cb)
    self.b->setU8Slice(self.b->TypedArray.subarray(~start=-32768))
    self.b->setU8SliceAt(chunk->TypedArray.subarray(~start=split), 32768)
    self.s.z = chunkLen - split + 32768
    self.s.i = 32766
    self.s.w = 32768
  } else {
    self.b->setU8SliceAt(chunk, self.s.z)
    self.s.z = self.s.z + chunkLen
  }
  self.s.l = final ? 1 : 0
  if self.s.z > self.s.w + 8191 || final {
    compressProcess(self, self.b, final, cb)
    self.s.w = self.s.i
    self.s.i = self.s.i - 2
  }
  if final {
    self.s = Deflate.makeDeflateState()
    self.o = {}
    self.b = et
  }
}

let compressFlush = (self: compressStream, ~sync=false): unit => {
  let cb = switch self.ondata {
  | Some(fn) => fn
  | None => fail(5)
  }
  if self.s.l != 0 {
    fail(4)
  }
  compressProcess(self, self.b, false, cb)
  self.s.w = self.s.i
  self.s.i = self.s.i - 2
  if sync {
    let c = Uint8Array.fromLength(6)
    setU8(c, 0, shr(self.s.r, 3))
    let ep = Deflate.wfblk(c, self.s.r, et)
    self.s.r = 0
    cb(c->TypedArray.subarray(~start=0, ~end=shr(ep, 3)), false)
  }
}

// --- Decompression streams (`Inflate`, `Gunzip`, `Unzlib`, `Decompress`) ---

type inflateStream = {
  ondata: option<ondata>,
  mutable s: Inflate.inflateState,
  mutable o: Uint8Array.t,
  mutable p: Uint8Array.t,
  mutable d: bool,
}

let makeInflate = (~opts: Inflate.decompressOptions={}, ~ondata: option<ondata>=?): inflateStream => {
  let o = Uint8Array.fromLength(32768)
  let dictLen = switch opts.dictionary {
  | Some(d) =>
    let dict = d->TypedArray.subarray(~start=-32768)
    o->setU8Slice(dict)
    dict->TypedArray.length
  | None => 0
  }
  {
    ondata,
    s: Inflate.makeInflateState(~i=0, ~b=dictLen),
    o,
    p: Uint8Array.fromLength(0),
    d: false,
  }
}

let inflateEnqueue = (self: inflateStream, c: Uint8Array.t): unit => {
  if self.ondata === None {
    fail(5)
  }
  if self.d {
    fail(4)
  }
  let pLen = self.p->TypedArray.length
  let cLen = c->TypedArray.length
  if pLen == 0 {
    self.p = c
  } else if cLen != 0 {
    let n = Uint8Array.fromLength(pLen + cLen)
    n->setU8Slice(self.p)
    n->setU8SliceAt(c, pLen)
    self.p = n
  }
}

let inflateConsume = (self: inflateStream, final: bool): unit => {
  self.d = final
  self.s.i = final ? 1 : 0
  let bts = self.s.b
  let dt = Inflate.inflt(self.p, self.s, ~buf=self.o)
  switch self.ondata {
  | Some(cb) => cb(slc(dt, bts, self.s.b), self.d)
  | None => fail(5)
  }
  self.o = slcFrom(dt, self.s.b - 32768)
  self.s.b = self.o->TypedArray.length
  self.p = slcFrom(self.p, shr(self.s.p, 3))
  self.s.p = band(self.s.p, 7)
}

let inflatePush = (self: inflateStream, chunk: Uint8Array.t, ~final=false): unit => {
  inflateEnqueue(self, chunk)
  inflateConsume(self, final)
}

type gunzipStream = {
  base: inflateStream,
  mutable v: int,
  mutable r: int,
}

let makeGunzip = (~opts: Inflate.decompressOptions={}, ~ondata: option<ondata>=?): gunzipStream => {
  base: makeInflate(~opts, ~ondata?),
  v: 1,
  r: 0,
}

let rec gunzipPush = (self: gunzipStream, chunk: Uint8Array.t, ~final=false): unit => {
  inflateEnqueue(self.base, chunk)
  self.r = self.r + chunk->TypedArray.length
  let doneEarly = ref(false)
  if self.v != 0 {
    let p = self.base.p->TypedArray.subarray(~start=self.v - 1)
    let pLen = p->TypedArray.length
    let s = pLen > 3 ? Framing.gzs(p) : 4
    if s > pLen && !final {
      doneEarly := true
    } else {
      self.base.p = p->TypedArray.subarray(~start=s)
      self.v = 0
    }
  }
  if !doneEarly.contents {
    inflateConsume(self.base, false)
    if self.base.s.f != 0 && self.base.s.l === None {
      self.v = shft(self.base.s.p) + 9
      self.base.s = Inflate.makeInflateState(~i=0)
      self.base.o = Uint8Array.fromLength(0)
      gunzipPush(self, Uint8Array.fromLength(0), ~final)
    } else if final {
      inflateConsume(self.base, true)
    }
  }
}

type unzlibStream = {
  base: inflateStream,
  mutable v: int,
}

let makeUnzlib = (~opts: Inflate.decompressOptions={}, ~ondata: option<ondata>=?): unzlibStream => {
  base: makeInflate(~opts, ~ondata?),
  v: opts.dictionary !== None ? 2 : 1,
}

let unzlibPush = (self: unzlibStream, chunk: Uint8Array.t, ~final=false): unit => {
  inflateEnqueue(self.base, chunk)
  let doneEarly = ref(false)
  if self.v != 0 {
    if self.base.p->TypedArray.length < 6 && !final {
      doneEarly := true
    } else {
      self.base.p = self.base.p->TypedArray.subarray(~start=Framing.zls(self.base.p, self.v - 1 != 0))
      self.v = 0
    }
  }
  if !doneEarly.contents {
    if final {
      if self.base.p->TypedArray.length < 4 {
        fail(6, ~msg="invalid zlib data")
      }
      self.base.p = self.base.p->TypedArray.subarray(~start=0, ~end=-4)
    }
    inflateConsume(self.base, final)
  }
}

type innerDecompress =
  | InnerGunzip(gunzipStream)
  | InnerInflate(inflateStream)
  | InnerUnzlib(unzlibStream)

type decompressStream = {
  o: Inflate.decompressOptions,
  ondata: option<ondata>,
  mutable s: option<innerDecompress>,
  mutable p: option<Uint8Array.t>,
}

let makeDecompress = (
  ~opts: Inflate.decompressOptions={},
  ~ondata: option<ondata>=?,
): decompressStream => {
  o: opts,
  ondata,
  s: None,
  p: None,
}

let decompressPush = (self: decompressStream, chunk: Uint8Array.t, ~final=false): unit => {
  if self.ondata === None {
    fail(5)
  }
  switch self.s {
  | None =>
    let buf = switch self.p {
    | Some(prev) if prev->TypedArray.length != 0 =>
      let prevLen = prev->TypedArray.length
      let n = Uint8Array.fromLength(prevLen + chunk->TypedArray.length)
      n->setU8Slice(prev)
      n->setU8SliceAt(chunk, prevLen)
      self.p = Some(n)
      n
    | _ =>
      self.p = Some(chunk)
      chunk
    }
    if buf->TypedArray.length > 2 {
      let inner = if getU8(buf, 0) == 31 && getU8(buf, 1) == 139 && getU8(buf, 2) == 8 {
        InnerGunzip(makeGunzip(~opts=self.o, ~ondata=?self.ondata))
      } else {
        let d0 = getU8(buf, 0)
        let d1 = getU8(buf, 1)
        if band(d0, 15) != 8 || shr(d0, 4) > 7 || Int.mod(bor(shl(d0, 8), d1), 31) != 0 {
          InnerInflate(makeInflate(~opts=self.o, ~ondata=?self.ondata))
        } else {
          InnerUnzlib(makeUnzlib(~opts=self.o, ~ondata=?self.ondata))
        }
      }
      self.s = Some(inner)
      self.p = None
      switch inner {
      | InnerGunzip(g) => gunzipPush(g, buf, ~final)
      | InnerInflate(inf) => inflatePush(inf, buf, ~final)
      | InnerUnzlib(u) => unzlibPush(u, buf, ~final)
      }
    }
  | Some(InnerGunzip(g)) => gunzipPush(g, chunk, ~final)
  | Some(InnerInflate(inf)) => inflatePush(inf, chunk, ~final)
  | Some(InnerUnzlib(u)) => unzlibPush(u, chunk, ~final)
  }
}
