package noblehashes

import U64.*
import Utils.*

/** noble's blake2.ts and _blake.ts: BLAKE2b and BLAKE2s on noble's shared _BLAKE2 base.
  *
  * BLAKE2b keeps noble's G1b/G2b on 64-bit words as low and high halves in a 32-word work vector
  * (noble's BBUF); BLAKE2s keeps G1s/G2s on 32-bit words. The two do not share a mix function.
  */
object Blake2:
  /** Message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and 11 repeat 0 and 1. */
  val BSIGMA: Array[Int] = Array(
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
    11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4,
    7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
    9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13,
    2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
    12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11,
    13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
    6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5,
    10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
  )

  /** The SHA-512 IV as little-endian low and high halves (noble's B2B_IV). */
  val B2B_IV: Array[Int] = Array(
    0xf3bcc908, 0x6a09e667, 0x84caa73b, 0xbb67ae85, 0xfe94f82b, 0x3c6ef372, 0x5f1d36f1, 0xa54ff53a,
    0xade682d1, 0x510e527f, 0x2b3e6c1f, 0x9b05688c, 0xfb41bd6b, 0x1f83d9ab, 0x137e2179, 0x5be0cd19,
  )

  /** noble's checkBlake2Opts: `keyLen` bounds both the digest and the key length. */
  def checkBlake2Opts(outputLen: Int, opts: Blake2Opts, keyLen: Int, saltLen: Int, persLen: Int): Unit =
    require(outputLen >= 1 && outputLen <= keyLen, s"\"dkLen\" must be 1..$keyLen, got $outputLen")
    require(opts.key.forall(k => k.length >= 1 && k.length <= keyLen), s"\"key\" expected to be undefined or of length=1..$keyLen")
    require(opts.salt.forall(_.length == saltLen), s"\"salt\" expected Uint8Array of length $saltLen")
    require(opts.personalization.forall(_.length == persLen), s"\"personalization\" expected Uint8Array of length $persLen")

/** noble's Blake2Opts: the options reference.mjs passes to every hash. */
final class Blake2Opts(
    val dkLen: Option[Double],
    val key: Option[Array[Byte]],
    val salt: Option[Array[Byte]],
    val personalization: Option[Array[Byte]],
)

/** noble's _BLAKE2: the final block is compressed with a flag, so a full buffer waits for more input. */
abstract class Blake2Base(val blockLen: Int, dkLen: Double) extends Hash:
  val outputLen: Int = anumber(dkLen, "outputLen")
  private val buffer = new Array[Byte](blockLen)

  /** Bytes compressed or buffered so far: the BLAKE2 offset counter. */
  protected var length = 0.0
  private var pos = 0

  /** Compresses the block of `data` at `offset`. */
  protected def compress(data: Array[Byte], offset: Int, isLast: Boolean): Unit

  /** The state as little-endian 32-bit words. */
  protected def get(): Array[Int]

  def update(data: Array[Byte]): this.type =
    val len = data.length
    var at = 0
    while at < len do
      if pos == blockLen then
        compress(buffer, 0, false)
        pos = 0
      val take = math.min(blockLen - pos, len - at)
      // Whole blocks that are not the last of the input are compressed straight from it.
      if take == blockLen && at + take < len then
        while at + blockLen < len do
          length += blockLen
          compress(data, at, false)
          at += blockLen
      else
        System.arraycopy(data, at, buffer, pos, take)
        pos += take
        length += take
        at += take
    this

  def digest(): Array[Byte] =
    java.util.Arrays.fill(buffer, pos, blockLen, 0.toByte)
    compress(buffer, 0, true)
    wordsToBytesLE(get(), outputLen)

  /** noble's constructor tail: the zero-padded key is the first block. */
  protected def absorbKey(key: Option[Array[Byte]]): Unit =
    for k <- key do
      val block = new Array[Byte](blockLen)
      System.arraycopy(k, 0, block, 0, k.length)
      update(block)

import Blake2.*

