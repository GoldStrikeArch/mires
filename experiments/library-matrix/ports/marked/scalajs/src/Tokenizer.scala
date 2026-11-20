// Block and inline Tokenizer ported directly from marked 18.1.1 (src/Tokenizer.ts).

package marked

import scala.scalajs.js
import JsInterop.*

final class Tokenizer(val options: MarkedOptions, val rules: RuleSet, val lexer: Lexer):
  private def outputLink(
      cap0: String,
      cap1: String,
      href: String,
      titleIn: String | Null,
      raw: String,
  ): Token | Null =
    val title: String | Null =
      if titleIn != null && titleIn.asInstanceOf[String].length > 0 then titleIn else null
    val text = cap1.jsReplaceRe(Rules.outputLinkReplace, "$1")
    val isImage = cap0.length > 0 && cap0.charAt(0) == '!'

    val outerInLink = lexer.state.inLink
    val outerLinkEmitted = lexer.state.linkEmitted
    val outerInRawBlock = lexer.state.inRawBlock
    lexer.state.inLink = true
    lexer.state.linkEmitted = false
    val tokens = lexer.inlineTokens(text)
    val textHasLink = lexer.state.linkEmitted
    lexer.state.linkEmitted = outerLinkEmitted
    lexer.state.inLink = outerInLink

    if !isImage then
      if textHasLink then
        lexer.state.inRawBlock = outerInRawBlock
        return null
      lexer.state.linkEmitted = true

    if isImage then new ImageToken(raw, href, title, text, tokens)
    else new LinkToken(raw, href, title, text, tokens, false)

  private def indentCodeCompensation(raw: String, text: String): String =
    val matchIndentToCode = Rules.indentCodeCompensation.exec(raw)
    if matchIndentToCode == null then text
    else
      val indentToCodeLen = undefStr(matchIndentToCode(1)).length
      val lines = text.jsSplit("\n")
      var i = 0
      val len = lines.length
      while i < len do
        val node = lines(i)
        val matchIndentInNode = Rules.beginningSpace.exec(node)
        if matchIndentInNode != null then
          val indentInNodeLen = undefStr(matchIndentInNode(0)).length
          lines(i) = node.jsSlice(if indentInNodeLen < indentToCodeLen then indentInNodeLen else indentToCodeLen)
        i += 1
      lines.join("\n")

  private def isLabelEndInsideToken(src: String, label: String, labelStart: Int): Boolean =
    if !label.jsIncludes("<") then false
    else
      val len = label.length
      var i = 0
      while i < len do
        val ch = label.charAt(i)
        if ch == '\\' then i += 2
        else if ch == '`' then
          val code = rules.inline.code.exec(label.jsSlice(i))
          if code != null then i += undefStr(code(0)).length
          else i += 1
        else if ch != '<' then i += 1
        else
          val tokenSrc = src.jsSlice(labelStart + i)
          var tok = rules.inline.tag.exec(tokenSrc)
          if tok == null then tok = rules.inline.autolink.exec(tokenSrc)
          if tok == null then i += 1
          else
            val tokLen = undefStr(tok(0)).length
            if tokLen > len - i then return true
            i += tokLen
      false

  def space(src: String): SpaceToken | Null =
    val cap = rules.block.newline.exec(src)
    if cap != null then
      val raw = undefStr(cap(0))
      if raw.length > 0 then new SpaceToken(raw) else null
    else null

  def code(src: String): CodeToken | Null =
    val cap = rules.block.code.exec(src)
    if cap != null then
      val cap0 = undefStr(cap(0))
      val raw = if options.pedantic then cap0 else Helpers.trimTrailingBlankLines(cap0)
      val text = raw.jsReplaceRe(Rules.codeRemoveIndent, "")
      new CodeToken(raw, text, null, false)
    else null

  def fences(src: String): CodeToken | Null =
    val fencesRe = rules.block.fences
    if fencesRe == null then return null
    val cap = fencesRe.exec(src)
    if cap != null then
      val raw = undefStr(cap(0))
      val body = undefStr(cap(3))
      val text = indentCodeCompensation(raw, body)
      val langRaw = cap(2)
      val lang: String | Null =
        if isTruthy(langRaw) then
          langRaw.asInstanceOf[String].jsTrim().jsReplaceRe(rules.inline.anyPunctuation, "$1")
        else undefStrOrNull(langRaw)
      new CodeToken(raw, text, lang, false)
    else null

  def heading(src: String): HeadingToken | Null =
    val cap = rules.block.heading.exec(src)
    if cap != null then
      var text = undefStr(cap(2)).jsTrim()
      if Rules.endingHash.test(text) then
        val trimmed = Helpers.rtrim(text, '#')
        if options.pedantic then text = trimmed.jsTrim()
        else if trimmed.isEmpty || Rules.endingSpaceTabChar.test(trimmed) then text = trimmed.jsTrim()
      new HeadingToken(
        Helpers.rtrim(undefStr(cap(0)), '\n'),
        undefStr(cap(1)).length,
        text,
        lexer.inline(text),
      )
    else null

  def hr(src: String): HrToken | Null =
    val cap = rules.block.hr.exec(src)
    if cap != null then new HrToken(Helpers.rtrim(undefStr(cap(0)), '\n'))
    else null

  def blockquote(src: String): BlockquoteToken | Null =
    val cap = rules.block.blockquote.exec(src)
    if cap == null then return null

    var lines = Helpers.rtrim(undefStr(cap(0)), '\n').jsSplit("\n")
    var raw = ""
    var text = ""
    val tokens = new js.Array[Token]()
    var done = false

    while lines.length > 0 && !done do
      var inBlockquote = false
      val currentLines = new js.Array[String]()
      var i = 0
      val lineCount = lines.length
      var scanDone = false
      while i < lineCount && !scanDone do
        val ln = lines(i)
        if Rules.blockquoteStart.test(ln) then
          currentLines.push(ln)
          inBlockquote = true
          i += 1
        else if !inBlockquote then
          currentLines.push(ln)
          i += 1
        else scanDone = true
      lines = lines.arrSlice(i)

      val currentRaw = currentLines.join("\n")
      val currentText = currentRaw
        .jsReplaceRe(Rules.blockquoteSetextReplace, "\n    $1")
        .jsReplaceRe(Rules.blockquoteSetextReplace2, "")
      raw = if raw.length > 0 then raw + "\n" + currentRaw else currentRaw
      text = if text.length > 0 then text + "\n" + currentText else currentText

      val top = lexer.state.top
      lexer.state.top = true
      lexer.blockTokens(currentText, tokens, true)
      lexer.state.top = top

      if lines.length == 0 then done = true
      else
        val lastToken: Token | Null = if tokens.length > 0 then tokens(tokens.length - 1) else null
        lastToken match
          case _: CodeToken | _: HeadingToken | _: HrToken | _: HtmlToken =>
            done = true
          case oldToken: BlockquoteToken =>
            val continuation = lines.join("\n")
            val newText = oldToken.raw + "\n" + continuation.jsReplaceRe(Rules.blockquoteSetextReplace2, "")
            val newToken = this.blockquote(newText).asInstanceOf[BlockquoteToken]
            tokens(tokens.length - 1) = newToken
            val leftover = newText.jsSlice(newToken.raw.length).jsReplaceRe(Rules.leadingNewline, "")
            val leftoverCount = if leftover.length > 0 then leftover.jsSplit("\n").length else 0
            val consumed = if leftoverCount > 0 then lines.arrSlice(0, -leftoverCount) else lines
            if consumed.length > 0 then raw = raw + "\n" + consumed.join("\n")
            text = text.jsSlice(0, text.length - oldToken.text.length) + newToken.text
            done = true
          case oldToken: ListToken =>
            val newText = oldToken.raw + "\n" + lines.join("\n")
            val newToken = this.list(newText).asInstanceOf[ListToken]
            tokens(tokens.length - 1) = newToken
            raw = raw.jsSlice(0, raw.length - oldToken.raw.length) + newToken.raw
            text = text.jsSlice(0, text.length - oldToken.raw.length) + newToken.raw
            lines = newText.jsSlice(newToken.raw.length).jsSplit("\n")
          case _ => ()

    new BlockquoteToken(raw, tokens, text)

  def list(srcIn: String): ListToken | Null =
    var cap = rules.block.list.exec(srcIn)
    if cap == null then return null

    val listSrc = srcIn
    var src = srcIn
    val rawBull = undefStr(cap(1)).jsTrim()
    val isOrdered = rawBull.length > 1
    val startNum = if isOrdered then parseDec(rawBull.jsSlice(0, -1)) else 0
    val listTok = new ListToken("", isOrdered, startNum, false, new js.Array[ListItemToken]())

    var bull =
      if isOrdered then "\\d{1,9}\\" + rawBull.jsSlice(-1)
      else "\\" + rawBull
    if options.pedantic && !isOrdered then bull = "[*+-]"

    val itemRegex = Rules.listItemRegex(bull)
    var endsWithBlankLine = false
    var itemsDone = false

    while src.length > 0 && !itemsDone do
      cap = itemRegex.exec(src)
      if cap == null || rules.block.hr.test(src) then itemsDone = true
      else
        var endEarly = false
        var itemContents = ""
        val itemStart = listSrc.length - src.length
        val cap0 = undefStr(cap(0))
        val cap1 = undefStr(cap(1))
        val cap2 = undefStr(cap(2))
        src = src.jsSlice(cap0.length)

        val firstLine = firstLineOf(cap2)
        val bulletIndent = cap1.length
        var line =
          if options.pedantic then Helpers.expandTabs(firstLine, bulletIndent)
          else
            firstLine.jsReplaceReFn1(
              Rules.leadingSpaceTab,
              (ws: String) => Helpers.expandTabs(ws, bulletIndent),
            )
        var nextLine = firstLineOf(src)
        var blankLine = line.jsTrim().isEmpty

        var indent = 0
        if options.pedantic then
          indent = 2
          itemContents = line.jsTrimStart()
        else if blankLine then indent = bulletIndent + 1
        else
          indent = line.jsSearch(Rules.nonSpaceChar)
          if indent > 4 then indent = 1
          itemContents = line.jsSlice(indent)
          indent += bulletIndent

        if blankLine && Rules.blankLine.test(nextLine) then
          src = src.jsSlice(nextLine.length + 1)
          endEarly = true

        if !endEarly then
          val nextBulletRegex = Rules.nextBulletRegex(indent)
          val hrRegex = Rules.hrRegex(indent)
          val fencesBeginRegex = Rules.fencesBeginRegex(indent)
          val headingBeginRegex = Rules.headingBeginRegex(indent)
          val htmlBeginRegex = Rules.htmlBeginRegex(indent)
          val blockquoteBeginRegex = Rules.blockquoteBeginRegex(indent)

          var linesDone = false
          while src.length > 0 && !linesDone do
            val rawLine = firstLineOf(src)
            nextLine = rawLine
            val nextLineWithoutTabs =
              if options.pedantic then
                nextLine = nextLine.jsReplaceRe(Rules.listReplaceNesting, "  ")
                nextLine
              else
                nextLine.jsReplaceReFn1(
                  Rules.leadingSpaceTab,
                  (ws: String) => Helpers.expandTabs(ws, 0),
                )

            if fencesBeginRegex.test(nextLine)
              || headingBeginRegex.test(nextLine)
              || htmlBeginRegex.test(nextLine)
              || blockquoteBeginRegex.test(nextLine)
              || nextBulletRegex.test(nextLine)
              || hrRegex.test(nextLine)
            then linesDone = true
            else
              if nextLineWithoutTabs.jsSearch(Rules.nonSpaceChar) >= indent || nextLine.jsTrim().isEmpty then
                itemContents += "\n" + nextLineWithoutTabs.jsSlice(indent)
              else if blankLine
                || line.jsReplaceRe(Rules.tabCharGlobal, "    ").jsSearch(Rules.nonSpaceChar) >= 4
                || fencesBeginRegex.test(line)
                || headingBeginRegex.test(line)
                || hrRegex.test(line)
              then linesDone = true
              else itemContents += "\n" + nextLine

              if !linesDone then
                blankLine = nextLine.jsTrim().isEmpty
                src = src.jsSlice(rawLine.length + 1)
                line = nextLineWithoutTabs.jsSlice(indent)

        val raw = listSrc.jsSlice(itemStart, listSrc.length - src.length)
        if !listTok.loose then
          if endsWithBlankLine then listTok.loose = true
          else if Rules.doubleBlankLine.test(raw) then endsWithBlankLine = true

        listTok.items.push(
          new ListItemToken(
            raw = raw,
            task = options.gfm && Rules.listIsTask.test(itemContents),
            checked = false,
            loose = false,
            text = itemContents,
            tokens = new js.Array[Token](),
          ),
        )

    if listTok.items.length == 0 then return null

    val lastItem = listTok.items(listTok.items.length - 1)
    lastItem.raw = lastItem.raw.jsTrimEnd()
    lastItem.text = lastItem.text.jsTrimEnd()
    listTok.raw = listSrc.jsSlice(0, listSrc.length - src.length).jsTrimEnd()

    val itemCount = listTok.items.length
    var idx = 0
    while idx < itemCount do
      val item = listTok.items(idx)
      lexer.state.top = false
      item.tokens = lexer.blockTokens(item.text, new js.Array[Token]())
      if !listTok.loose then
        var j = 0
        val tLen = item.tokens.length
        while j < tLen && !listTok.loose do
          item.tokens(j) match
            case sp: SpaceToken if Rules.anyLine.test(sp.raw) =>
              listTok.loose = true
            case _ => ()
          j += 1
      idx += 1

    idx = 0
    while idx < itemCount do
      val item = listTok.items(idx)
      val firstTok: Token | Null = if item.tokens.length > 0 then item.tokens(0) else null
      val isTextOrPara = firstTok.isInstanceOf[TextToken] || firstTok.isInstanceOf[ParagraphToken]
      if item.task && isTextOrPara then
        item.text = item.text.jsReplaceRe(Rules.listReplaceTask, "")
        firstTok match
          case pt: ParagraphToken =>
            pt.raw = pt.raw.jsReplaceRe(Rules.listReplaceTask, "")
            pt.text = pt.text.jsReplaceRe(Rules.listReplaceTask, "")
          case tt: TextToken =>
            tt.raw = tt.raw.jsReplaceRe(Rules.listReplaceTask, "")
            tt.text = tt.text.jsReplaceRe(Rules.listReplaceTask, "")
          case _ => ()

        var q = lexer.inlineQueue.length - 1
        var qFound = false
        while q >= 0 && !qFound do
          val entry = lexer.inlineQueue(q)
          if Rules.listIsTask.test(entry.src) then
            entry.src = entry.src.jsReplaceRe(Rules.listReplaceTask, "")
            qFound = true
          q -= 1

        val taskRaw = Rules.listTaskCheckbox.exec(item.raw)
        if taskRaw != null then
          val match0 = undefStr(taskRaw(0))
          val cbRaw = match0 + " "
          val cbChecked = match0 != "[ ]"
          val checkboxToken = new CheckboxToken(cbRaw, cbChecked)
          item.checked = cbChecked
          if listTok.loose then
            firstTok match
              case pt: ParagraphToken =>
                pt.raw = cbRaw + pt.raw
                pt.text = cbRaw + pt.text
                pt.tokens.unshift(checkboxToken)
              case tt: TextToken if tt.tokens != null =>
                tt.raw = cbRaw + tt.raw
                tt.text = cbRaw + tt.text
                tt.tokens.asInstanceOf[js.Array[Token]].unshift(checkboxToken)
              case _ =>
                val arr = new js.Array[Token]()
                arr.push(checkboxToken)
                item.tokens.unshift(new ParagraphToken(cbRaw, cbRaw, arr))
          else item.tokens.unshift(checkboxToken)
      else if item.task then item.task = false
      idx += 1

    if listTok.loose then
      idx = 0
      while idx < itemCount do
        val item = listTok.items(idx)
        item.loose = true
        var j = 0
        val tLen = item.tokens.length
        while j < tLen do
          item.tokens(j) match
            case tt: TextToken =>
              val childTokens =
                if tt.tokens != null then tt.tokens.asInstanceOf[js.Array[Token]]
                else new js.Array[Token]()
              item.tokens(j) = new ParagraphToken(tt.raw, tt.text, childTokens)
            case _ => ()
          j += 1
        idx += 1

    listTok

  def html(src: String): HtmlToken | Null =
    val cap = rules.block.html.exec(src)
    if cap != null then
      val raw = Helpers.trimTrailingBlankLines(undefStr(cap(0)))
      val tag1 = undefStr(cap(1))
      val pre = tag1 == "pre" || tag1 == "script" || tag1 == "style"
      new HtmlToken(raw, raw, true, pre)
    else null

  def defToken(src: String): DefToken | Null =
    val cap = rules.block.defRule.exec(src)
    if cap != null then
      val cap2 = undefStr(cap(2))
      if !Rules.startAngleBracket.test(cap2) && Helpers.findClosingBracket(cap2, '(', ')') != -1 then
        return null
      val tag = Helpers.normalizeLabel(undefStr(cap(1))).jsReplaceRe(Rules.multipleSpaceGlobal, " ")
      val href =
        if cap2.length > 0 then
          cap2.jsReplaceRe(Rules.hrefBrackets, "$1").jsReplaceRe(rules.inline.anyPunctuation, "$1")
        else ""
      val cap3 = cap(3)
      val title: String | Null =
        if isTruthy(cap3) then
          val s = cap3.asInstanceOf[String]
          s.substring(1, s.length - 1).jsReplaceRe(rules.inline.anyPunctuation, "$1")
        else undefStrOrNull(cap3)
      new DefToken(Helpers.rtrim(undefStr(cap(0)), '\n'), tag, href, title)
    else null

  def table(src: String): TableToken | Null =
    val tableRe = rules.block.table
    if tableRe == null then return null
    val cap = tableRe.exec(src)
    if cap == null then return null
    val cap2 = undefStr(cap(2))
    if !Rules.tableDelimiter.test(cap2) then return null

    val headers = Helpers.splitCells(undefStr(cap(1)), 0)
    val aligns = cap2.jsReplaceRe(Rules.tableAlignChars, "").jsSplit("|")
    if headers.length != aligns.length then return null

    val cap3 = undefStr(cap(3))
    val rows =
      if cap3.jsTrim().length > 0 then cap3.jsReplaceRe(Rules.tableRowBlankLine, "").jsSplit("\n")
      else new js.Array[String]()

    val alignArr = new js.Array[String | Null]()
    var i = 0
    val colCount = aligns.length
    while i < colCount do
      val a = aligns(i)
      if Rules.tableAlignRight.test(a) then alignArr.push("right")
      else if Rules.tableAlignCenter.test(a) then alignArr.push("center")
      else if Rules.tableAlignLeft.test(a) then alignArr.push("left")
      else alignArr.push(null)
      i += 1

    val headerArr = new js.Array[TableCell]()
    i = 0
    while i < colCount do
      val h = headers(i)
      headerArr.push(new TableCell(h, lexer.inline(h), true, alignArr(i)))
      i += 1

    val rowArr = new js.Array[js.Array[TableCell]]()
    var r = 0
    val rowCount = rows.length
    while r < rowCount do
      val cells = Helpers.splitCells(rows(r), colCount)
      val parsedRow = new js.Array[TableCell]()
      var c = 0
      while c < colCount do
        val cell = cells(c)
        parsedRow.push(new TableCell(cell, lexer.inline(cell), false, alignArr(c)))
        c += 1
      rowArr.push(parsedRow)
      r += 1

    new TableToken(Helpers.rtrim(undefStr(cap(0)), '\n'), headerArr, alignArr, rowArr)

  def lheading(src: String): HeadingToken | Null =
    val cap = rules.block.lheading.exec(src)
    if cap != null then
      val text = undefStr(cap(1)).jsTrim()
      val depth = if undefStr(cap(2)).charAt(0) == '=' then 1 else 2
      new HeadingToken(Helpers.rtrim(undefStr(cap(0)), '\n'), depth, text, lexer.inline(text))
    else null

  def paragraph(src: String): ParagraphToken | Null =
    val cap = rules.block.paragraph.exec(src)
    if cap != null then
      val cap1 = undefStr(cap(1))
      val text =
        if cap1.length > 0 && cap1.charAt(cap1.length - 1) == '\n' then cap1.jsSlice(0, -1)
        else cap1
      new ParagraphToken(undefStr(cap(0)), text, lexer.inline(text))
    else null

  def text(src: String): TextToken | Null =
    val cap = rules.block.text.exec(src)
    if cap != null then
      val raw = undefStr(cap(0))
      new TextToken(raw, raw, lexer.inline(raw), false)
    else null

  def escape(src: String): EscapeToken | Null =
    val cap = rules.inline.escape.exec(src)
    if cap != null then new EscapeToken(undefStr(cap(0)), undefStr(cap(1)))
    else null

  def tag(src: String): HtmlToken | Null =
    val cap = rules.inline.tag.exec(src)
    if cap != null then
      val cap0 = undefStr(cap(0))
      if !lexer.state.inLink && Rules.startATag.test(cap0) then lexer.state.inLink = true
      else if lexer.state.inLink && Rules.endATag.test(cap0) then lexer.state.inLink = false

      if !lexer.state.inRawBlock && Rules.startPreScriptTag.test(cap0) then lexer.state.inRawBlock = true
      else if lexer.state.inRawBlock && Rules.endPreScriptTag.test(cap0) then lexer.state.inRawBlock = false

      new HtmlToken(cap0, cap0, false, false)
    else null

  def link(src: String): Token | Null =
    if !lexer.state.linkParenPossible then return null
    val cap = rules.inline.link.exec(src)
    if cap == null then return null

    var cap0 = undefStr(cap(0))
    val cap1 = undefStr(cap(1))
    var cap2 = undefStr(cap(2))
    var cap3 = undefStr(cap(3))

    val labelStart = if cap0.length > 0 && cap0.charAt(0) == '!' then 2 else 1
    if !options.pedantic && isLabelEndInsideToken(src, cap1, labelStart) then return null

    val trimmedUrl = cap2.jsTrim()
    if !options.pedantic && Rules.startAngleBracket.test(trimmedUrl) then
      if !Rules.endAngleBracket.test(trimmedUrl) then return null
      val rtrimSlash = Helpers.rtrim(trimmedUrl.jsSlice(0, -1), '\\')
      if (trimmedUrl.length - rtrimSlash.length) % 2 == 0 then return null
    else
      val lastParenIndex = Helpers.findClosingBracket(cap2, '(', ')')
      if lastParenIndex == -2 then return null
      if lastParenIndex > -1 then
        val start = if cap0.length > 0 && cap0.charAt(0) == '!' then 5 else 4
        val linkLen = start + cap1.length + lastParenIndex
        cap2 = cap2.jsSlice(0, lastParenIndex)
        cap0 = cap0.jsSlice(0, linkLen).jsTrim()
        cap3 = ""

    var href = cap2
    var title = ""
    if options.pedantic then
      val linkCap = Rules.pedanticHrefTitle.exec(href)
      if linkCap != null then
        href = undefStr(linkCap(1))
        title = undefStr(linkCap(3))
    else title = if cap3.length > 0 then cap3.jsSlice(1, -1) else ""

    href = href.jsTrim()
    if Rules.startAngleBracket.test(href) then
      if options.pedantic && !Rules.endAngleBracket.test(trimmedUrl) then href = href.jsSlice(1)
      else href = href.jsSlice(1, -1)

    val finalHref = if href.length > 0 then href.jsReplaceRe(rules.inline.anyPunctuation, "$1") else href
    val finalTitle = if title.length > 0 then title.jsReplaceRe(rules.inline.anyPunctuation, "$1") else title
    outputLink(cap0, cap1, finalHref, finalTitle, cap0)

  def reflink(src: String, links: LinkMap): Token | Null =
    var cap = rules.inline.reflink.exec(src)
    if cap == null then cap = rules.inline.nolink.exec(src)
    if cap == null then return null

    val cap0 = undefStr(cap(0))
    val cap1 = undefStr(cap(1))
    val cap2 = undefStr(cap(2))
    val labelStart = if cap0.length > 0 && cap0.charAt(0) == '!' then 2 else 1
    if !options.pedantic && isLabelEndInsideToken(src, cap1, labelStart) then return null

    val linkKeyRaw = if cap2.length > 0 then cap2 else cap1
    val linkString = linkKeyRaw.jsReplaceRe(Rules.multipleSpaceGlobal, " ")
    val normKey = Helpers.normalizeLabel(linkString)
    val linkOpt = links(normKey)
    if js.isUndefined(linkOpt) then
      val ch = cap0.substring(0, 1)
      new TextToken(ch, ch, null, false)
    else
      val link = linkOpt.asInstanceOf[LinkDef]
      outputLink(cap0, cap1, link.href, link.title, cap0)

  def emStrong(src: String, maskedSrc: String, prevChar: String): Token | Null =
    var m = rules.inline.emStrongLDelim.exec(src)
    if m == null then return null
    val m1 = isTruthy(m(1))
    val m2 = isTruthy(m(2))
    val m3 = isTruthy(m(3))
    val m4 = isTruthy(m(4))
    if !m1 && !m2 && !m3 && !m4 then return null
    if m4 && Rules.unicodeAlphaNumeric.test(prevChar) then return null

    val hasNextPunct = m1 || m3
    if !hasNextPunct || prevChar.isEmpty || rules.inline.punctuation.test(prevChar) then
      val m0 = undefStr(m(0))
      val lLength = codePointLength(m0) - 1
      var delimTotal = lLength
      var midDelimTotal = 0
      val delimChar = m0.charAt(0)
      val midRun = prevChar.length == 1 && prevChar.charAt(0) == delimChar
      val endReg = if delimChar == '*' then rules.inline.emStrongRDelimAst else rules.inline.emStrongRDelimUnd
      endReg.lastIndex = 0
      val clippedMask = maskedSrc.jsSlice(-src.length + lLength)

      var stop = false
      while !stop do
        m = endReg.exec(clippedMask)
        if m == null then stop = true
        else
          val rDelim =
            if isTruthy(m(1)) then m(1).asInstanceOf[String]
            else if isTruthy(m(2)) then m(2).asInstanceOf[String]
            else if isTruthy(m(3)) then m(3).asInstanceOf[String]
            else if isTruthy(m(4)) then m(4).asInstanceOf[String]
            else if isTruthy(m(5)) then m(5).asInstanceOf[String]
            else if isTruthy(m(6)) then m(6).asInstanceOf[String]
            else ""
          if rDelim.length > 0 then
            var rLength = codePointLength(rDelim)
            if isTruthy(m(3)) || isTruthy(m(4)) then delimTotal += rLength
            else if (isTruthy(m(5)) || isTruthy(m(6))) && (lLength % 3 != 0) && ((lLength + rLength) % 3 == 0) then
              midDelimTotal += rLength
            else if (isTruthy(m(5)) || isTruthy(m(6))) && midRun then stop = true
            else
              delimTotal -= rLength
              if delimTotal <= 0 then
                val trimmedR = rLength + delimTotal + midDelimTotal
                if trimmedR < rLength then rLength = trimmedR
                val lastCharLength = firstCodePointUtf16Length(undefStr(m(0)))
                val raw = src.jsSlice(0, lLength + m.index + lastCharLength + rLength)
                val minLen = if lLength < rLength then lLength else rLength
                if minLen % 2 != 0 then
                  val text = raw.jsSlice(1, -1)
                  return new EmToken(raw, text, lexer.inlineTokens(text))
                else
                  val text = raw.jsSlice(2, -2)
                  return new StrongToken(raw, text, lexer.inlineTokens(text))
    null

  def codespan(src: String): CodespanToken | Null =
    val cap = rules.inline.code.exec(src)
    if cap != null then
      var text = undefStr(cap(2)).jsReplaceRe(Rules.newLineCharGlobal, " ")
      val hasNonSpaceChars = Rules.nonSpaceChar.test(text)
      val hasSpaceCharsOnBothEnds = Rules.startingSpaceChar.test(text) && Rules.endingSpaceChar.test(text)
      if hasNonSpaceChars && hasSpaceCharsOnBothEnds then text = text.substring(1, text.length - 1)
      new CodespanToken(undefStr(cap(0)), text)
    else null

  def br(src: String): BrToken | Null =
    val cap = rules.inline.br.exec(src)
    if cap != null then new BrToken(undefStr(cap(0)))
    else null

  def del(src: String, maskedSrc: String, prevChar: String): DelToken | Null =
    val lDelimRe = rules.inline.delLDelim
    if lDelimRe == null then return null
    var m = lDelimRe.exec(src)
    if m == null then return null

    val hasNextPunct = isTruthy(m(1))
    if !hasNextPunct || prevChar.isEmpty || rules.inline.punctuation.test(prevChar) then
      val lLength = codePointLength(undefStr(m(0))) - 1
      var delimTotal = lLength
      val endReg = rules.inline.delRDelim.asInstanceOf[js.RegExp]
      endReg.lastIndex = 0
      val clippedMask = maskedSrc.jsSlice(-src.length + lLength)

      var stop = false
      while !stop do
        m = endReg.exec(clippedMask)
        if m == null then stop = true
        else
          val rDelim =
            if isTruthy(m(1)) then m(1).asInstanceOf[String]
            else if isTruthy(m(2)) then m(2).asInstanceOf[String]
            else if isTruthy(m(3)) then m(3).asInstanceOf[String]
            else if isTruthy(m(4)) then m(4).asInstanceOf[String]
            else if isTruthy(m(5)) then m(5).asInstanceOf[String]
            else if isTruthy(m(6)) then m(6).asInstanceOf[String]
            else ""
          if rDelim.length > 0 then
            var rLength = codePointLength(rDelim)
            if rLength == lLength then
              if isTruthy(m(3)) || isTruthy(m(4)) then delimTotal += rLength
              else
                delimTotal -= rLength
                if delimTotal <= 0 then
                  val trimmedR = rLength + delimTotal
                  if trimmedR < rLength then rLength = trimmedR
                  val lastCharLength = firstCodePointUtf16Length(undefStr(m(0)))
                  val raw = src.jsSlice(0, lLength + m.index + lastCharLength + rLength)
                  val text = raw.jsSlice(lLength, -lLength)
                  return new DelToken(raw, text, lexer.inlineTokens(text))
    null

  def autolink(src: String): LinkToken | Null =
    val cap = rules.inline.autolink.exec(src)
    if cap != null then
      val text = undefStr(cap(1))
      val href = if undefStr(cap(2)) == "@" then "mailto:" + text else text
      val inner = new js.Array[Token]()
      inner.push(new TextToken(text, text, null, false))
      new LinkToken(undefStr(cap(0)), href, null, text, inner, true)
    else null

  def url(src: String): LinkToken | Null =
    val urlRe = rules.inline.url
    if urlRe == null then return null
    val cap = urlRe.exec(src)
    if cap == null then return null

    var cap0 = undefStr(cap(0))
    val text: String =
      if undefStr(cap(2)) == "@" then cap0
      else
        val backpedalRe = rules.inline.backpedal
        var done = false
        while !done do
          val prevCapZero = cap0
          val bp = if backpedalRe != null then backpedalRe.exec(cap0) else null
          cap0 = if bp != null then undefStr(bp(0)) else ""
          if prevCapZero == cap0 then done = true
        cap0
    val href =
      if undefStr(cap(2)) == "@" then "mailto:" + text
      else if undefStr(cap(1)) == "www." then "http://" + text
      else text
    val inner = new js.Array[Token]()
    inner.push(new TextToken(text, text, null, false))
    new LinkToken(cap0, href, null, text, inner, true)

  def inlineText(src: String): TextToken | Null =
    val cap = rules.inline.text.exec(src)
    if cap != null then
      val raw = undefStr(cap(0))
      val escaped = lexer.state.inRawBlock
      val text = if escaped then raw else Helpers.decodeNumericCharacterReferences(raw)
      new TextToken(raw, text, null, escaped)
    else null
