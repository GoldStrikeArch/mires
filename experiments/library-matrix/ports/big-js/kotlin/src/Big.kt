// big.js 7.0.1 (big.mjs): the decimal type, its parser, `round`, `stringify` and the methods the
// contract uses, function by function and loop by loop. A coefficient is a MutableList<Int> of
// digits, most significant first, mutated where upstream mutates its array. The exponent is a
// Double because upstream's is a JS number: past int32 and 2^53 it rounds as a double does, and
// Infinity and NaN exponents keep upstream's comparison chains.
import kotlin.math.floor
import kotlin.math.sqrt
import kotlin.math.truncate

private const val NAME = "[big.js] "
private const val INVALID = NAME + "Invalid "
private const val INVALID_DP = INVALID + "decimal places"
private const val INVALID_RM = INVALID + "rounding mode"
private const val DIV_BY_ZERO = NAME + "Division by zero"
private const val MAX_DP = 1_000_000
private const val MAX_POWER = 1_000_000
private const val NE = -7
private const val PE = 21
private val NUMERIC = Regex("""-?(\d+(\.\d*)?|\.\d+)([eE][+-]?\d+)?""")

class BigError(override val message: String) : Exception(message)

// Upstream's `n !== ~~n || n < lo || n > hi`: an integer within the range (all ranges lie in int32).
private fun Double.isIntIn(lo: Int, hi: Int) = this >= lo && this <= hi && this == floor(this)

// JS truthiness of a number, for upstream's `if (a = xe - ye)`.
private fun Double.isTruthy() = this != 0.0 && !isNaN()

// ECMAScript ToInt32, for upstream's `| 0` and `&` on exponents.
private fun toInt32(v: Double): Int {
    if (!v.isFinite()) return 0
    var m = truncate(v % 4294967296.0)
    if (m >= 2147483648.0) m -= 4294967296.0 else if (m < -2147483648.0) m += 4294967296.0
    return m.toInt()
}

// JS `Number(s)` for the strings upstream passes it: NaN where it is not a number.
private fun jsNumber(s: String) = s.toDoubleOrNull() ?: Double.NaN

// `Array.prototype.slice(0, end)`, where a negative end counts from the back.
private fun List<Int>.head(end: Double): List<Int> =
    subList(0, (if (end < 0) maxOf(size + end, 0.0) else minOf(end, size.toDouble())).toInt())

class Big(var s: Int, var e: Double, var c: MutableList<Int>) {
    fun copy() = Big(s, e, c.toMutableList())

    // Return 1, -1 or 0 as this is greater than, less than or equal to y.
    fun cmp(y: Big): Int {
        val xc = c
        val yc = y.c
        val i = s
        val j = y.s
        // Either zero?
        if (xc[0] == 0 || yc[0] == 0) return if (xc[0] == 0) (if (yc[0] == 0) 0 else -j) else i
        // Signs differ?
        if (i != j) return i
        val isneg = i < 0
        // Compare exponents.
        if (e != y.e) return if ((e > y.e) xor isneg) 1 else -1
        // Compare digit by digit.
        for (k in 0 until minOf(xc.size, yc.size)) {
            if (xc[k] != yc[k]) return if ((xc[k] > yc[k]) xor isneg) 1 else -1
        }
        // Compare lengths.
        return if (xc.size == yc.size) 0 else if ((xc.size > yc.size) xor isneg) 1 else -1
    }

