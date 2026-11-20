package marked

private val maskAnyPunctReplacer: (String, String?, String?) -> String = { match0, code, punct ->
    when {
        !punct.isNullOrBlankStr() -> "+".asJs().repeat(match0.length)
        !code.isNullOrBlankStr() -> {
            val c = code.unsafeCast<String>()
            c + match0.asJs().slice(c.length, -c.length).asJs().replaceRe(OTHER_RULES.backtickGlobal, "+") + c
        }
        else -> match0
    }
}

private val maskBlockSkipReplacer: (String, String?, String?) -> String = { match0, _, context ->
    val offset = context?.length ?: 0
    match0.asJs().slice(0, offset) + "[" + "a".asJs().repeat(match0.length - offset - 2) + "]"
}

class Lexer(val options: MarkedOptions) {
    val tokens: JsArray<Token> = JsArray()
    val links: JsMap<String, LinkDef> = JsMap()
    val state: LexerState = LexerState()
    var inlineQueue: JsArray<InlineQueueItem> = JsArray()
    private val tokenizer: Tokenizer = Tokenizer(
        options = options,
        rules = when {
            options.pedantic -> RULES_PEDANTIC
            options.gfm -> if (options.breaks) RULES_BREAKS else RULES_GFM
            else -> RULES_NORMAL
        },
        lexer = this,
    )

    fun lex(srcInput: String): JsArray<Token> {
        val src = srcInput.asJs().replaceRe(OTHER_RULES.carriageReturn, "\n")
        blockTokens(src, tokens)

        var i = 0
        while (i < inlineQueue.length) {
            val next = inlineQueue[i]
            inlineTokens(next.src, next.tokens)
            i++
        }
        inlineQueue = JsArray()

        return tokens
    }

    fun blockTokens(
        srcInput: String,
        outTokens: JsArray<Token> = JsArray(),
        lastParagraphClippedInit: Boolean = false,
    ): JsArray<Token> {
        var src = if (options.pedantic) {
            srcInput
                .asJs().replaceRe(OTHER_RULES.tabCharGlobal, "    ")
                .asJs().replaceRe(OTHER_RULES.spaceLine, "")
        } else {
            srcInput
        }
        var lastParagraphClipped = lastParagraphClippedInit

        var srcLength = src.length + 1
        while (src.length > 0) {
            if (src.length < srcLength) {
                srcLength = src.length
            } else {
                infiniteLoopError(src.asJs().charCodeAt(0))
                break
            }

            // newline
            val spaceTok = tokenizer.space(src)
            if (spaceTok != null) {
                src = src.asJs().substring(spaceTok.raw.length)
                val lastToken = outTokens.lastOrNull()
                if (spaceTok.raw.length == 1 && lastToken != null) {
                    lastToken.raw += "\n"
                } else {
                    outTokens.push(spaceTok)
                }
                continue
            }

            // code
            val codeTok = tokenizer.code(src)
            if (codeTok != null) {
                src = src.asJs().substring(codeTok.raw.length)
                val lastToken = outTokens.lastOrNull()
                if (lastToken is Token.Paragraph) {
                    lastToken.raw += (if (lastToken.raw.asJs().endsWith("\n")) "" else "\n") + codeTok.raw
                    lastToken.text += "\n" + codeTok.text
                    inlineQueue[inlineQueue.length - 1].src = lastToken.text
                } else if (lastToken is Token.Text) {
                    lastToken.raw += (if (lastToken.raw.asJs().endsWith("\n")) "" else "\n") + codeTok.raw
                    lastToken.text += "\n" + codeTok.text
                    inlineQueue[inlineQueue.length - 1].src = lastToken.text
                } else {
                    outTokens.push(codeTok)
                }
                continue
            }

            // fences
            val fencesTok = tokenizer.fences(src)
            if (fencesTok != null) {
                src = src.asJs().substring(fencesTok.raw.length)
                outTokens.push(fencesTok)
                continue
            }

            // heading
            val headingTok = tokenizer.heading(src)
            if (headingTok != null) {
                src = src.asJs().substring(headingTok.raw.length)
                outTokens.push(headingTok)
                continue
            }

            // hr
            val hrTok = tokenizer.hr(src)
            if (hrTok != null) {
                src = src.asJs().substring(hrTok.raw.length)
                outTokens.push(hrTok)
                continue
            }

            // blockquote
            val bqTok = tokenizer.blockquote(src)
            if (bqTok != null) {
                src = src.asJs().substring(bqTok.raw.length)
                outTokens.push(bqTok)
                continue
            }

            // list
            val listTok = tokenizer.list(src)
            if (listTok != null) {
                src = src.asJs().substring(listTok.raw.length)
                outTokens.push(listTok)
                continue
            }

            // html
            val htmlTok = tokenizer.html(src)
            if (htmlTok != null) {
                src = src.asJs().substring(htmlTok.raw.length)
                outTokens.push(htmlTok)
                continue
            }

            // def
            val defTok = tokenizer.def(src)
            if (defTok != null) {
                src = src.asJs().substring(defTok.raw.length)
                val lastToken = outTokens.lastOrNull()
                if (lastToken is Token.Paragraph) {
                    lastToken.raw += (if (lastToken.raw.asJs().endsWith("\n")) "" else "\n") + defTok.raw
                    lastToken.text += "\n" + defTok.raw
                    inlineQueue[inlineQueue.length - 1].src = lastToken.text
                } else if (lastToken is Token.Text) {
                    lastToken.raw += (if (lastToken.raw.asJs().endsWith("\n")) "" else "\n") + defTok.raw
                    lastToken.text += "\n" + defTok.raw
                    inlineQueue[inlineQueue.length - 1].src = lastToken.text
                } else if (!links.has(defTok.tag)) {
                    links.set(defTok.tag, LinkDef(defTok.href, defTok.title))
                    outTokens.push(defTok)
                }
                continue
            }

            // table (gfm)
            val tableTok = tokenizer.table(src)
            if (tableTok != null) {
                src = src.asJs().substring(tableTok.raw.length)
                outTokens.push(tableTok)
                continue
            }

            // lheading
            val lheadingTok = tokenizer.lheading(src)
            if (lheadingTok != null) {
                src = src.asJs().substring(lheadingTok.raw.length)
                outTokens.push(lheadingTok)
                continue
            }

            // top-level paragraph
            if (state.top) {
                val paraTok = tokenizer.paragraph(src)
                if (paraTok != null) {
                    val lastToken = outTokens.lastOrNull()
                    if (lastParagraphClipped && lastToken is Token.Paragraph) {
                        lastToken.raw += (if (lastToken.raw.asJs().endsWith("\n")) "" else "\n") + paraTok.raw
                        lastToken.text += "\n" + paraTok.text
                        inlineQueue.pop()
                        inlineQueue[inlineQueue.length - 1].src = lastToken.text
                    } else {
                        outTokens.push(paraTok)
                    }
                    lastParagraphClipped = false
                    src = src.asJs().substring(paraTok.raw.length)
                    continue
                }
            }

            // text
            val textTok = tokenizer.text(src)
            if (textTok != null) {
                src = src.asJs().substring(textTok.raw.length)
                val lastToken = outTokens.lastOrNull()
                if (lastToken is Token.Text) {
                    lastToken.raw += (if (lastToken.raw.asJs().endsWith("\n")) "" else "\n") + textTok.raw
                    lastToken.text += "\n" + textTok.text
                    inlineQueue.pop()
                    inlineQueue[inlineQueue.length - 1].src = lastToken.text
                } else {
                    outTokens.push(textTok)
                }
                continue
            }

            if (src.length > 0) {
                infiniteLoopError(src.asJs().charCodeAt(0))
                break
            }
        }

        state.top = true
        return outTokens
    }

