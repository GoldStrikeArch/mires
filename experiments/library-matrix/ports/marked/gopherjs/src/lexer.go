// Lexer mirroring marked 18.1.1 src/Lexer.ts.
package main

type lexerState struct {
	inLink            bool
	inRawBlock        bool
	linkEmitted       bool
	linkParenPossible bool
	top               bool
}

type inlineQueueEntry struct {
	src  jsStr
	tok  *Token
	cell *TableCell
}

// Lexer drives block and inline tokenization and pre-scan delimiter masking.
type Lexer struct {
	tokens      []*Token
	links       map[string]*LinkDef
	options     *Options
	state       lexerState
	inlineQueue []*inlineQueueEntry
	tokenizer   Tokenizer
}

func newLexer(options *Options) *Lexer {
	l := &Lexer{
		links:   make(map[string]*LinkDef),
		options: options,
	}
	l.state.linkParenPossible = true
	l.state.top = true
	var rules *Rules
	if options.pedantic {
		rules = &rulesPedantic
	} else if options.gfm {
		if options.breaks {
			rules = &rulesBreaks
		} else {
			rules = &rulesGfm
		}
	} else {
		rules = &rulesNormal
	}
	l.tokenizer.options = options
	l.tokenizer.rules = rules
	l.tokenizer.lexer = l
	return l
}

func lex(src jsStr, options *Options) []*Token {
	l := newLexer(options)
	src = src.Call("replace", l.tokenizer.rules.other.carriageReturn, jsNewline)
	l.tokens = l.blockTokens(src, l.tokens, false)
	for i := 0; i < len(l.inlineQueue); i++ {
		next := l.inlineQueue[i]
		if next.tok != nil {
			next.tok.tokens = l.inlineTokens(next.src, next.tok.tokens)
		} else if next.cell != nil {
			next.cell.tokens = l.inlineTokens(next.src, next.cell.tokens)
		}
	}
	l.inlineQueue = nil
	return l.tokens
}

func lexInline(src jsStr, options *Options) []*Token {
	l := newLexer(options)
	return l.inlineTokens(src, nil)
}

func (l *Lexer) inline(src jsStr, tok *Token) {
	l.inlineQueue = append(l.inlineQueue, &inlineQueueEntry{src: src, tok: tok})
}

func (l *Lexer) inlineCell(src jsStr, cell *TableCell) {
	l.inlineQueue = append(l.inlineQueue, &inlineQueueEntry{src: src, cell: cell})
}

