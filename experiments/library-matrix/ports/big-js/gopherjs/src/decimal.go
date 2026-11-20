// Decimal coefficients contain normalized base-ten digits, most significant first.
package main

import (
	"github.com/gopherjs/gopherjs/js"
	"math"
	"strconv"
)

type decimal struct {
	s int
	e float64
	c []int
}

func dec(s int, e float64, c []int) decimal { return decimal{s, e, c} }
func filled(n, d int) []int {
	a := make([]int, n)
	if d != 0 {
		for i := range a {
			a[i] = d
		}
	}
	return a
}
func cut(a []int, start, stop int) []int { return append([]int(nil), a[start:stop]...) }
func extend(a []int, d int) []int        { return append(a, d) }
func code(s string, i int) int           { return int(s[i]) }
func letter(n int) string                { return string(rune(n)) }
func strint(n int) string                { return strconv.Itoa(n) }
func cat(a, b string) string             { return a + b }
func error(msg string)                   { panic(msg) }
func machineRoot(s string) float64       { return math.Sqrt(js.Global.Get("Number").Invoke(s).Float()) }
func finite(x float64) bool              { return !math.IsInf(x, 0) && !math.IsNaN(x) }
func floatText(x float64) string         { return js.Global.Get("String").Invoke(x).String() }
func asFloat(n int) float64              { return float64(n) }
func asIndex(n float64) int              { return int(n) }
func maxExp(x, y float64) float64        { return math.Max(x, y) }
func minExp(x, y float64) float64        { return math.Min(x, y) }
func even(e float64) bool                { return math.Mod(e, 2) == 0 }
func halvedExponent(e float64) float64   { return math.Floor(e / 2) }
func parseExponent(s string, start, stop int) float64 {
	return js.Global.Get("Number").Invoke(s[start:stop]).Float()
}
func grow(a []int) []int { return append(a, make([]int, len(a))...) }

func normalizeFrom(sign int, exponent float64, digits []int, offset int) decimal {
	start := offset
	for (start < len(digits)) && (digits[start] == 0) {
		start = start + 1
	}
	stop := len(digits)
	for (stop > start) && (digits[(stop-1)] == 0) {
		stop = stop - 1
	}
	out := dec(sign, 0.0, filled(1, 0))
	if stop > start {
		out = dec(sign, (exponent - asFloat((start - offset))), cut(digits, start, stop))
	}
	return out
}

func normalize(sign int, exponent float64, digits []int) decimal {
	return normalizeFrom(sign, exponent, digits, 0)
}

func parse(text string) decimal {
	length := len(text)
	pos := 0
	sign := 1
	if (pos < length) && (code(text, pos) == 45) {
		sign = (-1)
		pos = pos + 1
	}
	digits := filled(length, 0)
	count := 0
	places := 0
	for (pos < length) && (code(text, pos) >= 48) && (code(text, pos) <= 57) {
		digits[count] = (code(text, pos) - 48)
		count = count + 1
		pos = pos + 1
		places = places + 1
	}
	if (pos < length) && (code(text, pos) == 46) {
		pos = pos + 1
		for (pos < length) && (code(text, pos) >= 48) && (code(text, pos) <= 57) {
			digits[count] = (code(text, pos) - 48)
			count = count + 1
			pos = pos + 1
		}
	}
	if count == 0 {
		error("[big.js] Invalid number")
	}
	scale := 0.0
	if (pos < length) && ((code(text, pos) == 69) || (code(text, pos) == 101)) {
		pos = pos + 1
		exponentStart := pos
		if (pos < length) && ((code(text, pos) == 45) || (code(text, pos) == 43)) {
			pos = pos + 1
		}
		start := pos
		for (pos < length) && (code(text, pos) >= 48) && (code(text, pos) <= 57) {
			pos = pos + 1
		}
		if pos == start {
			error("[big.js] Invalid number")
		}
		scale = parseExponent(text, exponentStart, pos)
	}
	if pos != length {
		error("[big.js] Invalid number")
	}
	leading := 0
	for (leading < count) && (digits[leading] == 0) {
		leading = leading + 1
	}
	return normalizeFrom(sign, (((asFloat(places) + scale) - asFloat(leading)) - 1.0), cut(digits, 0, count), leading)
}

func coefficient(x decimal, idx int) int {
	return (func() int {
		if (idx >= 0) && (idx < len(x.c)) {
			return x.c[idx]
		}
		return 0
	}())
}

