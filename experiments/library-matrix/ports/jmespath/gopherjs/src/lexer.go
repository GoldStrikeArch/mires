// jmespath's Lexer, method by method, over the expression's UTF-16 code units.

package main

import "github.com/gopherjs/gopherjs/js"

type tokenType int

const (
	tokEOF tokenType = iota
	tokUnquotedIdentifier
	tokQuotedIdentifier
	tokRbracket
	tokRparen
	tokComma
	tokColon
	tokRbrace
	tokNumber
	tokCurrent
	tokExpref
	tokPipe
	tokOr
	tokAnd
	tokEQ
	tokGT
	tokLT
	tokGTE
	tokLTE
	tokNE
	tokFlatten
	tokStar
	tokFilter
	tokDot
	tokNot
	tokLbrace
	tokLbracket
	tokLparen
	tokLiteral
)

// bindingPower is jmespath's table. It has no entry for Colon and Literal: `rbp < bindingPower[Colon]`
// is false, and so is `bindingPower[Literal] < 10` in _parseProjectionRHS, which therefore rejects a
// literal after a projection.
var bindingPower = map[tokenType]int{
	tokEOF: 0, tokUnquotedIdentifier: 0, tokQuotedIdentifier: 0, tokRbracket: 0, tokRparen: 0,
	tokComma: 0, tokRbrace: 0, tokNumber: 0, tokCurrent: 0, tokExpref: 0,
	tokPipe: 1, tokOr: 2, tokAnd: 3,
	tokEQ: 5, tokGT: 5, tokLT: 5, tokGTE: 5, tokLTE: 5, tokNE: 5,
	tokFlatten: 9, tokStar: 20, tokFilter: 21, tokDot: 40, tokNot: 45, tokLbrace: 50, tokLbracket: 55,
	tokLparen: 60,
}

// comparatorNames are jmespath's TOK_* names for the comparators, which name Comparator nodes.
var comparatorNames = map[tokenType]string{
	tokEQ: "EQ", tokNE: "NE", tokGT: "GT", tokGTE: "GTE", tokLT: "LT", tokLTE: "LTE",
}

var basicTokens = map[uint16]tokenType{
	'.': tokDot, '*': tokStar, ',': tokComma, ':': tokColon, '{': tokLbrace, '}': tokRbrace,
	']': tokRbracket, '(': tokLparen, ')': tokRparen, '@': tokCurrent,
}

type token struct {
	typ   tokenType
	value *js.Object
	start int
}