    // This divided by y, rounded if necessary to dp decimal places using rounding mode rm.
    fun div(y: Big, dp: Double, rm: Double): Big {
        val a = c // dividend
        val b = y.c // divisor
        val sign = if (s == y.s) 1 else -1
        if (!dp.isIntIn(0, MAX_DP)) throw BigError(INVALID_DP)
        // Divisor is zero?
        if (b[0] == 0) throw BigError(DIV_BY_ZERO)
        // Dividend is 0? Return +-0.
        if (a[0] == 0) return Big(sign, 0.0, mutableListOf(0))

        val bl = b.size
        val al = a.size
        var ai = bl
        val bz = (listOf(0) + b) // divisor with a leading zero
        var r = a.take(bl).toMutableList() // remainder
        val qc = mutableListOf<Int>() // quotient
        var qe = e - y.e
        var p = dp + qe + 1 // precision of the result
        var k = if (p < 0) 0.0 else p

        // Add zeros to make remainder as long as divisor.
        while (r.size < bl) r.add(0)

        var rl: Int
        var cmp = 0
        do {
            // n is how many times the divisor goes into current remainder.
            var n = 0
            while (n < 10) {
                // Compare divisor and remainder.
                rl = r.size
                if (bl != rl) {
                    cmp = if (bl > rl) 1 else -1
                } else {
                    cmp = 0
                    for (ri in 0 until bl) {
                        if (b[ri] != r[ri]) {
                            cmp = if (b[ri] > r[ri]) 1 else -1
                            break
                        }
                    }
                }
                // If divisor < remainder, subtract divisor from remainder.
                if (cmp >= 0) break
                // Remainder can't be more than 1 digit longer than divisor.
                // Equalise lengths using divisor with extra leading zero?
                val bt = if (rl == bl) b else bz
                while (rl > 0) {
                    if (r[--rl] < bt[rl]) {
                        var ri = rl
                        while (ri > 0 && r[--ri] == 0) r[ri] = 9
                        r[ri]--
                        r[rl] += 10
                    }
                    r[rl] -= bt[rl]
                }
                while (r[0] == 0) r.removeFirst()
                n++
            }
            // Add the digit n to the result array.
            qc.add(if (cmp != 0) n else n + 1)
            // Update the remainder. Upstream's [undefined], the dividend's end, is the empty list.
            if (r.isNotEmpty() && r[0] != 0 && cmp != 0) r.add(a.getOrElse(ai) { 0 })
            else r = if (ai < al) mutableListOf(a[ai]) else mutableListOf()
            // `k--` is falsy at 0 and at NaN, as `k-- > 0` is.
        } while ((ai++ < al || r.isNotEmpty()) && k-- > 0)

        // Leading zero? Do not remove if result is simply zero (qi == 1).
        val qi = qc.size
        if (qc[0] == 0 && qi != 1) {
            // There can't be more than one zero.
            qc.removeFirst()
            qe--
            p--
        }
        val q = Big(sign, qe, qc)
        // Round?
        if (qi > p) round(q, p, rm, r.isNotEmpty())
        return q
    }

    // This minus y.
    fun minus(that: Big): Big {
        val y = that.copy()
        val b = y.s
        // Signs differ?
        if (s != b) {
            y.s = -b
            return plus(y)
        }
        var xc = c.toMutableList()
        val xe = e
        var yc = y.c
        var ye = y.e
        // Either zero?
        if (xc[0] == 0 || yc[0] == 0) {
            if (yc[0] != 0) y.s = -b else if (xc[0] != 0) return copy() else y.s = 1
            return y
        }
        // Determine which is the bigger number. Prepend zeros to equalise exponents.
        var a = xe - ye
        var xlty: Boolean
        if (a.isTruthy()) {
            xlty = a < 0
            val t: MutableList<Int>
            if (xlty) {
                a = -a
                t = xc
            } else {
                ye = xe
                t = yc
            }
            t.addAll(0, List(a.toInt()) { 0 })
        } else {
            // Exponents equal. Check digit by digit.
            xlty = xc.size < yc.size
            for (i in 0 until minOf(xc.size, yc.size)) {
                if (xc[i] != yc[i]) {
                    xlty = xc[i] < yc[i]
                    break
                }
            }
            a = 0.0
        }
        // x < y? Point xc to the array of the bigger number.
        if (xlty) {
            val t = xc
            xc = yc
            yc = t
            y.s = -y.s
        }
        // Append zeros to xc if shorter. No need to add zeros to yc if shorter as subtraction only
        // needs to start at yc.length.
        while (xc.size < yc.size) xc.add(0)
        // Subtract yc from xc.
        var j = yc.size
        while (j > a) {
            if (xc[--j] < yc[j]) {
                var i = j
                while (i > 0 && xc[--i] == 0) xc[i] = 9
                xc[i]--
                xc[j] += 10
            }
            xc[j] -= yc[j]
        }
        // Remove trailing zeros.
        while (xc.lastOrNull() == 0) xc.removeLast()
        // Remove leading zeros and adjust exponent accordingly.
        while (xc.firstOrNull() == 0) {
            xc.removeFirst()
            --ye
        }
        if (xc.isEmpty()) {
            // n - n = +0
            y.s = 1
            // Result must be zero.
            xc = mutableListOf(0)
            ye = 0.0
        }
        y.c = xc
        y.e = ye
        return y
    }

