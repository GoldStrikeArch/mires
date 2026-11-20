package main

import "github.com/gopherjs/gopherjs/js"

func alpha(c int) bool {
	return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c == 95
}
func digit(c int) bool {
	return c >= 48 && c <= 57
}
func lex(source string) *[]token {
	result := tokensBuffer()
	length := strLen(source)
	i := 0
	for i < length {
		start := i
		c := codeAt(source, i)
		i += 1
		following := 0
		if i < length {
			following = codeAt(source, i)
		}
		k := ""
		value := js.Undefined
		if alpha(c) {
			for i < length && (alpha(codeAt(source, i)) || digit(codeAt(source, i))) {
				i += 1
			}
			value = str(part(source, start, i))
			k = "Field"
		} else if c == 45 || digit(c) {
			for i < length && (digit(codeAt(source, i)) || codeAt(source, i) == 45) {
				i += 1
			}
			value = parseInteger(part(source, start, i))
			k = "Number"
		} else if c == 34 || c == 39 || c == 96 {
			for i < length && codeAt(source, i) != c {
				if codeAt(source, i) == 92 && i+1 < length && (codeAt(source, i+1) == c || codeAt(source, i+1) == 92) {
					i += 1
				}
				i += 1
			}
			inner := part(source, start+1, i)
			if i < length {
				i += 1
			}
			if c == 34 {
				value = decodeJson(part(source, start, i))
				k = "Quoted"
			} else if c == 39 {
				value = str(replaceFirst(inner, slash()+"'", "'"))
				k = "Literal"
			} else {
				value = literal(inner)
				k = "Literal"
			}
		} else if c == 32 || c == 9 || c == 10 {
			k = "Skip"
		} else {
			pair := doubleToken(c, following)
			if pair != "" {
				i += 1
				k = pair
			} else {
				k = singleToken(c)
			}
		}
		if k != "Skip" {
			addToken(result, k, value)
		}
	}
	addToken(result, "EOF", js.Undefined)
	return result
}
func doubleToken(c int, next int) string {
	if c == 91 && next == 63 {
		return "Filter"
	}
	if c == 91 && next == 93 {
		return "Flatten"
	}
	if c == 38 && next == 38 {
		return "And"
	}
	if c == 124 && next == 124 {
		return "Or"
	}
	if c == 33 && next == 61 {
		return "NE"
	}
	if c == 61 && next == 61 {
		return "EQ"
	}
	if c == 60 && next == 61 {
		return "LTE"
	}
	if c == 62 && next == 61 {
		return "GTE"
	}
	return ""
}
func singleToken(c int) string {
	if c == 60 {
		return "LT"
	}
	if c == 62 {
		return "GT"
	}
	if c == 33 {
		return "Not"
	}
	if c == 38 {
		return "Expref"
	}
	if c == 124 {
		return "Pipe"
	}
	if c == 91 {
		return "Lbracket"
	}
	if c == 93 {
		return "Rbracket"
	}
	if c == 123 {
		return "Lbrace"
	}
	if c == 125 {
		return "Rbrace"
	}
	if c == 40 {
		return "Lparen"
	}
	if c == 41 {
		return "Rparen"
	}
	if c == 42 {
		return "Star"
	}
	if c == 64 {
		return "Current"
	}
	if c == 44 {
		return "Comma"
	}
	if c == 46 {
		return "Dot"
	}
	if c == 58 {
		return "Colon"
	}
	panic("syntax")
}
