// Entry of the Kotlin/JS fflate 0.8.3 port: mirrors libraries/fflate/reference.mjs.
//
// The boundary: the corpus input is read in place through external interfaces (no copy). Text
// payloads decode via host TextEncoder (UTF-8) or charCodeAt (Latin-1), zero-copy viewed as a
// Kotlin ByteArray (Int8Array); byte arrays are copied into a ByteArray. The result ByteArray is
// returned zero-copy as a raw JS Uint8Array over the same ArrayBuffer. `run` is a native ES module
// export through @JsExport.

import fflate.CompressOptions
import fflate.DecompressOptions
import fflate.DecompressStream
import fflate.DeflateStream
import fflate.FlateException
import fflate.GunzipStream
import fflate.GzipStream
import fflate.InflateStream
import fflate.UnzlibStream
import fflate.ZlibStream
import fflate.decompressSync
import fflate.deflateSync
import fflate.gunzipSync
import fflate.gzipSync
import fflate.inflateSync
import fflate.strFromU8
import fflate.strToU8
import fflate.stringToUtf16Le
import fflate.toUint8Array
import fflate.unzlibSync
import fflate.zlibSync
import org.khronos.webgl.Uint8Array

external interface PayloadJson {
    val text: String?
    val latin1: Boolean?
    val bytes: Array<Int>?
}

external interface OptionsJson {
    val level: Int?
    val mem: Int?
    val dictionary: PayloadJson?
    val mtime: Double?
    val filename: String?
    val outSize: Int?
}

external interface StreamStepJson {
    val action: String
    val data: PayloadJson?
    val final: Boolean?
    val sync: Boolean?
}

external interface InputJson {
    val op: String
    val data: PayloadJson?
    val text: String?
    val latin1: Boolean?
    val options: OptionsJson?
    val noHandler: Boolean?
    val steps: Array<StreamStepJson>?
    val expectErrorCode: Boolean?
}

private class RecordBuffer {
    private var chunks: Array<ByteArray?> = arrayOfNulls(8)
    private var finals: BooleanArray = BooleanArray(8)
    private var count: Int = 0
    private var totalBytes: Int = 0

    fun add(chunk: ByteArray, isFinal: Boolean) {
        if (count == chunks.size) {
            val newCap = chunks.size * 2
            val newChunks = arrayOfNulls<ByteArray>(newCap)
            for (i in 0 until count) newChunks[i] = chunks[i]
            val newFinals = BooleanArray(newCap)
            finals.copyInto(newFinals, 0, 0, count)
            chunks = newChunks
            finals = newFinals
        }
        chunks[count] = chunk
        finals[count] = isFinal
        count++
        totalBytes += 5 + chunk.size
    }

    fun frame(): ByteArray {
        val out = ByteArray(totalBytes)
        var offset = 0
        for (i in 0 until count) {
            val chunk = chunks[i]!!
            val len = chunk.size
            out[offset] = len.toByte()
            out[offset + 1] = (len ushr 8).toByte()
            out[offset + 2] = (len ushr 16).toByte()
            out[offset + 3] = (len ushr 24).toByte()
            out[offset + 4] = if (finals[i]) 1 else 0
            if (len > 0) chunk.copyInto(out, offset + 5, 0, len)
            offset += 5 + len
        }
        return out
    }
}

private fun payload(spec: PayloadJson): ByteArray {
    val bytes = spec.bytes
    if (bytes != null) {
        val len = bytes.size
        val out = ByteArray(len)
        for (i in 0 until len) {
            out[i] = bytes[i].toByte()
        }
        return out
    }
    return strToU8(spec.text!!, spec.latin1 == true)
}

private fun compressOptions(json: OptionsJson?): CompressOptions? {
    if (json == null) return null
    val dict = json.dictionary
    return CompressOptions(
        level = json.level,
        mem = json.mem,
        dictionary = if (dict != null) payload(dict) else null,
        mtime = json.mtime,
        filename = json.filename,
    )
}

