// big.js 7.0.1 (big.mjs): the decimal type, its parser, round, stringify and the methods the
// contract uses, function by function and loop by loop. A coefficient is a []int of digits, most
// significant first; upstream's array mutations are slice operations (shift is r = r[1:], unshift
// and prepending are appends onto a new slice). The exponent is a float64 because upstream's is a
// JS number: past int32 and 2^53 it rounds as a double does, and Infinity and NaN exponents keep
// upstream's comparison chains. Upstream throws; here every failure is an error value.
package main

import (
	"math"
	"strings"
)

const (
	maxDP    = 1e6
	maxPower = 1e6
	ne       = -7
	pe       = 21
)

type bigError string

func (e bigError) Error() string { return string(e) }

const (
	errInvalidNumber    bigError = "[big.js] Invalid number"
	errInvalidDP        bigError = "[big.js] Invalid decimal places"
	errInvalidRM        bigError = "[big.js] Invalid rounding mode"
	errInvalidPrecision bigError = "[big.js] Invalid precision"
	errInvalidExponent  bigError = "[big.js] Invalid exponent"
	errDivByZero        bigError = "[big.js] Division by zero"
	errNoRoot           bigError = "[big.js] No square root"
)

// Big is a decimal: sign s, exponent e and coefficient digits c.
type Big struct {
	s int
	e float64
	c []int
}

func (x *Big) copy() *Big { return &Big{x.s, x.e, append([]int(nil), x.c...)} }

// isIntIn is upstream's `n !== ~~n || n < lo || n > hi` check, negated: an integer in the range
// (all ranges lie within int32).
func isIntIn(n, lo, hi float64) bool { return n >= lo && n <= hi && n == math.Floor(n) }

// truthy is JS truthiness of a number, for upstream's `if (a = xe - ye)`.
func truthy(n float64) bool { return n != 0 && !math.IsNaN(n) }

// toInt32 is ECMAScript ToInt32, for upstream's `| 0` and `&` on exponents. GopherJS's math.Trunc
// is wrong above 2^31, so the truncation is Floor or Ceil.
func toInt32(v float64) int32 {
	if math.IsNaN(v) || math.IsInf(v, 0) {
		return 0
	}
	if v < 0 {
		v = math.Ceil(v)
	} else {
		v = math.Floor(v)
	}
	m := math.Mod(v, 4294967296)
	if m >= 2147483648 {
		m -= 4294967296
	} else if m < -2147483648 {
		m += 4294967296
	}
	return int32(m)
}

// head is `Array.prototype.slice(0, end)`, where a negative end counts from the back.
func head(c []int, end float64) []int {
	n := float64(len(c))
	if end < 0 {
		end = math.Max(n+end, 0)
	} else {
		end = math.Min(end, n)
	}
	return c[:int(end)]
}

func equal(a, b []int) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func zeros(n int) []int { return make([]int, n) }

func digitString(c []int) string {
	b := make([]byte, len(c))
	for i, d := range c {
		b[i] = byte('0' + d)
	}
	return string(b)
}

// skipDigits returns the index of the first non-digit of n at or after i.
func skipDigits(n string, i int) int {
	for i < len(n) && '0' <= n[i] && n[i] <= '9' {
		i++
	}
	return i
}

// numeric reports whether n matches upstream's NUMERIC, /^-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$/i:
// ASCII digits only, nothing trimmed, and $ is the end of the string.
func numeric(n string) bool {
	i := 0
	if i < len(n) && n[i] == '-' {
		i++
	}
	start := i
	i = skipDigits(n, i)
	whole, fraction := i-start, 0
	if i < len(n) && n[i] == '.' {
		start = i + 1
		i = skipDigits(n, start)
		fraction = i - start
	}
	if whole == 0 && fraction == 0 {
		return false
	}
	if i < len(n) && (n[i] == 'e' || n[i] == 'E') {
		i++
		if i < len(n) && (n[i] == '+' || n[i] == '-') {
			i++
		}
		if start = i; skipDigits(n, i) == start {
			return false
		}
		i = skipDigits(n, i)
	}
	return i == len(n)
}

