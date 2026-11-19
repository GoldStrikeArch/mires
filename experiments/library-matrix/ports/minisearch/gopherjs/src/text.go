// JavaScript strings as UTF-16 code units, and the two text functions MiniSearch's defaults are:
// tokenize (text.split(/[\n\r\p{Z}\p{P}]+/u)) and processTerm (term.toLowerCase()).
//
// Every string the index handles is a []uint16 of code units, because MiniSearch's radix tree
// splits edges, measures lengths and computes edit distances on code units, and a JavaScript
// string may hold lone surrogates (CONTRACT.md, "Strings are UTF-16"). A Go string would be UTF-8,
// and GopherJS's conversion of a JavaScript string to a Go string mishandles lone surrogates, so
// text never becomes a Go string: the boundary reads code units with charCodeAt and writes strings
// back with String.fromCharCode.

package main

import (
	"math"
	"unicode"
	"unicode/utf16"

	"github.com/gopherjs/gopherjs/js"
)

// units reads the code units of a JavaScript string.
func units(s *js.Object) []uint16 {
	text := make([]uint16, s.Length())
	for i := range text {
		text[i] = uint16(s.Call("charCodeAt", i).Int())
	}
	return text
}

// jsString is the JavaScript string of these code units, lone surrogates included, built on the
// host by String.fromCharCode (in chunks, well below the engines' limits on argument counts).
func jsString(text []uint16) *js.Object {
	const chunk = 8192
	fromCharCode := js.Global.Get("String").Get("fromCharCode")
	if len(text) <= chunk {
		return fromCharCode.Call("apply", nil, text)
	}
	s := fromCharCode.Call("apply", nil, text[:chunk])
	for start := chunk; start < len(text); start += chunk {
		s = s.Call("concat", fromCharCode.Call("apply", nil, text[start:min(start+chunk, len(text))]))
	}
	return s
}

// encode is a Go string literal (a message template, an option default) as UTF-16.
func encode(s string) []uint16 {
	return utf16.Encode([]rune(s))
}

// decode is UTF-16 text as a Go string, for Error() only: lone surrogates become U+FFFD there.
func decode(text []uint16) string {
	return string(utf16.Decode(text))
}

// equal is a === b on strings.
func equal(a, b []uint16) bool {
	if len(a) != len(b) {
		return false
	}
	for i, unit := range a {
		if unit != b[i] {
			return false
		}
	}
	return true
}

// equalASCII is text === s for an ASCII literal s.
func equalASCII(text []uint16, s string) bool {
	if len(text) != len(s) {
		return false
	}
	for i, unit := range text {
		if unit != uint16(s[i]) {
			return false
		}
	}
	return true
}

// hasPrefix is s.startsWith(prefix).
func hasPrefix(s, prefix []uint16) bool {
	return len(s) >= len(prefix) && equal(s[:len(prefix)], prefix)
}

// concat is a + b, always a new slice: labels and terms share backing arrays (they are slices of
// the document text), so appending to one in place could overwrite another.
func concat(a, b []uint16) []uint16 {
	joined := make([]uint16, len(a)+len(b))
	copy(joined, a)
	copy(joined[len(a):], b)
	return joined
}

// clip limits a slice's capacity to its length, so that an append always copies.
func clip(s []uint16) []uint16 {
	return s[:len(s):len(s)]
}

// join is parts.join(separator).
func join(parts [][]uint16, separator uint16) []uint16 {
	var joined []uint16
	for i, part := range parts {
		if i > 0 {
			joined = append(joined, separator)
		}
		joined = append(joined, part...)
	}
	return joined
}

// key holds the code units of text in a Go string, two bytes each, losslessly: a map key for
// UTF-16 text (a slice cannot be one, and a UTF-8 conversion would merge lone surrogates).
func key(text []uint16) string {
	bytes := make([]byte, 2*len(text))
	for i, unit := range text {
		bytes[2*i] = byte(unit >> 8)
		bytes[2*i+1] = byte(unit)
	}
	return string(bytes)
}

// codePointAt is the code point starting at index i of text, and its width in code units. A
// surrogate that is not half of a well-formed pair is a code point of its own (category Cs), as it
// is to a RegExp with the u flag.
func codePointAt(text []uint16, i int) (rune, int) {
	unit := rune(text[i])
	if utf16.IsSurrogate(unit) && i+1 < len(text) {
		if r := utf16.DecodeRune(unit, rune(text[i+1])); r != unicode.ReplacementChar {
			return r, 2
		}
	}
	return unit, 1
}

