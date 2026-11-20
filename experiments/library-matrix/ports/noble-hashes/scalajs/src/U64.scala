package noblehashes

/** noble's _u64.ts: a 64-bit word is a high and a low 32-bit half, each held in an Int.
  *
  * Shifts and rotations return one half, as in noble. Sums return both halves as a tuple; the
  * carry out of the low halves comes from an unsigned comparison of the wrapped sum with an addend,
  * where noble divides its unmasked JS-number sum by 2^32. The 3- to 5-term sums are noble's
  * add3L/add3H ... add5L/add5H, each computed in one function.
  */
object U64:
  /** High half of a 64-bit logical right shift by `s` in 0..31. */
  def shrSH(h: Int, l: Int, s: Int): Int = h >>> s

  /** Low half of a 64-bit logical right shift by `s` in 1..31. */
  def shrSL(h: Int, l: Int, s: Int): Int = (h << (32 - s)) | (l >>> s)

  /** High half of a 64-bit right rotation by `s` in 1..31. */
  def rotrSH(h: Int, l: Int, s: Int): Int = (h >>> s) | (l << (32 - s))

  /** Low half of a 64-bit right rotation by `s` in 1..31. */
  def rotrSL(h: Int, l: Int, s: Int): Int = (h << (32 - s)) | (l >>> s)

  /** High half of a 64-bit right rotation by `s` in 33..63. */
  def rotrBH(h: Int, l: Int, s: Int): Int = (h << (64 - s)) | (l >>> (s - 32))

  /** Low half of a 64-bit right rotation by `s` in 33..63. */
  def rotrBL(h: Int, l: Int, s: Int): Int = (h >>> (s - 32)) | (l << (64 - s))

  /** High half of a 64-bit right rotation by 32: the low half. */
  def rotr32H(h: Int, l: Int): Int = l

  /** Low half of a 64-bit right rotation by 32: the high half. */
  def rotr32L(h: Int, l: Int): Int = h

  /** noble's fromNumH: the high half of an integer in [0, 2^53). */
  def fromNumH(n: Double): Int = (n / 4294967296.0).toInt

  /** noble's fromNumL (`n >>> 0`): the low half of an integer in [0, 2^53), as a signed Int. */
  def fromNumL(n: Double): Int =
    val low = n % 4294967296.0
    (if low >= 2147483648.0 then low - 4294967296.0 else low).toInt

  /** 1 when adding `addend` wrapped the 32-bit `sum`, else 0. */
  private def carryOut(sum: Int, addend: Int): Int =
    if Integer.compareUnsigned(sum, addend) < 0 then 1 else 0

  /** The sum of two 64-bit words, as (high, low). */
  def add(ah: Int, al: Int, bh: Int, bl: Int): (Int, Int) =
    val l = al + bl
    (ah + bh + carryOut(l, bl), l)

  /** The sum of three 64-bit words, as (high, low). */
  def add3(ah: Int, al: Int, bh: Int, bl: Int, ch: Int, cl: Int): (Int, Int) =
    val ab = al + bl
    val l = ab + cl
    (ah + bh + ch + carryOut(ab, bl) + carryOut(l, cl), l)

  /** The sum of four 64-bit words, as (high, low). */
  def add4(ah: Int, al: Int, bh: Int, bl: Int, ch: Int, cl: Int, dh: Int, dl: Int): (Int, Int) =
    val ab = al + bl
    val abc = ab + cl
    val l = abc + dl
    (ah + bh + ch + dh + carryOut(ab, bl) + carryOut(abc, cl) + carryOut(l, dl), l)

  /** The sum of five 64-bit words, as (high, low). */
  def add5(ah: Int, al: Int, bh: Int, bl: Int, ch: Int, cl: Int, dh: Int, dl: Int, eh: Int, el: Int): (Int, Int) =
    val ab = al + bl
    val abc = ab + cl
    val abcd = abc + dl
    val l = abcd + el
    val carries = carryOut(ab, bl) + carryOut(abc, cl) + carryOut(abcd, dl) + carryOut(l, el)
    (ah + bh + ch + dh + eh + carries, l)
