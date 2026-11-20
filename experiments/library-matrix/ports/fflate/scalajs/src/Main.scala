// Boundary for the fflate 0.8.3 Scala.js port: Scala port of libraries/fflate/reference.mjs.
// Reads the JSON corpus input through @js.native facade traits (no copy) and returns a raw
// js.typedarray.Uint8Array. Exported as a native ES module export via @JSExportTopLevel("run").

package fflate

import scala.scalajs.js
import scala.scalajs.js.annotation.{JSExportTopLevel, JSName}
import scala.scalajs.js.typedarray.Uint8Array

@js.native
trait PayloadJson extends js.Object:
  val bytes: js.UndefOr[js.Array[Int]] = js.native
  val text: js.UndefOr[String] = js.native
  val latin1: js.UndefOr[Boolean] = js.native

@js.native
trait OptionsJson extends js.Object:
  val level: js.UndefOr[Int] = js.native
  val mem: js.UndefOr[Int] = js.native
  val dictionary: js.UndefOr[PayloadJson] = js.native
  val mtime: js.UndefOr[Double] = js.native
  val filename: js.UndefOr[String] = js.native
  val outSize: js.UndefOr[Int] = js.native

@js.native
trait StreamStepJson extends js.Object:
  val action: String = js.native
  val data: js.UndefOr[PayloadJson] = js.native
  @JSName("final")
  val isFinal: js.UndefOr[Boolean] = js.native
  val sync: js.UndefOr[Boolean] = js.native

@js.native
trait Input extends js.Object:
  val op: String = js.native
  val data: js.UndefOr[PayloadJson] = js.native
  val text: js.UndefOr[String] = js.native
  val latin1: js.UndefOr[Boolean] = js.native
  val options: js.UndefOr[OptionsJson] = js.native
  val noHandler: js.UndefOr[Boolean] = js.native
  val steps: js.UndefOr[js.Array[StreamStepJson]] = js.native
  val expectErrorCode: js.UndefOr[Boolean] = js.native

