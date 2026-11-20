package noblehashes

import Utils.*

/** noble's _md.ts: the Merkle–Damgård base class of SHA-256 and SHA-512, and its Chi and Maj. */
object Md:
  /** Bits of `b` where `a` is set, else of `c`. */
  def chi(a: Int, b: Int, c: Int): Int = (a & b) ^ (~a & c)

  /** The majority of each bit of `a`, `b` and `c`. */
  def maj(a: Int, b: Int, c: Int): Int = (a & b) ^ (a & c) ^ (b & c)

/** noble's HashMD: buffers partial blocks, then pads with 0x80, zeros and the bit length. */
abstract class HashMD(val blockLen: Int, val outputLen: Int, padOffset: Int) extends Hash:
  private val buffer = new Array[Byte](blockLen)
  private var length = 0.0
  private var pos = 0

  /** Compresses the `blockLen` bytes of `data` at `offset`. */
  protected def process(data: Array[Byte], offset: Int): Unit

  /** The state words, the high half before the low one for 64-bit words. */
  protected def get(): Array[Int]

  def update(data: Array[Byte]): this.type =
    val len = data.length
    var at = 0
    while at < len do
      val take = math.min(blockLen - pos, len - at)
      // With nothing buffered, whole blocks are compressed straight from the input.
      if take == blockLen then
        while blockLen <= len - at do
          process(data, at)
          at += blockLen
      else
        System.arraycopy(data, at, buffer, pos, take)
        pos += take
        at += take
        if pos == blockLen then
          process(buffer, 0)
          pos = 0
    length += len
    this

  def digest(): Array[Byte] =
    buffer(pos) = 0x80.toByte
    java.util.Arrays.fill(buffer, pos + 1, blockLen, 0.toByte)
    // No room for the length field: pad this block and put the length in the next.
    if padOffset > blockLen - (pos + 1) then
      process(buffer, 0)
      java.util.Arrays.fill(buffer, 0.toByte)
    // noble's setU64FromNum: only the low 64 bits of the length field; the rest stays zero.
    val bits = length * 8
    setUint32BE(buffer, blockLen - 8, U64.fromNumH(bits))
    setUint32BE(buffer, blockLen - 4, U64.fromNumL(bits))
    process(buffer, 0)
    val state = get()
    val out = new Array[Byte](outputLen)
    for i <- 0 until outputLen / 4 do setUint32BE(out, 4 * i, state(i))
    out
