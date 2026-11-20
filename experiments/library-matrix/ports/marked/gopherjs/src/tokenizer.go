// Tokenizer mirroring marked 18.1.1 src/Tokenizer.ts.
package main

import "github.com/gopherjs/gopherjs/js"

// Tokenizer executes block and inline grammar rules on JS strings.
type Tokenizer struct {
	options *Options
	rules   *Rules
	lexer   *Lexer
}

func outputLink(cap0 jsStr, cap1 jsStr, href jsStr, title jsStr, raw jsStr, lexer *Lexer, rules *Rules) *Token {
	text := cap1.Call("replace", rules.other.outputLinkReplace, jsDollar1)
	isImage := cap0.Call("charCodeAt", 0).Int() == '!'

	outerInLink := lexer.state.inLink
	outerLinkEmitted := lexer.state.linkEmitted
	outerInRawBlock := lexer.state.inRawBlock
	lexer.state.inLink = true
	lexer.state.linkEmitted = false
	tokens := lexer.inlineTokens(text, nil)
	textHasLink := lexer.state.linkEmitted
	lexer.state.linkEmitted = outerLinkEmitted
	lexer.state.inLink = outerInLink

	if !isImage {
		if textHasLink {
			lexer.state.inRawBlock = outerInRawBlock
			return nil
		}
		lexer.state.linkEmitted = true
	}

	kind := tokLink
	if isImage {
		kind = tokImage
	}
	return &Token{
		kind:      kind,
		raw:       raw,
		href:      href,
		title:     title,
		text:      text,
		hasTokens: true,
		tokens:    tokens,
	}
}

func indentCodeCompensation(raw jsStr, text jsStr, rules *Rules) jsStr {
	matchIndentToCode := rules.other.indentCodeCompensation.Call("exec", raw)
	if matchIndentToCode == nil {
		return text
	}
	indentLen := matchIndentToCode.Index(1).Length()
	lines := splitToSlice(text)
	for i := 0; i < len(lines); i++ {
		node := lines[i]
		m := rules.other.beginningSpace.Call("exec", node)
		if m != nil {
			n := m.Index(0).Length()
			if indentLen < n {
				n = indentLen
			}
			lines[i] = node.Call("slice", n)
		}
	}
	return joinLines(lines)
}

func isLabelEndInsideToken(src jsStr, label jsStr, labelStart int, rules *Rules) bool {
	if !label.Call("includes", jsLtChar).Bool() {
		return false
	}
	labelLen := label.Length()
	for i := 0; i < labelLen; i++ {
		ch := label.Call("charCodeAt", i).Int()
		if ch == '\\' {
			i++
			continue
		}
		if ch == '`' {
			code := rules.inline.code.Call("exec", label.Call("slice", i))
			if code != nil {
				i += code.Index(0).Length() - 1
				continue
			}
		}
		if ch != '<' {
			continue
		}
		tokenSrc := src.Call("slice", labelStart+i)
		token := rules.inline.tag.Call("exec", tokenSrc)
		if token == nil {
			token = rules.inline.autolink.Call("exec", tokenSrc)
		}
		if token == nil {
			continue
		}
		tokLen := token.Index(0).Length()
		if tokLen > labelLen-i {
			return true
		}
		i += tokLen - 1
	}
	return false
}

func indentIdx(indent int) int {
	idx := indent - 1
	if idx < 0 {
		return 0
	}
	if idx > 3 {
		return 3
	}
	return idx
}

func (t *Tokenizer) space(src jsStr) *Token {
	cap := t.rules.block.newline.Call("exec", src)
	if cap != nil {
		raw := cap.Index(0)
		if raw.Length() > 0 {
			return &Token{kind: tokSpace, raw: raw}
		}
	}
	return nil
}

func (t *Tokenizer) code(src jsStr) *Token {
	cap := t.rules.block.code.Call("exec", src)
	if cap != nil {
		raw := cap.Index(0)
		if !t.options.pedantic {
			raw = trimTrailingBlankLines(raw)
		}
		text := raw.Call("replace", t.rules.other.codeRemoveIndent, jsEmpty)
		return &Token{
			kind: tokCode,
			raw:  raw,
			text: text,
		}
	}
	return nil
}

func (t *Tokenizer) fences(src jsStr) *Token {
	if t.rules.block.fences == nil {
		return nil
	}
	cap := t.rules.block.fences.Call("exec", src)
	if cap != nil {
		raw := cap.Index(0)
		body := cap.Index(3)
		if body == js.Undefined || body == nil {
			body = jsEmpty
		}
		text := indentCodeCompensation(raw, body, t.rules)
		lang := cap.Index(2)
		if isTruthy(lang) {
			lang = lang.Call("trim").Call("replace", t.rules.inline.anyPunctuation, jsDollar1)
		} else {
			lang = jsEmpty
		}
		return &Token{
			kind: tokCode,
			raw:  raw,
			lang: lang,
			text: text,
		}
	}
	return nil
}