    // This plus y.
    fun plus(that: Big): Big {
        val y = that.copy()
        // Signs differ?
        if (s != y.s) {
            y.s = -y.s
            return minus(y)
        }
        var xc = c
        var yc = y.c
        var ye = y.e
        // Either zero?
        if (xc[0] == 0 || yc[0] == 0) {
            if (yc[0] == 0) {
                if (xc[0] != 0) return copy() else y.s = s
            }
            return y
        }
        xc = xc.toMutableList()
        // Prepend zeros to equalise exponents.
        var d = e - ye
        if (d.isTruthy()) {
            val t: MutableList<Int>
            if (d > 0) {
                ye = e
                t = yc
            } else {
                d = -d
                t = xc
            }
            t.addAll(0, List(d.toInt()) { 0 })
        }
        // Point xc to the longer array.
        if (xc.size - yc.size < 0) {
            val t = yc
            yc = xc
            xc = t
        }
        // Only start adding at yc.length - 1 as the further digits of xc can be left as they are.
        var k = 0
        for (i in yc.indices.reversed()) {
            xc[i] = xc[i] + yc[i] + k
            k = xc[i] / 10
            xc[i] %= 10
        }
        // No need to check for zero, as +x + +y != 0 && -x + -y != 0
        if (k != 0) {
            xc.add(0, k)
            ++ye
        }
        // Remove trailing zeros.
        while (xc.last() == 0) xc.removeLast()
        y.c = xc
        y.e = ye
        return y
    }

    // This raised to the integer power n; a negative power divides with dp and rm.
    fun pow(n: Double?, dp: Double, rm: Double): Big {
        var x = this
        val one = Big(1, 0.0, mutableListOf(1))
        var y = one
        if (n == null || !n.isIntIn(-MAX_POWER, MAX_POWER)) throw BigError(INVALID + "exponent")
        val isneg = n < 0
        var k = if (isneg) -n.toInt() else n.toInt()
        while (true) {
            if (k and 1 != 0) y = y.times(x)
            k = k shr 1
            if (k == 0) break
            x = x.times(x)
        }
        return if (isneg) one.div(y, dp, rm) else y
    }

    // This rounded to dp decimal places (0 when absent) using rounding mode rm.
    fun round(dp: Double?, rm: Double): Big {
        if (dp != null && !dp.isIntIn(-MAX_DP, MAX_DP)) throw BigError(INVALID_DP)
        return round(copy(), (dp ?: 0.0) + e + 1, rm)
    }

