// noble's _u64.ts: a 64-bit word is a high and a low 32-bit half, each held in an Int.
//
// Shifts and rotations return one half, as in noble. Sums return both halves as a Pair; the carry
// out of the low halves comes from an unsigned comparison of the wrapped sum with an addend, where
// noble divides its unmasked JS-number sum by 2^32. The 3- to 5-term sums are noble's
// add3L/add3H ... add5L/add5H, each computed in one function.
@file:Suppress("UNUSED_PARAMETER")

/** High half of a 64-bit logical right shift by [s] in 0..31. */
internal fun shrSH(h: Int, l: Int, s: Int): Int = h ushr s

/** Low half of a 64-bit logical right shift by [s] in 1..31. */
internal fun shrSL(h: Int, l: Int, s: Int): Int = (h shl 32 - s) or (l ushr s)

/** High half of a 64-bit right rotation by [s] in 1..31. */
internal fun rotrSH(h: Int, l: Int, s: Int): Int = (h ushr s) or (l shl 32 - s)

/** Low half of a 64-bit right rotation by [s] in 1..31. */
internal fun rotrSL(h: Int, l: Int, s: Int): Int = (h shl 32 - s) or (l ushr s)

/** High half of a 64-bit right rotation by [s] in 33..63. */
internal fun rotrBH(h: Int, l: Int, s: Int): Int = (h shl 64 - s) or (l ushr s - 32)

/** Low half of a 64-bit right rotation by [s] in 33..63. */
internal fun rotrBL(h: Int, l: Int, s: Int): Int = (h ushr s - 32) or (l shl 64 - s)

/** High half of a 64-bit right rotation by 32: the low half. */
internal fun rotr32H(h: Int, l: Int): Int = l

/** Low half of a 64-bit right rotation by 32: the high half. */
internal fun rotr32L(h: Int, l: Int): Int = h

/** noble's fromNumH: the high half of an integer in 0 until 2^53. */
internal fun fromNumH(n: Double): Int = (n / 4294967296.0).toInt()

/** noble's fromNumL (`n >>> 0`): the low half of an integer in 0 until 2^53. */
internal fun fromNumL(n: Double): Int = (n % 4294967296.0).toUInt().toInt()

/** 1 when adding [addend] wrapped the 32-bit [sum], else 0. */
private fun carryOut(sum: Int, addend: Int): Int = if (sum.toUInt() < addend.toUInt()) 1 else 0

/** The sum of two 64-bit words, as (high, low). */
internal fun add(ah: Int, al: Int, bh: Int, bl: Int): Pair<Int, Int> {
    val l = al + bl
    return Pair(ah + bh + carryOut(l, bl), l)
}

/** The sum of three 64-bit words, as (high, low). */
internal fun add3(ah: Int, al: Int, bh: Int, bl: Int, ch: Int, cl: Int): Pair<Int, Int> {
    val ab = al + bl
    val l = ab + cl
    return Pair(ah + bh + ch + carryOut(ab, bl) + carryOut(l, cl), l)
}

/** The sum of four 64-bit words, as (high, low). */
internal fun add4(ah: Int, al: Int, bh: Int, bl: Int, ch: Int, cl: Int, dh: Int, dl: Int): Pair<Int, Int> {
    val ab = al + bl
    val abc = ab + cl
    val l = abc + dl
    return Pair(ah + bh + ch + dh + carryOut(ab, bl) + carryOut(abc, cl) + carryOut(l, dl), l)
}

/** The sum of five 64-bit words, as (high, low). */
internal fun add5(
    ah: Int, al: Int, bh: Int, bl: Int, ch: Int, cl: Int, dh: Int, dl: Int, eh: Int, el: Int,
): Pair<Int, Int> {
    val ab = al + bl
    val abc = ab + cl
    val abcd = abc + dl
    val l = abcd + el
    val carries = carryOut(ab, bl) + carryOut(abc, cl) + carryOut(abcd, dl) + carryOut(l, el)
    return Pair(ah + bh + ch + dh + eh + carries, l)
}