    fun inline(src: String, outTokens: JsArray<Token> = JsArray()): JsArray<Token> {
        inlineQueue.push(InlineQueueItem(src, outTokens))
        return outTokens
    }

    private fun linkInText(text: String): Boolean {
        if (!text.asJs().includes("[")) {
            return false
        }

        val linkRule = tokenizer.rules.inline.link
        val blockSkip = tokenizer.rules.inline.blockSkip
        val savedBlockSkipLastIndex = blockSkip.lastIndex
        blockSkip.lastIndex = 0
        while (true) {
            val match = blockSkip.exec(text) ?: break
            val m0 = match.req(0)
            val idx = match.index
            if (linkRule.test(m0) && (idx == 0 || text[idx - 1] != '!')) {
                blockSkip.lastIndex = savedBlockSkipLastIndex
                return true
            }
            if (m0.length == 0) {
                blockSkip.lastIndex++
            }
        }
        blockSkip.lastIndex = savedBlockSkipLastIndex

        val reflinkSearch = tokenizer.rules.inline.reflinkSearch
        val savedReflinkLastIndex = reflinkSearch.lastIndex
        reflinkSearch.lastIndex = 0
        while (true) {
            val match = reflinkSearch.exec(text) ?: break
            val match0 = match.req(0)
            if (match0.length == 0) {
                reflinkSearch.lastIndex++
            }
            val refStart = match0.asJs().lastIndexOf("[")
            if (match0[0] == '!' || !links.has(normalizeLabel(match0.asJs().slice(refStart + 1, -1)))) {
                continue
            }
            if (refStart > 1) {
                val savedNested = reflinkSearch.lastIndex
                val hasInner = linkInText(match0.asJs().slice(1, refStart - 1))
                reflinkSearch.lastIndex = savedNested
                if (hasInner) {
                    continue
                }
            }
            reflinkSearch.lastIndex = savedReflinkLastIndex
            return true
        }
        reflinkSearch.lastIndex = savedReflinkLastIndex

        return false
    }

    private fun maskReflink(match0: String): String {
        val refStart = match0.asJs().lastIndexOf("[")
        if (!links.has(normalizeLabel(match0.asJs().slice(refStart + 1, -1)))) {
            return match0
        }
        if (refStart > 1 && match0[0] != '!') {
            val text = match0.asJs().slice(1, refStart - 1)
            if (linkInText(text)) {
                val reflinkSearch = tokenizer.rules.inline.reflinkSearch
                return "[" + text.asJs().replaceFn1(reflinkSearch) { m -> maskReflink(m) } +
                    "][" + "a".asJs().repeat(match0.length - refStart - 2) + "]"
            }
        }
        return "[" + "a".asJs().repeat(match0.length - 2) + "]"
    }