    // The square root of this, rounded to dp decimal places using rounding mode rm.
    fun sqrt(dp: Double, rm: Double): Big {
        val half = Big(1, -1.0, mutableListOf(5))
        // Zero?
        if (c[0] == 0) return copy()
        // Negative?
        if (s < 0) throw BigError(NAME + "No square root")
        // Estimate.
        var estimate = sqrt(jsNumber(stringify(this, true, true)))
        // Math.sqrt underflow/overflow?
        // Re-estimate: pass x coefficient to Math.sqrt as integer, then adjust the result exponent.
        var r = if (estimate == 0.0 || estimate == Double.POSITIVE_INFINITY) {
            var digits = c.joinToString("")
            if (toInt32(digits.length + e) and 1 == 0) digits += "0"
            estimate = sqrt(jsNumber(digits))
            val exponent = toInt32((e + 1) / 2) - (if (e < 0 || toInt32(e) and 1 != 0) 1 else 0)
            parse((if (estimate == Double.POSITIVE_INFINITY) "5" else mantissa(estimate)) + "e" + exponent)
        } else {
            parse(estimate.toString())
        }
        // Big.DP += 4 for the iteration; div validates it.
        val places = dp + 4
        val compared = r.e + places
        // Newton-Raphson iteration.
        do {
            val t = r
            r = half.times(t.plus(div(t, places, rm)))
        } while (t.c.head(compared) != r.c.head(compared))
        return round(r, dp + r.e + 1, rm)
    }

    // This times y.
    fun times(that: Big): Big {
        var xc = c
        var yc = that.c
        var a = xc.size
        var b = yc.size
        // Determine sign of result.
        val sign = if (s == that.s) 1 else -1
        // Return signed 0 if either 0.
        if (xc[0] == 0 || yc[0] == 0) return Big(sign, 0.0, mutableListOf(0))
        // Initialise exponent of result as x.e + y.e.
        var e = this.e + that.e
        // If array xc has fewer digits than yc, swap xc and yc, and lengths.
        if (a < b) {
            xc = yc.also { yc = xc }
            a = b.also { b = a }
        }
        // Initialise coefficient array of result with zeros.
        val c = MutableList(a + b) { 0 }
        // Multiply.
        var carry = 0
        for (i in b - 1 downTo 0) {
            carry = 0
            var j = a + i
            while (j > i) {
                // Current sum of products at this digit position, plus carry.
                carry += c[j] + yc[i] * xc[j - i - 1]
                c[j--] = carry % 10
                carry /= 10
            }
            c[j] = carry
        }
        // Increment result exponent if there is a final carry, otherwise remove leading zero.
        if (carry != 0) ++e else c.removeFirst()
        // Remove trailing zeros.
        while (c.last() == 0) c.removeLast()
        return Big(sign, e, c)
    }

    // Exponential notation with dp fixed decimal places, rounded using rounding mode rm.
    fun toExponential(dp: Double?, rm: Double): String {
        var x = this
        val n = c[0]
        if (dp != null) {
            if (!dp.isIntIn(0, MAX_DP)) throw BigError(INVALID_DP)
            val sd = dp + 1
            x = round(copy(), sd, rm)
            while (x.c.size < sd) x.c.add(0)
        }
        return stringify(x, true, n != 0)
    }

    // Normal notation with dp fixed decimal places, rounded using rounding mode rm.
    fun toFixed(dp: Double?, rm: Double): String {
        var x = this
        val n = c[0]
        if (dp != null) {
            if (!dp.isIntIn(0, MAX_DP)) throw BigError(INVALID_DP)
            x = round(copy(), dp + e + 1, rm)
            // x.e may have changed if the value is rounded up.
            val width = dp + x.e + 1
            while (x.c.size < width) x.c.add(0)
        }
        return stringify(x, false, n != 0)
    }

    // sd significant digits, rounded using rounding mode rm, in exponential notation if sd is less
    // than the number of digits of the integer part.
    fun toPrecision(sd: Double?, rm: Double): String {
        var x = this
        val n = c[0]
        if (sd != null) {
            if (!sd.isIntIn(1, MAX_DP)) throw BigError(INVALID + "precision")
            x = round(copy(), sd, rm)
            while (x.c.size < sd) x.c.add(0)
        }
        return stringify(x, sd != null && sd <= x.e || x.e <= NE || x.e >= PE, n != 0)
    }

    // Exponential notation at exponents of NE and below or PE and above; no sign for zero.
    override fun toString() = stringify(this, e <= NE || e >= PE, c[0] != 0)

