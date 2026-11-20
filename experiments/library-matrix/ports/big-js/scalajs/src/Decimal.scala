// Decimal coefficients contain normalized base-ten digits, most significant first.
package bigjs
import scala.scalajs.js
object DecimalMath {
case class Decimal(s:Int,e:Double,c:Array[Int])
private def dec(s:Int,e:Double,c:Array[Int])=Decimal(s,e,c)
private def size(a:Array[Int])=a.length
private def slen(s:String)=s.length
private def filled(n:Int,d:Int)=Array.fill(n)(d)
private def cut(a:Array[Int],start:Int,stop:Int)=a.slice(start,stop)
private def extend(a:Array[Int],d:Int)=a :+ d
private def code(s:String,i:Int)=s.charAt(i).toInt
private def letter(n:Int)=n.toChar.toString
private def strint(n:Int)=n.toString
private def cat(a:String,b:String)=a+b
private def machineRoot(s:String)=math.sqrt(s.toDouble)
private def finite(x:Double)=x.isFinite
private def floatText(x:Double)=js.Dynamic.global.String(x).asInstanceOf[String]
private def asFloat(n:Int)=n.toDouble
private def asIndex(n:Double)=n.toInt
private def maxExp(x:Double,y:Double)=math.max(x,y)
private def minExp(x:Double,y:Double)=math.min(x,y)
private def even(e:Double)=e % 2 == 0
private def halvedExponent(e:Double)=math.floor(e/2)
private def parseExponent(s:String,start:Int,stop:Int)=s.substring(start,stop).toDouble
private def grow(a:Array[Int])=java.util.Arrays.copyOf(a,a.length*2)
private def min(a:Int,b:Int)=math.min(a,b)
private def max(a:Int,b:Int)=math.max(a,b)
private def error(msg:String):Nothing=throw new IllegalArgumentException(msg)


def normalizeFrom(sign: Int, exponent: Double, digits: Array[Int], offset: Int): Decimal = {
    var start: Int = offset
    while (((start < size(digits)) && (digits(start) == 0))) {
        start = start + 1
    }
    var stop: Int = size(digits)
    while (((stop > start) && (digits((stop - 1)) == 0))) {
        stop = stop - 1
    }
    var out: Decimal = dec(sign, 0.0, filled(1, 0))
    if ((stop > start)) {
        out = dec(sign, (exponent - asFloat((start - offset))), cut(digits, start, stop))
    }
    out
}

def normalize(sign: Int, exponent: Double, digits: Array[Int]): Decimal = {
    normalizeFrom(sign, exponent, digits, 0)
}

def parse(text: String): Decimal = {
    val length: Int = slen(text)
    var pos: Int = 0
    var sign: Int = 1
    if (((pos < length) && (code(text, pos) == 45))) {
        sign = (-1)
        pos = pos + 1
    }
    val digits: Array[Int] = filled(length, 0)
    var count: Int = 0
    var places: Int = 0
    while (((pos < length) && (code(text, pos) >= 48) && (code(text, pos) <= 57))) {
        digits(count) = (code(text, pos) - 48)
        count = count + 1
        pos = pos + 1
        places = places + 1
    }
    if (((pos < length) && (code(text, pos) == 46))) {
        pos = pos + 1
        while (((pos < length) && (code(text, pos) >= 48) && (code(text, pos) <= 57))) {
            digits(count) = (code(text, pos) - 48)
            count = count + 1
            pos = pos + 1
        }
    }
    if ((count == 0)) {
        error("[big.js] Invalid number")
    }
    var scale: Double = 0.0
    if (((pos < length) && ((code(text, pos) == 69) || (code(text, pos) == 101)))) {
        pos = pos + 1
        val exponentStart: Int = pos
        if (((pos < length) && ((code(text, pos) == 45) || (code(text, pos) == 43)))) {
            pos = pos + 1
        }
        val start: Int = pos
        while (((pos < length) && (code(text, pos) >= 48) && (code(text, pos) <= 57))) {
            pos = pos + 1
        }
        if ((pos == start)) {
            error("[big.js] Invalid number")
        }
        scale = parseExponent(text, exponentStart, pos)
    }
    if ((pos != length)) {
        error("[big.js] Invalid number")
    }
    var leading: Int = 0
    while (((leading < count) && (digits(leading) == 0))) {
        leading = leading + 1
    }
    normalizeFrom(sign, (((asFloat(places) + scale) - asFloat(leading)) - 1.0), cut(digits, 0, count), leading)
}

def coefficient(x: Decimal, idx: Int): Int = {
    (if (((idx >= 0) && (idx < size(x.c)))) x.c(idx) else 0)
}

def cmp(x: Decimal, y: Decimal): Int = {
    var result: Int = 0
    if ((x.c(0) == 0)) {
        result = (if ((y.c(0) == 0)) 0 else (-y.s))
    } else {
        if (((y.c(0) == 0) || (x.s != y.s))) {
            result = x.s
        } else {
            if ((x.e != y.e)) {
                result = (if ((x.e > y.e)) x.s else (-x.s))
            } else {
                var i: Int = 0
                val limit: Int = min(size(x.c), size(y.c))
                while (((i < limit) && (x.c(i) == y.c(i)))) {
                    i = i + 1
                }
                if ((i < limit)) {
                    result = (if ((x.c(i) > y.c(i))) x.s else (-x.s))
                } else {
                    if ((size(x.c) != size(y.c))) {
                        result = (if ((size(x.c) > size(y.c))) x.s else (-x.s))
                    }
                }
            }
        }
    }
    result
}

def add(x: Decimal, y: Decimal, minus: Boolean): Decimal = {
    val sign: Int = (if (minus) (-y.s) else y.s)
    var result: Decimal = dec(sign, y.e, y.c)
    if ((x.c(0) == 0)) {
        if (((y.c(0) == 0) && (sign != x.s))) {
            result = dec(1, 0.0, filled(1, 0))
        }
    } else {
        if ((y.c(0) == 0)) {
            result = x
        } else {
            val same: Boolean = (x.s == sign)
            val reverse: Boolean = ((!same) && ((cmp(x, dec(x.s, y.e, y.c)) * x.s) < 0))
            val top: Double = maxExp(x.e, y.e)
            val offsetX: Int = asIndex((top - x.e))
            val offsetY: Int = asIndex((top - y.e))
            val width: Int = max((offsetX + size(x.c)), (offsetY + size(y.c)))
            val digits: Array[Int] = filled((width + 1), 0)
            var carry: Int = 0
            var p: Int = (width - 1)
            while ((p >= 0)) {
                val a: Int = coefficient(x, (p - offsetX))
                val b: Int = coefficient(y, (p - offsetY))
                val total: Int = (carry + (if (same) (a + b) else (if (reverse) (b - a) else (a - b))))
                carry = (if ((total < 0)) (-1) else (total / 10))
                digits((p + 1)) = (total - (carry * 10))
                p = p - 1
            }
            digits(0) = carry
            result = normalizeFrom((if (reverse) (-x.s) else x.s), (if ((carry != 0)) (top + 1.0) else top), digits, (if ((carry != 0)) 0 else 1))
            if ((result.c(0) == 0)) {
                result = dec(1, 0.0, result.c)
            }
        }
    }
    result
}

def times(x: Decimal, y: Decimal): Decimal = {
    val digits: Array[Int] = filled((size(x.c) + size(y.c)), 0)
    var i: Int = (size(x.c) - 1)
    while ((i >= 0)) {
        var carry: Int = 0
        var j: Int = (size(y.c) - 1)
        while ((j >= 0)) {
            val k: Int = ((i + j) + 1)
            val v: Int = (((x.c(i) * y.c(j)) + digits(k)) + carry)
            digits(k) = (v % 10)
            carry = (v / 10)
            j = j - 1
        }
        digits(i) = carry
        i = i - 1
    }
    val exponent: Double = (x.e + y.e)
    normalizeFrom((x.s * y.s), (if ((digits(0) == 0)) exponent else (exponent + 1.0)), digits, (if ((digits(0) == 0)) 1 else 0))
}

def rounded(x: Decimal, precision: Double, mode: Int, more: Boolean): Decimal = {
    if (((mode < 0) || (mode > 3))) {
        error("[big.js] Invalid rounding mode")
    }
    var result: Decimal = x
    if ((precision < asFloat(size(x.c)))) {
        val sd: Int = asIndex(maxExp(precision, (-1.0)))
        val guardDigit: Int = (if ((sd >= 0)) x.c(sd) else 0)
        val odd: Boolean = ((sd > 0) && ((x.c((sd - 1)) % 2) == 1))
        val sticky: Boolean = (((sd + 1) < size(x.c)) || more)
        val up: Boolean = (((mode == 1) && (guardDigit >= 5)) || ((mode == 2) && ((guardDigit > 5) || ((guardDigit == 5) && (odd || sticky)))) || ((mode == 3) && ((x.c(0) != 0) || more)))
        if ((sd <= 0)) {
            result = (if (up) dec(x.s, ((x.e - precision) + 1.0), filled(1, 1)) else dec(x.s, 0.0, filled(1, 0)))
        } else {
            val digits: Array[Int] = cut(x.c, 0, sd)
            var pos: Int = (sd - 1)
            if (up) {
                while (((pos >= 0) && (digits(pos) == 9))) {
                    digits(pos) = 0
                    pos = pos - 1
                }
                if ((pos >= 0)) {
                    digits(pos) = (digits(pos) + 1)
                }
            }
            result = (if ((pos < 0)) dec(x.s, (x.e + 1.0), filled(1, 1)) else normalize(x.s, x.e, digits))
        }
    }
    result
}

def natural(a: Array[Int], b: Array[Int]): Int = {
    var result: Int = 0
    if ((size(a) != size(b))) {
        result = (if ((size(a) > size(b))) 1 else (-1))
    } else {
        var i: Int = 0
        while (((i < size(a)) && (a(i) == b(i)))) {
            i = i + 1
        }
        if ((i < size(a))) {
            result = (if ((a(i) > b(i))) 1 else (-1))
        }
    }
    result
}

def divide(x: Decimal, y: Decimal, dp: Int, mode: Int): Decimal = {
    if (((dp < 0) || (dp > 1000000))) {
        error("[big.js] Invalid decimal places")
    }
    if ((y.c(0) == 0)) {
        error("[big.js] Division by zero")
    }
    val sign: Int = (x.s * y.s)
    var result: Decimal = dec(sign, 0.0, filled(1, 0))
    if ((x.c(0) != 0)) {
        var rest: Array[Int] = filled(size(y.c), 0)
        var pos: Int = 0
        while ((pos < size(y.c))) {
            rest(pos) = (if ((pos < size(x.c))) x.c(pos) else 0)
            pos = pos + 1
        }
        var exponent: Double = (x.e - y.e)
        var digits: Array[Int] = filled(16, 0)
        var count: Int = 0
        var complete: Boolean = false
        while ((!complete)) {
            var quotient: Int = 0
            while ((natural(rest, y.c) >= 0)) {
                val shift: Int = (size(rest) - size(y.c))
                var borrow: Int = 0
                var k: Int = (size(rest) - 1)
                while ((k >= 0)) {
                    val dv: Int = (if ((k >= shift)) y.c((k - shift)) else 0)
                    val v: Int = ((rest(k) - dv) - borrow)
                    borrow = (if ((v < 0)) 1 else 0)
                    rest(k) = (v + (borrow * 10))
                    k = k - 1
                }
                var start: Int = 0
                while (((start < size(rest)) && (rest(start) == 0))) {
                    start = start + 1
                }
                if ((start > 0)) {
                    rest = cut(rest, start, size(rest))
                }
                quotient = quotient + 1
            }
            if (((quotient == 0) && (count == 0))) {
                exponent = (exponent - 1.0)
            } else {
                if ((count == size(digits))) {
                    digits = grow(digits)
                }
                digits(count) = quotient
                count = count + 1
            }
            val more: Boolean = ((size(rest) > 0) || (pos < size(x.c)))
            complete = ((count > 0) && ((!more) || (asFloat(count) > ((asFloat(dp) + exponent) + 1.0))))
            if (complete) {
                result = dec(sign, exponent, cut(digits, 0, count))
                if ((asFloat(count) > ((asFloat(dp) + exponent) + 1.0))) {
                    result = rounded(result, ((asFloat(dp) + exponent) + 1.0), mode, more)
                }
            } else {
                val nxt: Int = (if ((pos < size(x.c))) x.c(pos) else 0)
                if (((size(rest) > 0) || (nxt != 0))) {
                    rest = extend(rest, nxt)
                }
                pos = pos + 1
            }
        }
    }
    result
}

def power(x: Decimal, n: Int, dp: Int, mode: Int): Decimal = {
    if (((n < (-1000000)) || (n > 1000000))) {
        error("[big.js] Invalid exponent")
    }
    val one: Decimal = dec(1, 0.0, filled(1, 1))
    var result: Decimal = one
    var base: Decimal = x
    var i: Int = (if ((n < 0)) (-n) else n)
    while ((i > 0)) {
        if (((i % 2) == 1)) {
            result = times(result, base)
        }
        i = (i / 2)
        if ((i > 0)) {
            base = times(base, base)
        }
    }
    if ((n < 0)) {
        result = divide(one, result, dp, mode)
    }
    result
}

def render(x: Decimal, op: String, n: Int, mode: Int): String = {
    var v: Decimal = x
    if ((n != (-2000002))) {
        var sd: Double = asFloat(n)
        if ((op == "toPrecision")) {
            if (((n < 1) || (n > 1000000))) {
                error("[big.js] Invalid precision")
            }
        } else {
            if (((n < 0) || (n > 1000000))) {
                error("[big.js] Invalid decimal places")
            }
            sd = (if ((op == "toFixed")) ((asFloat(n) + v.e) + 1.0) else asFloat((n + 1)))
        }
        v = rounded(v, sd, mode, false)
        val width: Double = (if ((op == "toFixed")) ((asFloat(n) + v.e) + 1.0) else sd)
        if ((width > asFloat(size(v.c)))) {
            val padded: Array[Int] = filled(asIndex(width), 0)
            var i: Int = 0
            while ((i < size(v.c))) {
                padded(i) = v.c(i)
                i = i + 1
            }
            v = dec(v.s, v.e, padded)
        }
    }
    val exponential: Boolean = ((op == "toExponential") || ((op != "toFixed") && ((v.e <= (-7.0)) || (v.e >= 21.0) || ((op == "toPrecision") && (n != (-2000002)) && (asFloat(n) <= v.e)))))
    var text: String = (if (((x.s < 0) && (x.c(0) != 0))) "-" else "")
    if (exponential) {
        text = cat(text, letter((48 + v.c(0))))
        if ((size(v.c) > 1)) {
            text = cat(text, ".")
            var j: Int = 1
            while ((j < size(v.c))) {
                text = cat(text, letter((48 + v.c(j))))
                j = j + 1
            }
        }
        text = cat(text, (if ((v.e >= 0.0)) "e+" else "e"))
        text = cat(text, floatText(v.e))
    } else {
        val point: Int = asIndex(v.e)
        var p: Int = max(0, point)
        while ((p >= 0)) {
            text = cat(text, letter((48 + coefficient(v, (point - p)))))
            p = p - 1
        }
        val last: Int = ((point - size(v.c)) + 1)
        if ((last < 0)) {
            text = cat(text, ".")
            p = (-1)
            while ((p >= last)) {
                text = cat(text, letter((48 + coefficient(v, (point - p)))))
                p = p - 1
            }
        }
    }
    text
}

def root(x: Decimal, dp: Int, mode: Int): Decimal = {
    var result: Decimal = x
    if (((x.s > 0) && (dp >= 0) && (dp <= 999996) && (mode >= 0) && (mode <= 3) && (x.e < asFloat((((-2) * dp) - 2))))) {
        result = (if ((mode == 3)) dec(1, asFloat((-dp)), filled(1, 1)) else dec(1, 0.0, filled(1, 0)))
    } else {
        if ((x.c(0) != 0)) {
            if ((x.s < 0)) {
                error("[big.js] No square root")
            }
            val estimate: Double = machineRoot(render(x, "toExponential", (-2000002), mode))
            if (((estimate > 0.0) && finite(estimate))) {
                result = parse(floatText(estimate))
            } else {
                var coefficient: String = ""
                var i: Int = 0
                while ((i < size(x.c))) {
                    coefficient = cat(coefficient, letter((48 + x.c(i))))
                    i = i + 1
                }
                if (even((asFloat(size(x.c)) + x.e))) {
                    coefficient = cat(coefficient, "0")
                }
                val fallback: Double = machineRoot(coefficient)
                val leading: Decimal = (if (finite(fallback)) parse(floatText(fallback)) else dec(1, 0.0, filled(1, 5)))
                result = dec(1, halvedExponent(x.e), leading.c)
            }
            val half: Decimal = dec(1, (-1.0), filled(1, 5))
            val work: Int = asIndex(minExp(1000001.0, maxExp(asFloat((dp + 4)), (5.0 - result.e))))
            val count: Double = maxExp(1.0, (result.e + asFloat(work)))
            var matched: Boolean = false
            while ((!matched)) {
                val nxt: Decimal = times(half, add(result, divide(x, result, work, mode), false))
                matched = (result.e == nxt.e)
                val length: Int = asIndex(minExp(count, asFloat(max(size(result.c), size(nxt.c)))))
                var j: Int = 0
                while ((matched && (j < length))) {
                    matched = ((j < size(result.c)) && (j < size(nxt.c)) && (result.c(j) == nxt.c(j)))
                    j = j + 1
                }
                result = nxt
            }
            result = rounded(result, ((asFloat(dp) + result.e) + 1.0), mode, false)
        }
    }
    result
}

def operate(op: String, xText: String, yText: String, n: Int, dp: Int, rm: Int, mode: Int): String = {
    val x: Decimal = parse(xText)
    var result: Decimal = x
    var out: String = ""
    if (((op == "plus") || (op == "minus") || (op == "times") || (op == "div") || (op == "cmp"))) {
        val y: Decimal = parse(yText)
        if ((op == "cmp")) {
            out = strint(cmp(x, y))
        } else {
            if ((op == "times")) {
                result = times(x, y)
            } else {
                if ((op == "div")) {
                    result = divide(x, y, dp, rm)
                } else {
                    result = add(x, y, (op == "minus"))
                }
            }
        }
    } else {
        if ((op == "round")) {
            val places: Int = (if ((n == (-2000002))) 0 else n)
            if (((places < (-1000000)) || (places > 1000000))) {
                error("[big.js] Invalid decimal places")
            }
            result = rounded(x, ((asFloat(places) + x.e) + 1.0), mode, false)
        } else {
            if ((op == "pow")) {
                result = power(x, n, dp, rm)
            } else {
                if ((op == "sqrt")) {
                    result = root(x, dp, rm)
                } else {
                    if (((op == "toString") || (op == "toExponential") || (op == "toFixed") || (op == "toPrecision"))) {
                        out = render(x, op, (if ((op == "toString")) (-2000002) else n), mode)
                    } else {
                        error("unknown decimal operation")
                    }
                }
            }
        }
    }
    if ((out == "")) {
        out = render(result, "toString", (-2000002), mode)
    }
    out
}
}
