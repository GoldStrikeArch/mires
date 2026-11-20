// Entry module of the ReScript port: mirrors `libraries/fflate/reference.mjs`.
// The corpus input JSON maps zero-copy onto ReScript records and `@tag` variants; one-shot ops
// return raw `Uint8Array` directly, `strFromU8` encodes exact UTF-16LE bytes, streaming ops frame
// every `ondata` callback as `[u32le(len), final, ...chunk]`, and `expectErrorCode` catches
// `FlateError` into `Uint8Array([0xff, code])`.

// Input variant constructors are matched from decoded corpus JSON, never constructed here.
@@warning("-37")

open Tables

type payload = {
  bytes?: array<int>,
  text?: string,
  latin1?: bool,
}

type compressOptionsJson = {
  level?: int,
  mem?: int,
  dictionary?: payload,
  mtime?: float,
  filename?: string,
}

type decompressOptionsJson = {
  dictionary?: payload,
  outSize?: int,
}

@tag("action")
type streamStep =
  | @as("push") Push({data: payload, final?: bool})
  | @as("flush") Flush({sync?: bool})

@tag("op")
type input =
  | @as("deflateSync")
  DeflateSync({data: payload, options?: compressOptionsJson, expectErrorCode?: bool})
  | @as("inflateSync")
  InflateSync({data: payload, options?: decompressOptionsJson, expectErrorCode?: bool})
  | @as("gzipSync")
  GzipSync({data: payload, options?: compressOptionsJson, expectErrorCode?: bool})
  | @as("gunzipSync")
  GunzipSync({data: payload, options?: decompressOptionsJson, expectErrorCode?: bool})
  | @as("zlibSync")
  ZlibSync({data: payload, options?: compressOptionsJson, expectErrorCode?: bool})
  | @as("unzlibSync")
  UnzlibSync({data: payload, options?: decompressOptionsJson, expectErrorCode?: bool})
  | @as("decompressSync")
  DecompressSync({data: payload, options?: decompressOptionsJson, expectErrorCode?: bool})
  | @as("strToU8") StrToU8({text: string, latin1?: bool, expectErrorCode?: bool})
  | @as("strFromU8") StrFromU8({data: payload, latin1?: bool, expectErrorCode?: bool})
  | @as("Deflate")
  Deflate({
      options?: compressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })
  | @as("Inflate")
  Inflate({
      options?: decompressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })
  | @as("Gzip")
  Gzip({
      options?: compressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })
  | @as("Gunzip")
  Gunzip({
      options?: decompressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })
  | @as("Zlib")
  Zlib({
      options?: compressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })
  | @as("Unzlib")
  Unzlib({
      options?: decompressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })
  | @as("Decompress")
  Decompress({
      options?: decompressOptionsJson,
      noHandler?: bool,
      steps: array<streamStep>,
      expectErrorCode?: bool,
    })

@get external expectErrorCodeField: input => option<bool> = "expectErrorCode"
@get external opField: input => string = "op"

let decodePayload = (spec: payload): Uint8Array.t =>
  switch spec.bytes {
  | Some(bytes) => Uint8Array.fromArray(bytes)
  | None =>
    let text = switch spec.text {
    | Some(t) => t
    | None => ""
    }
    let latin1 = switch spec.latin1 {
    | Some(true) => true
    | _ => false
    }
    Framing.strToU8(text, ~latin1)
  }

let toCompressOptions = (json: option<compressOptionsJson>): Deflate.compressOptions =>
  switch json {
  | None => {}
  | Some(o) => {
      level: ?o.level,
      mem: ?o.mem,
      dictionary: ?(
        switch o.dictionary {
        | Some(d) => Some(decodePayload(d))
        | None => None
        }
      ),
      mtime: ?o.mtime,
      filename: ?o.filename,
    }
  }

let toDecompressOptions = (json: option<decompressOptionsJson>): Inflate.decompressOptions =>
  switch json {
  | None => {}
  | Some(o) => {
      dictionary: ?(
        switch o.dictionary {
        | Some(d) => Some(decodePayload(d))
        | None => None
        }
      ),
      out: ?(
        switch o.outSize {
        | Some(sz) => Some(Uint8Array.fromLength(sz))
        | None => None
        }
      ),
    }
  }

let stringToUtf16Le = (text: string): Uint8Array.t => {
  let len = text->String.length
  let out = Uint8Array.fromLength(len * 2)
  for i in 0 to len - 1 {
    let unit = text->String.charCodeAtUnsafe(i)
    setU8(out, i * 2, band(unit, 0xff))
    setU8(out, i * 2 + 1, ushr(unit, 8))
  }
  out
}

let frameRecords = (records: array<(Uint8Array.t, bool)>): Uint8Array.t => {
  let total = ref(0)
  let count = records->Array.length
  for i in 0 to count - 1 {
    let (chunk, _) = records->Array.getUnsafe(i)
    total := total.contents + 5 + chunk->TypedArray.length
  }
  let out = Uint8Array.fromLength(total.contents)
  let offset = ref(0)
  for i in 0 to count - 1 {
    let (chunk, isFinal) = records->Array.getUnsafe(i)
    let len = chunk->TypedArray.length
    let off = offset.contents
    setU8(out, off, band(len, 0xff))
    setU8(out, off + 1, band(ushr(len, 8), 0xff))
    setU8(out, off + 2, band(ushr(len, 16), 0xff))
    setU8(out, off + 3, band(ushr(len, 24), 0xff))
    setU8(out, off + 4, isFinal ? 1 : 0)
    out->setU8SliceAt(chunk, off + 5)
    offset := off + 5 + len
  }
  out
}

