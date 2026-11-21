// big.js 7.0.1 (big.mjs): the decimal type, its parser, `round`, `stringify` and the methods the
// contract uses, function by function and loop by loop. A coefficient is an ArrayBuffer[Int] of
// digits, most significant first, mutated where upstream mutates its array. The exponent is a
// Double because upstream's is a JS number: past int32 and 2^53 it rounds as a double does, and
// Infinity and NaN exponents keep upstream's comparison chains. Scala has no prefix `--` and no
// `break`, so upstream's `r[--rl]` and `for (…; cond;) … break` loops are spelt out.
package bigjs

import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js
import scala.scalajs.js.JSNumberOps.*
import scala.scalajs.js.annotation.JSGlobal

private val Name = "[big.js] "
private val Invalid = Name + "Invalid "
private val InvalidDp = Invalid + "decimal places"
private val InvalidRm = Invalid + "rounding mode"
private val DivByZero = Name + "Division by zero"
private val MaxDp = 1000000
private val MaxPower = 1000000
private val Ne = -7
private val Pe = 21
// Upstream's NUMERIC, as the JS RegExp it is (no u flag: \d is ASCII, $ is the end of input).
private val Numeric = js.RegExp("""^-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$""", "i")

final class BigError(message: String) extends Exception(message)

type Digits = ArrayBuffer[Int]

extension (n: Double)
  // Upstream's `n !== ~~n || n < lo || n > hi`: an integer within the range (all ranges lie in int32).
  private def isIntIn(lo: Int, hi: Int): Boolean = n >= lo && n <= hi && n == math.floor(n)
  // JS truthiness of a number, for upstream's `if (a = xe - ye)`.
  private def truthy: Boolean = n != 0 && !n.isNaN

// ECMAScript ToInt32, for upstream's `| 0` and `&` on exponents.
private def toInt32(v: Double): Int =
  if v.isNaN || v.isInfinite then 0
  else
    val m = (if v < 0 then math.ceil(v) else math.floor(v)) % 4294967296.0
    (if m >= 2147483648.0 then m - 4294967296.0 else if m < -2147483648.0 then m + 4294967296.0 else m).toInt

// JS `Number(s)`, upstream's `+s` and Math.sqrt's own coercion.
@js.native @JSGlobal("Number")
private def jsNumber(s: String): Double = js.native

// The first index below n at which a and b differ, or -1.
private def firstDifference(a: Digits, b: Digits, n: Int): Int =
  var i = 0
  while i < n && a(i) == b(i) do i += 1
  if i < n then i else -1

// `Array.prototype.slice(0, end)`, where a negative end counts from the back.
private def head(c: Digits, end: Double): Digits =
  c.take((if end < 0 then math.max(c.length + end, 0) else math.min(end, c.length.toDouble)).toInt)