// parse is upstream's parse: the decimal string n, validated against NUMERIC.
func parse(n string) (*Big, error) {
	if !numeric(n) {
		return nil, errInvalidNumber
	}
	// Determine sign.
	x := &Big{s: 1}
	if n[0] == '-' {
		n = n[1:]
		x.s = -1
	}
	// Decimal point?
	dot := strings.IndexByte(n, '.')
	if dot > -1 {
		n = n[:dot] + n[dot+1:]
	}
	// Exponential form?
	var e float64
	if i := strings.IndexAny(n, "eE"); i > 0 {
		// Determine exponent.
		if dot < 0 {
			dot = i
		}
		e = float64(dot) + jsNumber(n[i+1:])
		n = n[:i]
	} else if dot < 0 {
		// Integer.
		e = float64(len(n))
	} else {
		e = float64(dot)
	}
	// Determine leading zeros.
	lead := 0
	for lead < len(n) && n[lead] == '0' {
		lead++
	}
	if lead == len(n) {
		// Zero.
		x.c = []int{0}
		return x, nil
	}
	// Determine trailing zeros.
	last := len(n) - 1
	for n[last] == '0' {
		last--
	}
	x.e = e - float64(lead) - 1
	// Convert string to array of digits without leading/trailing zeros.
	x.c = make([]int, 0, last-lead+1)
	for _, ch := range []byte(n[lead : last+1]) {
		x.c = append(x.c, int(ch-'0'))
	}
	return x, nil
}

// round is upstream's round: x rounded in place to sd significant digits using rounding mode
// rm. more says whether a division was truncated.
func round(x *Big, sd, rm float64, more bool) error {
	if rm != 0 && rm != 1 && rm != 2 && rm != 3 {
		return errInvalidRM
	}
	xc := x.c
	if sd < 1 {
		up := rm == 3 && (more || xc[0] != 0) || sd == 0 &&
			(rm == 1 && xc[0] >= 5 || rm == 2 && (xc[0] > 5 || xc[0] == 5 && (more || len(xc) > 1)))
		xc = xc[:1]
		if up {
			// 1, 0.1, 0.01, 0.001, 0.0001 etc.
			x.e = x.e - sd + 1
			xc[0] = 1
		} else {
			// Zero.
			xc[0] = 0
			x.e = 0
		}
	} else if sd < float64(len(xc)) {
		i := int(sd)
		// xc[i] is the digit after the digit that may be rounded up.
		up := rm == 1 && xc[i] >= 5 ||
			rm == 2 && (xc[i] > 5 || xc[i] == 5 && (more || i+1 < len(xc) || xc[i-1]%2 == 1)) ||
			rm == 3 && (more || xc[0] != 0)
		// Remove any digits after the required precision.
		xc = xc[:i]
		// Round up? Rounding up may mean the previous digit has to be rounded up.
		for up {
			i--
			xc[i]++
			if xc[i] <= 9 {
				break
			}
			xc[i] = 0
			if i == 0 {
				x.e++
				xc = append([]int{1}, xc...)
				break
			}
		}
		// Remove trailing zeros.
		for xc[len(xc)-1] == 0 {
			xc = xc[:len(xc)-1]
		}
	}
	x.c = xc
	return nil
}

// stringify is upstream's stringify: x in normal or exponential notation.
func stringify(x *Big, doExponential, isNonzero bool) string {
	e := x.e
	s := digitString(x.c)
	n := len(s)
	if doExponential {
		// Exponential notation.
		mantissa := s[:1]
		if n > 1 {
			mantissa += "." + s[1:]
		}
		sign := "e+"
		if e < 0 {
			sign = "e"
		}
		s = mantissa + sign + jsString(e)
	} else if e < 0 {
		// Normal notation.
		s = "0." + strings.Repeat("0", int(-e-1)) + s
	} else if e > 0 {
		if point := int(e) + 1; point > n {
			s += strings.Repeat("0", point-n)
		} else if point < n {
			s = s[:point] + "." + s[point:]
		}
	} else if n > 1 {
		s = s[:1] + "." + s[1:]
	}
	if x.s < 0 && isNonzero {
		return "-" + s
	}
	return s
}

// cmp returns 1, -1 or 0 as x is greater than, less than or equal to y.
func (x *Big) cmp(y *Big) int {
	xc, yc := x.c, y.c
	i, j := x.s, y.s
	// Either zero?
	if xc[0] == 0 || yc[0] == 0 {
		switch {
		case xc[0] != 0:
			return i
		case yc[0] != 0:
			return -j
		}
		return 0
	}
	// Signs differ?
	if i != j {
		return i
	}
	isneg := i < 0
	var greater bool
	if x.e != y.e {
		// Compare exponents.
		greater = x.e > y.e
	} else {
		// Compare digit by digit, then lengths.
		k := 0
		for k < len(xc) && k < len(yc) && xc[k] == yc[k] {
			k++
		}
		switch {
		case k < len(xc) && k < len(yc):
			greater = xc[k] > yc[k]
		case len(xc) == len(yc):
			return 0
		default:
			greater = len(xc) > len(yc)
		}
	}
	if greater != isneg {
		return 1
	}
	return -1
}