private fun decompressOptions(json: OptionsJson?): DecompressOptions? {
    if (json == null) return null
    val dict = json.dictionary
    val outSize = json.outSize
    return DecompressOptions(
        dictionary = if (dict != null) payload(dict) else null,
        out = if (outSize != null) ByteArray(outSize) else null,
    )
}

private fun runCompressStream(input: InputJson, stream: DeflateStream, records: RecordBuffer): ByteArray {
    val steps = input.steps!!
    for (i in 0 until steps.size) {
        val step = steps[i]
        when (step.action) {
            "push" -> stream.push(payload(step.data!!), step.final == true)
            "flush" -> stream.flush(step.sync == true)
            else -> throw IllegalArgumentException("unknown stream action ${step.action}")
        }
    }
    return records.frame()
}

private fun runInflateStream(input: InputJson, stream: InflateStream, records: RecordBuffer): ByteArray {
    val steps = input.steps!!
    for (i in 0 until steps.size) {
        val step = steps[i]
        when (step.action) {
            "push" -> stream.push(payload(step.data!!), step.final == true)
            else -> throw IllegalArgumentException("unknown stream action ${step.action}")
        }
    }
    return records.frame()
}

private fun runDecompressStream(input: InputJson, stream: DecompressStream, records: RecordBuffer): ByteArray {
    val steps = input.steps!!
    for (i in 0 until steps.size) {
        val step = steps[i]
        when (step.action) {
            "push" -> stream.push(payload(step.data!!), step.final == true)
            else -> throw IllegalArgumentException("unknown stream action ${step.action}")
        }
    }
    return records.frame()
}

private fun execute(input: InputJson): ByteArray = when (input.op) {
    "deflateSync" -> deflateSync(payload(input.data!!), compressOptions(input.options))
    "inflateSync" -> inflateSync(payload(input.data!!), decompressOptions(input.options))
    "gzipSync" -> gzipSync(payload(input.data!!), compressOptions(input.options))
    "gunzipSync" -> gunzipSync(payload(input.data!!), decompressOptions(input.options))
    "zlibSync" -> zlibSync(payload(input.data!!), compressOptions(input.options))
    "unzlibSync" -> unzlibSync(payload(input.data!!), decompressOptions(input.options))
    "decompressSync" -> decompressSync(payload(input.data!!), decompressOptions(input.options))
    "strToU8" -> strToU8(input.text!!, input.latin1 == true)
    "strFromU8" -> stringToUtf16Le(strFromU8(payload(input.data!!), input.latin1 == true))
    "Deflate" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runCompressStream(input, DeflateStream(compressOptions(input.options), cb), records)
    }
    "Gzip" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runCompressStream(input, GzipStream(compressOptions(input.options), cb), records)
    }
    "Zlib" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runCompressStream(input, ZlibStream(compressOptions(input.options), cb), records)
    }
    "Inflate" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runInflateStream(input, InflateStream(decompressOptions(input.options), cb), records)
    }
    "Gunzip" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runInflateStream(input, GunzipStream(decompressOptions(input.options), cb), records)
    }
    "Unzlib" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runInflateStream(input, UnzlibStream(decompressOptions(input.options), cb), records)
    }
    "Decompress" -> {
        val records = RecordBuffer()
        val cb: ((ByteArray, Boolean) -> Unit)? = if (input.noHandler == true) null else { c, f -> records.add(c, f) }
        runDecompressStream(input, DecompressStream(decompressOptions(input.options), cb), records)
    }
    else -> throw IllegalArgumentException("unknown operation ${input.op}")
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: InputJson): Uint8Array {
    if (input.expectErrorCode != true) return execute(input).toUint8Array()
    try {
        execute(input)
    } catch (error: FlateException) {
        val res = ByteArray(2)
        res[0] = 0xFF.toByte()
        res[1] = error.code.toByte()
        return res.toUint8Array()
    }
    throw RuntimeException("expected FlateError for op ${input.op}, but call succeeded")
}
