// Immutable compiled RegExp grammars and JS string constants mirroring marked 18.1.1 src/rules.ts.
package main

import "github.com/gopherjs/gopherjs/js"

type otherRules struct {
	codeRemoveIndent               *js.Object
	outputLinkReplace              *js.Object
	indentCodeCompensation         *js.Object
	beginningSpace                 *js.Object
	endingHash                     *js.Object
	startingSpaceChar              *js.Object
	endingSpaceChar                *js.Object
	endingSpaceTabChar             *js.Object
	nonSpaceChar                   *js.Object
	newLineCharGlobal              *js.Object
	tabCharGlobal                  *js.Object
	leadingSpaceTab                *js.Object
	multipleSpaceGlobal            *js.Object
	blankLine                      *js.Object
	doubleBlankLine                *js.Object
	blockquoteStart                *js.Object
	blockquoteSetextReplace        *js.Object
	blockquoteSetextReplace2       *js.Object
	leadingNewline                 *js.Object
	listReplaceNesting             *js.Object
	listIsTask                     *js.Object
	listReplaceTask                *js.Object
	listTaskCheckbox               *js.Object
	anyLine                        *js.Object
	hrefBrackets                   *js.Object
	tableDelimiter                 *js.Object
	tableAlignChars                *js.Object
	tableRowBlankLine              *js.Object
	tableAlignRight                *js.Object
	tableAlignCenter               *js.Object
	tableAlignLeft                 *js.Object
	startATag                      *js.Object
	endATag                        *js.Object
	startPreScriptTag              *js.Object
	endPreScriptTag                *js.Object
	startAngleBracket              *js.Object
	endAngleBracket                *js.Object
	pedanticHrefTitle              *js.Object
	unicodeAlphaNumeric            *js.Object
	numericCharacterReference      *js.Object
	numericCharacterReferenceOrAmp *js.Object
	escapeTest                     *js.Object
	escapeReplace                  *js.Object
	escapeTestNoEncode             *js.Object
	escapeReplaceNoEncode          *js.Object
	caret                          *js.Object
	percentDecode                  *js.Object
	splitPipe                      *js.Object
	slashPipe                      *js.Object
	carriageReturn                 *js.Object
	spaceLine                      *js.Object
	notSpaceStart                  *js.Object
	endingNewline                  *js.Object
	backtickGlobal                 *js.Object
	loneSurrogate                  *js.Object
	listItemStar                   *js.Object
	listItemPlus                   *js.Object
	listItemDash                   *js.Object
	listItemPedanticUnordered      *js.Object
	listItemOrderedDot             *js.Object
	listItemOrderedParen           *js.Object
	nextBulletRegex                [4]*js.Object
	hrRegex                        [4]*js.Object
	fencesBeginRegex               [4]*js.Object
	headingBeginRegex              [4]*js.Object
	htmlBeginRegex                 [4]*js.Object
	blockquoteBeginRegex           [4]*js.Object
}

type blockRules struct {
	blockquote *js.Object
	code       *js.Object
	def        *js.Object
	fences     *js.Object
	heading    *js.Object
	hr         *js.Object
	html       *js.Object
	lheading   *js.Object
	list       *js.Object
	newline    *js.Object
	paragraph  *js.Object
	table      *js.Object
	text       *js.Object
}

type inlineRules struct {
	backpedal                 *js.Object
	anyPunctuation            *js.Object
	anyPunctuationOutsideCode *js.Object
	autolink                  *js.Object
	blockSkip                 *js.Object
	br                        *js.Object
	code                      *js.Object
	del                       *js.Object
	delLDelim                 *js.Object
	delRDelim                 *js.Object
	emStrongLDelim            *js.Object
	emStrongRDelimAst         *js.Object
	emStrongRDelimUnd         *js.Object
	escape                    *js.Object
	link                      *js.Object
	nolink                    *js.Object
	punctuation               *js.Object
	reflink                   *js.Object
	reflinkSearch             *js.Object
	tag                       *js.Object
	text                      *js.Object
	url                       *js.Object
}

type Rules struct {
	other  *otherRules
	block  *blockRules
	inline *inlineRules
}