func cmp(x decimal, y decimal) int {
	result := 0
	if x.c[0] == 0 {
		result = (func() int {
			if y.c[0] == 0 {
				return 0
			}
			return (-y.s)
		}())
	} else {
		if (y.c[0] == 0) || (x.s != y.s) {
			result = x.s
		} else {
			if x.e != y.e {
				result = (func() int {
					if x.e > y.e {
						return x.s
					}
					return (-x.s)
				}())
			} else {
				i := 0
				limit := min(len(x.c), len(y.c))
				for (i < limit) && (x.c[i] == y.c[i]) {
					i = i + 1
				}
				if i < limit {
					result = (func() int {
						if x.c[i] > y.c[i] {
							return x.s
						}
						return (-x.s)
					}())
				} else {
					if len(x.c) != len(y.c) {
						result = (func() int {
							if len(x.c) > len(y.c) {
								return x.s
							}
							return (-x.s)
						}())
					}
				}
			}
		}
	}
	return result
}

func add(x decimal, y decimal, minus bool) decimal {
	sign := (func() int {
		if minus {
			return (-y.s)
		}
		return y.s
	}())
	result := dec(sign, y.e, y.c)
	if x.c[0] == 0 {
		if (y.c[0] == 0) && (sign != x.s) {
			result = dec(1, 0.0, filled(1, 0))
		}
	} else {
		if y.c[0] == 0 {
			result = x
		} else {
			same := (x.s == sign)
			reverse := ((!same) && ((cmp(x, dec(x.s, y.e, y.c)) * x.s) < 0))
			top := maxExp(x.e, y.e)
			offsetX := asIndex((top - x.e))
			offsetY := asIndex((top - y.e))
			width := max((offsetX + len(x.c)), (offsetY + len(y.c)))
			digits := filled((width + 1), 0)
			carry := 0
			p := (width - 1)
			for p >= 0 {
				a := coefficient(x, (p - offsetX))
				b := coefficient(y, (p - offsetY))
				total := (carry + (func() int {
					if same {
						return (a + b)
					}
					return (func() int {
						if reverse {
							return (b - a)
						}
						return (a - b)
					}())
				}()))
				carry = (func() int {
					if total < 0 {
						return (-1)
					}
					return (total / 10)
				}())
				digits[(p + 1)] = (total - (carry * 10))
				p = p - 1
			}
			digits[0] = carry
			result = normalizeFrom((func() int {
				if reverse {
					return (-x.s)
				}
				return x.s
			}()), (func() float64 {
				if carry != 0 {
					return (top + 1.0)
				}
				return top
			}()), digits, (func() int {
				if carry != 0 {
					return 0
				}
				return 1
			}()))
			if result.c[0] == 0 {
				result = dec(1, 0.0, result.c)
			}
		}
	}
	return result
}

func times(x decimal, y decimal) decimal {
	digits := filled((len(x.c) + len(y.c)), 0)
	i := (len(x.c) - 1)
	for i >= 0 {
		carry := 0
		j := (len(y.c) - 1)
		for j >= 0 {
			k := ((i + j) + 1)
			v := (((x.c[i] * y.c[j]) + digits[k]) + carry)
			digits[k] = (v % 10)
			carry = (v / 10)
			j = j - 1
		}
		digits[i] = carry
		i = i - 1
	}
	exponent := (x.e + y.e)
	return normalizeFrom((x.s * y.s), (func() float64 {
		if digits[0] == 0 {
			return exponent
		}
		return (exponent + 1.0)
	}()), digits, (func() int {
		if digits[0] == 0 {
			return 1
		}
		return 0
	}()))
}

func rounded(x decimal, precision float64, mode int, more bool) decimal {
	if (mode < 0) || (mode > 3) {
		error("[big.js] Invalid rounding mode")
	}
	result := x
	if precision < asFloat(len(x.c)) {
		sd := asIndex(maxExp(precision, (-1.0)))
		guardDigit := (func() int {
			if sd >= 0 {
				return x.c[sd]
			}
			return 0
		}())
		odd := ((sd > 0) && ((x.c[(sd-1)] % 2) == 1))
		sticky := (((sd + 1) < len(x.c)) || more)
		up := (((mode == 1) && (guardDigit >= 5)) || ((mode == 2) && ((guardDigit > 5) || ((guardDigit == 5) && (odd || sticky)))) || ((mode == 3) && ((x.c[0] != 0) || more)))
		if sd <= 0 {
			result = (func() decimal {
				if up {
					return dec(x.s, ((x.e - precision) + 1.0), filled(1, 1))
				}
				return dec(x.s, 0.0, filled(1, 0))
			}())
		} else {
			digits := cut(x.c, 0, sd)
			pos := (sd - 1)
			if up {
				for (pos >= 0) && (digits[pos] == 9) {
					digits[pos] = 0
					pos = pos - 1
				}
				if pos >= 0 {
					digits[pos] = (digits[pos] + 1)
				}
			}
			result = (func() decimal {
				if pos < 0 {
					return dec(x.s, (x.e + 1.0), filled(1, 1))
				}
				return normalize(x.s, x.e, digits)
			}())
		}
	}
	return result
}

