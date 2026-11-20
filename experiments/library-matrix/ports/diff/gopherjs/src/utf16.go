package main

import (
	"strings"
	"unicode/utf8"

	"github.com/gopherjs/gopherjs/js"
)

// Go strings are byte sequences. WTF-8 keeps isolated UTF-16 surrogates in
// their three-byte form and uses standard UTF-8 for every Unicode scalar.
func readText(v *js.Object) string {
	if v.Call("isWellFormed").Bool() {
		return v.String()
	}
	var b strings.Builder
	for i := 0; i < v.Length(); i++ {
		c := rune(v.Call("codePointAt", i).Int())
		if 0xd800 <= c && c <= 0xdfff {
			b.WriteByte(0xed)
			b.WriteByte(byte(0x80 | (c >> 6 & 63)))
			b.WriteByte(byte(0x80 | (c & 63)))
		} else {
			b.WriteRune(c)
			if c > 0xffff {
				i++
			}
		}
	}
	return b.String()
}

func point(s string) (rune, int) {
	if len(s) >= 3 && s[0] == 0xed && s[1] >= 0xa0 && s[1] < 0xc0 && s[2] >= 0x80 && s[2] < 0xc0 {
		return rune(0xd000 | int(s[1]&63)<<6 | int(s[2]&63)), 3
	}
	return utf8.DecodeRuneInString(s)
}

// units is JavaScript's length: the UTF-16 units of a WTF-8 string.
func units(s string) int {
	n := 0
	for len(s) > 0 {
		c, w := point(s)
		n++
		if c > 0xffff {
			n++
		}
		s = s[w:]
	}
	return n
}

// surrogate is a lone UTF-16 surrogate in WTF-8.
func surrogate(u rune) string {
	return string([]byte{0xed, byte(0x80 | u>>6&63), byte(0x80 | u&63)})
}

// dropFirstUnit is s.substring(1): dropping the first unit of an astral
// character leaves its low surrogate.
func dropFirstUnit(s string) string {
	if s == "" {
		return s
	}
	c, w := point(s)
	if c > 0xffff {
		return surrogate(0xdc00|(c-0x10000)&0x3ff) + s[w:]
	}
	return s[w:]
}

// dropLastUnit is s.substring(0, s.length - 1): dropping the last unit of an
// astral character leaves its high surrogate.
func dropLastUnit(s string) string {
	start := 0
	var last rune
	for i := 0; i < len(s); {
		c, w := point(s[i:])
		start, last = i, c
		i += w
	}
	if last > 0xffff {
		return s[:start] + surrogate(0xd800|(last-0x10000)>>10)
	}
	return s[:start]
}

func writeText(s string) *js.Object {
	if utf8.ValidString(s) {
		return js.Global.Get("String").Invoke(s)
	}
	parts := arr(0)
	for len(s) > 0 {
		c, n := point(s)
		parts.Call("push", js.Global.Get("String").Call("fromCodePoint", int(c)))
		s = s[n:]
	}
	return parts.Call("join", "")
}