// div is x divided by y, rounded if necessary to dp decimal places using rounding mode rm.
func (x *Big) div(y *Big, dp, rm float64) (*Big, error) {
	a, b := x.c, y.c // dividend, divisor
	sign := -1
	if x.s == y.s {
		sign = 1
	}
	if !isIntIn(dp, 0, maxDP) {
		return nil, errInvalidDP
	}
	// Divisor is zero?
	if b[0] == 0 {
		return nil, errDivByZero
	}
	// Dividend is 0? Return +-0.
	if a[0] == 0 {
		return &Big{sign, 0, []int{0}}, nil
	}

	bl, al := len(b), len(a)
	ai := bl
	bz := append([]int{0}, b...)                // divisor with a leading zero
	r := append([]int(nil), a[:min(bl, al)]...) // remainder
	var qc []int                                // quotient
	qe := x.e - y.e
	p := dp + qe + 1 // precision of the result
	k := p
	if p < 0 {
		k = 0
	}

	// Add zeros to make remainder as long as divisor.
	for len(r) < bl {
		r = append(r, 0)
	}

	for {
		// n is how many times the divisor goes into current remainder.
		n, cmp := 0, 0
		for ; n < 10; n++ {
			// Compare divisor and remainder.
			rl := len(r)
			if bl != rl {
				cmp = 1
				if bl < rl {
					cmp = -1
				}
			} else {
				cmp = 0
				for ri := 0; ri < bl; ri++ {
					if b[ri] != r[ri] {
						cmp = 1
						if b[ri] < r[ri] {
							cmp = -1
						}
						break
					}
				}
			}
			// If divisor < remainder, subtract divisor from remainder.
			if cmp >= 0 {
				break
			}
			// Remainder can't be more than 1 digit longer than divisor.
			// Equalise lengths using divisor with extra leading zero?
			bt := bz
			if rl == bl {
				bt = b
			}
			for rl > 0 {
				rl--
				if r[rl] < bt[rl] {
					ri := rl - 1
					for ri > 0 && r[ri] == 0 {
						r[ri] = 9
						ri--
					}
					r[ri]--
					r[rl] += 10
				}
				r[rl] -= bt[rl]
			}
			for r[0] == 0 {
				r = r[1:]
			}
		}
		// Add the digit n to the result array.
		if cmp == 0 {
			n++
		}
		qc = append(qc, n)
		// Update the remainder. Upstream's [undefined], the dividend's end, is the empty slice.
		if len(r) > 0 && r[0] != 0 && cmp != 0 {
			next := 0
			if ai < al {
				next = a[ai]
			}
			r = append(r, next)
		} else if ai < al {
			r = []int{a[ai]}
		} else {
			r = nil
		}
		more := ai < al || len(r) > 0
		ai++
		if !more {
			break
		}
		// `k--` is falsy at 0 and at NaN, as `k > 0` is.
		next := k > 0
		k--
		if !next {
			break
		}
	}

	// Leading zero? Do not remove if result is simply zero (qi == 1).
	qi := len(qc)
	if qc[0] == 0 && qi != 1 {
		// There can't be more than one zero.
		qc = qc[1:]
		qe--
		p--
	}
	q := &Big{sign, qe, qc}
	// Round?
	if float64(qi) > p {
		if err := round(q, p, rm, len(r) > 0); err != nil {
			return nil, err
		}
	}
	return q, nil
}