// isSeparator reports whether the code point is in /[\n\r\p{Z}\p{P}]/u, by Go's Unicode tables
// (Unicode 15.0 in Go 1.21; NOTES.md compares them with the contract's 17.0 table). Z in Latin-1
// is the space and the no-break space, and unicode.IsPunct reads a flag table there, which spares
// the common case unicode.Is's range walk (slow under GopherJS, NOTES.md).
func isSeparator(r rune) bool {
	if r <= unicode.MaxLatin1 {
		return r == '\n' || r == '\r' || r == ' ' || r == '\u00a0' || unicode.IsPunct(r)
	}
	return unicode.IsPunct(r) || unicode.Is(unicode.Z, r)
}

// tokenize is MiniSearch's default tokenizer, text.split(/[\n\r\p{Z}\p{P}]+/u): the text between
// maximal runs of separators. Like split, an empty text is one empty token, and a separator at
// either end leaves an empty token there. The tokens are slices of text.
func tokenize(text []uint16) [][]uint16 {
	var tokens [][]uint16
	start, i := 0, 0
	for i < len(text) {
		r, width := codePointAt(text, i)
		if !isSeparator(r) {
			i += width
			continue
		}
		tokens = append(tokens, text[start:i:i])
		for i += width; i < len(text); i += width {
			if r, width = codePointAt(text, i); !isSeparator(r) {
				break
			}
		}
		start = i
	}
	return append(tokens, text[start:len(text):len(text)])
}

// distinctCount is new Set(tokens).size: the number of distinct tokens. A field's few tokens are
// compared with each other (most differ in length); a long field goes through a set, whose keys
// cost an allocation each.
func distinctCount(tokens [][]uint16) int {
	if len(tokens) > 128 {
		seen := make(map[string]struct{}, len(tokens))
		for _, token := range tokens {
			seen[key(token)] = struct{}{}
		}
		return len(seen)
	}
	count := 0
next:
	for i, token := range tokens {
		for _, earlier := range tokens[:i] {
			if equal(token, earlier) {
				continue next
			}
		}
		count++
	}
	return count
}

// toLowerCase is processTerm, String.prototype.toLowerCase: Unicode default lower-casing with full
// mappings (İ becomes i + U+0307) and the Final_Sigma context. Go's unicode.ToLower maps one rune
// to one rune and knows no context, so a term holding any non-ASCII unit is lower-cased by the
// host (the contract allows it). An ASCII term is lower-cased here, A-Z to a-z, which is all the
// host would do to it. The result may be term itself.
func toLowerCase(term []uint16) []uint16 {
	upper := false
	for _, unit := range term {
		if unit >= 0x80 {
			return units(jsString(term).Call("toLowerCase"))
		}
		if 'A' <= unit && unit <= 'Z' {
			upper = true
		}
	}
	if !upper {
		return term
	}
	lower := make([]uint16, len(term))
	for i, unit := range term {
		if 'A' <= unit && unit <= 'Z' {
			unit += 'a' - 'A'
		}
		lower[i] = unit
	}
	return lower
}

// arrayIndex reports whether a property key is an array index, the canonical decimal spelling of
// an integer from 0 to 2^32 - 2, and its value. Object.keys lists such keys first, in ascending
// numeric order, then every other key in insertion order. (A double holds ten digits exactly;
// uint32 would overflow and uint64 is emulated under GopherJS.)
func arrayIndex(key []uint16) (float64, bool) {
	if len(key) == 0 || len(key) > 10 || (key[0] == '0' && len(key) > 1) {
		return 0, false
	}
	value := 0.0
	for _, unit := range key {
		if unit < '0' || unit > '9' {
			return 0, false
		}
		value = value*10 + float64(unit-'0')
	}
	return value, value <= 4294967294
}

// numberText is Number.prototype.toString for the numbers a document or a message holds: safe
// integers (CONTRACT.md, "Parity domain"), in decimal, -0 as "0". Not strconv: its float tables
// add some 50 KB to the GopherJS output, and its integer path goes through emulated int64.
func numberText(value float64) []uint16 {
	if math.Floor(value) != value || math.Abs(value) > 1<<53-1 {
		panic("minisearch (GopherJS port): numbers in documents and ids must be safe integers")
	}
	var digits [17]uint16 // a sign and the 16 digits of 2^53 - 1
	i := len(digits)
	magnitude := math.Abs(value)
	for {
		digit := math.Mod(magnitude, 10)
		i--
		digits[i] = '0' + uint16(digit)
		if magnitude = (magnitude - digit) / 10; magnitude == 0 {
			break
		}
	}
	if value < 0 {
		i--
		digits[i] = '-'
	}
	return append([]uint16(nil), digits[i:]...)
}