    companion object {
        // Upstream `parse`: the decimal string n, validated against NUMERIC.
        fun parse(source: String): Big {
            if (!NUMERIC.matches(source)) throw BigError(INVALID + "number")
            // Determine sign.
            val s = if (source[0] == '-') -1 else 1
            var n = if (s < 0) source.substring(1) else source
            // Decimal point?
            val dot = n.indexOf('.')
            if (dot > -1) n = n.replaceFirst(".", "")
            // Exponential form?
            val i = n.indexOfFirst { it == 'e' || it == 'E' }
            val e = if (i > 0) {
                // Determine exponent.
                (if (dot < 0) i else dot) + n.substring(i + 1).toDouble()
            } else {
                // Integer.
                (if (dot < 0) n.length else dot).toDouble()
            }
            if (i > 0) n = n.substring(0, i)
            // Determine leading zeros.
            val lead = n.indexOfFirst { it != '0' }
            // Zero.
            if (lead < 0) return Big(s, 0.0, mutableListOf(0))
            // Determine trailing zeros, then convert the digits between.
            val last = n.indexOfLast { it != '0' }
            return Big(s, e - lead - 1, n.substring(lead, last + 1).map { it - '0' }.toMutableList())
        }
    }
}

// JS `toExponential()` up to its `e`: the shortest digits of s in d.ddd form.
private fun mantissa(s: Double): String {
    val text: String = s.asDynamic().toExponential()
    return text.substring(0, text.indexOf('e'))
}

// Upstream `round`: x rounded in place to sd significant digits using rounding mode rm. `more`
// says whether a division was truncated.
private fun round(x: Big, sd: Double, rm: Double, more: Boolean = false): Big {
    val xc = x.c
    if (rm != 0.0 && rm != 1.0 && rm != 2.0 && rm != 3.0) throw BigError(INVALID_RM)
    if (sd < 1) {
        val up = rm == 3.0 && (more || xc[0] != 0) || sd == 0.0 && (
            rm == 1.0 && xc[0] >= 5 ||
                rm == 2.0 && (xc[0] > 5 || xc[0] == 5 && (more || xc.size > 1)))
        xc.subList(1, xc.size).clear()
        if (up) {
            // 1, 0.1, 0.01, 0.001, 0.0001 etc.
            x.e = x.e - sd + 1
            xc[0] = 1
        } else {
            // Zero.
            xc[0] = 0
            x.e = 0.0
        }
    } else if (sd < xc.size) {
        var i = sd.toInt()
        // xc[i] is the digit after the digit that may be rounded up.
        val up = rm == 1.0 && xc[i] >= 5 ||
            rm == 2.0 && (xc[i] > 5 || xc[i] == 5 && (more || i + 1 < xc.size || xc[i - 1] % 2 == 1)) ||
            rm == 3.0 && (more || xc[0] != 0)
        // Remove any digits after the required precision.
        xc.subList(i, xc.size).clear()
        // Round up?
        if (up) {
            // Rounding up may mean the previous digit has to be rounded up.
            while (++xc[--i] > 9) {
                xc[i] = 0
                if (i == 0) {
                    ++x.e
                    xc.add(0, 1)
                    break
                }
            }
        }
        // Remove trailing zeros.
        while (xc.last() == 0) xc.removeLast()
    }
    return x
}

// Upstream `stringify`: x in normal or exponential notation.
private fun stringify(x: Big, doExponential: Boolean, isNonzero: Boolean): String {
    val e = x.e
    var s = x.c.joinToString("")
    val n = s.length
    // Exponential notation?
    if (doExponential) {
        s = s[0] + (if (n > 1) "." + s.substring(1) else "") + (if (e < 0) "e" else "e+") + e
    } else if (e < 0) {
        // Normal notation.
        s = "0." + "0".repeat((-e - 1).toInt()) + s
    } else if (e > 0) {
        val point = e.toInt() + 1
        if (point > n) s += "0".repeat(point - n) else if (point < n) s = s.substring(0, point) + "." + s.substring(point)
    } else if (n > 1) {
        s = s[0] + "." + s.substring(1)
    }
    return if (x.s < 0 && isNonzero) "-$s" else s
}