func (t *Tokenizer) heading(src jsStr) *Token {
	cap := t.rules.block.heading.Call("exec", src)
	if cap != nil {
		text := cap.Index(2).Call("trim")
		if t.rules.other.endingHash.Call("test", text).Bool() {
			trimmed := rtrim(text, '#', false)
			if t.options.pedantic {
				text = trimmed.Call("trim")
			} else if trimmed.Length() == 0 || t.rules.other.endingSpaceTabChar.Call("test", trimmed).Bool() {
				text = trimmed.Call("trim")
			}
		}
		tok := &Token{
			kind:      tokHeading,
			raw:       rtrim(cap.Index(0), '\n', false),
			depth:     cap.Index(1).Length(),
			text:      text,
			hasTokens: true,
		}
		t.lexer.inline(text, tok)
		return tok
	}
	return nil
}

func (t *Tokenizer) hr(src jsStr) *Token {
	cap := t.rules.block.hr.Call("exec", src)
	if cap != nil {
		return &Token{
			kind: tokHr,
			raw:  rtrim(cap.Index(0), '\n', false),
		}
	}
	return nil
}

func (t *Tokenizer) blockquote(src jsStr) *Token {
	cap := t.rules.block.blockquote.Call("exec", src)
	if cap == nil {
		return nil
	}
	lines := splitToSlice(rtrim(cap.Index(0), '\n', false))
	raw := jsEmpty
	text := jsEmpty
	var tokens []*Token

	for len(lines) > 0 {
		inBlockquote := false
		var currentLines []jsStr
		i := 0
		for ; i < len(lines); i++ {
			if t.rules.other.blockquoteStart.Call("test", lines[i]).Bool() {
				currentLines = append(currentLines, lines[i])
				inBlockquote = true
			} else if !inBlockquote {
				currentLines = append(currentLines, lines[i])
			} else {
				break
			}
		}
		lines = lines[i:]

		currentRaw := joinLines(currentLines)
		currentText := currentRaw.
			Call("replace", t.rules.other.blockquoteSetextReplace, jsBqSetextRepl).
			Call("replace", t.rules.other.blockquoteSetextReplace2, jsEmpty)
		if raw.Length() > 0 {
			raw = raw.Call("concat", jsNewline, currentRaw)
		} else {
			raw = currentRaw
		}
		if text.Length() > 0 {
			text = text.Call("concat", jsNewline, currentText)
		} else {
			text = currentText
		}

		top := t.lexer.state.top
		t.lexer.state.top = true
		tokens = t.lexer.blockTokens(currentText, tokens, true)
		t.lexer.state.top = top

		if len(lines) == 0 {
			break
		}

		var lastToken *Token
		if len(tokens) > 0 {
			lastToken = tokens[len(tokens)-1]
		}
		if lastToken == nil {
			continue
		}
		if lastToken.kind == tokCode || lastToken.kind == tokHeading || lastToken.kind == tokHr || lastToken.kind == tokHTML {
			break
		} else if lastToken.kind == tokBlockquote {
			continuation := joinLines(lines)
			newText := lastToken.raw.Call("concat", jsNewline, continuation.Call("replace", t.rules.other.blockquoteSetextReplace2, jsEmpty))
			newToken := t.blockquote(newText)
			tokens[len(tokens)-1] = newToken

			leftover := newText.Call("substring", newToken.raw.Length()).Call("replace", t.rules.other.leadingNewline, jsEmpty)
			leftoverCount := 0
			if leftover.Length() > 0 {
				leftoverCount = leftover.Call("split", jsNewline).Length()
			}
			consumed := lines
			if leftoverCount > 0 {
				end := len(lines) - leftoverCount
				if end < 0 {
					end = 0
				}
				consumed = lines[:end]
			}
			if len(consumed) > 0 {
				raw = raw.Call("concat", jsNewline, joinLines(consumed))
			}
			text = text.Call("substring", 0, text.Length()-lastToken.text.Length()).Call("concat", newToken.text)
			break
		} else if lastToken.kind == tokList {
			newText := lastToken.raw.Call("concat", jsNewline, joinLines(lines))
			newToken := t.list(newText)
			tokens[len(tokens)-1] = newToken

			raw = raw.Call("substring", 0, raw.Length()-lastToken.raw.Length()).Call("concat", newToken.raw)
			text = text.Call("substring", 0, text.Length()-lastToken.raw.Length()).Call("concat", newToken.raw)
			lines = splitToSlice(newText.Call("substring", newToken.raw.Length()))
			continue
		}
	}

	return &Token{
		kind:   tokBlockquote,
		raw:    raw,
		tokens: tokens,
		text:   text,
	}
}

