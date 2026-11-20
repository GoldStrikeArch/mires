// Renderer and TextRenderer mirroring marked 18.1.1 src/Renderer.ts and src/TextRenderer.ts.
package main

// Renderer renders block and inline tokens to HTML JS strings.
type Renderer struct {
	options *Options
	parser  *Parser
}

func (r *Renderer) space(_ *Token) jsStr {
	return jsEmpty
}

func (r *Renderer) code(tok *Token) jsStr {
	langString := jsEmpty
	if tok.lang != nil && tok.lang.Length() > 0 {
		if m := otherGrammar.notSpaceStart.Call("exec", tok.lang); m != nil {
			langString = m.Index(0)
		}
	}

	code := jsEmpty
	if tok.text.Length() > 0 {
		code = tok.text.Call("replace", otherGrammar.endingNewline, jsEmpty).Call("concat", jsNewline)
	}
	escapedCode := code
	if !tok.escaped {
		escapedCode = escapeHtmlEntities(code, true)
	}

	if langString.Length() == 0 {
		return jsPreCodeOpen.Call("concat", escapedCode, jsPreCodeClose)
	}
	return jsPreCodeLangOpen.Call("concat", escapeHtmlEntities(langString, false), jsTagClose, escapedCode, jsPreCodeClose)
}

func (r *Renderer) blockquote(tok *Token) jsStr {
	body := r.parser.parse(tok.tokens)
	return jsBqOpen.Call("concat", body, jsBqClose)
}

func (r *Renderer) html(tok *Token) jsStr {
	return tok.text
}

func (r *Renderer) def(_ *Token) jsStr {
	return jsEmpty
}

func (r *Renderer) heading(tok *Token) jsStr {
	return jsHeadingOpen[tok.depth].Call("concat", r.parser.parseInline(tok.tokens), jsHeadingClose[tok.depth])
}

func (r *Renderer) hr(_ *Token) jsStr {
	return jsHrHTML
}

func (r *Renderer) list(tok *Token) jsStr {
	body := jsEmpty
	for i := 0; i < len(tok.items); i++ {
		body = body.Call("concat", r.listitem(tok.items[i]))
	}
	if tok.ordered {
		if tok.start != 1 {
			return jsOlStartOpen.Call("concat", intToString(tok.start), jsOlStartClose, body, jsOlClose)
		}
		return jsOlOpen.Call("concat", body, jsOlClose)
	}
	return jsUlOpen.Call("concat", body, jsUlClose)
}

func (r *Renderer) listitem(item *Token) jsStr {
	return jsLiOpen.Call("concat", r.parser.parse(item.tokens), jsLiClose)
}

func (r *Renderer) checkbox(tok *Token) jsStr {
	if tok.checked {
		return jsCheckboxChecked
	}
	return jsCheckboxUncheck
}

func (r *Renderer) paragraph(tok *Token) jsStr {
	return jsPOpen.Call("concat", r.parser.parseInline(tok.tokens), jsPClose)
}

func (r *Renderer) table(tok *Token) jsStr {
	cell := jsEmpty
	for j := 0; j < len(tok.header); j++ {
		cell = cell.Call("concat", r.tablecell(tok.header[j]))
	}
	header := r.tablerow(cell)

	body := jsEmpty
	for j := 0; j < len(tok.rows); j++ {
		row := tok.rows[j]
		cell = jsEmpty
		for k := 0; k < len(row); k++ {
			cell = cell.Call("concat", r.tablecell(row[k]))
		}
		body = body.Call("concat", r.tablerow(cell))
	}
	if body.Length() > 0 {
		body = jsTbodyOpen.Call("concat", body, jsTbodyClose)
	}
	return jsTableOpen.Call("concat", header, jsTableMid, body, jsTableClose)
}

func (r *Renderer) tablerow(text jsStr) jsStr {
	return jsTrOpen.Call("concat", text, jsTrClose)
}

func (r *Renderer) tablecell(cell *TableCell) jsStr {
	content := r.parser.parseInline(cell.tokens)
	if cell.header {
		return jsThOpen[cell.align].Call("concat", content, jsThClose)
	}
	return jsTdOpen[cell.align].Call("concat", content, jsTdClose)
}

func (r *Renderer) strong(tok *Token) jsStr {
	return jsStrongOpen.Call("concat", r.parser.parseInline(tok.tokens), jsStrongClose)
}

func (r *Renderer) em(tok *Token) jsStr {
	return jsEmOpen.Call("concat", r.parser.parseInline(tok.tokens), jsEmClose)
}

func (r *Renderer) codespan(tok *Token) jsStr {
	return jsCodeOpen.Call("concat", escapeHtmlEntities(tok.text, true), jsCodeClose)
}

func (r *Renderer) br(_ *Token) jsStr {
	return jsBrHTML
}

func (r *Renderer) del(tok *Token) jsStr {
	return jsDelOpen.Call("concat", r.parser.parseInline(tok.tokens), jsDelClose)
}

func (r *Renderer) link(tok *Token) jsStr {
	var parsedText jsStr
	if tok.autolink {
		parsedText = escapeHtmlEntities(tok.text, true)
	} else {
		parsedText = r.parser.parseInline(tok.tokens)
	}
	cleanHref := cleanUrl(tok.href)
	if cleanHref == nil {
		return parsedText
	}
	href := escapeHtmlEntities(cleanHref, tok.autolink)
	if tok.title != nil && tok.title.Length() > 0 {
		return jsAOpen.Call("concat", href, jsTitleAttr, escapeHtmlEntities(tok.title, false), jsQuoteGt, parsedText, jsAClose)
	}
	return jsAOpen.Call("concat", href, jsQuoteGt, parsedText, jsAClose)
}

func (r *Renderer) image(tok *Token) jsStr {
	text := tok.text
	if tok.hasTokens {
		text = r.parser.parseInlineText(tok.tokens)
	}
	cleanHref := cleanUrl(tok.href)
	if cleanHref == nil {
		return escapeHtmlEntities(text, false)
	}
	if tok.title != nil && tok.title.Length() > 0 {
		return jsImgSrc.Call("concat", escapeHtmlEntities(cleanHref, false), jsImgAlt, escapeHtmlEntities(text, false), jsTitleAttr, escapeHtmlEntities(tok.title, false), jsQuoteClose)
	}
	return jsImgSrc.Call("concat", escapeHtmlEntities(cleanHref, false), jsImgAlt, escapeHtmlEntities(text, false), jsQuoteClose)
}

func (r *Renderer) text(tok *Token) jsStr {
	if tok.hasTokens {
		return r.parser.parseInline(tok.tokens)
	}
	if tok.escaped {
		return tok.text
	}
	return escapeHtmlEntities(tok.text, false)
}

// TextRenderer flattens inline tokens to raw text for image alt attributes.
type TextRenderer struct{}

func (*TextRenderer) strong(tok *Token) jsStr   { return tok.text }
func (*TextRenderer) em(tok *Token) jsStr       { return tok.text }
func (*TextRenderer) codespan(tok *Token) jsStr { return tok.text }
func (*TextRenderer) del(tok *Token) jsStr      { return tok.text }
func (*TextRenderer) html(tok *Token) jsStr     { return tok.text }
func (*TextRenderer) text(tok *Token) jsStr     { return tok.text }
func (*TextRenderer) link(tok *Token) jsStr     { return tok.text }
func (*TextRenderer) image(tok *Token) jsStr    { return tok.text }
func (*TextRenderer) br(_ *Token) jsStr         { return jsEmpty }
func (*TextRenderer) checkbox(tok *Token) jsStr { return tok.raw }
