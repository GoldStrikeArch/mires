package longmatrix

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel
import java.lang.{Long as JLong}

@js.native
trait Command extends js.Object:
  val op: String = js.native
  val s: String = js.native
  // JS numbers, as long.js reads them. An Int facade would wrap them with `| 0`, so a radix of
  // 2^32 + 10 would read as 10.
  val n: Double = js.native
  val r: Double = js.native
  val a: js.Array[Int] = js.native
  val b: js.Array[Int] = js.native
  val u: Boolean = js.native
  val v: Boolean = js.native

@js.native
trait Input extends js.Object:
  val cases: js.Array[Command] = js.native

object Main:
  // long.js's radix rules on a JS number: `radix || 10`, then a RangeError outside 2..36.
  private def radix(r: Double): Int =
    if r == 0 || r.isNaN then 10
    else if r < 2 || r > 36 then throw new IllegalArgumentException("radix")
    else r.toInt

  // long.js's `numBits &= 63` on a JS number: the low six bits of ToInt32(n), which are those of
  // trunc(n) modulo 64. `%` is exact, and `toInt` maps the NaN of an infinite or NaN count to 0.
  private def shiftCount(n: Double): Int = (n % 64).toInt & 63

  private def parse(text: String, base: Double): Long =
    if text.isEmpty then throw new IllegalArgumentException("empty string")
    if text == "NaN" || text == "Infinity" || text == "+Infinity" || text == "-Infinity" then return 0L
    val r = radix(base)
    if text.head == '-' then return -parse(text.tail, r.toDouble)
    if text.contains('-') then throw new IllegalArgumentException("interior hyphen")
    var result = 0L
    var start = 0
    while start < text.length do
      val end = math.min(start + 8, text.length)
      var i = start
      while i < end && " \n\t\r\f\u000b".indexOf(text(i)) >= 0 do i += 1
      if i < end && text(i) == '+' then i += 1
      if r == 16 && i + 1 < end && text(i) == '0' && (text(i+1) == 'x' || text(i+1) == 'X') then i += 2
      var part = 0L
      while i < end && Character.digit(text(i), r) >= 0 do
        part = part * r + Character.digit(text(i), r)
        i += 1
      i = start
      while i < end do
        result *= r
        i += 1
      result += part
      start = end
    result

  private def format(a: Long, unsigned: Boolean, r: Double): String =
    if unsigned then JLong.toUnsignedString(a, radix(r)) else JLong.toString(a, radix(r))

  private def divide(a: Long, b: Long, unsigned: Boolean): Long =
    if unsigned then JLong.divideUnsigned(a,b)
    else if (a >> 32) == Int.MinValue && b == -1L then a
    else a / b

  private def compare(a: Long,b: Long,u: Boolean,v: Boolean): Int =
    if a == b && (u == v || a >= 0) then return 0
    val an = !u && a < 0
    val bn = !v && b < 0
    val less = if an != bn then an else if u then JLong.compareUnsigned(a,b) < 0 else a-b < 0
    if less then -1 else 1

  private def words(p: js.Array[Int]): Long = (p(1).toLong << 32) | (p(0).toLong & 0xffffffffL)

  private def evaluate(c: Command): String =
    val a = words(c.a)
    val b = words(c.b)
    val n = c.n
    val value = c.op match
      case "fromBits" | "toString" => a
      case "fromString" => parse(c.s,n)
      case "add" => a + b
      case "subtract" => a - b
      case "multiply" => a * b
      case "divide" => if b == 0 then return "!" else divide(a,b,c.u)
      case "modulo" => if b == 0 then return "!" else if c.u then JLong.remainderUnsigned(a,b) else a % b
      case "and" => a & b
      case "or" => a | b
      case "xor" => a ^ b
      case "shiftLeft" => a << shiftCount(n)
      case "shiftRight" => a >> shiftCount(n)
      case "shiftRightUnsigned" => a >>> shiftCount(n)
      case "rotl" => JLong.rotateLeft(a,shiftCount(n))
      case "clz" => return JLong.numberOfLeadingZeros(a).toString
      case "ctz" => return JLong.numberOfTrailingZeros(a).toString
      case "compare" => return compare(a,b,c.u,c.v).toString
      case "mix" =>
        var acc = a
        var check = 0
        var i = 0
        while i < n do
          acc = JLong.rotateLeft(acc * (b | 1L) + i + 1,11) ^ b
          acc += divide(acc,3+(i & 255),c.u)
          if (i & 7) == 0 then
            val s = format(acc,c.u,c.r)
            check = check * 33 + s.length + s(0).toInt
          i += 1
        return format(acc,c.u,c.r) + ":" + check
      case _ => return "!"
    format(value,c.u,c.r)

  @JSExportTopLevel("run")
  def run(input: Input): String =
    input.cases.iterator.map { c =>
      try evaluate(c)
      catch case _: IllegalArgumentException => "!"
    }.mkString("\n")
