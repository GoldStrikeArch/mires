package noblehashes

import Utils.*

/** noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.
  *
  * The state is noble's state32: 25 lanes of two 32-bit words, the low word first. noble derives
  * the pi, rho and iota tables at load time; they are written out here.
  */
object Sha3:
  /** Pi: the word index each rho step moves its lane to (noble's SHA3_PI). */
  private val SHA3_PI = Array(20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2)

  /** Rho: the left rotation of each step (noble's SHA3_ROTL). */
  private val SHA3_ROTL = Array(1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44)

  /** Iota: the low and high words of each round constant (noble's SHA3_IOTA_H and SHA3_IOTA_L). */
  private val SHA3_IOTA_H = Array(
    0x00000001, 0x00008082, 0x0000808a, 0x80008000, 0x0000808b, 0x80000001, 0x80008081, 0x00008009,
    0x0000008a, 0x00000088, 0x80008009, 0x8000000a, 0x8000808b, 0x0000008b, 0x00008089, 0x00008003,
    0x00008002, 0x00000080, 0x0000800a, 0x8000000a, 0x80008081, 0x00008080, 0x80000001, 0x80008008,
  )
  private val SHA3_IOTA_L = Array(
    0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000,
    0x80000000, 0x80000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000, 0x00000000, 0x80000000,
  )

  // Left rotations of a lane by s in 1..31 (S) or 33..63 (B). As in noble, h is the lane's first
  // (low) word and l its second (high) word.
  private def rotlSH(h: Int, l: Int, s: Int) = (h << s) | (l >>> (32 - s))
  private def rotlSL(h: Int, l: Int, s: Int) = (l << s) | (h >>> (32 - s))
  private def rotlBH(h: Int, l: Int, s: Int) = (l << (s - 32)) | (h >>> (64 - s))
  private def rotlBL(h: Int, l: Int, s: Int) = (h << (s - 32)) | (l >>> (64 - s))
  private def rotlH(h: Int, l: Int, s: Int) = if s > 32 then rotlBH(h, l, s) else rotlSH(h, l, s)
  private def rotlL(h: Int, l: Int, s: Int) = if s > 32 then rotlBL(h, l, s) else rotlSL(h, l, s)

  /** noble's keccakP: the 24 rounds of Keccak-f[1600] on `s`, with `b` as the theta scratch. */
  def keccakP(s: Array[Int], b: Array[Int]): Unit =
    for round <- 0 until 24 do
      // Theta
      for x <- 0 until 10 do b(x) = s(x) ^ s(x + 10) ^ s(x + 20) ^ s(x + 30) ^ s(x + 40)
      for column <- 0 until 5 do
        val x = 2 * column
        val idx1 = (x + 8) % 10
        val idx0 = (x + 2) % 10
        val b0 = b(idx0)
        val b1 = b(idx0 + 1)
        val th = rotlH(b0, b1, 1) ^ b(idx1)
        val tl = rotlL(b0, b1, 1) ^ b(idx1 + 1)
        for row <- 0 until 5 do
          s(x + 10 * row) ^= th
          s(x + 10 * row + 1) ^= tl
      // Rho and pi
      var curH = s(2)
      var curL = s(3)
      for t <- 0 until 24 do
        val shift = SHA3_ROTL(t)
        val th = rotlH(curH, curL, shift)
        val tl = rotlL(curH, curL, shift)
        val pi = SHA3_PI(t)
        curH = s(pi)
        curL = s(pi + 1)
        s(pi) = th
        s(pi + 1) = tl
      // Chi
      for row <- 0 until 5 do
        val y = 10 * row
        val b0 = s(y)
        val b1 = s(y + 1)
        val b2 = s(y + 2)
        val b3 = s(y + 3)
        for x <- 0 until 6 do s(y + x) ^= ~s(y + x + 2) & s(y + x + 4)
        s(y + 6) ^= ~s(y + 8) & b0
        s(y + 7) ^= ~s(y + 9) & b1
        s(y + 8) ^= ~b0 & b2
        s(y + 9) ^= ~b1 & b3
      // Iota
      s(0) ^= SHA3_IOTA_H(round)
      s(1) ^= SHA3_IOTA_L(round)

/** noble's Keccak sponge with a byte-length rate `blockLen` and domain `suffix`. */
final class Keccak(val blockLen: Int, suffix: Int, val outputLen: Int) extends Hash:
  private val state32 = new Array[Int](50)
  private val theta = new Array[Int](10)
  private var pos = 0
  private var posOut = 0

  /** XORs `byte` into state byte `index`, as noble does through its byte view of state32. */
  private def xorByte(index: Int, byte: Int): Unit =
    state32(index >> 2) ^= byte << (8 * (index & 3))

  /** The state byte at `index`. */
  private def stateByte(index: Int): Byte = (state32(index >> 2) >>> (8 * (index & 3))).toByte

  private def keccak(): Unit =
    Sha3.keccakP(state32, theta)
    posOut = 0
    pos = 0

  def update(data: Array[Byte]): this.type =
    val len = data.length
    var at = 0
    while at < len do
      // Whole blocks with nothing buffered are absorbed a word at a time.
      if pos == 0 && len - at >= blockLen then
        for i <- 0 until blockLen / 4 do state32(i) ^= getUint32LE(data, at + 4 * i)
        at += blockLen
        pos = blockLen
        keccak()
      else
        val take = math.min(blockLen - pos, len - at)
        for _ <- 0 until take do
          xorByte(pos, data(at) & 0xff)
          pos += 1
          at += 1
        if pos == blockLen then keccak()
    this

  private def finish(): Unit =
    // The suffix holds the domain bits and the first padding bit; 0x80 closes pad10*1.
    xorByte(pos, suffix)
    if (suffix & 0x80) != 0 && pos == blockLen - 1 then keccak()
    xorByte(blockLen - 1, 0x80)
    keccak()

  def digest(): Array[Byte] =
    finish()
    val out = new Array[Byte](outputLen)
    var at = 0
    while at < outputLen do
      if posOut >= blockLen then keccak()
      val take = math.min(blockLen - posOut, outputLen - at)
      for _ <- 0 until take do
        out(at) = stateByte(posOut)
        posOut += 1
        at += 1
    out