let makeRecorder = (noHandler: option<bool>): (array<(Uint8Array.t, bool)>, option<Stream.ondata>) => {
  let records: array<(Uint8Array.t, bool)> = []
  let ondata = switch noHandler {
  | Some(true) => None
  | _ => Some((chunk, isFinal) => records->Array.push((copyU8FromU8(chunk), isFinal)))
  }
  (records, ondata)
}

let runCompressStream = (
  stream: Stream.compressStream,
  steps: array<streamStep>,
  records: array<(Uint8Array.t, bool)>,
): Uint8Array.t => {
  let len = steps->Array.length
  for i in 0 to len - 1 {
    switch steps->Array.getUnsafe(i) {
    | Push({data, ?final}) =>
      Stream.compressPush(stream, decodePayload(data), ~final=final === Some(true))
    | Flush({?sync}) => Stream.compressFlush(stream, ~sync=sync === Some(true))
    }
  }
  frameRecords(records)
}

let runDecompressSteps = (
  push: (Uint8Array.t, ~final: bool=?) => unit,
  steps: array<streamStep>,
  records: array<(Uint8Array.t, bool)>,
): Uint8Array.t => {
  let len = steps->Array.length
  for i in 0 to len - 1 {
    switch steps->Array.getUnsafe(i) {
    | Push({data, ?final}) => push(decodePayload(data), ~final=final === Some(true))
    | Flush(_) => JsError.TypeError.make("stream does not support flush")->JsError.throw
    }
  }
  frameRecords(records)
}

let execute = (input: input): Uint8Array.t =>
  switch input {
  | DeflateSync({data, ?options}) =>
    Deflate.deflateSync(decodePayload(data), ~opts=toCompressOptions(options))
  | InflateSync({data, ?options}) =>
    Inflate.inflateSync(decodePayload(data), ~opts=toDecompressOptions(options))
  | GzipSync({data, ?options}) =>
    Framing.gzipSync(decodePayload(data), ~opts=toCompressOptions(options))
  | GunzipSync({data, ?options}) =>
    Framing.gunzipSync(decodePayload(data), ~opts=toDecompressOptions(options))
  | ZlibSync({data, ?options}) =>
    Framing.zlibSync(decodePayload(data), ~opts=toCompressOptions(options))
  | UnzlibSync({data, ?options}) =>
    Framing.unzlibSync(decodePayload(data), ~opts=toDecompressOptions(options))
  | DecompressSync({data, ?options}) =>
    Framing.decompressSync(decodePayload(data), ~opts=toDecompressOptions(options))
  | StrToU8({text, ?latin1}) => Framing.strToU8(text, ~latin1=latin1 === Some(true))
  | StrFromU8({data, ?latin1}) =>
    stringToUtf16Le(Framing.strFromU8(decodePayload(data), ~latin1=latin1 === Some(true)))
  | Deflate({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeDeflate(~opts=toCompressOptions(options), ~ondata?)
    runCompressStream(stream, steps, records)
  | Gzip({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeGzip(~opts=toCompressOptions(options), ~ondata?)
    runCompressStream(stream, steps, records)
  | Zlib({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeZlib(~opts=toCompressOptions(options), ~ondata?)
    runCompressStream(stream, steps, records)
  | Inflate({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeInflate(~opts=toDecompressOptions(options), ~ondata?)
    runDecompressSteps(
      (chunk, ~final=?) => Stream.inflatePush(stream, chunk, ~final?),
      steps,
      records,
    )
  | Gunzip({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeGunzip(~opts=toDecompressOptions(options), ~ondata?)
    runDecompressSteps(
      (chunk, ~final=?) => Stream.gunzipPush(stream, chunk, ~final?),
      steps,
      records,
    )
  | Unzlib({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeUnzlib(~opts=toDecompressOptions(options), ~ondata?)
    runDecompressSteps(
      (chunk, ~final=?) => Stream.unzlibPush(stream, chunk, ~final?),
      steps,
      records,
    )
  | Decompress({?options, ?noHandler, steps}) =>
    let (records, ondata) = makeRecorder(noHandler)
    let stream = Stream.makeDecompress(~opts=toDecompressOptions(options), ~ondata?)
    runDecompressSteps(
      (chunk, ~final=?) => Stream.decompressPush(stream, chunk, ~final?),
      steps,
      records,
    )
  }

let run = (input: input): Uint8Array.t =>
  if expectErrorCodeField(input) === Some(true) {
    switch execute(input) {
    | _ =>
      JsError.make(`expected FlateError for op ${opField(input)}, but call succeeded`)->JsError.throw
    | exception FlateError(code, _) => Uint8Array.fromArray([0xff, code])
    }
  } else {
    execute(input)
  }