// minus is x minus y.
func (x *Big) minus(that *Big) *Big {
	y := that.copy()
	b := y.s
	// Signs differ?
	if x.s != b {
		y.s = -b
		return x.plus(y)
	}
	xc, yc := append([]int(nil), x.c...), y.c
	xe, ye := x.e, y.e
	// Either zero?
	if xc[0] == 0 || yc[0] == 0 {
		if yc[0] != 0 {
			y.s = -b
		} else if xc[0] != 0 {
			y = x.copy()
		} else {
			y.s = 1
		}
		return y
	}
	// Determine which is the bigger number. Prepend zeros to equalise exponents.
	a := xe - ye
	var xlty bool
	if truthy(a) {
		if xlty = a < 0; xlty {
			a = -a
			xc = append(zeros(int(a)), xc...)
		} else {
			ye = xe
			yc = append(zeros(int(a)), yc...)
		}
	} else {
		// Exponents equal. Check digit by digit.
		xlty = len(xc) < len(yc)
		for i := 0; i < len(xc) && i < len(yc); i++ {
			if xc[i] != yc[i] {
				xlty = xc[i] < yc[i]
				break
			}
		}
		a = 0
	}
	// x < y? Point xc to the array of the bigger number.
	if xlty {
		xc, yc = yc, xc
		y.s = -y.s
	}
	// Append zeros to xc if shorter. No need to add zeros to yc if shorter as subtraction only
	// needs to start at yc.length.
	for len(xc) < len(yc) {
		xc = append(xc, 0)
	}
	// Subtract yc from xc.
	for j := len(yc); float64(j) > a; {
		j--
		if xc[j] < yc[j] {
			i := j - 1
			for i > 0 && xc[i] == 0 {
				xc[i] = 9
				i--
			}
			xc[i]--
			xc[j] += 10
		}
		xc[j] -= yc[j]
	}
	// Remove trailing zeros.
	for len(xc) > 0 && xc[len(xc)-1] == 0 {
		xc = xc[:len(xc)-1]
	}
	// Remove leading zeros and adjust exponent accordingly.
	for len(xc) > 0 && xc[0] == 0 {
		xc = xc[1:]
		ye--
	}
	if len(xc) == 0 {
		// n - n = +0
		y.s = 1
		// Result must be zero.
		xc, ye = []int{0}, 0
	}
	y.c, y.e = xc, ye
	return y
}

// plus is x plus y.
func (x *Big) plus(that *Big) *Big {
	y := that.copy()
	// Signs differ?
	if x.s != y.s {
		y.s = -y.s
		return x.minus(y)
	}
	xe, xc, ye, yc := x.e, x.c, y.e, y.c
	// Either zero?
	if xc[0] == 0 || yc[0] == 0 {
		if yc[0] == 0 {
			if xc[0] != 0 {
				return x.copy()
			}
			y.s = x.s
		}
		return y
	}
	xc = append([]int(nil), xc...)
	// Prepend zeros to equalise exponents.
	if e := xe - ye; truthy(e) {
		if e > 0 {
			ye = xe
			yc = append(zeros(int(e)), yc...)
		} else {
			xc = append(zeros(int(-e)), xc...)
		}
	}
	// Point xc to the longer array.
	if len(xc)-len(yc) < 0 {
		xc, yc = yc, xc
	}
	// Only start adding at yc.length - 1 as the further digits of xc can be left as they are.
	k := 0
	for i := len(yc) - 1; i >= 0; i-- {
		xc[i] += yc[i] + k
		k = xc[i] / 10
		xc[i] %= 10
	}
	// No need to check for zero, as +x + +y != 0 && -x + -y != 0
	if k != 0 {
		xc = append([]int{k}, xc...)
		ye++
	}
	// Remove trailing zeros.
	for xc[len(xc)-1] == 0 {
		xc = xc[:len(xc)-1]
	}
	y.c, y.e = xc, ye
	return y
}

// pow is x raised to the integer power n; a negative power divides with dp and rm.
func (x *Big) pow(n *float64, dp, rm float64) (*Big, error) {
	one := &Big{1, 0, []int{1}}
	y := one
	if n == nil || !isIntIn(*n, -maxPower, maxPower) {
		return nil, errInvalidExponent
	}
	isneg := *n < 0
	k := int(math.Abs(*n))
	for {
		if k&1 != 0 {
			y = y.times(x)
		}
		k >>= 1
		if k == 0 {
			break
		}
		x = x.times(x)
	}
	if isneg {
		return one.div(y, dp, rm)
	}
	return y, nil
}

// round is x rounded to dp decimal places (0 when absent) using rounding mode rm.
func (x *Big) round(dp *float64, rm float64) (*Big, error) {
	places := 0.0
	if dp != nil {
		if !isIntIn(*dp, -maxDP, maxDP) {
			return nil, errInvalidDP
		}
		places = *dp
	}
	y := x.copy()
	return y, round(y, places+x.e+1, rm, false)
}

