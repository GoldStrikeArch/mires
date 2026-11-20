package main

import (
	"github.com/gopherjs/gopherjs/js"
	"math/bits"
	"strconv"
	"strings"
)

func base(r int) int {
	if r == 0 {
		return 10
	}
	if r < 2 || r > 36 {
		return -1
	}
	return r
}

func parse(s string, r int) (uint64, bool) {
	neg := false
	for {
		if s == "" {
			return 0, false
		}
		switch s {
		case "NaN", "Infinity", "+Infinity", "-Infinity":
			return 0, true
		}
		r = base(r)
		if r < 0 {
			return 0, false
		}
		if s[0] != '-' {
			break
		}
		neg = !neg
		s = s[1:]
	}
	if strings.Contains(s, "-") {
		return 0, false
	}
	var value uint64
	for len(s) > 0 {
		size := len(s)
		if size > 8 {
			size = 8
		}
		group := strings.TrimLeft(s[:size], " \t\r\n\v\f")
		group = strings.TrimPrefix(group, "+")
		if r == 16 && len(group) >= 2 && (group[:2] == "0x" || group[:2] == "0X") {
			group = group[2:]
		}
		var part uint64
		for i := 0; i < len(group); i++ {
			d := strings.IndexByte("0123456789abcdefghijklmnopqrstuvwxyz", asciiLower(group[i]))
			if d < 0 || d >= r {
				break
			}
			part = part*uint64(r) + uint64(d)
		}
		for i := 0; i < size; i++ {
			value *= uint64(r)
		}
		value += part
		s = s[size:]
	}
	if neg {
		value = -value
	}
	return value, true
}
func asciiLower(c byte) byte {
	if c >= 'A' && c <= 'Z' {
		return c + 32
	}
	return c
}

func format(a uint64, u bool, r int) string {
	r = base(r)
	if r < 0 {
		return "!"
	}
	if u {
		return strconv.FormatUint(a, r)
	}
	return strconv.FormatInt(int64(a), r)
}
func divide(a, b uint64, u bool) uint64 {
	if u {
		return a / b
	}
	if a>>32 == 0x80000000 && b == ^uint64(0) {
		return a
	}
	return uint64(int64(a) / int64(b))
}
func compare(a, b uint64, u, v bool) int {
	if a == b && (u == v || int64(a) >= 0) {
		return 0
	}
	an, bn := !u && int64(a) < 0, !v && int64(b) < 0
	if an != bn {
		if an {
			return -1
		}
		return 1
	}
	if (u && a < b) || (!u && int64(a-b) < 0) {
		return -1
	}
	return 1
}
func words(c *js.Object, key string) uint64 {
	w := c.Get(key)
	return uint64(uint32(w.Index(1).Int()))<<32 | uint64(uint32(w.Index(0).Int()))
}
func command(c *js.Object) string {
	a, b := words(c, "a"), words(c, "b")
	n, r := c.Get("n").Int(), c.Get("r").Int()
	u, v := c.Get("u").Bool(), c.Get("v").Bool()
	switch c.Get("op").String() {
	case "fromBits", "toString":
	case "fromString":
		var ok bool
		a, ok = parse(c.Get("s").String(), n)
		if !ok {
			return "!"
		}
	case "add":
		a += b
	case "subtract":
		a -= b
	case "multiply":
		a *= b
	case "divide":
		if b == 0 {
			return "!"
		}
		a = divide(a, b, u)
	case "modulo":
		if b == 0 {
			return "!"
		}
		if u {
			a %= b
		} else {
			a = uint64(int64(a) % int64(b))
		}
	case "and":
		a &= b
	case "or":
		a |= b
	case "xor":
		a ^= b
	case "shiftLeft":
		a <<= uint(n & 63)
	case "shiftRight":
		a = uint64(int64(a) >> uint(n&63))
	case "shiftRightUnsigned":
		a >>= uint(n & 63)
	case "rotl":
		a = bits.RotateLeft64(a, n)
	case "clz":
		return strconv.Itoa(bits.LeadingZeros64(a))
	case "ctz":
		return strconv.Itoa(bits.TrailingZeros64(a))
	case "compare":
		return strconv.Itoa(compare(a, b, u, v))
	case "mix":
		if base(r) < 0 {
			return "!"
		}
		var checksum int32
		for i := 0; i < n; i++ {
			a = bits.RotateLeft64(a*(b|1)+uint64(i+1), 11) ^ b
			a += divide(a, uint64(3+(i&255)), u)
			if i&7 == 0 {
				s := format(a, u, r)
				if s == "!" {
					return "!"
				}
				checksum = checksum*33 + int32(len(s)) + int32(s[0])
			}
		}
		return format(a, u, r) + ":" + strconv.Itoa(int(checksum))
	default:
		return "!"
	}
	return format(a, u, r)
}
func main() {
	js.Module.Get("exports").Set("run", func(input *js.Object) string {
		cmds := input.Get("cases")
		out := make([]string, cmds.Length())
		for i := range out {
			out[i] = command(cmds.Index(i))
		}
		return strings.Join(out, "\n")
	})
}