func (t *Tokenizer) list(src jsStr) *Token {
	cap := t.rules.block.list.Call("exec", src)
	if cap == nil {
		return nil
	}

	listSrc := src
	listSrcLen := listSrc.Length()
	bull := cap.Index(1).Call("trim")
	isOrdered := bull.Length() > 1

	start := 0
	if isOrdered {
		start = parseDecJS(bull.Call("slice", 0, -1))
	}

	list := &Token{
		kind:    tokList,
		raw:     jsEmpty,
		ordered: isOrdered,
		start:   start,
		loose:   false,
	}

	var itemRegex *js.Object
	if isOrdered {
		if bull.Call("charCodeAt", bull.Length()-1).Int() == '.' {
			itemRegex = t.rules.other.listItemOrderedDot
		} else {
			itemRegex = t.rules.other.listItemOrderedParen
		}
	} else if t.options.pedantic {
		itemRegex = t.rules.other.listItemPedanticUnordered
	} else {
		switch bull.Call("charCodeAt", 0).Int() {
		case '*':
			itemRegex = t.rules.other.listItemStar
		case '+':
			itemRegex = t.rules.other.listItemPlus
		default:
			itemRegex = t.rules.other.listItemDash
		}
	}

	endsWithBlankLine := false
	for src.Length() > 0 {
		endEarly := false
		itemContents := jsEmpty
		cap = itemRegex.Call("exec", src)
		if cap == nil {
			break
		}
		if t.rules.block.hr.Call("test", src).Bool() {
			break
		}

		itemStart := listSrcLen - src.Length()
		src = src.Call("substring", cap.Index(0).Length())

		firstLine := firstLineOf(cap.Index(2))
		bulletIndent := cap.Index(1).Length()
		var line jsStr
		if t.options.pedantic {
			line = expandTabs(firstLine, bulletIndent)
		} else {
			line = expandLeadingTabs(firstLine, bulletIndent)
		}
		nextLine := firstLineOf(src)
		blankLine := line.Call("trim").Length() == 0

		indent := 0
		if t.options.pedantic {
			indent = 2
			itemContents = line.Call("trimStart")
		} else if blankLine {
			indent = bulletIndent + 1
		} else {
			indent = searchNonSpace(line)
			if indent > 4 {
				indent = 1
			}
			itemContents = line.Call("slice", indent)
			indent += bulletIndent
		}

		if blankLine && t.rules.other.blankLine.Call("test", nextLine).Bool() {
			src = src.Call("substring", nextLine.Length()+1)
			endEarly = true
		}

		if !endEarly {
			idx := indentIdx(indent)
			nextBulletRegex := t.rules.other.nextBulletRegex[idx]
			hrRegex := t.rules.other.hrRegex[idx]
			fencesBeginRegex := t.rules.other.fencesBeginRegex[idx]
			headingBeginRegex := t.rules.other.headingBeginRegex[idx]
			htmlBeginRegex := t.rules.other.htmlBeginRegex[idx]
			blockquoteBeginRegex := t.rules.other.blockquoteBeginRegex[idx]

			for src.Length() > 0 {
				rawLine := firstLineOf(src)
				nextLine = rawLine
				var nextLineWithoutTabs jsStr
				if t.options.pedantic {
					nextLine = nextLine.Call("replace", t.rules.other.listReplaceNesting, jsTwoSpaces)
					nextLineWithoutTabs = nextLine
				} else {
					nextLineWithoutTabs = expandLeadingTabs(nextLine, 0)
				}

				if fencesBeginRegex.Call("test", nextLine).Bool() {
					break
				}
				if headingBeginRegex.Call("test", nextLine).Bool() {
					break
				}
				if htmlBeginRegex.Call("test", nextLine).Bool() {
					break
				}
				if blockquoteBeginRegex.Call("test", nextLine).Bool() {
					break
				}
				if nextBulletRegex.Call("test", nextLine).Bool() {
					break
				}
				if hrRegex.Call("test", nextLine).Bool() {
					break
				}

				if searchNonSpace(nextLineWithoutTabs) >= indent || nextLine.Call("trim").Length() == 0 {
					itemContents = itemContents.Call("concat", jsNewline, nextLineWithoutTabs.Call("slice", indent))
				} else {
					if blankLine {
						break
					}
					if searchNonSpace(line.Call("replace", t.rules.other.tabCharGlobal, jsFourSpaces)) >= 4 {
						break
					}
					if fencesBeginRegex.Call("test", line).Bool() {
						break
					}
					if headingBeginRegex.Call("test", line).Bool() {
						break
					}
					if hrRegex.Call("test", line).Bool() {
						break
					}
					itemContents = itemContents.Call("concat", jsNewline, nextLine)
				}

				blankLine = nextLine.Call("trim").Length() == 0
				src = src.Call("substring", rawLine.Length()+1)
				line = nextLineWithoutTabs.Call("slice", indent)
			}
		}

		raw := listSrc.Call("slice", itemStart, listSrcLen-src.Length())
		if !list.loose {
			if endsWithBlankLine {
				list.loose = true
			} else if t.rules.other.doubleBlankLine.Call("test", raw).Bool() {
				endsWithBlankLine = true
			}
		}

		list.items = append(list.items, &Token{
			kind:  tokListItem,
			raw:   raw,
			task:  t.options.gfm && t.rules.other.listIsTask.Call("test", itemContents).Bool(),
			loose: false,
			text:  itemContents,
		})
	}

	if len(list.items) == 0 {
		return nil
	}
	lastItem := list.items[len(list.items)-1]
	lastItem.raw = lastItem.raw.Call("trimEnd")
	lastItem.text = lastItem.text.Call("trimEnd")
	list.raw = listSrc.Call("slice", 0, listSrcLen-src.Length()).Call("trimEnd")

	// First pass: tokenize items and finalize list.loose from spacers before placing checkboxes.
	for i := 0; i < len(list.items); i++ {
		item := list.items[i]
		t.lexer.state.top = false
		item.tokens = t.lexer.blockTokens(item.text, nil, false)
		if !list.loose {
			for j := 0; j < len(item.tokens); j++ {
				if item.tokens[j].kind == tokSpace && t.rules.other.anyLine.Call("test", item.tokens[j].raw).Bool() {
					list.loose = true
					break
				}
			}
		}
	}

	// Second pass: place task checkboxes using the final list.loose.
	for i := 0; i < len(list.items); i++ {
		item := list.items[i]
		var itemToken *Token
		if len(item.tokens) > 0 {
			itemToken = item.tokens[0]
		}
		if item.task && itemToken != nil && (itemToken.kind == tokText || itemToken.kind == tokParagraph) {
			item.text = item.text.Call("replace", t.rules.other.listReplaceTask, jsEmpty)
			itemToken.raw = itemToken.raw.Call("replace", t.rules.other.listReplaceTask, jsEmpty)
			itemToken.text = itemToken.text.Call("replace", t.rules.other.listReplaceTask, jsEmpty)
			for q := len(t.lexer.inlineQueue) - 1; q >= 0; q-- {
				if t.rules.other.listIsTask.Call("test", t.lexer.inlineQueue[q].src).Bool() {
					t.lexer.inlineQueue[q].src = t.lexer.inlineQueue[q].src.Call("replace", t.rules.other.listReplaceTask, jsEmpty)
					break
				}
			}

			taskRaw := t.rules.other.listTaskCheckbox.Call("exec", item.raw)
			if taskRaw != nil {
				box := taskRaw.Index(0)
				checkboxToken := &Token{
					kind:    tokCheckbox,
					raw:     box.Call("concat", jsSpace),
					checked: box != jsUncheckedBox,
				}
				item.checked = checkboxToken.checked
				if list.loose {
					if itemToken.hasTokens {
						itemToken.raw = checkboxToken.raw.Call("concat", itemToken.raw)
						itemToken.text = checkboxToken.raw.Call("concat", itemToken.text)
						itemToken.tokens = append([]*Token{checkboxToken}, itemToken.tokens...)
					} else {
						pTok := &Token{
							kind:      tokParagraph,
							raw:       checkboxToken.raw,
							text:      checkboxToken.raw,
							hasTokens: true,
							tokens:    []*Token{checkboxToken},
						}
						item.tokens = append([]*Token{pTok}, item.tokens...)
					}
				} else {
					item.tokens = append([]*Token{checkboxToken}, item.tokens...)
				}
			}
		} else if item.task {
			item.task = false
		}
	}

	if list.loose {
		for i := 0; i < len(list.items); i++ {
			item := list.items[i]
			item.loose = true
			for j := 0; j < len(item.tokens); j++ {
				if item.tokens[j].kind == tokText {
					item.tokens[j].kind = tokParagraph
				}
			}
		}
	}

	return list
}