func natural(a []int, b []int) int {
	result := 0
	if len(a) != len(b) {
		result = (func() int {
			if len(a) > len(b) {
				return 1
			}
			return (-1)
		}())
	} else {
		i := 0
		for (i < len(a)) && (a[i] == b[i]) {
			i = i + 1
		}
		if i < len(a) {
			result = (func() int {
				if a[i] > b[i] {
					return 1
				}
				return (-1)
			}())
		}
	}
	return result
}

func divide(x decimal, y decimal, dp int, mode int) decimal {
	if (dp < 0) || (dp > 1000000) {
		error("[big.js] Invalid decimal places")
	}
	if y.c[0] == 0 {
		error("[big.js] Division by zero")
	}
	sign := (x.s * y.s)
	result := dec(sign, 0.0, filled(1, 0))
	if x.c[0] != 0 {
		rest := filled(len(y.c), 0)
		pos := 0
		for pos < len(y.c) {
			rest[pos] = (func() int {
				if pos < len(x.c) {
					return x.c[pos]
				}
				return 0
			}())
			pos = pos + 1
		}
		exponent := (x.e - y.e)
		digits := filled(16, 0)
		count := 0
		complete := false
		for !complete {
			quotient := 0
			for natural(rest, y.c) >= 0 {
				shift := (len(rest) - len(y.c))
				borrow := 0
				k := (len(rest) - 1)
				for k >= 0 {
					dv := (func() int {
						if k >= shift {
							return y.c[(k - shift)]
						}
						return 0
					}())
					v := ((rest[k] - dv) - borrow)
					borrow = (func() int {
						if v < 0 {
							return 1
						}
						return 0
					}())
					rest[k] = (v + (borrow * 10))
					k = k - 1
				}
				start := 0
				for (start < len(rest)) && (rest[start] == 0) {
					start = start + 1
				}
				if start > 0 {
					rest = cut(rest, start, len(rest))
				}
				quotient = quotient + 1
			}
			if (quotient == 0) && (count == 0) {
				exponent = (exponent - 1.0)
			} else {
				if count == len(digits) {
					digits = grow(digits)
				}
				digits[count] = quotient
				count = count + 1
			}
			more := ((len(rest) > 0) || (pos < len(x.c)))
			complete = ((count > 0) && ((!more) || (asFloat(count) > ((asFloat(dp) + exponent) + 1.0))))
			if complete {
				result = dec(sign, exponent, cut(digits, 0, count))
				if asFloat(count) > ((asFloat(dp) + exponent) + 1.0) {
					result = rounded(result, ((asFloat(dp) + exponent) + 1.0), mode, more)
				}
			} else {
				nxt := (func() int {
					if pos < len(x.c) {
						return x.c[pos]
					}
					return 0
				}())
				if (len(rest) > 0) || (nxt != 0) {
					rest = extend(rest, nxt)
				}
				pos = pos + 1
			}
		}
	}
	return result
}

func power(x decimal, n int, dp int, mode int) decimal {
	if (n < (-1000000)) || (n > 1000000) {
		error("[big.js] Invalid exponent")
	}
	one := dec(1, 0.0, filled(1, 1))
	result := one
	base := x
	i := (func() int {
		if n < 0 {
			return (-n)
		}
		return n
	}())
	for i > 0 {
		if (i % 2) == 1 {
			result = times(result, base)
		}
		i = (i / 2)
		if i > 0 {
			base = times(base, base)
		}
	}
	if n < 0 {
		result = divide(one, result, dp, mode)
	}
	return result
}

