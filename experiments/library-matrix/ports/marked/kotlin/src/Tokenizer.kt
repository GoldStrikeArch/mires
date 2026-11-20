package marked

import kotlin.js.RegExp

private val expandTabsZero: (String) -> String = { ws -> expandTabs(ws, 0) }

private fun outputLink(
    cap0: String,
    cap1: String,
    href: String,
    linkTitle: String?,
    raw: String,
    lexer: Lexer,
    rules: Rules,
): Token? {
    val title = if (linkTitle.isNullOrBlankStr()) null else linkTitle
    val text = cap1.asJs().replaceRe(rules.other.outputLinkReplace, "$1")
    val isImage = cap0.length > 0 && cap0[0] == '!'

    val outerInLink = lexer.state.inLink
    val outerLinkEmitted = lexer.state.linkEmitted
    val outerInRawBlock = lexer.state.inRawBlock
    lexer.state.inLink = true
    lexer.state.linkEmitted = false
    val tokens = lexer.inlineTokens(text)
    val textHasLink = lexer.state.linkEmitted
    lexer.state.linkEmitted = outerLinkEmitted
    lexer.state.inLink = outerInLink

    if (!isImage) {
        if (textHasLink) {
            lexer.state.inRawBlock = outerInRawBlock
            return null
        }
        lexer.state.linkEmitted = true
        return Token.Link(
            raw = raw,
            href = href,
            title = title,
            text = text,
            tokens = tokens,
        )
    }

    return Token.Image(
        raw = raw,
        href = href,
        title = title,
        text = text,
        tokens = tokens,
    )
}

private fun indentCodeCompensation(raw: String, text: String, rules: Rules): String {
    val matchIndentToCode = rules.other.indentCodeCompensation.exec(raw) ?: return text
    val indentToCodeLen = matchIndentToCode.req(1).length
    val lines = text.asJs().splitStr("\n")
    for (i in 0 until lines.length) {
        val node = lines[i]
        val matchIndentInNode = rules.other.beginningSpace.exec(node)
        if (matchIndentInNode != null) {
            val indentInNodeLen = matchIndentInNode.req(0).length
            val strip = if (indentInNodeLen < indentToCodeLen) indentInNodeLen else indentToCodeLen
            lines[i] = node.asJs().slice(strip)
        }
    }
    return lines.join("\n")
}

private fun isLabelEndInsideToken(src: String, label: String, labelStart: Int, rules: Rules): Boolean {
    if (!label.asJs().includes("<")) {
        return false
    }
    var i = 0
    val len = label.length
    while (i < len) {
        val ch = label[i]
        if (ch == '\\') {
            i += 2
            continue
        }
        if (ch == '`') {
            val code = rules.inline.code.exec(label.asJs().slice(i))
            if (code != null) {
                i += code.req(0).length
                continue
            }
        }
        if (ch != '<') {
            i++
            continue
        }
        val tokenSrc = src.asJs().slice(labelStart + i)
        val token = rules.inline.tag.exec(tokenSrc) ?: rules.inline.autolink.exec(tokenSrc)
        if (token == null) {
            i++
            continue
        }
        val tokenLen = token.req(0).length
        if (tokenLen > len - i) {
            return true
        }
        i += tokenLen
    }
    return false
}