func (t *Tokenizer) html(src jsStr) *Token {
	cap := t.rules.block.html.Call("exec", src)
	if cap != nil {
		raw := trimTrailingBlankLines(cap.Index(0))
		return &Token{
			kind: tokHTML,
			raw:  raw,
			text: raw,
		}
	}
	return nil
}

func (t *Tokenizer) def(src jsStr) *Token {
	cap := t.rules.block.def.Call("exec", src)
	if cap != nil {
		cap2 := cap.Index(2)
		if !t.rules.other.startAngleBracket.Call("test", cap2).Bool() && findClosingBracket(cap2) != -1 {
			return nil
		}
		tag := normalizeLabel(cap.Index(1)).Call("replace", t.rules.other.multipleSpaceGlobal, jsSpace).String()
		href := jsEmpty
		if isTruthy(cap2) {
			href = cap2.Call("replace", t.rules.other.hrefBrackets, jsDollar1).Call("replace", t.rules.inline.anyPunctuation, jsDollar1)
		}
		var title jsStr
		cap3 := cap.Index(3)
		if isTruthy(cap3) {
			title = cap3.Call("substring", 1, cap3.Length()-1).Call("replace", t.rules.inline.anyPunctuation, jsDollar1)
		}
		return &Token{
			kind:  tokDef,
			tag:   tag,
			raw:   rtrim(cap.Index(0), '\n', false),
			href:  href,
			title: title,
		}
	}
	return nil
}