/** noble's _BLAKE2b. */
final class Blake2b(opts: Blake2Opts) extends Blake2Base(128, opts.dkLen.getOrElse(64.0)):
  /** v0l, v0h, ..., v7l, v7h. */
  private val v = B2B_IV.clone()

  /** The work vector of 16 words as low and high halves (noble's BBUF). */
  private val bbuf = new Array[Int](32)

  /** The block being compressed as little-endian 32-bit words. */
  private val msg = new Array[Int](32)

  checkBlake2Opts(outputLen, opts, 64, 16, 16)
  // RFC 7693 §2.5: p[0] = 0x0101kknn goes into the low half of h[0].
  v(0) ^= outputLen | (opts.key.fold(0)(_.length) << 8) | (0x01 << 16) | (0x01 << 24)
  for salt <- opts.salt; i <- 0 until 4 do v(8 + i) ^= getUint32LE(salt, 4 * i)
  for pers <- opts.personalization; i <- 0 until 4 do v(12 + i) ^= getUint32LE(pers, 4 * i)
  absorbKey(opts.key)

  protected def get(): Array[Int] = v

  // G's first half: a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24).
  private def g1b(a: Int, b: Int, c: Int, d: Int, x: Int): Unit =
    val (ah, al) = add3(bbuf(2 * a + 1), bbuf(2 * a), bbuf(2 * b + 1), bbuf(2 * b), msg(x + 1), msg(x))
    var th = bbuf(2 * d + 1) ^ ah
    var tl = bbuf(2 * d) ^ al
    val dh = rotr32H(th, tl)
    val dl = rotr32L(th, tl)
    val (ch, cl) = add(bbuf(2 * c + 1), bbuf(2 * c), dh, dl)
    th = bbuf(2 * b + 1) ^ ch
    tl = bbuf(2 * b) ^ cl
    bbuf(2 * a) = al
    bbuf(2 * a + 1) = ah
    bbuf(2 * b) = rotrSL(th, tl, 24)
    bbuf(2 * b + 1) = rotrSH(th, tl, 24)
    bbuf(2 * c) = cl
    bbuf(2 * c + 1) = ch
    bbuf(2 * d) = dl
    bbuf(2 * d + 1) = dh

  // G's second half: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63).
  private def g2b(a: Int, b: Int, c: Int, d: Int, x: Int): Unit =
    val (ah, al) = add3(bbuf(2 * a + 1), bbuf(2 * a), bbuf(2 * b + 1), bbuf(2 * b), msg(x + 1), msg(x))
    var th = bbuf(2 * d + 1) ^ ah
    var tl = bbuf(2 * d) ^ al
    val dh = rotrSH(th, tl, 16)
    val dl = rotrSL(th, tl, 16)
    val (ch, cl) = add(bbuf(2 * c + 1), bbuf(2 * c), dh, dl)
    th = bbuf(2 * b + 1) ^ ch
    tl = bbuf(2 * b) ^ cl
    bbuf(2 * a) = al
    bbuf(2 * a + 1) = ah
    bbuf(2 * b) = rotrBL(th, tl, 63)
    bbuf(2 * b + 1) = rotrBH(th, tl, 63)
    bbuf(2 * c) = cl
    bbuf(2 * c + 1) = ch
    bbuf(2 * d) = dl
    bbuf(2 * d + 1) = dh

  protected def compress(data: Array[Byte], offset: Int, isLast: Boolean): Unit =
    for i <- 0 until 32 do msg(i) = getUint32LE(data, offset + 4 * i)
    System.arraycopy(v, 0, bbuf, 0, 16)
    System.arraycopy(B2B_IV, 0, bbuf, 16, 16)
    // The byte counter goes into v12; v14 is inverted on the last block.
    bbuf(24) = B2B_IV(8) ^ fromNumL(length)
    bbuf(25) = B2B_IV(9) ^ fromNumH(length)
    if isLast then
      bbuf(28) = ~bbuf(28)
      bbuf(29) = ~bbuf(29)
    // BSIGMA picks 64-bit message words; each spans two entries of msg.
    var j = 0
    def next(): Int =
      j += 1
      2 * BSIGMA(j - 1)
    for _ <- 0 until 12 do
      g1b(0, 4, 8, 12, next())
      g2b(0, 4, 8, 12, next())
      g1b(1, 5, 9, 13, next())
      g2b(1, 5, 9, 13, next())
      g1b(2, 6, 10, 14, next())
      g2b(2, 6, 10, 14, next())
      g1b(3, 7, 11, 15, next())
      g2b(3, 7, 11, 15, next())
      g1b(0, 5, 10, 15, next())
      g2b(0, 5, 10, 15, next())
      g1b(1, 6, 11, 12, next())
      g2b(1, 6, 11, 12, next())
      g1b(2, 7, 8, 13, next())
      g2b(2, 7, 8, 13, next())
      g1b(3, 4, 9, 14, next())
      g2b(3, 4, 9, 14, next())
    for i <- 0 until 16 do v(i) ^= bbuf(i) ^ bbuf(i + 16)