// sqrt is the square root of x, rounded to dp decimal places using rounding mode rm.
func (x *Big) sqrt(dp, rm float64) (*Big, error) {
	half := &Big{1, -1, []int{5}}
	// Zero?
	if x.c[0] == 0 {
		return x.copy(), nil
	}
	// Negative?
	if x.s < 0 {
		return nil, errNoRoot
	}
	// Estimate.
	s := math.Sqrt(jsNumber(stringify(x, true, true)))
	var text string
	// Math.sqrt underflow/overflow?
	// Re-estimate: pass x coefficient to Math.sqrt as integer, then adjust the result exponent.
	if e := x.e; s == 0 || math.IsInf(s, 1) {
		c := digitString(x.c)
		if toInt32(float64(len(c))+e)&1 == 0 {
			c += "0"
		}
		s = math.Sqrt(jsNumber(c))
		// `| 0` gives an int32, but the subtraction is a double one: it can reach -2^31 - 1.
		exponent := float64(toInt32((e + 1) / 2))
		if e < 0 || toInt32(e)&1 != 0 {
			exponent--
		}
		mantissa := "5e"
		if !math.IsInf(s, 1) {
			mantissa = toExponential(s)
			mantissa = mantissa[:strings.IndexByte(mantissa, 'e')+1]
		}
		text = mantissa + jsString(exponent)
	} else {
		text = jsString(s)
	}
	r, err := parse(text)
	if err != nil {
		return nil, err
	}
	// Big.DP += 4 for the iteration; div validates it.
	places := dp + 4
	compared := r.e + places
	// Newton-Raphson iteration.
	for {
		t := r
		q, err := x.div(t, places, rm)
		if err != nil {
			return nil, err
		}
		r = half.times(t.plus(q))
		if equal(head(t.c, compared), head(r.c, compared)) {
			break
		}
	}
	return r, round(r, dp+r.e+1, rm, false)
}

// times is x times y.
func (x *Big) times(y *Big) *Big {
	xc, yc := x.c, y.c
	a, b := len(xc), len(yc)
	// Determine sign of result.
	sign := -1
	if x.s == y.s {
		sign = 1
	}
	// Return signed 0 if either 0.
	if xc[0] == 0 || yc[0] == 0 {
		return &Big{sign, 0, []int{0}}
	}
	// Initialise exponent of result as x.e + y.e.
	e := x.e + y.e
	// If array xc has fewer digits than yc, swap xc and yc, and lengths.
	if a < b {
		xc, yc, a, b = yc, xc, b, a
	}
	// Initialise coefficient array of result with zeros.
	c := make([]int, a+b)
	// Multiply.
	carry := 0
	for i := b - 1; i >= 0; i-- {
		carry = 0
		j := a + i
		for j > i {
			// Current sum of products at this digit position, plus carry.
			carry += c[j] + yc[i]*xc[j-i-1]
			c[j] = carry % 10
			j--
			carry /= 10
		}
		c[j] = carry
	}
	// Increment result exponent if there is a final carry, otherwise remove leading zero.
	if carry != 0 {
		e++
	} else {
		c = c[1:]
	}
	// Remove trailing zeros.
	for c[len(c)-1] == 0 {
		c = c[:len(c)-1]
	}
	return &Big{sign, e, c}
}

// toExponential is x in exponential notation with dp fixed decimal places, rounded using
// rounding mode rm.
func (x *Big) toExponential(dp *float64, rm float64) (string, error) {
	n := x.c[0]
	if dp != nil {
		if !isIntIn(*dp, 0, maxDP) {
			return "", errInvalidDP
		}
		sd := *dp + 1
		x = x.copy()
		if err := round(x, sd, rm, false); err != nil {
			return "", err
		}
		for float64(len(x.c)) < sd {
			x.c = append(x.c, 0)
		}
	}
	return stringify(x, true, n != 0), nil
}

// toFixed is x in normal notation with dp fixed decimal places, rounded using rounding mode rm.
func (x *Big) toFixed(dp *float64, rm float64) (string, error) {
	n := x.c[0]
	if dp != nil {
		if !isIntIn(*dp, 0, maxDP) {
			return "", errInvalidDP
		}
		y := x.copy()
		if err := round(y, *dp+x.e+1, rm, false); err != nil {
			return "", err
		}
		// y.e may have changed if the value is rounded up.
		for float64(len(y.c)) < *dp+y.e+1 {
			y.c = append(y.c, 0)
		}
		x = y
	}
	return stringify(x, false, n != 0), nil
}

// toPrecision is x rounded to sd significant digits using rounding mode rm, in exponential
// notation if sd is less than the number of digits of the integer part.
func (x *Big) toPrecision(sd *float64, rm float64) (string, error) {
	n := x.c[0]
	if sd != nil {
		if !isIntIn(*sd, 1, maxDP) {
			return "", errInvalidPrecision
		}
		x = x.copy()
		if err := round(x, *sd, rm, false); err != nil {
			return "", err
		}
		for float64(len(x.c)) < *sd {
			x.c = append(x.c, 0)
		}
	}
	return stringify(x, sd != nil && *sd <= x.e || x.e <= ne || x.e >= pe, n != 0), nil
}

// String is x in exponential notation at exponents of NE and below or PE and above, with no
// sign for zero.
func (x *Big) String() string { return stringify(x, x.e <= ne || x.e >= pe, x.c[0] != 0) }