func isAlpha(c uint16) bool    { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_' }
func isNum(c uint16) bool      { return (c >= '0' && c <= '9') || c == '-' }
func isAlphaNum(c uint16) bool { return isAlpha(c) || (c >= '0' && c <= '9') }

// isJSWhiteSpace is String.prototype.trimLeft's set: ECMAScript WhiteSpace and LineTerminator.
func isJSWhiteSpace(c uint16) bool {
	switch c {
	case '\t', '\n', '\v', '\f', '\r', ' ', 0xa0, 0x1680, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff:
		return true
	}
	return c >= 0x2000 && c <= 0x200a
}

type lexer struct {
	stream  *js.Object // the expression, a JS string
	units   []uint16
	current int
}

// at is stream[i] as a code unit, or 0 past the end (where jmespath reads undefined).
func (l *lexer) at(i int) uint16 {
	if i < len(l.units) {
		return l.units[i]
	}
	return 0
}

// slice is stream.slice(start, end) on the host, which clamps end to the length.
func (l *lexer) slice(start, end int) *js.Object { return l.stream.Call("slice", start, end) }

func tokenize(stream *js.Object) ([]token, error) {
	l := &lexer{stream: stream, units: make([]uint16, stream.Length())}
	for i := range l.units {
		l.units[i] = uint16(stream.Call("charCodeAt", i).Int())
	}
	var tokens []token
	for l.current < len(l.units) {
		start := l.current
		c := l.units[start]
		basic, isBasic := basicTokens[c]
		switch {
		case isAlpha(c):
			tokens = append(tokens, token{tokUnquotedIdentifier, l.consumeUnquotedIdentifier(), start})
		case isBasic:
			tokens = append(tokens, token{basic, l.slice(start, start+1), start})
			l.current++
		case isNum(c):
			tokens = append(tokens, l.consumeNumber())
		case c == '[':
			tokens = append(tokens, l.consumeLBracket())
		case c == '"':
			value, err := l.consumeQuotedIdentifier()
			if err != nil {
				return nil, err
			}
			tokens = append(tokens, token{tokQuotedIdentifier, value, start})
		case c == '\'':
			tokens = append(tokens, token{tokLiteral, l.consumeRawStringLiteral(), start})
		case c == '`':
			value, err := l.consumeLiteral()
			if err != nil {
				return nil, err
			}
			tokens = append(tokens, token{tokLiteral, value, start})
		case c == '<' || c == '>' || c == '=' || c == '!':
			t, err := l.consumeOperator()
			if err != nil {
				return nil, err
			}
			tokens = append(tokens, t)
		case c == ' ' || c == '\t' || c == '\n':
			l.current++
		case c == '&' || c == '|':
			l.current++
			doubled := l.at(l.current) == c
			if doubled {
				l.current++
			}
			var typ tokenType
			switch {
			case c == '&' && doubled:
				typ = tokAnd
			case c == '&':
				typ = tokExpref
			case doubled:
				typ = tokOr
			default:
				typ = tokPipe
			}
			tokens = append(tokens, token{typ, l.slice(start, l.current), start})
		default:
			return nil, syntaxError("Unknown character")
		}
	}
	return tokens, nil
}

func (l *lexer) consumeUnquotedIdentifier() *js.Object {
	start := l.current
	l.current++
	for l.current < len(l.units) && isAlphaNum(l.units[l.current]) {
		l.current++
	}
	return l.slice(start, l.current)
}

// skipQuoted skips a quoted body, where a backslash escapes quote or another backslash.
func (l *lexer) skipQuoted(quote uint16) {
	for l.current < len(l.units) && l.units[l.current] != quote {
		next := l.at(l.current + 1)
		if l.units[l.current] == '\\' && (next == '\\' || next == quote) {
			l.current += 2
		} else {
			l.current++
		}
	}
}

func (l *lexer) consumeQuotedIdentifier() (*js.Object, error) {
	start := l.current
	l.current++
	l.skipQuoted('"')
	l.current++
	return parseJSON(l.slice(start, l.current))
}

// consumeRawStringLiteral: an unterminated raw string runs to the end of the expression, as in
// jmespath.
func (l *lexer) consumeRawStringLiteral() *js.Object {
	start := l.current
	l.current++
	l.skipQuoted('\'')
	l.current++
	return l.slice(start+1, l.current-1).Call("replace", `\'`, "'")
}

func (l *lexer) consumeNumber() token {
	start := l.current
	l.current++
	for l.current < len(l.units) && isNum(l.units[l.current]) {
		l.current++
	}
	return token{tokNumber, parseInt(l.slice(start, l.current)), start}
}

func (l *lexer) consumeLBracket() token {
	start := l.current
	l.current++
	switch l.at(l.current) {
	case '?':
		l.current++
		return token{tokFilter, l.slice(start, l.current), start}
	case ']':
		l.current++
		return token{tokFlatten, l.slice(start, l.current), start}
	}
	return token{tokLbracket, l.slice(start, l.current), start}
}

// consumeOperator: a lone `=` is the one character jmespath accepts here without making a token: it
// pushes undefined, and parsing fails on it with a TypeError, which is a syntax error. That error
// is returned here.
func (l *lexer) consumeOperator() (token, error) {
	start := l.current
	startingChar := l.units[start]
	l.current++
	equalsFollows := l.at(l.current) == '='
	if equalsFollows {
		l.current++
	}
	var typ tokenType
	switch {
	case startingChar == '!' && equalsFollows:
		typ = tokNE
	case startingChar == '!':
		typ = tokNot
	case startingChar == '<' && equalsFollows:
		typ = tokLTE
	case startingChar == '<':
		typ = tokLT
	case startingChar == '>' && equalsFollows:
		typ = tokGTE
	case startingChar == '>':
		typ = tokGT
	case equalsFollows:
		typ = tokEQ
	default:
		return token{}, syntaxError("Unknown operator: =")
	}
	return token{typ, l.slice(start, l.current), start}, nil
}

func (l *lexer) consumeLiteral() (*js.Object, error) {
	l.current++
	start := l.current
	l.skipQuoted('`')
	// trimLeft, then the first `\`` becomes a backtick.
	trimmed := start
	for trimmed < l.current && isJSWhiteSpace(l.units[trimmed]) {
		trimmed++
	}
	literalString := l.slice(trimmed, l.current).Call("replace", "\\`", "`")
	l.current++
	if looksLikeJSON(literalString) {
		return parseJSON(literalString)
	}
	return parseJSON(jsString(`"`).Call("concat", literalString, `"`))
}

func looksLikeJSON(literalString *js.Object) bool {
	if literalString.Length() == 0 {
		return false
	}
	first := uint16(literalString.Call("charCodeAt", 0).Int())
	switch {
	case first == '[' || first == '{' || first == '"':
		return true
	case isNum(first):
		_, err := parseJSON(literalString)
		return err == nil
	default:
		text := literalString.String()
		return text == "true" || text == "false" || text == "null"
	}
}