/** noble's _BLAKE2s, whose IV is SHA-256's. */
final class Blake2s(opts: Blake2Opts) extends Blake2Base(64, opts.dkLen.getOrElse(32.0)):
  /** v0 ... v7. */
  private val v = Sha2.SHA256_IV.clone()

  /** v0 ... v15 of one compression (noble's _compress locals). */
  private val work = new Array[Int](16)

  /** The block being compressed as little-endian 32-bit words. */
  private val msg = new Array[Int](16)

  checkBlake2Opts(outputLen, opts, 32, 8, 8)
  v(0) ^= outputLen | (opts.key.fold(0)(_.length) << 8) | (0x01 << 16) | (0x01 << 24)
  for salt <- opts.salt; i <- 0 until 2 do v(4 + i) ^= getUint32LE(salt, 4 * i)
  for pers <- opts.personalization; i <- 0 until 2 do v(6 + i) ^= getUint32LE(pers, 4 * i)
  absorbKey(opts.key)

  protected def get(): Array[Int] = v

  // G's first half: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12).
  private def g1s(a: Int, b: Int, c: Int, d: Int, x: Int): Unit =
    work(a) = work(a) + work(b) + x
    work(d) = Integer.rotateRight(work(d) ^ work(a), 16)
    work(c) = work(c) + work(d)
    work(b) = Integer.rotateRight(work(b) ^ work(c), 12)

  // G's second half: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7).
  private def g2s(a: Int, b: Int, c: Int, d: Int, x: Int): Unit =
    work(a) = work(a) + work(b) + x
    work(d) = Integer.rotateRight(work(d) ^ work(a), 8)
    work(c) = work(c) + work(d)
    work(b) = Integer.rotateRight(work(b) ^ work(c), 7)

  protected def compress(data: Array[Byte], offset: Int, isLast: Boolean): Unit =
    for i <- 0 until 16 do msg(i) = getUint32LE(data, offset + 4 * i)
    // v8..v15 come from the IV, with the byte counter in v12 and v13 and v14 inverted last.
    System.arraycopy(v, 0, work, 0, 8)
    System.arraycopy(Sha2.SHA256_IV, 0, work, 8, 8)
    work(12) ^= fromNumL(length)
    work(13) ^= fromNumH(length)
    if isLast then work(14) = ~work(14)
    var j = 0
    def next(): Int =
      j += 1
      msg(BSIGMA(j - 1))
    for _ <- 0 until 10 do
      g1s(0, 4, 8, 12, next())
      g2s(0, 4, 8, 12, next())
      g1s(1, 5, 9, 13, next())
      g2s(1, 5, 9, 13, next())
      g1s(2, 6, 10, 14, next())
      g2s(2, 6, 10, 14, next())
      g1s(3, 7, 11, 15, next())
      g2s(3, 7, 11, 15, next())
      g1s(0, 5, 10, 15, next())
      g2s(0, 5, 10, 15, next())
      g1s(1, 6, 11, 12, next())
      g2s(1, 6, 11, 12, next())
      g1s(2, 7, 8, 13, next())
      g2s(2, 7, 8, 13, next())
      g1s(3, 4, 9, 14, next())
      g2s(3, 4, 9, 14, next())
    for i <- 0 until 8 do v(i) ^= work(i) ^ work(i + 8)
