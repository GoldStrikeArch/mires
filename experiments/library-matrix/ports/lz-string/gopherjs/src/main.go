package main

import (
	"github.com/gopherjs/gopherjs/js"
	"strings"
)

const base64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/="
const uri = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$"

func width(format string) int {
	switch format {
	case "utf16":
		return 15
	case "base64", "uri":
		return 6
	case "raw", "bytes":
		return 16
	}
	panic("unknown format")
}

type codeWidth struct{ bits, left int }

func (w *codeWidth) advance() {
	w.left--
	if w.left == 0 {
		w.left = 1 << w.bits
		w.bits++
	}
}

// Go strings here contain two bytes per code unit, including surrogate halves.
func symbol(unit int) string { return string([]byte{byte(unit >> 8), byte(unit)}) }
func units(text string) []int {
	out := make([]int, len(text)/2)
	for i := range out {
		out[i] = int(text[2*i])<<8 | int(text[2*i+1])
	}
	return out
}
func encodedAscii(text string) string {
	var out strings.Builder
	for i := 0; i < len(text); i++ {
		out.WriteString(symbol(int(text[i])))
	}
	return out.String()
}

func compress(text, format string) []int {
	dictionary := make(map[string]int)
	pending := make(map[string]bool)
	words := make([]int, 0)
	size := width(format)
	codes := codeWidth{2, 2}
	word, used, next := 0, 0, 3
	write := func(n, value int) {
		for bit := 0; bit < n; bit++ {
			word = word<<1 | (value >> bit & 1)
			used++
			if used == size {
				words = append(words, word)
				used = 0
				word = 0
			}
		}
	}
	emit := func(w string) {
		if pending[w] {
			delete(pending, w)
			c := int(w[0])<<8 | int(w[1])
			if c < 256 {
				write(codes.bits, 0)
				write(8, c)
			} else {
				write(codes.bits, 1)
				write(16, c)
			}
			codes.advance()
		} else {
			write(codes.bits, dictionary[w])
		}
		codes.advance()
	}
	prefix, special := "", encodedAscii("__proto__")
	for i := 0; i < len(text); i += 2 {
		c := text[i : i+2]
		if _, ok := dictionary[c]; !ok {
			dictionary[c] = next
			next++
			pending[c] = true
		}
		phrase := prefix + c
		if _, ok := dictionary[phrase]; ok {
			prefix = phrase
			continue
		}
		emit(prefix)
		if phrase != special {
			dictionary[phrase] = next
		}
		next++
		prefix = c
	}
	if prefix != "" {
		emit(prefix)
	}
	write(codes.bits, 2)
	write(size-used, 0)
	if format == "raw" {
		return words
	}
	out := make([]int, 0, len(words))
	for _, w := range words {
		switch format {
		case "bytes":
			out = append(out, w>>8, w&255)
		case "utf16":
			out = append(out, w+32)
		case "base64":
			out = append(out, int(base64[w]))
		case "uri":
			out = append(out, int(uri[w]))
		}
	}
	if format == "utf16" {
		out = append(out, 32)
	}
	if format == "base64" {
		for len(out)%4 != 0 {
			out = append(out, 61)
		}
	}
	return out
}

func decompress(data []int, format string) (string, bool) {
	if len(data) == 0 {
		return "", false
	}
	size := width(format)
	var words []int
	if format == "bytes" {
		if len(data)%2 != 0 {
			panic("odd byte count")
		}
		for i := 0; i < len(data); i += 2 {
			words = append(words, data[i]*256+data[i+1])
		}
	} else {
		for _, c := range data {
			w := c
			switch format {
			case "utf16":
				w = c - 32
			case "uri", "base64":
				alphabet := base64
				if format == "uri" {
					alphabet = uri
					if c == 32 {
						c = 43
					}
				}
				w = strings.IndexRune(alphabet, rune(c))
				if w < 0 {
					w = 0
				}
			}
			words = append(words, w)
		}
	}
	index, position := 0, size
	read := func(n int) int {
		value := 0
		for bit := 0; bit < n; bit++ {
			word := 0
			if index < len(words) {
				word = words[index]
			}
			position--
			value |= ((word >> position) & 1) << bit
			if position == 0 {
				position = size
				index++
			}
		}
		return value
	}
	head := read(2)
	if head == 2 {
		return "", true
	}
	previous := ""
	if head == 0 {
		previous = symbol(read(8))
	} else if head == 1 {
		previous = symbol(read(16))
	}
	dictionary := []string{"", "", "", previous}
	var output strings.Builder
	output.WriteString(previous)
	codes := codeWidth{3, 4}
	for index < len(words) {
		code := read(codes.bits)
		if code == 2 {
			return output.String(), true
		}
		if code <= 1 {
			bits := 8
			if code == 1 {
				bits = 16
			}
			code = len(dictionary)
			dictionary = append(dictionary, symbol(read(bits)))
			codes.advance()
		}
		var entry string
		if code < len(dictionary) {
			entry = dictionary[code]
		} else if code == len(dictionary) {
			if previous == "" {
				panic("undefined initial phrase")
			}
			entry = previous + previous[:2]
		}
		if entry == "" {
			return "", false
		}
		output.WriteString(entry)
		if previous == "" {
			previous = encodedAscii("undefined")
		}
		dictionary = append(dictionary, previous+entry[:2])
		previous = entry
		codes.advance()
	}
	return "", true
}

func array(values []int) *js.Object {
	a := js.Global.Get("Array").New(len(values))
	for i, n := range values {
		a.SetIndex(i, n)
	}
	return a
}
func run(input *js.Object) *js.Object {
	data := input.Get("data")
	if data == nil || data == js.Undefined {
		return array(nil)
	}
	format, op := input.Get("format").String(), input.Get("op").String()
	values := make([]int, data.Length())
	for i := range values {
		values[i] = data.Index(i).Int()
	}
	if op == "decompress" {
		text, ok := decompress(values, format)
		if !ok {
			return nil
		}
		return array(units(text))
	}
	var b strings.Builder
	for _, n := range values {
		b.WriteString(symbol(n))
	}
	text := b.String()
	if times := input.Get("repeat"); times != js.Undefined {
		text = strings.Repeat(text, times.Int())
	}
	packed := compress(text, format)
	if op == "compress" {
		return array(packed)
	}
	restored, ok := decompress(packed, format)
	if op != "roundtrip" || !ok || restored != text {
		panic("roundtrip mismatch")
	}
	return array([]int{len(packed), len(text) / 2})
}
func main() { js.Module.Get("exports").Set("run", run) }