func (t *Tokenizer) table(src jsStr) *Token {
	if t.rules.block.table == nil {
		return nil
	}
	cap := t.rules.block.table.Call("exec", src)
	if cap == nil {
		return nil
	}
	cap2 := cap.Index(2)
	if !t.rules.other.tableDelimiter.Call("test", cap2).Bool() {
		return nil
	}

	headers := splitCells(cap.Index(1), 0)
	alignsArr := cap2.Call("replace", t.rules.other.tableAlignChars, jsEmpty).Call("split", jsPipe)
	if len(headers) != alignsArr.Length() {
		return nil
	}

	var rawRows []jsStr
	cap3 := cap.Index(3)
	if cap3 != js.Undefined && cap3 != nil && cap3.Call("trim").Length() > 0 {
		rawRows = splitToSlice(cap3.Call("replace", t.rules.other.tableRowBlankLine, jsEmpty))
	}

	colCount := len(headers)
	aligns := make([]AlignKind, colCount)
	for i := 0; i < colCount; i++ {
		a := alignsArr.Index(i)
		if t.rules.other.tableAlignRight.Call("test", a).Bool() {
			aligns[i] = alignRight
		} else if t.rules.other.tableAlignCenter.Call("test", a).Bool() {
			aligns[i] = alignCenter
		} else if t.rules.other.tableAlignLeft.Call("test", a).Bool() {
			aligns[i] = alignLeft
		} else {
			aligns[i] = alignNone
		}
	}

	headerCells := make([]*TableCell, colCount)
	for i := 0; i < colCount; i++ {
		cell := &TableCell{
			text:   headers[i],
			header: true,
			align:  aligns[i],
		}
		t.lexer.inlineCell(headers[i], cell)
		headerCells[i] = cell
	}

	bodyRows := make([][]*TableCell, len(rawRows))
	for r := 0; r < len(rawRows); r++ {
		parts := splitCells(rawRows[r], colCount)
		rowCells := make([]*TableCell, colCount)
		for i := 0; i < colCount; i++ {
			cell := &TableCell{
				text:   parts[i],
				header: false,
				align:  aligns[i],
			}
			t.lexer.inlineCell(parts[i], cell)
			rowCells[i] = cell
		}
		bodyRows[r] = rowCells
	}

	return &Token{
		kind:   tokTable,
		raw:    rtrim(cap.Index(0), '\n', false),
		header: headerCells,
		rows:   bodyRows,
	}
}

func (t *Tokenizer) lheading(src jsStr) *Token {
	cap := t.rules.block.lheading.Call("exec", src)
	if cap != nil {
		text := cap.Index(1).Call("trim")
		depth := 2
		if cap.Index(2).Call("charCodeAt", 0).Int() == '=' {
			depth = 1
		}
		tok := &Token{
			kind:      tokHeading,
			raw:       rtrim(cap.Index(0), '\n', false),
			depth:     depth,
			text:      text,
			hasTokens: true,
		}
		t.lexer.inline(text, tok)
		return tok
	}
	return nil
}

func (t *Tokenizer) paragraph(src jsStr) *Token {
	cap := t.rules.block.paragraph.Call("exec", src)
	if cap != nil {
		cap1 := cap.Index(1)
		text := cap1
		if cap1.Length() > 0 && cap1.Call("charCodeAt", cap1.Length()-1).Int() == '\n' {
			text = cap1.Call("slice", 0, -1)
		}
		tok := &Token{
			kind:      tokParagraph,
			raw:       cap.Index(0),
			text:      text,
			hasTokens: true,
		}
		t.lexer.inline(text, tok)
		return tok
	}
	return nil
}

func (t *Tokenizer) text(src jsStr) *Token {
	cap := t.rules.block.text.Call("exec", src)
	if cap != nil {
		raw := cap.Index(0)
		tok := &Token{
			kind:      tokText,
			raw:       raw,
			text:      raw,
			hasTokens: true,
		}
		t.lexer.inline(raw, tok)
		return tok
	}
	return nil
}

func (t *Tokenizer) escape(src jsStr) *Token {
	cap := t.rules.inline.escape.Call("exec", src)
	if cap != nil {
		return &Token{
			kind: tokEscape,
			raw:  cap.Index(0),
			text: cap.Index(1),
		}
	}
	return nil
}