final class Big(var s: Int, var e: Double, var c: Digits):
  def copy: Big = Big(s, e, c.clone())

  // Return 1, -1 or 0 as this is greater than, less than or equal to y.
  def cmp(y: Big): Int =
    val xc = c
    val yc = y.c
    val i = s
    val j = y.s
    // Either zero?
    if xc(0) == 0 || yc(0) == 0 then return if xc(0) == 0 then (if yc(0) == 0 then 0 else -j) else i
    // Signs differ?
    if i != j then return i
    val isneg = i < 0
    // Compare exponents.
    if e != y.e then return if (e > y.e) ^ isneg then 1 else -1
    // Compare digit by digit.
    val k = firstDifference(xc, yc, math.min(xc.length, yc.length))
    if k >= 0 then return if (xc(k) > yc(k)) ^ isneg then 1 else -1
    // Compare lengths.
    if xc.length == yc.length then 0 else if (xc.length > yc.length) ^ isneg then 1 else -1

  // This divided by y, rounded if necessary to dp decimal places using rounding mode rm.
  def div(y: Big, dp: Double, rm: Double): Big =
    val a = c // dividend
    val b = y.c // divisor
    val sign = if s == y.s then 1 else -1
    if !dp.isIntIn(0, MaxDp) then throw BigError(InvalidDp)
    // Divisor is zero?
    if b(0) == 0 then throw BigError(DivByZero)
    // Dividend is 0? Return +-0.
    if a(0) == 0 then return Big(sign, 0, ArrayBuffer(0))

    val bl = b.length
    val al = a.length
    var ai = bl
    val bz = 0 +: b // divisor with a leading zero
    var r = a.take(bl) // remainder
    val qc = ArrayBuffer.empty[Int] // quotient
    var qe = e - y.e
    var p = dp + qe + 1 // precision of the result
    var k = if p < 0 then 0.0 else p

    // Compare divisor and remainder.
    def compare(): Int =
      if bl != r.length then (if bl > r.length then 1 else -1)
      else
        val i = firstDifference(b, r, bl)
        if i < 0 then 0 else if b(i) > r(i) then 1 else -1

    // Subtract the divisor from the remainder. The remainder can't be more than 1 digit longer
    // than the divisor: equalise lengths using the divisor with an extra leading zero.
    def subtract(): Unit =
      val bt = if r.length == bl then b else bz
      var rl = r.length
      while rl > 0 do
        rl -= 1
        if r(rl) < bt(rl) then
          var ri = rl - 1
          while ri > 0 && r(ri) == 0 do
            r(ri) = 9
            ri -= 1
          r(ri) -= 1
          r(rl) += 10
        r(rl) -= bt(rl)
      while r(0) == 0 do r.remove(0)

    // Add zeros to make remainder as long as divisor.
    while r.length < bl do r += 0

    while
      // n is how many times the divisor goes into current remainder (at most nine).
      var n = 0
      var cmp = compare()
      // If divisor < remainder, subtract divisor from remainder.
      while cmp < 0 do
        subtract()
        n += 1
        cmp = compare()
      // Add the digit n to the result array.
      qc += (if cmp != 0 then n else n + 1)
      // Update the remainder. Upstream's [undefined], the dividend's end, is the empty buffer.
      if r.nonEmpty && r(0) != 0 && cmp != 0 then r += a.lift(ai).getOrElse(0)
      else r = if ai < al then ArrayBuffer(a(ai)) else ArrayBuffer.empty
      val more = ai < al || r.nonEmpty
      ai += 1
      // `k--` is falsy at 0 and at NaN, as `k > 0` is.
      more && { val go = k > 0; k -= 1; go }
    do ()

    // Leading zero? Do not remove if result is simply zero (qi == 1).
    val qi = qc.length
    if qc(0) == 0 && qi != 1 then
      // There can't be more than one zero.
      qc.remove(0)
      qe -= 1
      p -= 1
    val q = Big(sign, qe, qc)
    // Round?
    if qi > p then bigjs.round(q, p, rm, r.nonEmpty)
    q

  // This minus y.
  def minus(that: Big): Big =
    val y = that.copy
    val b = y.s
    // Signs differ?
    if s != b then
      y.s = -b
      return plus(y)
    var xc = c.clone()
    val xe = e
    var yc = y.c
    var ye = y.e
    // Either zero?
    if xc(0) == 0 || yc(0) == 0 then
      if yc(0) != 0 then y.s = -b
      else if xc(0) != 0 then return copy
      else y.s = 1
      return y
    // Determine which is the bigger number. Prepend zeros to equalise exponents.
    var a = xe - ye
    var xlty = false
    if a.truthy then
      xlty = a < 0
      val t =
        if xlty then
          a = -a
          xc
        else
          ye = xe
          yc
      t.prependAll(ArrayBuffer.fill(a.toInt)(0))
    else
      // Exponents equal. Check digit by digit.
      val i = firstDifference(xc, yc, math.min(xc.length, yc.length))
      xlty = if i >= 0 then xc(i) < yc(i) else xc.length < yc.length
      a = 0
    // x < y? Point xc to the array of the bigger number.
    if xlty then
      val t = xc
      xc = yc
      yc = t
      y.s = -y.s
    // Append zeros to xc if shorter. No need to add zeros to yc if shorter as subtraction only
    // needs to start at yc.length.
    while xc.length < yc.length do xc += 0
    // Subtract yc from xc.
    var j = yc.length
    while j > a do
      j -= 1
      if xc(j) < yc(j) then
        var i = j - 1
        while i > 0 && xc(i) == 0 do
          xc(i) = 9
          i -= 1
        xc(i) -= 1
        xc(j) += 10
      xc(j) -= yc(j)
    // Remove trailing zeros.
    while xc.lastOption.contains(0) do xc.dropRightInPlace(1)
    // Remove leading zeros and adjust exponent accordingly.
    while xc.headOption.contains(0) do
      xc.remove(0)
      ye -= 1
    if xc.isEmpty then
      // n - n = +0
      y.s = 1
      // Result must be zero.
      xc = ArrayBuffer(0)
      ye = 0
    y.c = xc
    y.e = ye
    y

  // This plus y.
  def plus(that: Big): Big =
    val y = that.copy
    // Signs differ?
    if s != y.s then
      y.s = -y.s
      return minus(y)
    var xc = c
    var yc = y.c
    var ye = y.e
    // Either zero?
    if xc(0) == 0 || yc(0) == 0 then
      if yc(0) == 0 then
        if xc(0) != 0 then return copy
        y.s = s
      return y
    xc = xc.clone()
    // Prepend zeros to equalise exponents.
    var d = e - ye
    if d.truthy then
      val t =
        if d > 0 then
          ye = e
          yc
        else
          d = -d
          xc
      t.prependAll(ArrayBuffer.fill(d.toInt)(0))
    // Point xc to the longer array.
    if xc.length - yc.length < 0 then
      val t = yc
      yc = xc
      xc = t
    // Only start adding at yc.length - 1 as the further digits of xc can be left as they are.
    var k = 0
    var i = yc.length
    while i > 0 do
      i -= 1
      xc(i) += yc(i) + k
      k = xc(i) / 10
      xc(i) %= 10
    // No need to check for zero, as +x + +y != 0 && -x + -y != 0
    if k != 0 then
      xc.prepend(k)
      ye += 1
    // Remove trailing zeros.
    while xc.last == 0 do xc.dropRightInPlace(1)
    y.c = xc
    y.e = ye
    y

  // This raised to the integer power n; a negative power divides with dp and rm.
  def pow(n: Option[Double], dp: Double, rm: Double): Big =
    var x = this
    val one = Big(1, 0, ArrayBuffer(1))
    var y = one
    val power = n.filter(_.isIntIn(-MaxPower, MaxPower)).getOrElse(throw BigError(Invalid + "exponent"))
    val isneg = power < 0
    var k = power.abs.toInt
    while
      if (k & 1) != 0 then y = y.times(x)
      k >>= 1
      k != 0 && { x = x.times(x); true }
    do ()
    if isneg then one.div(y, dp, rm) else y

  // This rounded to dp decimal places (0 when absent) using rounding mode rm.
  def round(dp: Option[Double], rm: Double): Big =
    if dp.exists(!_.isIntIn(-MaxDp, MaxDp)) then throw BigError(InvalidDp)
    bigjs.round(copy, dp.getOrElse(0.0) + e + 1, rm)

  // The square root of this, rounded to dp decimal places using rounding mode rm.
  def sqrt(dp: Double, rm: Double): Big =
    val half = Big(1, -1, ArrayBuffer(5))
    // Zero?
    if c(0) == 0 then return copy
    // Negative?
    if s < 0 then throw BigError(Name + "No square root")
    // Estimate.
    var estimate = math.sqrt(jsNumber(stringify(this, true, true)))
    // Math.sqrt underflow/overflow?
    // Re-estimate: pass x coefficient to Math.sqrt as integer, then adjust the result exponent.
    var r =
      if estimate == 0 || estimate == Double.PositiveInfinity then
        var digits = c.mkString
        if (toInt32(digits.length + e) & 1) == 0 then digits += "0"
        estimate = math.sqrt(jsNumber(digits))
        // `| 0` gives an int32, but the subtraction is a double one: it can reach -2^31 - 1.
        val exponent = toInt32((e + 1) / 2).toDouble - (if e < 0 || (toInt32(e) & 1) != 0 then 1 else 0)
        if estimate == Double.PositiveInfinity then Big.parse(s"5e$exponent")
        else
          val text = estimate.toExponential()
          Big.parse(text.substring(0, text.indexOf('e') + 1) + exponent)
      else Big.parse(estimate.toString)
    // Big.DP += 4 for the iteration; div validates it.
    val places = dp + 4
    val compared = r.e + places
    // Newton-Raphson iteration.
    while
      val t = r
      r = half.times(t.plus(div(t, places, rm)))
      head(t.c, compared) != head(r.c, compared)
    do ()
    bigjs.round(r, dp + r.e + 1, rm)

  // This times y.
  def times(that: Big): Big =
    var xc = c
    var yc = that.c
    var a = xc.length
    var b = yc.length
    // Determine sign of result.
    val sign = if s == that.s then 1 else -1
    // Return signed 0 if either 0.
    if xc(0) == 0 || yc(0) == 0 then return Big(sign, 0, ArrayBuffer(0))
    // Initialise exponent of result as x.e + y.e.
    var exponent = e + that.e
    // If array xc has fewer digits than yc, swap xc and yc, and lengths.
    if a < b then
      val t = xc
      xc = yc
      yc = t
      a = xc.length
      b = yc.length
    // Initialise coefficient array of result with zeros.
    val product = ArrayBuffer.fill(a + b)(0)
    // Multiply.
    var carry = 0
    var i = b
    while i > 0 do
      i -= 1
      carry = 0
      var j = a + i
      while j > i do
        // Current sum of products at this digit position, plus carry.
        carry += product(j) + yc(i) * xc(j - i - 1)
        product(j) = carry % 10
        j -= 1
        carry /= 10
      product(j) = carry
    // Increment result exponent if there is a final carry, otherwise remove leading zero.
    if carry != 0 then exponent += 1 else product.remove(0)
    // Remove trailing zeros.
    while product.last == 0 do product.dropRightInPlace(1)
    Big(sign, exponent, product)

  // Exponential notation with dp fixed decimal places, rounded using rounding mode rm.
  def toExponential(dp: Option[Double], rm: Double): String =
    val n = c(0)
    val x = dp.fold(this) { dp =>
      if !dp.isIntIn(0, MaxDp) then throw BigError(InvalidDp)
      val x = bigjs.round(copy, dp + 1, rm)
      while x.c.length < dp + 1 do x.c += 0
      x
    }
    stringify(x, true, n != 0)

  // Normal notation with dp fixed decimal places, rounded using rounding mode rm.
  def toFixed(dp: Option[Double], rm: Double): String =
    val n = c(0)
    val x = dp.fold(this) { dp =>
      if !dp.isIntIn(0, MaxDp) then throw BigError(InvalidDp)
      val x = bigjs.round(copy, dp + e + 1, rm)
      // x.e may have changed if the value is rounded up.
      while x.c.length < dp + x.e + 1 do x.c += 0
      x
    }
    stringify(x, false, n != 0)

  // sd significant digits, rounded using rounding mode rm, in exponential notation if sd is less
  // than the number of digits of the integer part.
  def toPrecision(sd: Option[Double], rm: Double): String =
    val n = c(0)
    val x = sd.fold(this) { sd =>
      if !sd.isIntIn(1, MaxDp) then throw BigError(Invalid + "precision")
      val x = bigjs.round(copy, sd, rm)
      while x.c.length < sd do x.c += 0
      x
    }
    stringify(x, sd.exists(_ <= x.e) || x.e <= Ne || x.e >= Pe, n != 0)

  // Exponential notation at exponents of NE and below or PE and above; no sign for zero.
  override def toString: String = stringify(this, e <= Ne || e >= Pe, c(0) != 0)

