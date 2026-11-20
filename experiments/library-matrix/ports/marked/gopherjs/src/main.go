// Package main is marked 18.1.1 in Go, compiled to JavaScript by GopherJS: a faithful port of the
// synchronous Markdown-to-HTML compiler subset defined in libraries/marked/CONTRACT.md.
//
//   - options.go: Marked options, token kinds, alignment enums, and AST token structs;
//   - rules.go: immutable compiled RegExp grammar tables (normal, gfm, breaks, pedantic) and
//     read-only JS string constants initialised once in init();
//   - helpers.go: escapeHtmlEntities, decodeNumericCharacterReferences, cleanUrl, splitCells,
//     rtrim, trimTrailingBlankLines, normalizeLabel, findClosingBracket, expandTabs;
//   - tokenizer.go: Tokenizer block and inline rules, outputLink, indentCodeCompensation,
//     isLabelEndInsideToken;
//   - lexer.go: Lexer (lex, lexInline, blockTokens, inlineTokens, pre-scan delimiter masking);
//   - renderer.go: Renderer and TextRenderer;
//   - parser.go: Parser (parse, parseInline, parseInlineText);
//   - boundary.go: corpus input decoding and op dispatch.
//
// run is published on js.Module.exports, which the builder's facade binds and re-exports.
package main

import "github.com/gopherjs/gopherjs/js"

func main() {
	js.Module.Get("exports").Set("run", run)
}