func (t *Tokenizer) tag(src jsStr) *Token {
	cap := t.rules.inline.tag.Call("exec", src)
	if cap != nil {
		raw := cap.Index(0)
		if !t.lexer.state.inLink && t.rules.other.startATag.Call("test", raw).Bool() {
			t.lexer.state.inLink = true
		} else if t.lexer.state.inLink && t.rules.other.endATag.Call("test", raw).Bool() {
			t.lexer.state.inLink = false
		}
		if !t.lexer.state.inRawBlock && t.rules.other.startPreScriptTag.Call("test", raw).Bool() {
			t.lexer.state.inRawBlock = true
		} else if t.lexer.state.inRawBlock && t.rules.other.endPreScriptTag.Call("test", raw).Bool() {
			t.lexer.state.inRawBlock = false
		}
		return &Token{
			kind: tokHTML,
			raw:  raw,
			text: raw,
		}
	}
	return nil
}

func (t *Tokenizer) link(src jsStr) *Token {
	if !t.lexer.state.linkParenPossible {
		return nil
	}
	cap := t.rules.inline.link.Call("exec", src)
	if cap == nil {
		return nil
	}
	cap0 := cap.Index(0)
	cap1 := cap.Index(1)
	cap2 := cap.Index(2)
	cap3 := cap.Index(3)
	if cap3 == js.Undefined || cap3 == nil {
		cap3 = jsEmpty
	}

	labelStart := 1
	if cap0.Call("charCodeAt", 0).Int() == '!' {
		labelStart = 2
	}
	if !t.options.pedantic && isLabelEndInsideToken(src, cap1, labelStart, t.rules) {
		return nil
	}

	trimmedUrl := cap2.Call("trim")
	if !t.options.pedantic && t.rules.other.startAngleBracket.Call("test", trimmedUrl).Bool() {
		if !t.rules.other.endAngleBracket.Call("test", trimmedUrl).Bool() {
			return nil
		}
		rtrimSlash := rtrim(trimmedUrl.Call("slice", 0, -1), '\\', false)
		if (trimmedUrl.Length()-rtrimSlash.Length())%2 == 0 {
			return nil
		}
	} else {
		lastParenIndex := findClosingBracket(cap2)
		if lastParenIndex == -2 {
			return nil
		}
		if lastParenIndex > -1 {
			start := 4
			if cap0.Call("charCodeAt", 0).Int() == '!' {
				start = 5
			}
			linkLen := start + cap1.Length() + lastParenIndex
			cap2 = cap2.Call("substring", 0, lastParenIndex)
			cap0 = cap0.Call("substring", 0, linkLen).Call("trim")
			cap3 = jsEmpty
		}
	}

	href := cap2
	title := jsEmpty
	if t.options.pedantic {
		pm := t.rules.other.pedanticHrefTitle.Call("exec", href)
		if pm != nil {
			href = pm.Index(1)
			title = pm.Index(3)
		}
	} else if cap3.Length() > 0 {
		title = cap3.Call("slice", 1, -1)
	}

	href = href.Call("trim")
	if t.rules.other.startAngleBracket.Call("test", href).Bool() {
		if t.options.pedantic && !t.rules.other.endAngleBracket.Call("test", trimmedUrl).Bool() {
			href = href.Call("slice", 1)
		} else {
			href = href.Call("slice", 1, -1)
		}
	}

	if href.Length() > 0 {
		href = href.Call("replace", t.rules.inline.anyPunctuation, jsDollar1)
	}
	if title.Length() > 0 {
		title = title.Call("replace", t.rules.inline.anyPunctuation, jsDollar1)
	}
	return outputLink(cap0, cap1, href, title, cap0, t.lexer, t.rules)
}

func (t *Tokenizer) reflink(src jsStr, links map[string]*LinkDef) *Token {
	cap := t.rules.inline.reflink.Call("exec", src)
	if cap == nil {
		cap = t.rules.inline.nolink.Call("exec", src)
	}
	if cap == nil {
		return nil
	}
	cap0 := cap.Index(0)
	cap1 := cap.Index(1)
	cap2 := cap.Index(2)

	labelStart := 1
	if cap0.Call("charCodeAt", 0).Int() == '!' {
		labelStart = 2
	}
	if !t.options.pedantic && isLabelEndInsideToken(src, cap1, labelStart, t.rules) {
		return nil
	}

	linkSrc := cap1
	if isTruthy(cap2) {
		linkSrc = cap2
	}
	linkKey := normalizeLabel(linkSrc.Call("replace", t.rules.other.multipleSpaceGlobal, jsSpace)).String()
	linkDef, ok := links[linkKey]
	if !ok {
		ch := cap0.Call("slice", 0, 1)
		return &Token{
			kind: tokText,
			raw:  ch,
			text: ch,
		}
	}
	return outputLink(cap0, cap1, linkDef.href, linkDef.title, cap0, t.lexer, t.rules)
}

