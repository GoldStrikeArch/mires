package noblehashes

/** noble's Hash: an incremental hash with its block and digest lengths. */
trait Hash:
  def blockLen: Int
  def outputLen: Int
  def update(data: Array[Byte]): Hash
  def digest(): Array[Byte]

/** The parts of noble's utils.ts the selected hashes use, and the byte-order reads and writes of
  * its DataView and Uint32Array views.
  */
object Utils:
  /** noble's anumber: `n` must be a safe integer >= 0. Values above Int.MaxValue saturate. */
  def anumber(n: Double, title: String): Int =
    require(n >= 0 && n <= 9007199254740991.0 && n == math.floor(n), s"\"$title\" expected integer >= 0, got $n")
    n.toInt

  /** DataView.getUint32(offset, false). */
  def getUint32BE(data: Array[Byte], offset: Int): Int =
    (data(offset) << 24) | ((data(offset + 1) & 0xff) << 16) | ((data(offset + 2) & 0xff) << 8) | (data(offset + 3) & 0xff)

  /** A little-endian 32-bit word, as noble's Uint32Array views read on little-endian hosts. */
  def getUint32LE(data: Array[Byte], offset: Int): Int =
    (data(offset) & 0xff) | ((data(offset + 1) & 0xff) << 8) | ((data(offset + 2) & 0xff) << 16) | (data(offset + 3) << 24)

  /** DataView.setUint32(offset, value, false). */
  def setUint32BE(data: Array[Byte], offset: Int, value: Int): Unit =
    for i <- 0 until 4 do data(offset + i) = (value >>> (24 - 8 * i)).toByte

  /** The first `count` bytes of the little-endian encoding of `words`. */
  def wordsToBytesLE(words: Array[Int], count: Int): Array[Byte] =
    Array.tabulate(count)(i => (words(i / 4) >>> (8 * (i % 4))).toByte)

  /** noble's bytesToHex. */
  def bytesToHex(bytes: Array[Byte]): String =
    val digits = "0123456789abcdef"
    val out = new StringBuilder(bytes.length * 2)
    for b <- bytes do
      out += digits((b >> 4) & 15)
      out += digits(b & 15)
    out.toString