object Big:
  // Upstream `parse`: the decimal string n, validated against NUMERIC.
  def parse(source: String): Big =
    if !Numeric.test(source) then throw BigError(Invalid + "number")
    // Determine sign.
    val s = if source(0) == '-' then -1 else 1
    var n = if s < 0 then source.substring(1) else source
    // Decimal point?
    val dot = n.indexOf('.')
    if dot > -1 then n = n.substring(0, dot) + n.substring(dot + 1)
    // Exponential form?
    val i = n.indexWhere(ch => ch == 'e' || ch == 'E')
    val e =
      if i > 0 then
        // Determine exponent.
        val exponent = (if dot < 0 then i else dot) + n.substring(i + 1).toDouble
        n = n.substring(0, i)
        exponent
      // Integer.
      else (if dot < 0 then n.length else dot).toDouble
    // Determine leading zeros.
    val lead = n.indexWhere(_ != '0')
    // Zero.
    if lead < 0 then Big(s, 0, ArrayBuffer(0))
    else
      // Determine trailing zeros, then convert the digits between.
      val last = n.lastIndexWhere(_ != '0')
      Big(s, e - lead - 1, ArrayBuffer.tabulate(last - lead + 1)(k => n(lead + k) - '0'))

// Upstream `round`: x rounded in place to sd significant digits using rounding mode rm. `more`
// says whether a division was truncated.
private def round(x: Big, sd: Double, rm: Double, more: Boolean = false): Big =
  val xc = x.c
  if rm != 0 && rm != 1 && rm != 2 && rm != 3 then throw BigError(InvalidRm)
  if sd < 1 then
    val up = rm == 3 && (more || xc(0) != 0) || sd == 0 && (
      rm == 1 && xc(0) >= 5 ||
        rm == 2 && (xc(0) > 5 || xc(0) == 5 && (more || xc.length > 1)))
    xc.takeInPlace(1)
    if up then
      // 1, 0.1, 0.01, 0.001, 0.0001 etc.
      x.e = x.e - sd + 1
      xc(0) = 1
    else
      // Zero.
      xc(0) = 0
      x.e = 0
  else if sd < xc.length then
    var i = sd.toInt
    // xc(i) is the digit after the digit that may be rounded up.
    val up = rm == 1 && xc(i) >= 5 ||
      rm == 2 && (xc(i) > 5 || xc(i) == 5 && (more || i + 1 < xc.length || xc(i - 1) % 2 == 1)) ||
      rm == 3 && (more || xc(0) != 0)
    // Remove any digits after the required precision.
    xc.takeInPlace(i)
    // Round up? Rounding up may mean the previous digit has to be rounded up.
    var carry = up
    while carry do
      i -= 1
      xc(i) += 1
      carry = xc(i) > 9
      if carry then
        xc(i) = 0
        if i == 0 then
          x.e += 1
          xc.prepend(1)
          carry = false
    // Remove trailing zeros.
    while xc.last == 0 do xc.dropRightInPlace(1)
  x

// Upstream `stringify`: x in normal or exponential notation.
private def stringify(x: Big, doExponential: Boolean, isNonzero: Boolean): String =
  val e = x.e
  var s = x.c.mkString
  val n = s.length
  // Exponential notation?
  if doExponential then
    s = s"${s(0)}${if n > 1 then "." + s.substring(1) else ""}${if e < 0 then "e" else "e+"}$e"
  // Normal notation.
  else if e < 0 then s = "0." + "0" * (-e - 1).toInt + s
  else if e > 0 then
    val point = e.toInt + 1
    if point > n then s += "0" * (point - n)
    else if point < n then s = s.substring(0, point) + "." + s.substring(point)
  else if n > 1 then s = s"${s(0)}.${s.substring(1)}"
  if x.s < 0 && isNonzero then "-" + s else s
