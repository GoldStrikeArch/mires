// Options, token types, and AST structures mirroring marked 18.1.1 (defaults.ts, Tokens.ts).
package main

import "github.com/gopherjs/gopherjs/js"

// jsStr aliases *js.Object for JavaScript primitive strings. Keeping Markdown and HTML text as
// JS strings avoids GopherJS's per-call UTF-8 <-> UTF-16 $externalize/$internalize loops when
// invoking host RegExp rules and guarantees exact UTF-16 code-unit indexing (e.g. linkLen
// truncation, maskedSrc alignment, astral prevChar).
type jsStr = *js.Object

// Options mirrors MarkedOptions for the contracted subset.
type Options struct {
	gfm      bool
	breaks   bool
	pedantic bool
}

func defaultOptions() *Options {
	return &Options{
		gfm:      true,
		breaks:   false,
		pedantic: false,
	}
}

// TokenKind identifies the concrete marked token variant without interface dispatch.
type TokenKind uint8

const (
	tokSpace TokenKind = iota
	tokCode
	tokHeading
	tokHr
	tokBlockquote
	tokList
	tokListItem
	tokCheckbox
	tokHTML
	tokDef
	tokTable
	tokParagraph
	tokText
	tokEscape
	tokStrong
	tokEm
	tokCodespan
	tokBr
	tokDel
	tokLink
	tokImage
)

// AlignKind identifies GFM table column alignment.
type AlignKind uint8

const (
	alignNone AlignKind = iota
	alignLeft
	alignCenter
	alignRight
)

// LinkDef holds a normalized link reference definition from block.def.
type LinkDef struct {
	href  jsStr
	title jsStr
}

// TableCell holds a single GFM table header or body cell.
type TableCell struct {
	text   jsStr
	tokens []*Token
	header bool
	align  AlignKind
}

// Token represents any block or inline marked token as a single concrete struct, avoiding
// Go interface calls (which GopherJS compiles into $restore state machines).
type Token struct {
	kind TokenKind

	raw  jsStr
	text jsStr

	// code
	lang jsStr

	// heading
	depth int

	// list
	ordered bool
	start   int
	loose   bool
	items   []*Token

	// list_item / checkbox
	task    bool
	checked bool

	// def / link / image
	tag      string
	href     jsStr
	title    jsStr
	autolink bool

	// table
	header []*TableCell
	rows   [][]*TableCell

	// text / escape / inline containers
	escaped   bool
	hasTokens bool
	tokens    []*Token
}