class Tokenizer(
    val options: MarkedOptions,
    val rules: Rules,
    val lexer: Lexer,
) {
    fun space(src: String): Token.Space? {
        val cap = rules.block.newline.exec(src) ?: return null
        val cap0 = cap.req(0)
        if (cap0.length > 0) {
            return Token.Space(cap0)
        }
        return null
    }

    fun code(src: String): Token.Code? {
        val cap = rules.block.code.exec(src) ?: return null
        val cap0 = cap.req(0)
        val raw = if (options.pedantic) cap0 else trimTrailingBlankLines(cap0)
        val text = raw.asJs().replaceRe(rules.other.codeRemoveIndent, "")
        return Token.Code(
            raw = raw,
            text = text,
            codeBlockStyle = "indented",
        )
    }

    fun fences(src: String): Token.Code? {
        val fencesRule = rules.block.fences ?: return null
        val cap = fencesRule.exec(src) ?: return null
        val raw = cap.req(0)
        val text = indentCodeCompensation(raw, cap.opt(3) ?: "", rules)
        val cap2 = cap.opt(2)
        val lang = if (!cap2.isNullOrBlankStr()) {
            cap2.unsafeCast<String>().asJs().trim().asJs().replaceRe(rules.inline.anyPunctuation, "$1")
        } else {
            cap2
        }
        return Token.Code(
            raw = raw,
            lang = lang,
            text = text,
        )
    }

    fun heading(src: String): Token.Heading? {
        val cap = rules.block.heading.exec(src) ?: return null
        var text = cap.req(2).asJs().trim()
        if (rules.other.endingHash.test(text)) {
            val trimmed = rtrim(text, '#')
            if (options.pedantic) {
                text = trimmed.asJs().trim()
            } else if (trimmed.length == 0 || rules.other.endingSpaceTabChar.test(trimmed)) {
                text = trimmed.asJs().trim()
            }
        }
        return Token.Heading(
            raw = rtrim(cap.req(0), '\n'),
            depth = cap.req(1).length,
            text = text,
            tokens = lexer.inline(text),
        )
    }

    fun hr(src: String): Token.Hr? {
        val cap = rules.block.hr.exec(src) ?: return null
        return Token.Hr(rtrim(cap.req(0), '\n'))
    }

    fun blockquote(src: String): Token.Blockquote? {
        val cap = rules.block.blockquote.exec(src) ?: return null
        var lines = rtrim(cap.req(0), '\n').asJs().splitStr("\n")
        var raw = ""
        var text = ""
        val tokens = JsArray<Token>()

        while (lines.length > 0) {
            var inBlockquote = false
            val currentLines = JsArray<String>()
            var i = 0
            while (i < lines.length) {
                val line = lines[i]
                if (rules.other.blockquoteStart.test(line)) {
                    currentLines.push(line)
                    inBlockquote = true
                } else if (!inBlockquote) {
                    currentLines.push(line)
                } else {
                    break
                }
                i++
            }
            lines = lines.slice(i)

            val currentRaw = currentLines.join("\n")
            val currentText = currentRaw
                .asJs().replaceRe(rules.other.blockquoteSetextReplace, "\n    $1")
                .asJs().replaceRe(rules.other.blockquoteSetextReplace2, "")
            raw = if (raw.length > 0) "$raw\n$currentRaw" else currentRaw
            text = if (text.length > 0) "$text\n$currentText" else currentText

            val top = lexer.state.top
            lexer.state.top = true
            lexer.blockTokens(currentText, tokens, true)
            lexer.state.top = top

            if (lines.length == 0) {
                break
            }

            val lastToken = tokens.lastOrNull()
            if (lastToken is Token.Code || lastToken is Token.Heading || lastToken is Token.Hr || lastToken is Token.Html) {
                break
            } else if (lastToken is Token.Blockquote) {
                val continuation = lines.join("\n")
                val newText = lastToken.raw + "\n" + continuation.asJs().replaceRe(rules.other.blockquoteSetextReplace2, "")
                val newToken = blockquote(newText)!!
                tokens[tokens.length - 1] = newToken

                val leftoverRaw = newText.asJs().substring(newToken.raw.length)
                val leftover = if (leftoverRaw.length > 0 && leftoverRaw[0] == '\n') {
                    leftoverRaw.asJs().substring(1)
                } else {
                    leftoverRaw
                }
                val leftoverCount = if (leftover.length > 0) leftover.asJs().splitStr("\n").length else 0
                val consumed = if (leftoverCount > 0) lines.slice(0, -leftoverCount) else lines
                if (consumed.length > 0) {
                    raw = "$raw\n${consumed.join("\n")}"
                }
                text = text.asJs().substring(0, text.length - lastToken.text.length) + newToken.text
                break
            } else if (lastToken is Token.List) {
                val newText = lastToken.raw + "\n" + lines.join("\n")
                val newToken = list(newText)!!
                tokens[tokens.length - 1] = newToken

                raw = raw.asJs().substring(0, raw.length - lastToken.raw.length) + newToken.raw
                text = text.asJs().substring(0, text.length - lastToken.raw.length) + newToken.raw
                lines = newText.asJs().substring(newToken.raw.length).asJs().splitStr("\n")
                continue
            }
        }

        return Token.Blockquote(
            raw = raw,
            tokens = tokens,
            text = text,
        )
    }

    fun list(srcInput: String): Token.List? {
        var src = srcInput
        var cap = rules.block.list.exec(src) ?: return null
        val listSrc = src
        var bull = cap.req(1).asJs().trim()
        val isOrdered = bull.length > 1

        val listToken = Token.List(
            raw = "",
            ordered = isOrdered,
            start = if (isOrdered) parseInt(bull.asJs().slice(0, -1), 10) else 0,
            loose = false,
            items = JsArray(),
        )

        bull = if (isOrdered) "\\d{1,9}\\${bull.asJs().slice(-1)}" else "\\$bull"
        if (options.pedantic) {
            bull = if (isOrdered) bull else "[*+-]"
        }

        val itemRegex = rules.other.listItemRegex(bull)
        var endsWithBlankLine = false

        while (src.length > 0) {
            var endEarly = false
            var itemContents = ""
            cap = itemRegex.exec(src) ?: break
            if (rules.block.hr.test(src)) {
                break
            }

            val itemStart = listSrc.length - src.length
            src = src.asJs().substring(cap.req(0).length)

            val firstLine = cap.req(2).asJs().splitStrLimit("\n", 1)[0]
            val bulletIndent = cap.req(1).length
            var line = if (options.pedantic) {
                expandTabs(firstLine, bulletIndent)
            } else {
                firstLine.asJs().replaceFn1(rules.other.leadingSpaceTab) { ws -> expandTabs(ws, bulletIndent) }
            }
            var nextLine = src.asJs().splitStrLimit("\n", 1)[0]
            var blankLine = line.asJs().trim().length == 0

            var indent: Int
            if (options.pedantic) {
                indent = 2
                itemContents = line.asJs().trimStart()
            } else if (blankLine) {
                indent = bulletIndent + 1
            } else {
                indent = line.asJs().search(rules.other.nonSpaceChar)
                if (indent > 4) indent = 1
                itemContents = line.asJs().slice(indent)
                indent += bulletIndent
            }

            if (blankLine && rules.other.blankLine.test(nextLine)) {
                src = src.asJs().substring(nextLine.length + 1)
                endEarly = true
            }

            if (!endEarly) {
                val nextBulletRegex = rules.other.nextBulletRegex(indent)
                val hrRegex = rules.other.hrRegex(indent)
                val fencesBeginRegex = rules.other.fencesBeginRegex(indent)
                val headingBeginRegex = rules.other.headingBeginRegex(indent)
                val htmlBeginRegex = rules.other.htmlBeginRegex(indent)
                val blockquoteBeginRegex = rules.other.blockquoteBeginRegex(indent)

                while (src.length > 0) {
                    val rawLine = src.asJs().splitStrLimit("\n", 1)[0]
                    val nextLineWithoutTabs: String
                    nextLine = rawLine

                    if (options.pedantic) {
                        nextLine = nextLine.asJs().replaceRe(rules.other.listReplaceNesting, "  ")
                        nextLineWithoutTabs = nextLine
                    } else {
                        nextLineWithoutTabs = nextLine.asJs().replaceFn1(rules.other.leadingSpaceTab, expandTabsZero)
                    }

                    if (fencesBeginRegex.test(nextLine)) break
                    if (headingBeginRegex.test(nextLine)) break
                    if (htmlBeginRegex.test(nextLine)) break
                    if (blockquoteBeginRegex.test(nextLine)) break
                    if (nextBulletRegex.test(nextLine)) break
                    if (hrRegex.test(nextLine)) break

                    if (nextLineWithoutTabs.asJs().search(rules.other.nonSpaceChar) >= indent || nextLine.asJs().trim().length == 0) {
                        itemContents += "\n" + nextLineWithoutTabs.asJs().slice(indent)
                    } else {
                        if (blankLine) break
                        if (line.asJs().replaceRe(rules.other.tabCharGlobal, "    ").asJs().search(rules.other.nonSpaceChar) >= 4) break
                        if (fencesBeginRegex.test(line)) break
                        if (headingBeginRegex.test(line)) break
                        if (hrRegex.test(line)) break

                        itemContents += "\n" + nextLine
                    }

                    blankLine = nextLine.asJs().trim().length == 0
                    src = src.asJs().substring(rawLine.length + 1)
                    line = nextLineWithoutTabs.asJs().slice(indent)
                }
            }

            val raw = listSrc.asJs().slice(itemStart, listSrc.length - src.length)
            if (!listToken.loose) {
                if (endsWithBlankLine) {
                    listToken.loose = true
                } else if (rules.other.doubleBlankLine.test(raw)) {
                    endsWithBlankLine = true
                }
            }

            listToken.items.push(
                Token.ListItem(
                    raw = raw,
                    task = options.gfm && rules.other.listIsTask.test(itemContents),
                    checked = false,
                    loose = false,
                    text = itemContents,
                    tokens = JsArray(),
                ),
            )
        }

        val lastItem = listToken.items.lastOrNull() ?: return null
        lastItem.raw = lastItem.raw.asJs().trimEnd()
        lastItem.text = lastItem.text.asJs().trimEnd()
        listToken.raw = listSrc.asJs().slice(0, listSrc.length - src.length).asJs().trimEnd()

        // First pass: tokenize items and finalize list.loose
        for (i in 0 until listToken.items.length) {
            val item = listToken.items[i]
            lexer.state.top = false
            item.tokens = lexer.blockTokens(item.text, JsArray())

            if (!listToken.loose) {
                var hasMultipleLineBreaks = false
                for (j in 0 until item.tokens.length) {
                    val t = item.tokens[j]
                    if (t is Token.Space && rules.other.anyLine.test(t.raw)) {
                        hasMultipleLineBreaks = true
                        break
                    }
                }
                listToken.loose = hasMultipleLineBreaks
            }
        }

        // Second pass: place task checkboxes using final list.loose
        for (i in 0 until listToken.items.length) {
            val item = listToken.items[i]
            val itemToken = if (item.tokens.length > 0) item.tokens[0] else null
            if (item.task && (itemToken is Token.Text || itemToken is Token.Paragraph)) {
                item.text = item.text.asJs().replaceRe(rules.other.listReplaceTask, "")
                if (itemToken is Token.Text) {
                    itemToken.raw = itemToken.raw.asJs().replaceRe(rules.other.listReplaceTask, "")
                    itemToken.text = itemToken.text.asJs().replaceRe(rules.other.listReplaceTask, "")
                } else if (itemToken is Token.Paragraph) {
                    itemToken.raw = itemToken.raw.asJs().replaceRe(rules.other.listReplaceTask, "")
                    itemToken.text = itemToken.text.asJs().replaceRe(rules.other.listReplaceTask, "")
                }
                for (qIdx in lexer.inlineQueue.length - 1 downTo 0) {
                    val qItem = lexer.inlineQueue[qIdx]
                    if (rules.other.listIsTask.test(qItem.src)) {
                        qItem.src = qItem.src.asJs().replaceRe(rules.other.listReplaceTask, "")
                        break
                    }
                }

                val taskRaw = rules.other.listTaskCheckbox.exec(item.raw)
                if (taskRaw != null) {
                    val matchedBox = taskRaw.req(0)
                    val checkboxToken = Token.Checkbox(
                        raw = "$matchedBox ",
                        checked = matchedBox != "[ ]",
                    )
                    item.checked = checkboxToken.checked
                    if (listToken.loose) {
                        val first = if (item.tokens.length > 0) item.tokens[0] else null
                        if (first is Token.Paragraph) {
                            first.raw = checkboxToken.raw + first.raw
                            first.text = checkboxToken.raw + first.text
                            first.tokens.unshift(checkboxToken)
                        } else if (first is Token.Text && first.tokens != null) {
                            first.raw = checkboxToken.raw + first.raw
                            first.text = checkboxToken.raw + first.text
                            first.tokens.unshift(checkboxToken)
                        } else {
                            val cbTokens = JsArray<Token>()
                            cbTokens.push(checkboxToken)
                            item.tokens.unshift(
                                Token.Paragraph(
                                    raw = checkboxToken.raw,
                                    text = checkboxToken.raw,
                                    tokens = cbTokens,
                                ),
                            )
                        }
                    } else {
                        item.tokens.unshift(checkboxToken)
                    }
                }
            } else if (item.task) {
                item.task = false
            }
        }

        // Set all items to loose if list is loose
        if (listToken.loose) {
            for (i in 0 until listToken.items.length) {
                val item = listToken.items[i]
                item.loose = true
                for (j in 0 until item.tokens.length) {
                    val t = item.tokens[j]
                    if (t is Token.Text) {
                        item.tokens[j] = Token.Paragraph(t.raw, t.text, t.tokens ?: JsArray())
                    }
                }
            }
        }

        return listToken
    }

    fun html(src: String): Token.Html? {
        val cap = rules.block.html.exec(src) ?: return null
        val raw = trimTrailingBlankLines(cap.req(0))
        val cap1 = cap.opt(1)
        return Token.Html(
            raw = raw,
            block = true,
            pre = cap1 == "pre" || cap1 == "script" || cap1 == "style",
            text = raw,
        )
    }

    fun def(src: String): Token.Def? {
        val cap = rules.block.def.exec(src) ?: return null
        val cap2 = cap.opt(2) ?: ""
        if (!rules.other.startAngleBracket.test(cap2) && findClosingBracket(cap2, "()") != -1) {
            return null
        }
        val tag = normalizeLabel(cap.req(1)).asJs().replaceRe(rules.other.multipleSpaceGlobal, " ")
        val href = if (cap2.length > 0) {
            cap2.asJs().replaceRe(rules.other.hrefBrackets, "$1")
                .asJs().replaceRe(rules.inline.anyPunctuation, "$1")
        } else {
            ""
        }
        val cap3 = cap.opt(3)
        val title = if (!cap3.isNullOrBlankStr()) {
            val s = cap3.unsafeCast<String>()
            s.asJs().substring(1, s.length - 1).asJs().replaceRe(rules.inline.anyPunctuation, "$1")
        } else {
            cap3
        }
        return Token.Def(
            tag = tag,
            raw = rtrim(cap.req(0), '\n'),
            href = href,
            title = title,
        )
    }

    fun table(src: String): Token.Table? {
        val tableRule = rules.block.table ?: return null
        val cap = tableRule.exec(src) ?: return null
        val cap2 = cap.req(2)
        if (!rules.other.tableDelimiter.test(cap2)) {
            return null
        }

        val headers = splitCells(cap.req(1))
        val aligns = cap2.asJs().replaceRe(rules.other.tableAlignChars, "").asJs().splitStr("|")
        val cap3 = cap.opt(3)
        val rows = if (cap3 != null && cap3.asJs().trim().length > 0) {
            cap3.asJs().replaceRe(rules.other.tableRowBlankLine, "").asJs().splitStr("\n")
        } else {
            JsArray()
        }

        if (headers.length != aligns.length) {
            return null
        }

        val itemAlign = JsArray<String?>()
        for (i in 0 until aligns.length) {
            val align = aligns[i]
            when {
                rules.other.tableAlignRight.test(align) -> itemAlign.push("right")
                rules.other.tableAlignCenter.test(align) -> itemAlign.push("center")
                rules.other.tableAlignLeft.test(align) -> itemAlign.push("left")
                else -> itemAlign.push(null)
            }
        }

        val itemHeader = JsArray<TableCell>()
        for (i in 0 until headers.length) {
            val h = headers[i]
            itemHeader.push(
                TableCell(
                    text = h,
                    tokens = lexer.inline(h),
                    header = true,
                    align = itemAlign[i],
                ),
            )
        }

        val itemRows = JsArray<JsArray<TableCell>>()
        for (r in 0 until rows.length) {
            val splitRow = splitCells(rows[r], itemHeader.length)
            val rowCells = JsArray<TableCell>()
            for (c in 0 until splitRow.length) {
                val cellText = splitRow[c]
                rowCells.push(
                    TableCell(
                        text = cellText,
                        tokens = lexer.inline(cellText),
                        header = false,
                        align = itemAlign[c],
                    ),
                )
            }
            itemRows.push(rowCells)
        }

        return Token.Table(
            raw = rtrim(cap.req(0), '\n'),
            header = itemHeader,
            align = itemAlign,
            rows = itemRows,
        )
    }

    fun lheading(src: String): Token.Heading? {
        val cap = rules.block.lheading.exec(src) ?: return null
        val text = cap.req(1).asJs().trim()
        return Token.Heading(
            raw = rtrim(cap.req(0), '\n'),
            depth = if (cap.req(2)[0] == '=') 1 else 2,
            text = text,
            tokens = lexer.inline(text),
        )
    }

    fun paragraph(src: String): Token.Paragraph? {
        val cap = rules.block.paragraph.exec(src) ?: return null
        val cap1 = cap.req(1)
        val text = if (cap1.length > 0 && cap1[cap1.length - 1] == '\n') {
            cap1.asJs().slice(0, -1)
        } else {
            cap1
        }
        return Token.Paragraph(
            raw = cap.req(0),
            text = text,
            tokens = lexer.inline(text),
        )
    }

    fun text(src: String): Token.Text? {
        val cap = rules.block.text.exec(src) ?: return null
        val cap0 = cap.req(0)
        return Token.Text(
            raw = cap0,
            text = cap0,
            tokens = lexer.inline(cap0),
        )
    }

    fun escape(src: String): Token.Escape? {
        val cap = rules.inline.escape.exec(src) ?: return null
        return Token.Escape(
            raw = cap.req(0),
            text = cap.req(1),
        )
    }

    fun tag(src: String): Token.Html? {
        val cap = rules.inline.tag.exec(src) ?: return null
        val cap0 = cap.req(0)
        if (!lexer.state.inLink && rules.other.startATag.test(cap0)) {
            lexer.state.inLink = true
        } else if (lexer.state.inLink && rules.other.endATag.test(cap0)) {
            lexer.state.inLink = false
        }
        if (!lexer.state.inRawBlock && rules.other.startPreScriptTag.test(cap0)) {
            lexer.state.inRawBlock = true
        } else if (lexer.state.inRawBlock && rules.other.endPreScriptTag.test(cap0)) {
            lexer.state.inRawBlock = false
        }
        return Token.Html(
            raw = cap0,
            inLink = lexer.state.inLink,
            inRawBlock = lexer.state.inRawBlock,
            block = false,
            text = cap0,
        )
    }

    fun link(src: String): Token? {
        if (!lexer.state.linkParenPossible) {
            return null
        }
        val cap = rules.inline.link.exec(src) ?: return null
        var cap0 = cap.req(0)
        val cap1 = cap.req(1)
        var cap2 = cap.req(2)
        var cap3 = cap.opt(3)

        val labelStart = if (cap0[0] == '!') 2 else 1
        if (!options.pedantic && isLabelEndInsideToken(src, cap1, labelStart, rules)) {
            return null
        }

        val trimmedUrl = cap2.asJs().trim()
        if (!options.pedantic && rules.other.startAngleBracket.test(trimmedUrl)) {
            if (!rules.other.endAngleBracket.test(trimmedUrl)) {
                return null
            }
            val rtrimSlash = rtrim(trimmedUrl.asJs().slice(0, -1), '\\')
            if ((trimmedUrl.length - rtrimSlash.length) % 2 == 0) {
                return null
            }
        } else {
            val lastParenIndex = findClosingBracket(cap2, "()")
            if (lastParenIndex == -2) {
                return null
            }
            if (lastParenIndex > -1) {
                val start = if (cap0[0] == '!') 5 else 4
                val linkLen = start + cap1.length + lastParenIndex
                cap2 = cap2.asJs().substring(0, lastParenIndex)
                cap0 = cap0.asJs().substring(0, linkLen).asJs().trim()
                cap3 = ""
            }
        }

        var href = cap2
        var title = ""
        if (options.pedantic) {
            val pedanticMatch = rules.other.pedanticHrefTitle.exec(href)
            if (pedanticMatch != null) {
                href = pedanticMatch.req(1)
                title = pedanticMatch.req(3)
            }
        } else {
            title = if (!cap3.isNullOrBlankStr()) cap3.unsafeCast<String>().asJs().slice(1, -1) else ""
        }

        href = href.asJs().trim()
        if (rules.other.startAngleBracket.test(href)) {
            href = if (options.pedantic && !rules.other.endAngleBracket.test(trimmedUrl)) {
                href.asJs().slice(1)
            } else {
                href.asJs().slice(1, -1)
            }
        }

        val unescapedHref = if (href.length > 0) href.asJs().replaceRe(rules.inline.anyPunctuation, "$1") else href
        val unescapedTitle = if (title.length > 0) title.asJs().replaceRe(rules.inline.anyPunctuation, "$1") else title
        return outputLink(cap0, cap1, unescapedHref, unescapedTitle, cap0, lexer, rules)
    }

    fun reflink(src: String, links: JsMap<String, LinkDef>): Token? {
        val cap = rules.inline.reflink.exec(src) ?: rules.inline.nolink.exec(src) ?: return null
        val cap0 = cap.req(0)
        val cap1 = cap.req(1)
        val cap2 = cap.opt(2)

        val labelStart = if (cap0[0] == '!') 2 else 1
        if (!options.pedantic && isLabelEndInsideToken(src, cap1, labelStart, rules)) {
            return null
        }

        val rawRef = if (!cap2.isNullOrBlankStr()) cap2.unsafeCast<String>() else cap1
        val linkString = rawRef.asJs().replaceRe(rules.other.multipleSpaceGlobal, " ")
        val linkDef = links.get(normalizeLabel(linkString))
        if (linkDef == null) {
            val text = cap0.asJs().substring(0, 1)
            return Token.Text(raw = text, text = text)
        }
        return outputLink(cap0, cap1, linkDef.href, linkDef.title, cap0, lexer, rules)
    }

    fun emStrong(src: String, maskedSrcInput: String, prevChar: String = ""): Token? {
        var match = rules.inline.emStrongLDelim.exec(src) ?: return null
        val m1 = match.opt(1)
        val m2 = match.opt(2)
        val m3 = match.opt(3)
        val m4 = match.opt(4)
        if (m1.isNullOrBlankStr() && m2.isNullOrBlankStr() && m3.isNullOrBlankStr() && m4.isNullOrBlankStr()) {
            return null
        }

        if (!m4.isNullOrBlankStr() && rules.other.unicodeAlphaNumeric.test(prevChar)) {
            return null
        }

        val nextChar = if (!m1.isNullOrBlankStr()) m1.unsafeCast<String>() else if (!m3.isNullOrBlankStr()) m3.unsafeCast<String>() else ""
        if (nextChar.length == 0 || prevChar.length == 0 || rules.inline.punctuation.exec(prevChar) != null) {
            val match0 = match.req(0)
            val lLength = codePointCount(match0) - 1
            var delimTotal = lLength
            var midDelimTotal = 0

            val delimChar = match0[0]
            val midRun = prevChar.length == 1 && prevChar[0] == delimChar
            val endReg: RegExp = if (delimChar == '*') {
                rules.inline.emStrongRDelimAst
            } else {
                rules.inline.emStrongRDelimUnd
            }
            endReg.lastIndex = 0

            val maskedSrc = maskedSrcInput.asJs().slice(-src.length + lLength)

            while (true) {
                match = endReg.exec(maskedSrc) ?: break
                val g1 = match.opt(1)
                val g2 = match.opt(2)
                val g3 = match.opt(3)
                val g4 = match.opt(4)
                val g5 = match.opt(5)
                val g6 = match.opt(6)
                val rDelim = when {
                    !g1.isNullOrBlankStr() -> g1.unsafeCast<String>()
                    !g2.isNullOrBlankStr() -> g2.unsafeCast<String>()
                    !g3.isNullOrBlankStr() -> g3.unsafeCast<String>()
                    !g4.isNullOrBlankStr() -> g4.unsafeCast<String>()
                    !g5.isNullOrBlankStr() -> g5.unsafeCast<String>()
                    !g6.isNullOrBlankStr() -> g6.unsafeCast<String>()
                    else -> continue
                }

                var rLength = codePointCount(rDelim)

                if (!g3.isNullOrBlankStr() || !g4.isNullOrBlankStr()) {
                    delimTotal += rLength
                    continue
                } else if (!g5.isNullOrBlankStr() || !g6.isNullOrBlankStr()) {
                    if (lLength % 3 != 0 && (lLength + rLength) % 3 == 0) {
                        midDelimTotal += rLength
                        continue
                    }
                    if (midRun) {
                        break
                    }
                }

                delimTotal -= rLength
                if (delimTotal > 0) continue

                val adjusted = rLength + delimTotal + midDelimTotal
                if (adjusted < rLength) rLength = adjusted
                val lastCharLength = firstCodePointLength(match.req(0))
                val raw = src.asJs().slice(0, lLength + match.index + lastCharLength + rLength)
                endReg.lastIndex = 0

                val minLen = if (lLength < rLength) lLength else rLength
                if (minLen % 2 != 0) {
                    val text = raw.asJs().slice(1, -1)
                    return Token.Em(
                        raw = raw,
                        text = text,
                        tokens = lexer.inlineTokens(text),
                    )
                }

                val text = raw.asJs().slice(2, -2)
                return Token.Strong(
                    raw = raw,
                    text = text,
                    tokens = lexer.inlineTokens(text),
                )
            }
            endReg.lastIndex = 0
        }
        return null
    }

    fun codespan(src: String): Token.Codespan? {
        val cap = rules.inline.code.exec(src) ?: return null
        var text = cap.req(2).asJs().replaceRe(rules.other.newLineCharGlobal, " ")
        val hasNonSpaceChars = rules.other.nonSpaceChar.test(text)
        val hasSpaceCharsOnBothEnds = rules.other.startingSpaceChar.test(text) && rules.other.endingSpaceChar.test(text)
        if (hasNonSpaceChars && hasSpaceCharsOnBothEnds) {
            text = text.asJs().substring(1, text.length - 1)
        }
        return Token.Codespan(
            raw = cap.req(0),
            text = text,
        )
    }

    fun br(src: String): Token.Br? {
        val cap = rules.inline.br.exec(src) ?: return null
        return Token.Br(cap.req(0))
    }

    fun del(src: String, maskedSrcInput: String, prevChar: String = ""): Token.Del? {
        val delLDelim = rules.inline.delLDelim ?: return null
        val endReg = rules.inline.delRDelim ?: return null
        var match = delLDelim.exec(src) ?: return null
        val nextChar = match.opt(1) ?: ""

        if (nextChar.length == 0 || prevChar.length == 0 || rules.inline.punctuation.exec(prevChar) != null) {
            val lLength = codePointCount(match.req(0)) - 1
            var delimTotal = lLength

            endReg.lastIndex = 0
            val maskedSrc = maskedSrcInput.asJs().slice(-src.length + lLength)

            while (true) {
                match = endReg.exec(maskedSrc) ?: break
                val g1 = match.opt(1)
                val g2 = match.opt(2)
                val g3 = match.opt(3)
                val g4 = match.opt(4)
                val g5 = match.opt(5)
                val g6 = match.opt(6)
                val rDelim = when {
                    !g1.isNullOrBlankStr() -> g1.unsafeCast<String>()
                    !g2.isNullOrBlankStr() -> g2.unsafeCast<String>()
                    !g3.isNullOrBlankStr() -> g3.unsafeCast<String>()
                    !g4.isNullOrBlankStr() -> g4.unsafeCast<String>()
                    !g5.isNullOrBlankStr() -> g5.unsafeCast<String>()
                    !g6.isNullOrBlankStr() -> g6.unsafeCast<String>()
                    else -> continue
                }

                var rLength = codePointCount(rDelim)
                if (rLength != lLength) continue

                if (!g3.isNullOrBlankStr() || !g4.isNullOrBlankStr()) {
                    delimTotal += rLength
                    continue
                }

                delimTotal -= rLength
                if (delimTotal > 0) continue

                val adjusted = rLength + delimTotal
                if (adjusted < rLength) rLength = adjusted
                val lastCharLength = firstCodePointLength(match.req(0))
                val raw = src.asJs().slice(0, lLength + match.index + lastCharLength + rLength)
                endReg.lastIndex = 0

                val text = raw.asJs().slice(lLength, -lLength)
                return Token.Del(
                    raw = raw,
                    text = text,
                    tokens = lexer.inlineTokens(text),
                )
            }
            endReg.lastIndex = 0
        }
        return null
    }

    fun autolink(src: String): Token.Link? {
        val cap = rules.inline.autolink.exec(src) ?: return null
        val text = cap.req(1)
        val href = if (cap.opt(2) == "@") "mailto:$text" else text
        val childTokens = JsArray<Token>()
        childTokens.push(Token.Text(raw = text, text = text))
        return Token.Link(
            raw = cap.req(0),
            text = text,
            href = href,
            title = null,
            autolink = true,
            tokens = childTokens,
        )
    }

    fun url(src: String): Token.Link? {
        val urlRule = rules.inline.url ?: return null
        val cap = urlRule.exec(src) ?: return null
        var cap0 = cap.req(0)
        val text: String
        val href: String
        if (cap.opt(2) == "@") {
            text = cap0
            href = "mailto:$text"
        } else {
            val backpedal = rules.inline.backpedal!!
            var prevCapZero: String
            do {
                prevCapZero = cap0
                cap0 = backpedal.exec(cap0)?.opt(0) ?: ""
            } while (prevCapZero != cap0)
            text = cap0
            href = if (cap.opt(1) == "www.") "http://$cap0" else cap0
        }
        val childTokens = JsArray<Token>()
        childTokens.push(Token.Text(raw = text, text = text))
        return Token.Link(
            raw = cap0,
            text = text,
            href = href,
            title = null,
            autolink = true,
            tokens = childTokens,
        )
    }

    fun inlineText(src: String): Token.Text? {
        val cap = rules.inline.text.exec(src) ?: return null
        val cap0 = cap.req(0)
        val escaped = lexer.state.inRawBlock
        return Token.Text(
            raw = cap0,
            text = if (escaped) cap0 else decodeNumericCharacterReferences(cap0),
            escaped = escaped,
        )
    }
}