    fun inlineTokens(src: String, outTokens: JsArray<Token> = JsArray()): JsArray<Token> {
        val prevLinkParenPossible = state.linkParenPossible
        state.linkParenPossible = prevLinkParenPossible && src.asJs().includes(")")
        try {
            return inlineTokensInner(src, outTokens)
        } finally {
            state.linkParenPossible = prevLinkParenPossible
        }
    }

    private fun inlineTokensInner(srcInput: String, outTokens: JsArray<Token>): JsArray<Token> {
        var src = srcInput
        var maskedSrc = src

        // Mask out reflinks
        if (links.size > 0 && src.asJs().includes("[")) {
            val reflinkSearch = tokenizer.rules.inline.reflinkSearch
            maskedSrc = maskedSrc.asJs().replaceFn1(reflinkSearch) { m -> maskReflink(m) }
        }

        // Mask out escaped characters (leaving code spans intact, masking inner backticks)
        maskedSrc = maskedSrc.asJs().replaceFn3(tokenizer.rules.inline.anyPunctuationOutsideCode, maskAnyPunctReplacer)

        // Mask out other blocks (links, code spans, html)
        maskedSrc = maskedSrc.asJs().replaceFn3(tokenizer.rules.inline.blockSkip, maskBlockSkipReplacer)

        var keepPrevChar = false
        var prevChar = ""
        var srcLength = src.length + 1
        while (src.length > 0) {
            if (src.length < srcLength) {
                srcLength = src.length
            } else {
                infiniteLoopError(src.asJs().charCodeAt(0))
                break
            }

            if (!keepPrevChar) {
                prevChar = ""
            }
            keepPrevChar = false

            // escape
            val escTok = tokenizer.escape(src)
            if (escTok != null) {
                src = src.asJs().substring(escTok.raw.length)
                outTokens.push(escTok)
                continue
            }

            // tag
            val tagTok = tokenizer.tag(src)
            if (tagTok != null) {
                src = src.asJs().substring(tagTok.raw.length)
                outTokens.push(tagTok)
                continue
            }

            // link
            val linkTok = tokenizer.link(src)
            if (linkTok != null) {
                src = src.asJs().substring(linkTok.raw.length)
                outTokens.push(linkTok)
                continue
            }

            // reflink, nolink
            val refTok = tokenizer.reflink(src, links)
            if (refTok != null) {
                src = src.asJs().substring(refTok.raw.length)
                val lastToken = outTokens.lastOrNull()
                if (refTok is Token.Text && lastToken is Token.Text) {
                    lastToken.raw += refTok.raw
                    lastToken.text += refTok.text
                } else {
                    outTokens.push(refTok)
                }
                continue
            }

            // em & strong
            val emTok = tokenizer.emStrong(src, maskedSrc, prevChar)
            if (emTok != null) {
                src = src.asJs().substring(emTok.raw.length)
                outTokens.push(emTok)
                continue
            }

            // code
            val codeTok = tokenizer.codespan(src)
            if (codeTok != null) {
                src = src.asJs().substring(codeTok.raw.length)
                outTokens.push(codeTok)
                continue
            }

            // br
            val brTok = tokenizer.br(src)
            if (brTok != null) {
                src = src.asJs().substring(brTok.raw.length)
                outTokens.push(brTok)
                continue
            }

            // del (gfm)
            val delTok = tokenizer.del(src, maskedSrc, prevChar)
            if (delTok != null) {
                src = src.asJs().substring(delTok.raw.length)
                outTokens.push(delTok)
                continue
            }

            // autolink
            val autoTok = tokenizer.autolink(src)
            if (autoTok != null) {
                src = src.asJs().substring(autoTok.raw.length)
                outTokens.push(autoTok)
                continue
            }

            // url (gfm)
            if (!state.inLink) {
                val urlTok = tokenizer.url(src)
                if (urlTok != null) {
                    src = src.asJs().substring(urlTok.raw.length)
                    outTokens.push(urlTok)
                    continue
                }
            }

            // text
            val textTok = tokenizer.inlineText(src)
            if (textTok != null) {
                src = src.asJs().substring(textTok.raw.length)
                val lastChar = textTok.raw.asJs().slice(-1)
                if (lastChar != "_") {
                    prevChar = lastChar
                }
                keepPrevChar = true
                val lastToken = outTokens.lastOrNull()
                if (lastToken is Token.Text) {
                    lastToken.raw += textTok.raw
                    lastToken.text += textTok.text
                } else {
                    outTokens.push(textTok)
                }
                continue
            }

            if (src.length > 0) {
                infiniteLoopError(src.asJs().charCodeAt(0))
                break
            }
        }

        return outTokens
    }

    private fun infiniteLoopError(byte: Int) {
        throw RuntimeException("Infinite loop on byte: $byte")
    }
}