func (t *Tokenizer) emStrong(src jsStr, maskedSrc jsStr, prevChar jsStr) *Token {
	match := t.rules.inline.emStrongLDelim.Call("exec", src)
	if match == nil {
		return nil
	}
	m1 := match.Index(1)
	m2 := match.Index(2)
	m3 := match.Index(3)
	m4 := match.Index(4)
	if !isTruthy(m1) && !isTruthy(m2) && !isTruthy(m3) && !isTruthy(m4) {
		return nil
	}

	if isTruthy(m4) && prevChar.Length() > 0 && t.rules.other.unicodeAlphaNumeric.Call("test", prevChar).Bool() {
		return nil
	}

	nextChar := jsEmpty
	if isTruthy(m1) {
		nextChar = m1
	} else if isTruthy(m3) {
		nextChar = m3
	}

	if nextChar.Length() == 0 || prevChar.Length() == 0 || t.rules.inline.punctuation.Call("test", prevChar).Bool() {
		m0 := match.Index(0)
		lLength := codePointLen(m0) - 1
		delimTotal := lLength
		midDelimTotal := 0

		delimChar := m0.Call("charCodeAt", 0).Int()
		midRun := prevChar.Length() == 1 && prevChar.Call("charCodeAt", 0).Int() == delimChar
		endReg := t.rules.inline.emStrongRDelimUnd
		if delimChar == '*' {
			endReg = t.rules.inline.emStrongRDelimAst
		}
		endReg.Set("lastIndex", 0)

		clipped := maskedSrc.Call("slice", -src.Length()+lLength)
		for {
			match = endReg.Call("exec", clipped)
			if match == nil {
				break
			}
			g1 := match.Index(1)
			g2 := match.Index(2)
			g3 := match.Index(3)
			g4 := match.Index(4)
			g5 := match.Index(5)
			g6 := match.Index(6)

			var rDelim jsStr
			if isTruthy(g1) {
				rDelim = g1
			} else if isTruthy(g2) {
				rDelim = g2
			} else if isTruthy(g3) {
				rDelim = g3
			} else if isTruthy(g4) {
				rDelim = g4
			} else if isTruthy(g5) {
				rDelim = g5
			} else if isTruthy(g6) {
				rDelim = g6
			}
			if rDelim == nil {
				continue
			}

			rLength := rDelim.Length()
			if isTruthy(g3) || isTruthy(g4) {
				delimTotal += rLength
				continue
			} else if isTruthy(g5) || isTruthy(g6) {
				if lLength%3 != 0 && (lLength+rLength)%3 == 0 {
					midDelimTotal += rLength
					continue
				}
				if midRun {
					break
				}
			}

			delimTotal -= rLength
			if delimTotal > 0 {
				continue
			}

			if rLength+delimTotal+midDelimTotal < rLength {
				rLength = rLength + delimTotal + midDelimTotal
			}
			lastCharLen := firstCodePointLen(match.Index(0))
			raw := src.Call("slice", 0, lLength+match.Get("index").Int()+lastCharLen+rLength)

			minLen := lLength
			if rLength < minLen {
				minLen = rLength
			}
			if minLen%2 != 0 {
				text := raw.Call("slice", 1, -1)
				return &Token{
					kind:      tokEm,
					raw:       raw,
					text:      text,
					hasTokens: true,
					tokens:    t.lexer.inlineTokens(text, nil),
				}
			}
			text := raw.Call("slice", 2, -2)
			return &Token{
				kind:      tokStrong,
				raw:       raw,
				text:      text,
				hasTokens: true,
				tokens:    t.lexer.inlineTokens(text, nil),
			}
		}
	}
	return nil
}

func (t *Tokenizer) codespan(src jsStr) *Token {
	cap := t.rules.inline.code.Call("exec", src)
	if cap != nil {
		text := cap.Index(2).Call("replace", t.rules.other.newLineCharGlobal, jsSpace)
		hasNonSpace := t.rules.other.nonSpaceChar.Call("test", text).Bool()
		hasSpaceBothEnds := t.rules.other.startingSpaceChar.Call("test", text).Bool() && t.rules.other.endingSpaceChar.Call("test", text).Bool()
		if hasNonSpace && hasSpaceBothEnds {
			text = text.Call("substring", 1, text.Length()-1)
		}
		return &Token{
			kind: tokCodespan,
			raw:  cap.Index(0),
			text: text,
		}
	}
	return nil
}

func (t *Tokenizer) br(src jsStr) *Token {
	cap := t.rules.inline.br.Call("exec", src)
	if cap != nil {
		return &Token{
			kind: tokBr,
			raw:  cap.Index(0),
		}
	}
	return nil
}