object Main:
  @JSExportTopLevel("run")
  def run(input: Input): Uint8Array =
    val expectErr = !js.isUndefined(input.expectErrorCode) && input.expectErrorCode.asInstanceOf[Boolean]
    if !expectErr then execute(input)
    else
      try execute(input)
      catch
        case err: FlateError =>
          val res = new Uint8Array(2)
          res(0) = 0xff.toShort
          res(1) = err.code.toShort
          return res
      throw new RuntimeException("expected FlateError for op " + input.op + ", but call succeeded")

  private def readPayload(spec: PayloadJson): Array[Byte] =
    if !js.isUndefined(spec.bytes) then
      val arr = spec.bytes.asInstanceOf[js.Array[Int]]
      val len = arr.length
      val out = new Array[Byte](len)
      var i = 0
      while i < len do
        out(i) = arr(i).toByte
        i += 1
      out
    else
      val text = spec.text.asInstanceOf[String]
      val latin1 = !js.isUndefined(spec.latin1) && spec.latin1.asInstanceOf[Boolean]
      Codec.strToBytes(text, latin1)

  private def readCompressOptions(optJson: js.UndefOr[OptionsJson]): CompressOptions =
    if js.isUndefined(optJson) then CompressOptions.empty
    else
      val j = optJson.asInstanceOf[OptionsJson]
      val hasLevel = !js.isUndefined(j.level)
      val level = if hasLevel then j.level.asInstanceOf[Int] else 6
      val hasMem = !js.isUndefined(j.mem)
      val mem = if hasMem then j.mem.asInstanceOf[Int] else 0
      val dict = if !js.isUndefined(j.dictionary) then readPayload(j.dictionary.asInstanceOf[PayloadJson]) else null
      val hasMtime = !js.isUndefined(j.mtime)
      val mtime = if hasMtime then j.mtime.asInstanceOf[Double] else 0.0
      val filename = if !js.isUndefined(j.filename) then j.filename.asInstanceOf[String] else null
      new CompressOptions(hasLevel, level, hasMem, mem, dict, hasMtime, mtime, filename)

  private def readDecompressOptions(optJson: js.UndefOr[OptionsJson]): DecompressOptions =
    if js.isUndefined(optJson) then DecompressOptions.empty
    else
      val j = optJson.asInstanceOf[OptionsJson]
      val dict = if !js.isUndefined(j.dictionary) then readPayload(j.dictionary.asInstanceOf[PayloadJson]) else null
      val out = if !js.isUndefined(j.outSize) then new Array[Byte](j.outSize.asInstanceOf[Int]) else null
      new DecompressOptions(dict, out)

  private def frameRecords(chunks: js.Array[Uint8Array], finals: js.Array[Boolean]): Uint8Array =
    val count = chunks.length
    var total = 0
    var i = 0
    while i < count do
      total += 5 + chunks(i).length
      i += 1
    val out = new Uint8Array(total)
    var offset = 0
    i = 0
    while i < count do
      val chunk = chunks(i)
      val len = chunk.length
      out(offset) = (len & 0xff).toShort
      out(offset + 1) = ((len >>> 8) & 0xff).toShort
      out(offset + 2) = ((len >>> 16) & 0xff).toShort
      out(offset + 3) = ((len >>> 24) & 0xff).toShort
      out(offset + 4) = (if finals(i) then 1 else 0).toShort
      out.set(chunk, offset + 5)
      offset += 5 + len
      i += 1
    out

  private def runCompressStream(kind: String, input: Input): Uint8Array =
    val opts = readCompressOptions(input.options)
    val noHandler = !js.isUndefined(input.noHandler) && input.noHandler.asInstanceOf[Boolean]
    val chunks = new js.Array[Uint8Array]()
    val finals = new js.Array[Boolean]()
    val cb: DataHandler | Null =
      if noHandler then null
      else
        (chunk: Uint8Array, isFinal: Boolean) =>
          val copy = new Uint8Array(chunk.length)
          copy.set(chunk)
          chunks.push(copy)
          finals.push(isFinal)
    val stream: Deflate =
      if kind == "Deflate" then new Deflate(opts, cb)
      else if kind == "Gzip" then new Gzip(opts, cb)
      else new Zlib(opts, cb)
    val steps = input.steps.asInstanceOf[js.Array[StreamStepJson]]
    val stepCount = steps.length
    var i = 0
    while i < stepCount do
      val step = steps(i)
      if step.action == "push" then
        val isFinal = !js.isUndefined(step.isFinal) && step.isFinal.asInstanceOf[Boolean]
        stream.push(readPayload(step.data.asInstanceOf[PayloadJson]), isFinal)
      else if step.action == "flush" then
        val sync = !js.isUndefined(step.sync) && step.sync.asInstanceOf[Boolean]
        stream.flush(sync)
      else throw new IllegalArgumentException("unknown stream action " + step.action)
      i += 1
    frameRecords(chunks, finals)

  private def runDecompressStream(kind: String, input: Input): Uint8Array =
    val opts = readDecompressOptions(input.options)
    val noHandler = !js.isUndefined(input.noHandler) && input.noHandler.asInstanceOf[Boolean]
    val chunks = new js.Array[Uint8Array]()
    val finals = new js.Array[Boolean]()
    val cb: DataHandler | Null =
      if noHandler then null
      else
        (chunk: Uint8Array, isFinal: Boolean) =>
          val copy = new Uint8Array(chunk.length)
          copy.set(chunk)
          chunks.push(copy)
          finals.push(isFinal)
    val stream: DecompressStream =
      if kind == "Inflate" then new Inflate(opts, cb)
      else if kind == "Gunzip" then new Gunzip(opts, cb)
      else if kind == "Unzlib" then new Unzlib(opts, cb)
      else new Decompress(opts, cb)
    val steps = input.steps.asInstanceOf[js.Array[StreamStepJson]]
    val stepCount = steps.length
    var i = 0
    while i < stepCount do
      val step = steps(i)
      if step.action == "push" then
        val isFinal = !js.isUndefined(step.isFinal) && step.isFinal.asInstanceOf[Boolean]
        stream.push(readPayload(step.data.asInstanceOf[PayloadJson]), isFinal)
      else throw new IllegalArgumentException("unknown stream action " + step.action)
      i += 1
    frameRecords(chunks, finals)

  private def execute(input: Input): Uint8Array =
    input.op match
      case "deflateSync" => DeflateCore.deflateSync(readPayload(input.data.asInstanceOf[PayloadJson]), readCompressOptions(input.options))
      case "inflateSync" => InflateCore.inflateSync(readPayload(input.data.asInstanceOf[PayloadJson]), readDecompressOptions(input.options))
      case "gzipSync"    => Framing.gzipSync(readPayload(input.data.asInstanceOf[PayloadJson]), readCompressOptions(input.options))
      case "gunzipSync"  => Framing.gunzipSync(readPayload(input.data.asInstanceOf[PayloadJson]), readDecompressOptions(input.options))
      case "zlibSync"    => Framing.zlibSync(readPayload(input.data.asInstanceOf[PayloadJson]), readCompressOptions(input.options))
      case "unzlibSync"  => Framing.unzlibSync(readPayload(input.data.asInstanceOf[PayloadJson]), readDecompressOptions(input.options))
      case "decompressSync" =>
        Framing.decompressSync(readPayload(input.data.asInstanceOf[PayloadJson]), readDecompressOptions(input.options))
      case "strToU8" =>
        val latin1 = !js.isUndefined(input.latin1) && input.latin1.asInstanceOf[Boolean]
        Codec.strToU8(input.text.asInstanceOf[String], latin1)
      case "strFromU8" =>
        val latin1 = !js.isUndefined(input.latin1) && input.latin1.asInstanceOf[Boolean]
        Codec.stringToUtf16Le(Codec.strFromU8(readPayload(input.data.asInstanceOf[PayloadJson]), latin1))
      case "Deflate"    => runCompressStream("Deflate", input)
      case "Gzip"       => runCompressStream("Gzip", input)
      case "Zlib"       => runCompressStream("Zlib", input)
      case "Inflate"    => runDecompressStream("Inflate", input)
      case "Gunzip"     => runDecompressStream("Gunzip", input)
      case "Unzlib"     => runDecompressStream("Unzlib", input)
      case "Decompress" => runDecompressStream("Decompress", input)
      case other        => throw new IllegalArgumentException("unknown operation " + other)
