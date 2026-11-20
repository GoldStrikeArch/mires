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
