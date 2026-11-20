// Parser mirroring marked 18.1.1 src/Parser.ts, using concrete dispatch without interfaces.
package main

// Parser walks block and inline token lists and delegates formatting to Renderer / TextRenderer.
type Parser struct {
	options      *Options
	renderer     Renderer
	textRenderer TextRenderer
}

func newParser(options *Options) *Parser {
	p := &Parser{options: options}
	p.renderer.options = options
	p.renderer.parser = p
	return p
}

func parse(tokens []*Token, options *Options) jsStr {
	return newParser(options).parse(tokens)
}

func parseInline(tokens []*Token, options *Options) jsStr {
	return newParser(options).parseInline(tokens)
}

func (p *Parser) parse(tokens []*Token) jsStr {
	out := jsEmpty
	for i := 0; i < len(tokens); i++ {
		tok := tokens[i]
		switch tok.kind {
		case tokSpace:
			out = out.Call("concat", p.renderer.space(tok))
		case tokHr:
			out = out.Call("concat", p.renderer.hr(tok))
		case tokHeading:
			out = out.Call("concat", p.renderer.heading(tok))
		case tokCode:
			out = out.Call("concat", p.renderer.code(tok))
		case tokTable:
			out = out.Call("concat", p.renderer.table(tok))
		case tokBlockquote:
			out = out.Call("concat", p.renderer.blockquote(tok))
		case tokList:
			out = out.Call("concat", p.renderer.list(tok))
		case tokCheckbox:
			out = out.Call("concat", p.renderer.checkbox(tok))
		case tokHTML:
			out = out.Call("concat", p.renderer.html(tok))
		case tokDef:
			out = out.Call("concat", p.renderer.def(tok))
		case tokParagraph:
			out = out.Call("concat", p.renderer.paragraph(tok))
		case tokText:
			out = out.Call("concat", p.renderer.text(tok))
		default:
			panic("Token type was not found.")
		}
	}
	return out
}

func (p *Parser) parseInline(tokens []*Token) jsStr {
	out := jsEmpty
	for i := 0; i < len(tokens); i++ {
		tok := tokens[i]
		switch tok.kind {
		case tokEscape:
			out = out.Call("concat", p.renderer.text(tok))
		case tokHTML:
			out = out.Call("concat", p.renderer.html(tok))
		case tokLink:
			out = out.Call("concat", p.renderer.link(tok))
		case tokImage:
			out = out.Call("concat", p.renderer.image(tok))
		case tokCheckbox:
			out = out.Call("concat", p.renderer.checkbox(tok))
		case tokStrong:
			out = out.Call("concat", p.renderer.strong(tok))
		case tokEm:
			out = out.Call("concat", p.renderer.em(tok))
		case tokCodespan:
			out = out.Call("concat", p.renderer.codespan(tok))
		case tokBr:
			out = out.Call("concat", p.renderer.br(tok))
		case tokDel:
			out = out.Call("concat", p.renderer.del(tok))
		case tokText:
			out = out.Call("concat", p.renderer.text(tok))
		default:
			panic("Token type was not found.")
		}
	}
	return out
}

func (p *Parser) parseInlineText(tokens []*Token) jsStr {
	out := jsEmpty
	for i := 0; i < len(tokens); i++ {
		tok := tokens[i]
		switch tok.kind {
		case tokEscape, tokText:
			out = out.Call("concat", p.textRenderer.text(tok))
		case tokHTML:
			out = out.Call("concat", p.textRenderer.html(tok))
		case tokLink:
			out = out.Call("concat", p.textRenderer.link(tok))
		case tokImage:
			out = out.Call("concat", p.textRenderer.image(tok))
		case tokCheckbox:
			out = out.Call("concat", p.textRenderer.checkbox(tok))
		case tokStrong:
			out = out.Call("concat", p.textRenderer.strong(tok))
		case tokEm:
			out = out.Call("concat", p.textRenderer.em(tok))
		case tokCodespan:
			out = out.Call("concat", p.textRenderer.codespan(tok))
		case tokBr:
			out = out.Call("concat", p.textRenderer.br(tok))
		case tokDel:
			out = out.Call("concat", p.textRenderer.del(tok))
		default:
			panic("Token type was not found.")
		}
	}
	return out
}