func (t *Tokenizer) del(src jsStr, maskedSrc jsStr, prevChar jsStr) *Token {
	if t.rules.inline.delLDelim == nil {
		return nil
	}
	match := t.rules.inline.delLDelim.Call("exec", src)
	if match == nil {
		return nil
	}
	nextChar := jsEmpty
	if m1 := match.Index(1); isTruthy(m1) {
		nextChar = m1
	}

	if nextChar.Length() == 0 || prevChar.Length() == 0 || t.rules.inline.punctuation.Call("test", prevChar).Bool() {
		lLength := codePointLen(match.Index(0)) - 1
		delimTotal := lLength

		endReg := t.rules.inline.delRDelim
		endReg.Set("lastIndex", 0)
		clipped := maskedSrc.Call("slice", -src.Length()+lLength)

		for {
			match = endReg.Call("exec", clipped)
			if match == nil {
				break
			}
			g1 := match.Index(1)
			g2 := match.Index(2)
			g3 := match.Index(3)
			g4 := match.Index(4)
			g5 := match.Index(5)
			g6 := match.Index(6)

			var rDelim jsStr
			if isTruthy(g1) {
				rDelim = g1
			} else if isTruthy(g2) {
				rDelim = g2
			} else if isTruthy(g3) {
				rDelim = g3
			} else if isTruthy(g4) {
				rDelim = g4
			} else if isTruthy(g5) {
				rDelim = g5
			} else if isTruthy(g6) {
				rDelim = g6
			}
			if rDelim == nil {
				continue
			}

			rLength := rDelim.Length()
			if rLength != lLength {
				continue
			}
			if isTruthy(g3) || isTruthy(g4) {
				delimTotal += rLength
				continue
			}

			delimTotal -= rLength
			if delimTotal > 0 {
				continue
			}

			if rLength+delimTotal < rLength {
				rLength = rLength + delimTotal
			}
			lastCharLen := firstCodePointLen(match.Index(0))
			raw := src.Call("slice", 0, lLength+match.Get("index").Int()+lastCharLen+rLength)
			text := raw.Call("slice", lLength, -lLength)
			return &Token{
				kind:      tokDel,
				raw:       raw,
				text:      text,
				hasTokens: true,
				tokens:    t.lexer.inlineTokens(text, nil),
			}
		}
	}
	return nil
}

func (t *Tokenizer) autolink(src jsStr) *Token {
	cap := t.rules.inline.autolink.Call("exec", src)
	if cap != nil {
		text := cap.Index(1)
		href := text
		if cap2 := cap.Index(2); cap2 != js.Undefined && cap2 != nil && cap2.Length() == 1 && cap2.Call("charCodeAt", 0).Int() == '@' {
			href = jsMailto.Call("concat", text)
		}
		return &Token{
			kind:      tokLink,
			raw:       cap.Index(0),
			text:      text,
			href:      href,
			autolink:  true,
			hasTokens: true,
			tokens: []*Token{
				{
					kind: tokText,
					raw:  text,
					text: text,
				},
			},
		}
	}
	return nil
}

func (t *Tokenizer) url(src jsStr) *Token {
	if t.rules.inline.url == nil {
		return nil
	}
	cap := t.rules.inline.url.Call("exec", src)
	if cap != nil {
		cap0 := cap.Index(0)
		cap1 := cap.Index(1)
		cap2 := cap.Index(2)
		var text, href jsStr
		if cap2 != js.Undefined && cap2 != nil && cap2.Length() == 1 && cap2.Call("charCodeAt", 0).Int() == '@' {
			text = cap0
			href = jsMailto.Call("concat", text)
		} else {
			for {
				prevCap0 := cap0
				bm := t.rules.inline.backpedal.Call("exec", cap0)
				if bm != nil {
					cap0 = bm.Index(0)
				} else {
					cap0 = jsEmpty
				}
				if prevCap0 == cap0 {
					break
				}
			}
			text = cap0
			if cap1 != js.Undefined && cap1 != nil && cap1 == jsWwwDot {
				href = jsHttp.Call("concat", cap0)
			} else {
				href = cap0
			}
		}
		return &Token{
			kind:      tokLink,
			raw:       cap0,
			text:      text,
			href:      href,
			autolink:  true,
			hasTokens: true,
			tokens: []*Token{
				{
					kind: tokText,
					raw:  text,
					text: text,
				},
			},
		}
	}
	return nil
}

func (t *Tokenizer) inlineText(src jsStr) *Token {
	cap := t.rules.inline.text.Call("exec", src)
	if cap != nil {
		raw := cap.Index(0)
		escaped := t.lexer.state.inRawBlock
		text := raw
		if !escaped {
			text = decodeNumericCharacterReferences(raw)
		}
		return &Token{
			kind:    tokText,
			raw:     raw,
			text:    text,
			escaped: escaped,
		}
	}
	return nil
}