func render(x decimal, op string, n int, mode int) string {
	v := x
	if n != (-2000002) {
		sd := asFloat(n)
		if op == "toPrecision" {
			if (n < 1) || (n > 1000000) {
				error("[big.js] Invalid precision")
			}
		} else {
			if (n < 0) || (n > 1000000) {
				error("[big.js] Invalid decimal places")
			}
			sd = (func() float64 {
				if op == "toFixed" {
					return ((asFloat(n) + v.e) + 1.0)
				}
				return asFloat((n + 1))
			}())
		}
		v = rounded(v, sd, mode, false)
		width := (func() float64 {
			if op == "toFixed" {
				return ((asFloat(n) + v.e) + 1.0)
			}
			return sd
		}())
		if width > asFloat(len(v.c)) {
			padded := filled(asIndex(width), 0)
			i := 0
			for i < len(v.c) {
				padded[i] = v.c[i]
				i = i + 1
			}
			v = dec(v.s, v.e, padded)
		}
	}
	exponential := ((op == "toExponential") || ((op != "toFixed") && ((v.e <= (-7.0)) || (v.e >= 21.0) || ((op == "toPrecision") && (n != (-2000002)) && (asFloat(n) <= v.e)))))
	text := (func() string {
		if (x.s < 0) && (x.c[0] != 0) {
			return "-"
		}
		return ""
	}())
	if exponential {
		text = cat(text, letter((48 + v.c[0])))
		if len(v.c) > 1 {
			text = cat(text, ".")
			j := 1
			for j < len(v.c) {
				text = cat(text, letter((48 + v.c[j])))
				j = j + 1
			}
		}
		text = cat(text, (func() string {
			if v.e >= 0.0 {
				return "e+"
			}
			return "e"
		}()))
		text = cat(text, floatText(v.e))
	} else {
		point := asIndex(v.e)
		p := max(0, point)
		for p >= 0 {
			text = cat(text, letter((48 + coefficient(v, (point-p)))))
			p = p - 1
		}
		last := ((point - len(v.c)) + 1)
		if last < 0 {
			text = cat(text, ".")
			p = (-1)
			for p >= last {
				text = cat(text, letter((48 + coefficient(v, (point-p)))))
				p = p - 1
			}
		}
	}
	return text
}

func root(x decimal, dp int, mode int) decimal {
	result := x
	if (x.s > 0) && (dp >= 0) && (dp <= 999996) && (mode >= 0) && (mode <= 3) && (x.e < asFloat((((-2) * dp) - 2))) {
		result = (func() decimal {
			if mode == 3 {
				return dec(1, asFloat((-dp)), filled(1, 1))
			}
			return dec(1, 0.0, filled(1, 0))
		}())
	} else {
		if x.c[0] != 0 {
			if x.s < 0 {
				error("[big.js] No square root")
			}
			estimate := machineRoot(render(x, "toExponential", (-2000002), mode))
			if (estimate > 0.0) && finite(estimate) {
				result = parse(floatText(estimate))
			} else {
				coefficient := ""
				i := 0
				for i < len(x.c) {
					coefficient = cat(coefficient, letter((48 + x.c[i])))
					i = i + 1
				}
				if even((asFloat(len(x.c)) + x.e)) {
					coefficient = cat(coefficient, "0")
				}
				fallback := machineRoot(coefficient)
				leading := (func() decimal {
					if finite(fallback) {
						return parse(floatText(fallback))
					}
					return dec(1, 0.0, filled(1, 5))
				}())
				result = dec(1, halvedExponent(x.e), leading.c)
			}
			half := dec(1, (-1.0), filled(1, 5))
			work := asIndex(minExp(1000001.0, maxExp(asFloat((dp+4)), (5.0-result.e))))
			count := maxExp(1.0, (result.e + asFloat(work)))
			matched := false
			for !matched {
				nxt := times(half, add(result, divide(x, result, work, mode), false))
				matched = (result.e == nxt.e)
				length := asIndex(minExp(count, asFloat(max(len(result.c), len(nxt.c)))))
				j := 0
				for matched && (j < length) {
					matched = ((j < len(result.c)) && (j < len(nxt.c)) && (result.c[j] == nxt.c[j]))
					j = j + 1
				}
				result = nxt
			}
			result = rounded(result, ((asFloat(dp) + result.e) + 1.0), mode, false)
		}
	}
	return result
}

func operate(op string, xText string, yText string, n int, dp int, rm int, mode int) string {
	x := parse(xText)
	result := x
	out := ""
	if (op == "plus") || (op == "minus") || (op == "times") || (op == "div") || (op == "cmp") {
		y := parse(yText)
		if op == "cmp" {
			out = strint(cmp(x, y))
		} else {
			if op == "times" {
				result = times(x, y)
			} else {
				if op == "div" {
					result = divide(x, y, dp, rm)
				} else {
					result = add(x, y, (op == "minus"))
				}
			}
		}
	} else {
		if op == "round" {
			places := (func() int {
				if n == (-2000002) {
					return 0
				}
				return n
			}())
			if (places < (-1000000)) || (places > 1000000) {
				error("[big.js] Invalid decimal places")
			}
			result = rounded(x, ((asFloat(places) + x.e) + 1.0), mode, false)
		} else {
			if op == "pow" {
				result = power(x, n, dp, rm)
			} else {
				if op == "sqrt" {
					result = root(x, dp, rm)
				} else {
					if (op == "toString") || (op == "toExponential") || (op == "toFixed") || (op == "toPrecision") {
						out = render(x, op, (func() int {
							if op == "toString" {
								return (-2000002)
							}
							return n
						}()), mode)
					} else {
						error("unknown decimal operation")
					}
				}
			}
		}
	}
	if out == "" {
		out = render(result, "toString", (-2000002), mode)
	}
	return out
}