func (l *Lexer) blockTokens(src jsStr, tokens []*Token, lastParagraphClipped bool) []*Token {
	if l.options.pedantic {
		src = src.
			Call("replace", l.tokenizer.rules.other.tabCharGlobal, jsFourSpaces).
			Call("replace", l.tokenizer.rules.other.spaceLine, jsEmpty)
	}

	srcLength := src.Length() + 1
	for src.Length() > 0 {
		curLen := src.Length()
		if curLen < srcLength {
			srcLength = curLen
		} else {
			panic("Infinite loop on byte: " + intToString(src.Call("charCodeAt", 0).Int()))
		}

		// newline / space
		if token := l.tokenizer.space(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			if token.raw.Length() == 1 && len(tokens) > 0 {
				lastToken := tokens[len(tokens)-1]
				lastToken.raw = lastToken.raw.Call("concat", jsNewline)
			} else {
				tokens = append(tokens, token)
			}
			continue
		}

		// code (indented)
		if token := l.tokenizer.code(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			var lastToken *Token
			if len(tokens) > 0 {
				lastToken = tokens[len(tokens)-1]
			}
			if lastToken != nil && (lastToken.kind == tokParagraph || lastToken.kind == tokText) {
				if lastToken.raw.Length() > 0 && lastToken.raw.Call("charCodeAt", lastToken.raw.Length()-1).Int() == '\n' {
					lastToken.raw = lastToken.raw.Call("concat", token.raw)
				} else {
					lastToken.raw = lastToken.raw.Call("concat", jsNewline, token.raw)
				}
				lastToken.text = lastToken.text.Call("concat", jsNewline, token.text)
				l.inlineQueue[len(l.inlineQueue)-1].src = lastToken.text
			} else {
				tokens = append(tokens, token)
			}
			continue
		}

		// fences
		if token := l.tokenizer.fences(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// heading
		if token := l.tokenizer.heading(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// hr
		if token := l.tokenizer.hr(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// blockquote
		if token := l.tokenizer.blockquote(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// list
		if token := l.tokenizer.list(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// html
		if token := l.tokenizer.html(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// def
		if token := l.tokenizer.def(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			var lastToken *Token
			if len(tokens) > 0 {
				lastToken = tokens[len(tokens)-1]
			}
			if lastToken != nil && (lastToken.kind == tokParagraph || lastToken.kind == tokText) {
				if lastToken.raw.Length() > 0 && lastToken.raw.Call("charCodeAt", lastToken.raw.Length()-1).Int() == '\n' {
					lastToken.raw = lastToken.raw.Call("concat", token.raw)
				} else {
					lastToken.raw = lastToken.raw.Call("concat", jsNewline, token.raw)
				}
				lastToken.text = lastToken.text.Call("concat", jsNewline, token.raw)
				l.inlineQueue[len(l.inlineQueue)-1].src = lastToken.text
			} else if _, exists := l.links[token.tag]; !exists {
				l.links[token.tag] = &LinkDef{
					href:  token.href,
					title: token.title,
				}
				tokens = append(tokens, token)
			}
			continue
		}

		// table (gfm)
		if token := l.tokenizer.table(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// lheading
		if token := l.tokenizer.lheading(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// top-level paragraph
		if l.state.top {
			if token := l.tokenizer.paragraph(src); token != nil {
				var lastToken *Token
				if len(tokens) > 0 {
					lastToken = tokens[len(tokens)-1]
				}
				if lastParagraphClipped && lastToken != nil && lastToken.kind == tokParagraph {
					if lastToken.raw.Length() > 0 && lastToken.raw.Call("charCodeAt", lastToken.raw.Length()-1).Int() == '\n' {
						lastToken.raw = lastToken.raw.Call("concat", token.raw)
					} else {
						lastToken.raw = lastToken.raw.Call("concat", jsNewline, token.raw)
					}
					lastToken.text = lastToken.text.Call("concat", jsNewline, token.text)
					l.inlineQueue = l.inlineQueue[:len(l.inlineQueue)-1]
					l.inlineQueue[len(l.inlineQueue)-1].src = lastToken.text
				} else {
					tokens = append(tokens, token)
				}
				lastParagraphClipped = false
				src = src.Call("substring", token.raw.Length())
				continue
			}
		}

		// text
		if token := l.tokenizer.text(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			var lastToken *Token
			if len(tokens) > 0 {
				lastToken = tokens[len(tokens)-1]
			}
			if lastToken != nil && lastToken.kind == tokText {
				if lastToken.raw.Length() > 0 && lastToken.raw.Call("charCodeAt", lastToken.raw.Length()-1).Int() == '\n' {
					lastToken.raw = lastToken.raw.Call("concat", token.raw)
				} else {
					lastToken.raw = lastToken.raw.Call("concat", jsNewline, token.raw)
				}
				lastToken.text = lastToken.text.Call("concat", jsNewline, token.text)
				l.inlineQueue = l.inlineQueue[:len(l.inlineQueue)-1]
				l.inlineQueue[len(l.inlineQueue)-1].src = lastToken.text
			} else {
				tokens = append(tokens, token)
			}
			continue
		}

		if src.Length() > 0 {
			panic("Infinite loop on byte: " + intToString(src.Call("charCodeAt", 0).Int()))
		}
	}

	l.state.top = true
	return tokens
}

// linkInText checks whether link text already holds a non-image link.
func (l *Lexer) linkInText(text jsStr) bool {
	if !text.Call("includes", jsLBracket).Bool() {
		return false
	}

	linkRule := l.tokenizer.rules.inline.link
	blockSkip := l.tokenizer.rules.inline.blockSkip
	blockSkip.Set("lastIndex", 0)
	for {
		m := blockSkip.Call("exec", text)
		if m == nil {
			break
		}
		idx := m.Get("index").Int()
		if linkRule.Call("test", m.Index(0)).Bool() && (idx == 0 || text.Call("charCodeAt", idx-1).Int() != '!') {
			return true
		}
	}

	reflinkSearch := l.tokenizer.rules.inline.reflinkSearch
	nextIdx := 0
	for {
		reflinkSearch.Set("lastIndex", nextIdx)
		m := reflinkSearch.Call("exec", text)
		if m == nil {
			break
		}
		nextIdx = reflinkSearch.Get("lastIndex").Int()
		match0 := m.Index(0)
		refStart := match0.Call("lastIndexOf", jsLBracket).Int()
		if match0.Call("charCodeAt", 0).Int() == '!' {
			continue
		}
		label := normalizeLabel(match0.Call("slice", refStart+1, -1)).String()
		if _, ok := l.links[label]; !ok {
			continue
		}
		if refStart > 1 && l.linkInText(match0.Call("slice", 1, refStart-1)) {
			continue
		}
		return true
	}

	return false
}

// maskReflinks runs reflinkSearch globally, saving/restoring lastIndex across recursive calls.
func (l *Lexer) maskReflinks(src jsStr) jsStr {
	re := l.tokenizer.rules.inline.reflinkSearch
	out := jsEmpty
	lastEnd := 0
	nextIdx := 0
	changed := false
	for {
		re.Set("lastIndex", nextIdx)
		m := re.Call("exec", src)
		if m == nil {
			break
		}
		idx := m.Get("index").Int()
		match0 := m.Index(0)
		nextIdx = re.Get("lastIndex").Int()

		refStart := match0.Call("lastIndexOf", jsLBracket).Int()
		label := normalizeLabel(match0.Call("slice", refStart+1, -1)).String()
		if _, ok := l.links[label]; !ok {
			continue
		}

		changed = true
		if idx > lastEnd {
			out = out.Call("concat", src.Call("slice", lastEnd, idx))
		}
		var rep jsStr
		if refStart > 1 && match0.Call("charCodeAt", 0).Int() != '!' {
			innerText := match0.Call("slice", 1, refStart-1)
			if l.linkInText(innerText) {
				rep = jsLBracket.Call("concat", l.maskReflinks(innerText), jsRBracketLBracket, jsA.Call("repeat", match0.Length()-refStart-2), jsRBracket)
			} else {
				rep = jsLBracket.Call("concat", jsA.Call("repeat", match0.Length()-2), jsRBracket)
			}
		} else {
			rep = jsLBracket.Call("concat", jsA.Call("repeat", match0.Length()-2), jsRBracket)
		}
		out = out.Call("concat", rep)
		lastEnd = idx + match0.Length()
	}
	if !changed {
		return src
	}
	if lastEnd < src.Length() {
		out = out.Call("concat", src.Call("slice", lastEnd))
	}
	return out
}

func (l *Lexer) maskPunctuationOutsideCode(src jsStr) jsStr {
	re := l.tokenizer.rules.inline.anyPunctuationOutsideCode
	re.Set("lastIndex", 0)
	out := jsEmpty
	lastEnd := 0
	changed := false
	for {
		m := re.Call("exec", src)
		if m == nil {
			break
		}
		match0 := m.Index(0)
		code := m.Index(1)
		punct := m.Index(2)
		if !isTruthy(punct) && !isTruthy(code) {
			continue
		}
		changed = true
		idx := m.Get("index").Int()
		if idx > lastEnd {
			out = out.Call("concat", src.Call("slice", lastEnd, idx))
		}
		if isTruthy(punct) {
			out = out.Call("concat", jsPlus.Call("repeat", match0.Length()))
		} else {
			cl := code.Length()
			inner := match0.Call("slice", cl, -cl).Call("replace", l.tokenizer.rules.other.backtickGlobal, jsPlus)
			out = out.Call("concat", code, inner, code)
		}
		lastEnd = idx + match0.Length()
	}
	if !changed {
		return src
	}
	if lastEnd < src.Length() {
		out = out.Call("concat", src.Call("slice", lastEnd))
	}
	return out
}

func (l *Lexer) maskBlockSkip(src jsStr) jsStr {
	re := l.tokenizer.rules.inline.blockSkip
	re.Set("lastIndex", 0)
	out := jsEmpty
	lastEnd := 0
	changed := false
	for {
		m := re.Call("exec", src)
		if m == nil {
			break
		}
		changed = true
		idx := m.Get("index").Int()
		if idx > lastEnd {
			out = out.Call("concat", src.Call("slice", lastEnd, idx))
		}
		match0 := m.Index(0)
		offset := 0
		if ctx := m.Index(2); isTruthy(ctx) {
			offset = ctx.Length()
		}
		if offset > 0 {
			out = out.Call("concat", match0.Call("slice", 0, offset), jsLBracket, jsA.Call("repeat", match0.Length()-offset-2), jsRBracket)
		} else {
			out = out.Call("concat", jsLBracket, jsA.Call("repeat", match0.Length()-2), jsRBracket)
		}
		lastEnd = idx + match0.Length()
	}
	if !changed {
		return src
	}
	if lastEnd < src.Length() {
		out = out.Call("concat", src.Call("slice", lastEnd))
	}
	return out
}

func (l *Lexer) inlineTokens(src jsStr, tokens []*Token) []*Token {
	prevLinkParenPossible := l.state.linkParenPossible
	l.state.linkParenPossible = prevLinkParenPossible && src.Call("includes", jsRParen).Bool()
	res := l.inlineTokensInner(src, tokens)
	l.state.linkParenPossible = prevLinkParenPossible
	return res
}

func (l *Lexer) inlineTokensInner(src jsStr, tokens []*Token) []*Token {
	maskedSrc := src
	if len(l.links) > 0 && src.Call("includes", jsLBracket).Bool() {
		maskedSrc = l.maskReflinks(maskedSrc)
	}
	maskedSrc = l.maskPunctuationOutsideCode(maskedSrc)
	maskedSrc = l.maskBlockSkip(maskedSrc)

	keepPrevChar := false
	prevChar := jsEmpty
	srcLength := src.Length() + 1

	for src.Length() > 0 {
		curLen := src.Length()
		if curLen < srcLength {
			srcLength = curLen
		} else {
			panic("Infinite loop on byte: " + intToString(src.Call("charCodeAt", 0).Int()))
		}

		if !keepPrevChar {
			prevChar = jsEmpty
		}
		keepPrevChar = false

		// escape
		if token := l.tokenizer.escape(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// tag
		if token := l.tokenizer.tag(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// link
		if token := l.tokenizer.link(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// reflink, nolink
		if token := l.tokenizer.reflink(src, l.links); token != nil {
			src = src.Call("substring", token.raw.Length())
			if token.kind == tokText && len(tokens) > 0 && tokens[len(tokens)-1].kind == tokText {
				lastToken := tokens[len(tokens)-1]
				lastToken.raw = lastToken.raw.Call("concat", token.raw)
				lastToken.text = lastToken.text.Call("concat", token.text)
			} else {
				tokens = append(tokens, token)
			}
			continue
		}

		// em & strong
		if token := l.tokenizer.emStrong(src, maskedSrc, prevChar); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// code
		if token := l.tokenizer.codespan(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// br
		if token := l.tokenizer.br(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// del (gfm)
		if token := l.tokenizer.del(src, maskedSrc, prevChar); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// autolink
		if token := l.tokenizer.autolink(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			tokens = append(tokens, token)
			continue
		}

		// url (gfm)
		if !l.state.inLink {
			if token := l.tokenizer.url(src); token != nil {
				src = src.Call("substring", token.raw.Length())
				tokens = append(tokens, token)
				continue
			}
		}

		// text
		if token := l.tokenizer.inlineText(src); token != nil {
			src = src.Call("substring", token.raw.Length())
			lastChar := token.raw.Call("slice", -1)
			if lastChar != jsUnderscore {
				prevChar = lastChar
			}
			keepPrevChar = true
			if len(tokens) > 0 && tokens[len(tokens)-1].kind == tokText {
				lastToken := tokens[len(tokens)-1]
				lastToken.raw = lastToken.raw.Call("concat", token.raw)
				lastToken.text = lastToken.text.Call("concat", token.text)
			} else {
				tokens = append(tokens, token)
			}
			continue
		}

		if src.Length() > 0 {
			panic("Infinite loop on byte: " + intToString(src.Call("charCodeAt", 0).Int()))
		}
	}

	return tokens
}