// Single module-level var block holding read-only compiled grammar rules and JS string constants,
// initialised once in init() and never mutated afterwards (matching marked's module-load rules).
var (
	jsRegExp        *js.Object
	jsEncodeURI     *js.Object
	jsFromCodePoint *js.Object
	jsStringCtor    *js.Object

	otherGrammar  otherRules
	blockNormal   blockRules
	blockGfm      blockRules
	blockPedantic blockRules
	inlineNormal  inlineRules
	inlineGfm     inlineRules
	inlineBreaks  inlineRules
	inlinePedant  inlineRules

	rulesNormal   Rules
	rulesGfm      Rules
	rulesBreaks   Rules
	rulesPedantic Rules

	// Pre-allocated read-only JS primitive strings for zero-conversion hot paths.
	jsEmpty            jsStr
	jsNewline          jsStr
	jsSpace            jsStr
	jsTwoSpaces        jsStr
	jsFourSpaces       jsStr
	jsSpaces           [5]jsStr
	jsTab              jsStr
	jsPercent          jsStr
	jsPipe             jsStr
	jsSpacePipe        jsStr
	jsDollar1          jsStr
	jsBqSetextRepl     jsStr
	jsAmp              jsStr
	jsLt               jsStr
	jsGt               jsStr
	jsQuot             jsStr
	jsApos             jsStr
	jsReplacementChar  jsStr
	jsPlus             jsStr
	jsA                jsStr
	jsLBracket         jsStr
	jsRBracket         jsStr
	jsRBracketLBracket jsStr
	jsLParen           jsStr
	jsRParen           jsStr
	jsLtChar           jsStr
	jsUnderscore       jsStr
	jsMailto           jsStr
	jsHttp             jsStr
	jsWwwDot           jsStr
	jsUncheckedBox     jsStr

	// HTML renderer fragments.
	jsPreCodeOpen     jsStr
	jsPreCodeLangOpen jsStr
	jsTagClose        jsStr
	jsPreCodeClose    jsStr
	jsBqOpen          jsStr
	jsBqClose         jsStr
	jsHrHTML          jsStr
	jsHeadingOpen     [7]jsStr
	jsHeadingClose    [7]jsStr
	jsUlOpen          jsStr
	jsUlClose         jsStr
	jsOlOpen          jsStr
	jsOlStartOpen     jsStr
	jsOlStartClose    jsStr
	jsOlClose         jsStr
	jsLiOpen          jsStr
	jsLiClose         jsStr
	jsCheckboxChecked jsStr
	jsCheckboxUncheck jsStr
	jsPOpen           jsStr
	jsPClose          jsStr
	jsTableOpen       jsStr
	jsTableMid        jsStr
	jsTableClose      jsStr
	jsTbodyOpen       jsStr
	jsTbodyClose      jsStr
	jsTrOpen          jsStr
	jsTrClose         jsStr
	jsThOpen          [4]jsStr
	jsTdOpen          [4]jsStr
	jsThClose         jsStr
	jsTdClose         jsStr
	jsStrongOpen      jsStr
	jsStrongClose     jsStr
	jsEmOpen          jsStr
	jsEmClose         jsStr
	jsCodeOpen        jsStr
	jsCodeClose       jsStr
	jsBrHTML          jsStr
	jsDelOpen         jsStr
	jsDelClose        jsStr
	jsAOpen           jsStr
	jsTitleAttr       jsStr
	jsQuoteGt         jsStr
	jsAClose          jsStr
	jsImgSrc          jsStr
	jsImgAlt          jsStr
	jsQuoteClose      jsStr
)

func mkStr(s string) jsStr {
	return jsStringCtor.Invoke(s)
}

func mkRe(pattern string, flags string) *js.Object {
	return jsRegExp.New(pattern, flags)
}

type regexEditor struct {
	source jsStr
	opt    string
}

func editStr(pattern string, opt string) *regexEditor {
	return &regexEditor{source: mkStr(pattern), opt: opt}
}

func editRe(re *js.Object, opt string) *regexEditor {
	return &regexEditor{source: re.Get("source"), opt: opt}
}

func (e *regexEditor) replaceStr(name string, val string) *regexEditor {
	valSrc := mkStr(val).Call("replace", otherGrammar.caret, jsDollar1)
	e.source = e.source.Call("replace", name, valSrc)
	return e
}

func (e *regexEditor) replaceStrRe(name string, val *js.Object) *regexEditor {
	valSrc := val.Get("source").Call("replace", otherGrammar.caret, jsDollar1)
	e.source = e.source.Call("replace", name, valSrc)
	return e
}

func (e *regexEditor) replaceReStr(name *js.Object, val string) *regexEditor {
	valSrc := mkStr(val).Call("replace", otherGrammar.caret, jsDollar1)
	e.source = e.source.Call("replace", name, valSrc)
	return e
}

func (e *regexEditor) replaceRe(name *js.Object, val *js.Object) *regexEditor {
	valSrc := val.Get("source").Call("replace", otherGrammar.caret, jsDollar1)
	e.source = e.source.Call("replace", name, valSrc)
	return e
}

func (e *regexEditor) getRegex() *js.Object {
	return jsRegExp.New(e.source, e.opt)
}

func mkListItemRe(bull string) *js.Object {
	return mkRe(`^( {0,3}`+bull+`)((?:[\t ][^\n]*)?(?:\n|$))`, "")
}

func createParagraphRe(paragraphCore string, hr *js.Object, fencesInterrupt string, listInterrupt string, htmlInterrupt string, tagList string) *js.Object {
	return editStr(paragraphCore, "").
		replaceStrRe("hr", hr).
		replaceStr("heading", ` {0,3}#{1,6}(?:\s|$)`).
		replaceStr("|lheading", "").
		replaceStr("|table", "").
		replaceStr("blockquote", ` {0,3}>`).
		replaceStr("fences", fencesInterrupt).
		replaceStr("list", listInterrupt).
		replaceStr("html", htmlInterrupt).
		replaceStr("tag", tagList).
		getRegex()
}

func init() {
	jsRegExp = js.Global.Get("RegExp")
	jsEncodeURI = js.Global.Get("encodeURI")
	jsStringCtor = js.Global.Get("String")
	jsFromCodePoint = jsStringCtor.Get("fromCodePoint")

	jsEmpty = mkStr("")
	jsNewline = mkStr("\n")
	jsSpace = mkStr(" ")
	jsTwoSpaces = mkStr("  ")
	jsFourSpaces = mkStr("    ")
	jsSpaces[0] = jsEmpty
	jsSpaces[1] = jsSpace
	jsSpaces[2] = jsTwoSpaces
	jsSpaces[3] = mkStr("   ")
	jsSpaces[4] = jsFourSpaces
	jsTab = mkStr("\t")
	jsPercent = mkStr("%")
	jsPipe = mkStr("|")
	jsSpacePipe = mkStr(" |")
	jsDollar1 = mkStr("$1")
	jsBqSetextRepl = mkStr("\n    $1")
	jsAmp = mkStr("&amp;")
	jsLt = mkStr("&lt;")
	jsGt = mkStr("&gt;")
	jsQuot = mkStr("&quot;")
	jsApos = mkStr("&#39;")
	jsReplacementChar = mkStr("\uFFFD")
	jsPlus = mkStr("+")
	jsA = mkStr("a")
	jsLBracket = mkStr("[")
	jsRBracket = mkStr("]")
	jsRBracketLBracket = mkStr("][")
	jsLParen = mkStr("(")
	jsRParen = mkStr(")")
	jsLtChar = mkStr("<")
	jsUnderscore = mkStr("_")
	jsMailto = mkStr("mailto:")
	jsHttp = mkStr("http://")
	jsWwwDot = mkStr("www.")
	jsUncheckedBox = mkStr("[ ]")

	jsPreCodeOpen = mkStr("<pre><code>")
	jsPreCodeLangOpen = mkStr(`<pre><code class="language-`)
	jsTagClose = mkStr(`">`)
	jsPreCodeClose = mkStr("</code></pre>\n")
	jsBqOpen = mkStr("<blockquote>\n")
	jsBqClose = mkStr("</blockquote>\n")
	jsHrHTML = mkStr("<hr>\n")
	for d := 1; d <= 6; d++ {
		ds := intToString(d)
		jsHeadingOpen[d] = mkStr("<h" + ds + ">")
		jsHeadingClose[d] = mkStr("</h" + ds + ">\n")
	}
	jsUlOpen = mkStr("<ul>\n")
	jsUlClose = mkStr("</ul>\n")
	jsOlOpen = mkStr("<ol>\n")
	jsOlStartOpen = mkStr(`<ol start="`)
	jsOlStartClose = mkStr("\">\n")
	jsOlClose = mkStr("</ol>\n")
	jsLiOpen = mkStr("<li>")
	jsLiClose = mkStr("</li>\n")
	jsCheckboxChecked = mkStr(`<input checked="" disabled="" type="checkbox"> `)
	jsCheckboxUncheck = mkStr(`<input disabled="" type="checkbox"> `)
	jsPOpen = mkStr("<p>")
	jsPClose = mkStr("</p>\n")
	jsTableOpen = mkStr("<table>\n<thead>\n")
	jsTableMid = mkStr("</thead>\n")
	jsTableClose = mkStr("</table>\n")
	jsTbodyOpen = mkStr("<tbody>")
	jsTbodyClose = mkStr("</tbody>")
	jsTrOpen = mkStr("<tr>\n")
	jsTrClose = mkStr("</tr>\n")
	jsThOpen[alignNone] = mkStr("<th>")
	jsThOpen[alignLeft] = mkStr(`<th align="left">`)
	jsThOpen[alignCenter] = mkStr(`<th align="center">`)
	jsThOpen[alignRight] = mkStr(`<th align="right">`)
	jsTdOpen[alignNone] = mkStr("<td>")
	jsTdOpen[alignLeft] = mkStr(`<td align="left">`)
	jsTdOpen[alignCenter] = mkStr(`<td align="center">`)
	jsTdOpen[alignRight] = mkStr(`<td align="right">`)
	jsThClose = mkStr("</th>\n")
	jsTdClose = mkStr("</td>\n")
	jsStrongOpen = mkStr("<strong>")
	jsStrongClose = mkStr("</strong>")
	jsEmOpen = mkStr("<em>")
	jsEmClose = mkStr("</em>")
	jsCodeOpen = mkStr("<code>")
	jsCodeClose = mkStr("</code>")
	jsBrHTML = mkStr("<br>")
	jsDelOpen = mkStr("<del>")
	jsDelClose = mkStr("</del>")
	jsAOpen = mkStr(`<a href="`)
	jsTitleAttr = mkStr(`" title="`)
	jsQuoteGt = mkStr(`">`)
	jsAClose = mkStr("</a>")
	jsImgSrc = mkStr(`<img src="`)
	jsImgAlt = mkStr(`" alt="`)
	jsQuoteClose = mkStr(`">`)

	tagList := "address|article|aside|base|basefont|blockquote|body|caption" +
		"|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption" +
		"|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe" +
		"|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option" +
		"|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title" +
		"|tr|track|ul"

	// other rules (must init caret first so regexEditor works).
	otherGrammar.caret = mkRe(`(^|[^\[])\^`, "g")
	otherGrammar.codeRemoveIndent = mkRe(`^(?: {0,3}\t| {1,4})`, "gm")
	otherGrammar.outputLinkReplace = mkRe(`\\([\[\]])`, "g")
	otherGrammar.indentCodeCompensation = mkRe("^(\\s+)(?:```)", "")
	otherGrammar.beginningSpace = mkRe(`^\s+`, "")
	otherGrammar.endingHash = mkRe(`#$`, "")
	otherGrammar.startingSpaceChar = mkRe(`^ `, "")
	otherGrammar.endingSpaceChar = mkRe(` $`, "")
	otherGrammar.endingSpaceTabChar = mkRe(`[ \t]$`, "")
	otherGrammar.nonSpaceChar = mkRe(`[^ ]`, "")
	otherGrammar.newLineCharGlobal = mkRe(`\n`, "g")
	otherGrammar.tabCharGlobal = mkRe(`\t`, "g")
	otherGrammar.leadingSpaceTab = mkRe(`^[ \t]+`, "")
	otherGrammar.multipleSpaceGlobal = mkRe(`\s+`, "g")
	otherGrammar.blankLine = mkRe(`^[ \t]*$`, "")
	otherGrammar.doubleBlankLine = mkRe(`\n[ \t]*\n[ \t]*$`, "")
	otherGrammar.blockquoteStart = mkRe(`^ {0,3}>`, "")
	otherGrammar.blockquoteSetextReplace = mkRe(`\n {0,3}((?:=+|-+) *)(?=\n|$)`, "g")
	otherGrammar.blockquoteSetextReplace2 = mkRe(`^ {0,3}>[ \t]?`, "gm")
	otherGrammar.leadingNewline = mkRe(`^\n`, "")
	otherGrammar.listReplaceNesting = mkRe(`^ {1,4}(?=( {4})*[^ ])`, "g")
	otherGrammar.listIsTask = mkRe(`^\[[ xX]\] +\S`, "")
	otherGrammar.listReplaceTask = mkRe(`^\[[ xX]\] +`, "")
	otherGrammar.listTaskCheckbox = mkRe(`\[[ xX]\]`, "")
	otherGrammar.anyLine = mkRe(`\n.*\n`, "")
	otherGrammar.hrefBrackets = mkRe(`^<(.*)>$`, "")
	otherGrammar.tableDelimiter = mkRe(`[:|]`, "")
	otherGrammar.tableAlignChars = mkRe(`^\||\| *$`, "g")
	otherGrammar.tableRowBlankLine = mkRe(`\n[ \t]*$`, "")
	otherGrammar.tableAlignRight = mkRe(`^ *-+: *$`, "")
	otherGrammar.tableAlignCenter = mkRe(`^ *:-+: *$`, "")
	otherGrammar.tableAlignLeft = mkRe(`^ *:-+ *$`, "")
	otherGrammar.startATag = mkRe(`^<a `, "i")
	otherGrammar.endATag = mkRe(`^<\/a>`, "i")
	otherGrammar.startPreScriptTag = mkRe(`^<(pre|code|kbd|script)(\s|>)`, "i")
	otherGrammar.endPreScriptTag = mkRe(`^<\/(pre|code|kbd|script)(\s|>)`, "i")
	otherGrammar.startAngleBracket = mkRe(`^<`, "")
	otherGrammar.endAngleBracket = mkRe(`>$`, "")
	otherGrammar.pedanticHrefTitle = mkRe(`^([^'"]*[^\s])\s+(['"])(.*)\2`, "")
	otherGrammar.unicodeAlphaNumeric = mkRe(`[\p{L}\p{N}]`, "u")
	otherGrammar.numericCharacterReference = mkRe(`&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));`, "")
	otherGrammar.numericCharacterReferenceOrAmp = mkRe(`&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)`, "g")
	otherGrammar.escapeTest = mkRe(`[&<>"']`, "")
	otherGrammar.escapeReplace = mkRe(`[&<>"']`, "g")
	otherGrammar.escapeTestNoEncode = mkRe(`[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)`, "")
	otherGrammar.escapeReplaceNoEncode = mkRe(`[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)`, "g")
	otherGrammar.percentDecode = mkRe(`%25`, "g")
	otherGrammar.splitPipe = mkRe(` \|`, "")
	otherGrammar.slashPipe = mkRe(`\\\|`, "g")
	otherGrammar.carriageReturn = mkRe(`\r\n|\r`, "g")
	otherGrammar.spaceLine = mkRe(`^ +$`, "gm")
	otherGrammar.notSpaceStart = mkRe(`^\S*`, "")
	otherGrammar.endingNewline = mkRe(`\n$`, "")
	otherGrammar.backtickGlobal = mkRe("`", "g")
	otherGrammar.loneSurrogate = mkRe(`[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]`, "")

	// Precompiled immutable listItemRegex variants (the 6 fixed grammar patterns).
	otherGrammar.listItemStar = mkListItemRe(`\*`)
	otherGrammar.listItemPlus = mkListItemRe(`\+`)
	otherGrammar.listItemDash = mkListItemRe(`\-`)
	otherGrammar.listItemPedanticUnordered = mkListItemRe(`[*+-]`)
	otherGrammar.listItemOrderedDot = mkListItemRe(`\d{1,9}\.`)
	otherGrammar.listItemOrderedParen = mkListItemRe(`\d{1,9}\)`)

	// Precompiled indent-clamped regexes (indent - 1 clamped to 0..3).
	for i := 0; i < 4; i++ {
		ds := intToString(i)
		otherGrammar.nextBulletRegex[i] = mkRe(`^ {0,`+ds+`}(?:[*+-]|\d{1,9}[.)])((?:[ \t][^\n]*)?(?:\n|$))`, "")
		otherGrammar.hrRegex[i] = mkRe(`^ {0,`+ds+`}((?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)`, "")
		otherGrammar.fencesBeginRegex[i] = mkRe("^ {0,"+ds+"}(?:```|~~~)", "")
		otherGrammar.headingBeginRegex[i] = mkRe(`^ {0,`+ds+`}#`, "")
		otherGrammar.htmlBeginRegex[i] = mkRe(`^ {0,`+ds+`}(?:</?(?:`+tagList+`)(?: +|$|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)`, "i")
		otherGrammar.blockquoteBeginRegex[i] = mkRe(`^ {0,`+ds+`}>`, "")
	}

	// Block-level grammar.
	newline := mkRe(`^(?:[ \t]*(?:\n|$))+`, "")
	blockCode := mkRe(`^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+`, "")
	fences := mkRe("^ {0,3}(`{3,}(?=[^`\\n]*(?:\\n|$))|~{3,})([^\\n]*)(?:\\n|$)(?:|([\\s\\S]*?)(?:\\n|$))(?: {0,3}\\1[~`]* *(?=\\n|$)|$)", "")
	hr := mkRe(`^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)`, "")
	heading := mkRe(`^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)`, "")
	bullet := mkRe(` {0,3}(?:[*+-]|\d{1,9}[.)])`, "")
	lheadingCore := `^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |fences|blockquote|heading|hr|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)`

	reBullG := mkRe(`bull`, "g")
	reBlockCodeG := mkRe(`blockCode`, "g")
	reFencesG := mkRe(`fences`, "g")
	reBlockquoteG := mkRe(`blockquote`, "g")
	reHeadingG := mkRe(`heading`, "g")
	reHrG := mkRe(`hr`, "g")
	reHtmlG := mkRe(`html`, "g")
	rePipeTableG := mkRe(`\|table`, "g")
	reTableG := mkRe(`table`, "g")

	lheading := editStr(lheadingCore, "").
		replaceRe(reBullG, bullet).
		replaceReStr(reBlockCodeG, `(?: {4}| {0,3}\t)`).
		replaceReStr(reFencesG, " {0,3}(?:`{3,}|~{3,})").
		replaceReStr(reBlockquoteG, ` {0,3}>`).
		replaceReStr(reHeadingG, ` {0,3}#{1,6}(?:\s|$)`).
		replaceReStr(reHrG, ` {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)`).
		replaceReStr(reHtmlG, ` {0,3}<[^\n>]+>\n`).
		replaceReStr(rePipeTableG, "").
		getRegex()

	lheadingGfm := editStr(lheadingCore, "").
		replaceRe(reBullG, bullet).
		replaceReStr(reBlockCodeG, `(?: {4}| {0,3}\t)`).
		replaceReStr(reFencesG, " {0,3}(?:`{3,}|~{3,})").
		replaceReStr(reBlockquoteG, ` {0,3}>`).
		replaceReStr(reHeadingG, ` {0,3}#{1,6}(?:\s|$)`).
		replaceReStr(reHrG, ` {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)`).
		replaceReStr(reHtmlG, ` {0,3}<[^\n>]+>\n`).
		replaceReStr(reTableG, ` {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n`).
		getRegex()

	paragraphCore := `^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)`
	blockText := mkRe(`^[^\n]+`, "")
	blockLabel := mkRe(`(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+`, "")
	def := editStr(`^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)`, "").
		replaceStrRe("label", blockLabel).
		replaceStr("title", `(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))`).
		getRegex()

	listRe := editStr(`^(bull)([ \t][^\n]*?)?(?:\n|$)`, "").
		replaceRe(reBullG, bullet).
		getRegex()

	commentRe := mkRe(`<!--(?:-?>|[\s\S]*?(?:-->|$))`, "")
	htmlBlock := editStr(
		`^ {0,3}(?:`+
			`<(script|pre|style|textarea)[\s>][\s\S]*?(?:</\1>[^\n]*\n*|$)`+
			`|comment[^\n]*(\n+|$)`+
			`|<\?[\s\S]*?(?:\?>[^\n]*\n*|$)`+
			`|<![A-Z][\s\S]*?(?:>[^\n]*\n*|$)`+
			`|<!\[CDATA\[[\s\S]*?(?:\]\]>[^\n]*\n*|$)`+
			`|</?(tag)(?: +|\n|/?>)[\s\S]*?(?:(?:\n[ \t]*)+\n|$)`+
			`|<(?!(?:script|pre|style|textarea)(?![a-z0-9-]))([a-z][a-z0-9-]*)(?:attribute)*? */?>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)`+
			`|</[a-z][a-z0-9-]*\s*>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)`+
			`)`, "i").
		replaceStrRe("comment", commentRe).
		replaceStr("tag", tagList).
		replaceStr("attribute", " +[a-zA-Z:_][\\w.:-]*(?: *= *\"[^\"\\n]*\"| *= *'[^'\\n]*'| *= *[^\\s\"'=<>`]+)?").
		getRegex()

	fencesInterrupt := " {0,3}(?:`{3,}(?=[^`\\n]*(?:\\n|$))|~~~)[^\\n]*(?:\\n|$)"
	htmlInterrupt := ` {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)`

	paragraph := createParagraphRe(paragraphCore, hr, fencesInterrupt, ` {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]`, htmlInterrupt, tagList)
	blockquoteParagraph := createParagraphRe(paragraphCore, hr, fencesInterrupt, ` {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)`, htmlInterrupt, tagList)

	blockquote := editStr(`^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+`, "").
		replaceStrRe("paragraph", blockquoteParagraph).
		getRegex()

	blockNormal = blockRules{
		blockquote: blockquote,
		code:       blockCode,
		def:        def,
		fences:     fences,
		heading:    heading,
		hr:         hr,
		html:       htmlBlock,
		lheading:   lheading,
		list:       listRe,
		newline:    newline,
		paragraph:  paragraph,
		table:      nil,
		text:       blockText,
	}

	gfmTable := editStr(
		`^ *([^\n ].*)\n`+
			` {0,3}((?:\| *)?:?-+:? *(?:\| *:?-+:? *)*(?:\| *)?)`+
			`(?:\n((?:(?! *\n|hr|heading|blockquote|code|fences|list|html).*(?:\n|$))*)\n*|$)`, "").
		replaceStrRe("hr", hr).
		replaceStr("heading", ` {0,3}#{1,6}(?:\s|$)`).
		replaceStr("blockquote", ` {0,3}>`).
		replaceStr("code", `(?: {4}| {0,3}\t)[^\n]`).
		replaceStr("fences", fencesInterrupt).
		replaceStr("list", ` {0,3}(?:[*+-]|1[.)])[ \t]`).
		replaceStr("html", htmlInterrupt).
		replaceStr("tag", tagList).
		getRegex()

	blockGfm = blockNormal
	blockGfm.lheading = lheadingGfm
	blockGfm.table = gfmTable
	blockGfm.paragraph = editStr(paragraphCore, "").
		replaceStrRe("hr", hr).
		replaceStr("heading", ` {0,3}#{1,6}(?:\s|$)`).
		replaceStr("|lheading", "").
		replaceStrRe("table", gfmTable).
		replaceStr("blockquote", ` {0,3}>`).
		replaceStr("fences", fencesInterrupt).
		replaceStr("list", ` {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]`).
		replaceStr("html", htmlInterrupt).
		replaceStr("tag", tagList).
		getRegex()

	reTagG := mkRe(`tag`, "g")
	blockPedantic = blockNormal
	blockPedantic.html = editStr(
		`^ *(?:comment *(?:\n|\s*$)`+
			`|<(tag)[\s\S]+?</\1> *(?:\n{2,}|\s*$)`+
			`|<tag(?:"[^"]*"|'[^']*'|\s[^'"/>\s]*)*?/?> *(?:\n{2,}|\s*$))`, "").
		replaceStrRe("comment", commentRe).
		replaceReStr(reTagG, `(?!(?:`+
			`a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub`+
			`|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)`+
			`\b)\w+(?!:|[^\w\s@]*@)\b`).
		getRegex()
	blockPedantic.def = mkRe(`^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)`, "")
	blockPedantic.heading = mkRe(`^(#{1,6})(.*)(?:\n+|$)`, "")
	blockPedantic.fences = nil
	blockPedantic.lheading = mkRe(`^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)`, "")
	blockPedantic.paragraph = editStr(paragraphCore, "").
		replaceStrRe("hr", hr).
		replaceStr("heading", " *#{1,6} *[^\n]").
		replaceStrRe("lheading", lheading).
		replaceStr("|table", "").
		replaceStr("blockquote", ` {0,3}>`).
		replaceStr("|fences", "").
		replaceStr("|list", "").
		replaceStr("|html", "").
		replaceStr("|tag", "").
		getRegex()

	// Inline-level grammar.
	escapeRe := mkRe("^\\\\([!\"#$%&'()*+,\\-./:;<=>?@\\[\\]\\\\^_`{|}~])", "")
	inlineCode := mkRe("^(`+)([^`]|[^`][\\s\\S]*?[^`])\\1(?!`)", "")
	brRe := mkRe(`^( {2,}|\\)\n(?!\s*$)[ \t]*`, "")
	inlineText := mkRe("^(`+|[^`])(?:(?= {2,}\\n)|[\\s\\S]*?(?:(?=[\\\\<!\\[`*_]|\\b_|$)|[^ ](?= {2,}\\n)))", "")

	punctPat := `[\p{P}\p{S}]`
	punctOrSpacePat := `[\s\p{P}\p{S}]`
	notPunctOrSpacePat := `[^\s\p{P}\p{S}]`
	openQuotePat := `[\p{Pi}\p{Ps}"']`
	punctGfmPat := `(?!~)[\p{P}\p{S}]`
	punctOrSpaceGfmPat := `(?!~)[\s\p{P}\p{S}]`
	notPunctOrSpaceGfmPat := `(?:[^\s\p{P}\p{S}]|~)`

	rePunctSpaceG := mkRe(`punctSpace`, "g")
	reNotPunctSpaceG := mkRe(`notPunctSpace`, "g")
	rePunctG := mkRe(`punct`, "g")
	reOpenQuoteG := mkRe(`openQuote`, "g")

	punctuation := editStr(`^((?![*_])punctSpace)`, "u").
		replaceReStr(rePunctSpaceG, punctOrSpacePat).
		getRegex()

	blockSkip := editStr(`link|precode-code|html`, "g").
		replaceStr("link", "\\[(?:[^\\[\\]`]|(?<a>`+)[^`]+\\k<a>(?!`))*?\\]\\((?:\\\\[\\s\\S]|[^\\\\\\(\\)]|\\((?:\\\\[\\s\\S]|[^\\\\\\(\\)])*\\))*\\)").
		replaceStr("precode-", "(?<!`)()").
		replaceStr("code", "(?<b>`+)[^`]+\\k<b>(?!`)").
		replaceStr("html", `<(?! )[^<>]*?>`).
		getRegex()

	emStrongLDelimCore := `^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?`
	emStrongLDelim := editStr(emStrongLDelimCore, "u").
		replaceReStr(rePunctG, punctPat).
		getRegex()
	emStrongLDelimGfm := editStr(emStrongLDelimCore, "u").
		replaceReStr(rePunctG, punctGfmPat).
		getRegex()

	emStrongLDelimPedanticCore := `^(?:\*+(?:((?!\*)(?!openQuote)punct)|([^\s*]))?)|^_+(?:((?!_)(?!openQuote)punct)|([^\s_]))?`
	emStrongLDelimPedantic := editStr(emStrongLDelimPedanticCore, "u").
		replaceReStr(reOpenQuoteG, openQuotePat).
		replaceReStr(rePunctG, punctPat).
		getRegex()

	emStrongRDelimAstCore := `^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)` +
		`|[^*]+(?=[^*])` +
		`|(?!\*)punct(\*+)(?=[\s]|$)` +
		`|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)` +
		`|(?!\*)punctSpace(\*+)(?=notPunctSpace)` +
		`|[\s](\*+)(?!\*)(?=punct)` +
		`|(?!\*)punct(\*+)(?!\*)(?=punct)` +
		`|notPunctSpace(\*+)(?=notPunctSpace)`

	emStrongRDelimAst := editStr(emStrongRDelimAstCore, "gu").
		replaceReStr(reNotPunctSpaceG, notPunctOrSpacePat).
		replaceReStr(rePunctSpaceG, punctOrSpacePat).
		replaceReStr(rePunctG, punctPat).
		getRegex()

	emStrongRDelimAstGfm := editStr(emStrongRDelimAstCore, "gu").
		replaceReStr(reNotPunctSpaceG, notPunctOrSpaceGfmPat).
		replaceReStr(rePunctSpaceG, punctOrSpaceGfmPat).
		replaceReStr(rePunctG, punctGfmPat).
		getRegex()

	emStrongRDelimAstPedanticCore := `^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)` +
		`|[^*]+(?=[^*])` +
		`|(?!\*)punct(\*+)(?=[\s]|$)` +
		`|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)` +
		`|(?!\*)[\s](\*+)(?=notPunctSpace)` +
		`|[\s](\*+)(?!\*)(?=punct)` +
		`|(?!\*)punct(\*+)(?!\*)(?=punct)` +
		`|(?:(?!\*)punct|notPunctSpace)(\*+)(?!\*)(?=notPunctSpace)`

	emStrongRDelimAstPedantic := editStr(emStrongRDelimAstPedanticCore, "gu").
		replaceReStr(reNotPunctSpaceG, notPunctOrSpacePat).
		replaceReStr(rePunctSpaceG, punctOrSpacePat).
		replaceReStr(rePunctG, punctPat).
		getRegex()

	emStrongRDelimUnd := editStr(
		`^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)`+
			`|[^_]+(?=[^_])`+
			`|(?!_)punct(_+)(?=[\s]|$)`+
			`|notPunctSpace(_+)(?!_)(?=punctSpace|$)`+
			`|(?!_)punctSpace(_+)(?=notPunctSpace)`+
			`|[\s](_+)(?!_)(?=punct)`+
			`|(?!_)punct(_+)(?!_)(?=punct)`, "gu").
		replaceReStr(reNotPunctSpaceG, notPunctOrSpacePat).
		replaceReStr(rePunctSpaceG, punctOrSpacePat).
		replaceReStr(rePunctG, punctPat).
		getRegex()

	emStrongRDelimUndPedanticCore := `^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)` +
		`|[^_]+(?=[^_])` +
		`|(?!_)punct(_+)(?=[\s]|$)` +
		`|notPunctSpace(_+)(?!_)(?=punctSpace|$)` +
		`|(?!_)[\s](_+)(?=notPunctSpace)` +
		`|[\s](_+)(?!_)(?=punct)` +
		`|(?!_)punct(_+)(?!_)(?=punct)` +
		`|(?:(?!_)punct|notPunctSpace)(_+)(?!_)(?=notPunctSpace)`

	emStrongRDelimUndPedantic := editStr(emStrongRDelimUndPedanticCore, "gu").
		replaceReStr(reNotPunctSpaceG, notPunctOrSpacePat).
		replaceReStr(rePunctSpaceG, punctOrSpacePat).
		replaceReStr(rePunctG, punctPat).
		getRegex()

	delLDelim := editStr(`^~~?(?:((?!~)punct)|[^\s~])`, "u").
		replaceReStr(rePunctG, punctPat).
		getRegex()

	delRDelimCore := `^[^~]+(?=[^~])` +
		`|(?!~)punct(~~?)(?=[\s]|$)` +
		`|notPunctSpace(~~?)(?!~)(?=punctSpace|$)` +
		`|(?!~)punctSpace(~~?)(?=notPunctSpace)` +
		`|[\s](~~?)(?!~)(?=punct)` +
		`|(?!~)punct(~~?)(?!~)(?=punct)` +
		`|notPunctSpace(~~?)(?=notPunctSpace)`

	delRDelim := editStr(delRDelimCore, "gu").
		replaceReStr(reNotPunctSpaceG, notPunctOrSpacePat).
		replaceReStr(rePunctSpaceG, punctOrSpacePat).
		replaceReStr(rePunctG, punctPat).
		getRegex()

	anyPunctuation := editStr(`\\(punct)`, "gu").
		replaceReStr(rePunctG, punctPat).
		getRegex()

	anyPunctuationOutsideCode := editStr("(`+)(?:[^`]|[^`][\\s\\S]*?[^`])\\1(?!`)|`+|\\\\(punct)", "gu").
		replaceReStr(rePunctG, punctPat).
		getRegex()

	autolink := editStr(`^<(scheme:[^\s\x00-\x1f<>]*|email)>`, "").
		replaceStr("scheme", `[a-zA-Z][a-zA-Z0-9+.-]{1,31}`).
		replaceStr("email", "[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])").
		getRegex()

	inlineComment := editRe(commentRe, "").
		replaceStr(`(?:-->|$)`, `-->`).
		getRegex()

	tagRe := editStr(
		`^comment`+
			`|^</[a-zA-Z][a-zA-Z0-9-]*\s*>`+
			`|^<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\s*/?>`+
			`|^<\?[\s\S]*?\?>`+
			`|^<![a-zA-Z]+\s[\s\S]*?>`+
			`|^<!\[CDATA\[[\s\S]*?\]\]>`, "").
		replaceStrRe("comment", inlineComment).
		replaceStr("attribute", "\\s+[a-zA-Z:_][\\w.:-]*(?:\\s*=\\s*\"[^\"]*\"|\\s*=\\s*'[^']*'|\\s*=\\s*[^\\s\"'=<>`]+)?").
		getRegex()

	inlineLabelBrackets := mkRe(`\[(?:\\[\s\S]|[^\[\]\\])*\]`, "")
	inlineLabel := editStr("(?:\\[(?:brackets|\\\\[\\s\\S]|[^\\[\\]\\\\])*\\]|\\\\[\\s\\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\\])|[^\\[\\]\\\\`])*?", "").
		replaceStrRe("brackets", inlineLabelBrackets).
		getRegex()

	linkRe := editStr(`^!?\[(label)\]\([ \t\n]*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?[ \t\n]*\)`, "").
		replaceStrRe("label", inlineLabel).
		replaceStr("href", `<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))`).
		replaceStr("title", `"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)`).
		getRegex()

	reflinkRe := editStr(`^!?\[(label)\]\[(ref)\]`, "").
		replaceStrRe("label", inlineLabel).
		replaceStrRe("ref", blockLabel).
		getRegex()

	nolinkRe := editStr(`^!?\[(ref)\](?:\[\])?`, "").
		replaceStrRe("ref", blockLabel).
		getRegex()

	boundedBlockLabel := mkRe(`(?!\s*\])(?:\\[\s\S]|[^\[\]\\]){1,999}`, "")
	boundedInlineLabel := editStr("(?:[^\\[\\]\\\\`]*(?:\\[(?:brackets|\\\\[\\s\\S]|[^\\[\\]\\\\])*\\]|\\\\[\\s\\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\\]))){0,999}?[^\\[\\]\\\\`]*?", "").
		replaceStrRe("brackets", inlineLabelBrackets).
		getRegex()

	reflinkSearch := editStr(`reflink|nolink(?!\()`, "g").
		replaceStrRe("reflink", editStr(`^!?\[(label)\]\[(ref)\]`, "").
			replaceStrRe("label", boundedInlineLabel).
			replaceStrRe("ref", boundedBlockLabel).
			getRegex()).
		replaceStrRe("nolink", editStr(`^!?\[(ref)\](?:\[\])?`, "").
			replaceStrRe("ref", boundedBlockLabel).
			getRegex()).
		getRegex()

	caseInsensitiveProtocol := `[hH][tT][tT][pP][sS]?|[fF][tT][pP]`
	extendedEmail := `[A-Za-z0-9._+-]+@[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])`
	reEmailG := mkRe(`email`, "g")
	extendedEmailProtocol := editStr(`(?:mailto:email|xmpp:email(?:\/[A-Za-z0-9@.]+)?)`, "").
		replaceReStr(reEmailG, extendedEmail).
		getRegex()

	inlineNormal = inlineRules{
		backpedal:                 nil,
		anyPunctuation:            anyPunctuation,
		anyPunctuationOutsideCode: anyPunctuationOutsideCode,
		autolink:                  autolink,
		blockSkip:                 blockSkip,
		br:                        brRe,
		code:                      inlineCode,
		del:                       nil,
		delLDelim:                 nil,
		delRDelim:                 nil,
		emStrongLDelim:            emStrongLDelim,
		emStrongRDelimAst:         emStrongRDelimAst,
		emStrongRDelimUnd:         emStrongRDelimUnd,
		escape:                    escapeRe,
		link:                      linkRe,
		nolink:                    nolinkRe,
		punctuation:               punctuation,
		reflink:                   reflinkRe,
		reflinkSearch:             reflinkSearch,
		tag:                       tagRe,
		text:                      inlineText,
		url:                       nil,
	}

	inlinePedant = inlineNormal
	inlinePedant.emStrongLDelim = emStrongLDelimPedantic
	inlinePedant.emStrongRDelimAst = emStrongRDelimAstPedantic
	inlinePedant.emStrongRDelimUnd = emStrongRDelimUndPedantic
	inlinePedant.link = editStr(`^!?\[(label)\]\((.*?)\)`, "").
		replaceStrRe("label", inlineLabel).
		getRegex()
	inlinePedant.reflink = editStr(`^!?\[(label)\]\s*\[([^\]]*)\]`, "").
		replaceStrRe("label", inlineLabel).
		getRegex()

	reEmailProtocolG := mkRe(`emailProtocol`, "g")
	inlineGfm = inlineNormal
	inlineGfm.emStrongRDelimAst = emStrongRDelimAstGfm
	inlineGfm.emStrongLDelim = emStrongLDelimGfm
	inlineGfm.delLDelim = delLDelim
	inlineGfm.delRDelim = delRDelim
	inlineGfm.url = editStr(`^emailProtocol|^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email`, "").
		replaceStrRe("emailProtocol", extendedEmailProtocol).
		replaceStr("protocol", caseInsensitiveProtocol).
		replaceStr("email", `[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])`).
		getRegex()
	inlineGfm.backpedal = mkRe(`(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+`, "")
	inlineGfm.del = mkRe(`^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)`, "")
	inlineGfm.text = editStr("^(?:[^a-zA-Z0-9](?=emailProtocol)|(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\\n)|(?=[a-zA-Z0-9.!#$%&'*+\\/=?_`{\\|}~-]+@)|[\\s\\S]*?(?:(?=[\\\\<!\\[`*~_]|\\b_|protocol:\\/\\/|www\\.|$)|[^ ](?= {2,}\\n)|[^a-zA-Z0-9](?=emailProtocol)|[^a-zA-Z0-9.!#$%&'*+\\/=?_`{\\|}~-](?=[a-zA-Z0-9.!#$%&'*+\\/=?_`{\\|}~-]+@))))", "").
		replaceStr("protocol", caseInsensitiveProtocol).
		replaceReStr(reEmailProtocolG, `(?:mailto|xmpp):`).
		getRegex()

	reBrace2G := mkRe(`\{2,\}`, "g")
	inlineBreaks = inlineGfm
	inlineBreaks.br = editRe(brRe, "").
		replaceStr("{2,}", "*").
		getRegex()
	inlineBreaks.text = editRe(inlineGfm.text, "").
		replaceStr(`\b_`, `\b_| {2,}\n`).
		replaceReStr(reBrace2G, "*").
		getRegex()

	rulesNormal = Rules{other: &otherGrammar, block: &blockNormal, inline: &inlineNormal}
	rulesGfm = Rules{other: &otherGrammar, block: &blockGfm, inline: &inlineGfm}
	rulesBreaks = Rules{other: &otherGrammar, block: &blockGfm, inline: &inlineBreaks}
	rulesPedantic = Rules{other: &otherGrammar, block: &blockPedantic, inline: &inlinePedant}
}
