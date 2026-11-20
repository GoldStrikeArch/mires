// Block and inline Lexer ported directly from marked 18.1.1 (src/Lexer.ts).

package marked

import scala.scalajs.js
import JsInterop.*

final class Lexer(val options: MarkedOptions):
  val tokens: js.Array[Token] = new js.Array[Token]()
  val links: LinkMap = JsObjectStatic.create(null)
  val inlineQueue: js.Array[InlineQueueEntry] = new js.Array[InlineQueueEntry]()
  val state: LexerState = new LexerState()
  val rules: RuleSet = Rules.select(options)
  val tokenizer: Tokenizer = new Tokenizer(options, rules, this)

  def lex(srcIn: String): js.Array[Token] =
    val src = srcIn.jsReplaceRe(Rules.carriageReturn, "\n")
    blockTokens(src, tokens, false)
    var i = 0
    while i < inlineQueue.length do
      val next = inlineQueue(i)
      inlineTokens(next.src, next.tokens)
      i += 1
    inlineQueue.length = 0
    tokens

  def blockTokens(
      srcIn: String,
      toks: js.Array[Token] = new js.Array[Token](),
      lastParagraphClippedIn: Boolean = false,
  ): js.Array[Token] =
    var src =
      if options.pedantic then
        srcIn.jsReplaceRe(Rules.tabCharGlobal, "    ").jsReplaceRe(Rules.spaceLine, "")
      else srcIn
    var lastParagraphClipped = lastParagraphClippedIn
    var srcLength = Int.MaxValue

    while src.length > 0 do
      if src.length < srcLength then srcLength = src.length
      else throw new RuntimeException("Infinite loop on byte: " + src.charAt(0).toInt)

      var matched = false

      // newline / space
      val sp = tokenizer.space(src)
      if sp != null then
        src = src.jsSlice(sp.raw.length)
        if sp.raw.length == 1 && toks.length > 0 then toks(toks.length - 1).raw += "\n"
        else toks.push(sp)
        matched = true

      // indented code
      if !matched then
        val cd = tokenizer.code(src)
        if cd != null then
          src = src.jsSlice(cd.raw.length)
          val lastTok: Token | Null = if toks.length > 0 then toks(toks.length - 1) else null
          lastTok match
            case pt: ParagraphToken =>
              pt.raw += (if pt.raw.jsEndsWith("\n") then "" else "\n") + cd.raw
              pt.text += "\n" + cd.text
              inlineQueue(inlineQueue.length - 1).src = pt.text
            case tt: TextToken =>
              tt.raw += (if tt.raw.jsEndsWith("\n") then "" else "\n") + cd.raw
              tt.text += "\n" + cd.text
              inlineQueue(inlineQueue.length - 1).src = tt.text
            case _ =>
              toks.push(cd)
          matched = true

      // fences
      if !matched then
        val fn = tokenizer.fences(src)
        if fn != null then
          src = src.jsSlice(fn.raw.length)
          toks.push(fn)
          matched = true

      // heading
      if !matched then
        val hd = tokenizer.heading(src)
        if hd != null then
          src = src.jsSlice(hd.raw.length)
          toks.push(hd)
          matched = true

      // hr
      if !matched then
        val hrTok = tokenizer.hr(src)
        if hrTok != null then
          src = src.jsSlice(hrTok.raw.length)
          toks.push(hrTok)
          matched = true

      // blockquote
      if !matched then
        val bq = tokenizer.blockquote(src)
        if bq != null then
          src = src.jsSlice(bq.raw.length)
          toks.push(bq)
          matched = true

      // list
      if !matched then
        val ls = tokenizer.list(src)
        if ls != null then
          src = src.jsSlice(ls.raw.length)
          toks.push(ls)
          matched = true

      // html
      if !matched then
        val ht = tokenizer.html(src)
        if ht != null then
          src = src.jsSlice(ht.raw.length)
          toks.push(ht)
          matched = true

      // def
      if !matched then
        val df = tokenizer.defToken(src)
        if df != null then
          src = src.jsSlice(df.raw.length)
          val lastTok: Token | Null = if toks.length > 0 then toks(toks.length - 1) else null
          lastTok match
            case pt: ParagraphToken =>
              pt.raw += (if pt.raw.jsEndsWith("\n") then "" else "\n") + df.raw
              pt.text += "\n" + df.raw
              inlineQueue(inlineQueue.length - 1).src = pt.text
            case tt: TextToken =>
              tt.raw += (if tt.raw.jsEndsWith("\n") then "" else "\n") + df.raw
              tt.text += "\n" + df.raw
              inlineQueue(inlineQueue.length - 1).src = tt.text
            case _ =>
              if js.isUndefined(links(df.tag)) then
                links(df.tag) = new LinkDef(df.href, df.title)
                toks.push(df)
          matched = true

      // table (gfm)
      if !matched then
        val tb = tokenizer.table(src)
        if tb != null then
          src = src.jsSlice(tb.raw.length)
          toks.push(tb)
          matched = true

      // lheading
      if !matched then
        val lh = tokenizer.lheading(src)
        if lh != null then
          src = src.jsSlice(lh.raw.length)
          toks.push(lh)
          matched = true

      // top-level paragraph
      if !matched && state.top then
        val pg = tokenizer.paragraph(src)
        if pg != null then
          val lastTok: Token | Null = if toks.length > 0 then toks(toks.length - 1) else null
          if lastParagraphClipped && lastTok.isInstanceOf[ParagraphToken] then
            val pt = lastTok.asInstanceOf[ParagraphToken]
            pt.raw += (if pt.raw.jsEndsWith("\n") then "" else "\n") + pg.raw
            pt.text += "\n" + pg.text
            inlineQueue.pop()
            inlineQueue(inlineQueue.length - 1).src = pt.text
          else toks.push(pg)
          lastParagraphClipped = false
          src = src.jsSlice(pg.raw.length)
          matched = true

      // text
      if !matched then
        val tx = tokenizer.text(src)
        if tx != null then
          src = src.jsSlice(tx.raw.length)
          val lastTok: Token | Null = if toks.length > 0 then toks(toks.length - 1) else null
          lastTok match
            case tt: TextToken =>
              tt.raw += (if tt.raw.jsEndsWith("\n") then "" else "\n") + tx.raw
              tt.text += "\n" + tx.text
              inlineQueue.pop()
              inlineQueue(inlineQueue.length - 1).src = tt.text
            case _ =>
              toks.push(tx)
          matched = true

      if !matched && src.length > 0 then
        throw new RuntimeException("Infinite loop on byte: " + src.charAt(0).toInt)

    state.top = true
    toks

  def inline(src: String, toks: js.Array[Token] = new js.Array[Token]()): js.Array[Token] =
    inlineQueue.push(new InlineQueueEntry(src, toks))
    toks

  private def linkInText(text: String): Boolean =
    if !text.jsIncludes("[") then return false

    val linkRule = rules.inline.link
    val blockSkipRe = rules.inline.blockSkip
    val savedBlockSkip = blockSkipRe.lastIndex
    blockSkipRe.lastIndex = 0
    try
      var m = blockSkipRe.exec(text)
      while m != null do
        val m0 = undefStr(m(0))
        if linkRule.test(m0) && (m.index == 0 || text.charAt(m.index - 1) != '!') then return true
        if m0.isEmpty then blockSkipRe.lastIndex += 1
        m = blockSkipRe.exec(text)
    finally blockSkipRe.lastIndex = savedBlockSkip

    val reflinkSearchRe = rules.inline.reflinkSearch
    val savedRefSearch = reflinkSearchRe.lastIndex
    reflinkSearchRe.lastIndex = 0
    try
      var m = reflinkSearchRe.exec(text)
      while m != null do
        val m0 = undefStr(m(0))
        val nextIdx = reflinkSearchRe.lastIndex
        val refStart = m0.lastIndexOf('[')
        if m0.charAt(0) != '!' && JsObjectStatic.hasOwn(links, Helpers.normalizeLabel(m0.jsSlice(refStart + 1, -1))) then
          val nestedHasLink =
            if refStart > 1 then
              val nested = linkInText(m0.jsSlice(1, refStart - 1))
              reflinkSearchRe.lastIndex = nextIdx
              nested
            else false
          if !nestedHasLink then return true
        if m0.isEmpty then reflinkSearchRe.lastIndex = nextIdx + 1
        m = reflinkSearchRe.exec(text)
    finally reflinkSearchRe.lastIndex = savedRefSearch

    false

  def inlineTokens(src: String, toks: js.Array[Token] = new js.Array[Token]()): js.Array[Token] =
    val prevLinkParenPossible = state.linkParenPossible
    state.linkParenPossible = prevLinkParenPossible && src.jsIncludes(")")
    try inlineTokensInner(src, toks)
    finally state.linkParenPossible = prevLinkParenPossible

  private def maskReflink(match0: String): String =
    val refStart = match0.lastIndexOf('[')
    if !JsObjectStatic.hasOwn(links, Helpers.normalizeLabel(match0.jsSlice(refStart + 1, -1))) then match0
    else if refStart > 1 && match0.charAt(0) != '!' then
      val text = match0.jsSlice(1, refStart - 1)
      if linkInText(text) then
        "[" + text.jsReplaceReFn1(rules.inline.reflinkSearch, maskReflinkFn) + "][" +
          "a".jsRepeat(match0.length - refStart - 2) + "]"
      else "[" + "a".jsRepeat(match0.length - 2) + "]"
    else "[" + "a".jsRepeat(match0.length - 2) + "]"

  private val maskReflinkFn: js.Function1[String, String] = (m0: String) => maskReflink(m0)

  private def inlineTokensInner(srcIn: String, toks: js.Array[Token]): js.Array[Token] =
    var src = srcIn
    var maskedSrc = src

    if src.jsIncludes("[") then
      maskedSrc = maskedSrc.jsReplaceReFn1(rules.inline.reflinkSearch, maskReflinkFn)

    maskedSrc = maskedSrc.jsReplaceReFn3(rules.inline.anyPunctuationOutsideCode, Lexer.maskPunctOutsideCode)
    maskedSrc = maskedSrc.jsReplaceReFn3(rules.inline.blockSkip, Lexer.maskBlockSkip)

    var keepPrevChar = false
    var prevChar = ""
    var srcLength = Int.MaxValue

    while src.length > 0 do
      if src.length < srcLength then srcLength = src.length
      else throw new RuntimeException("Infinite loop on byte: " + src.charAt(0).toInt)

      if !keepPrevChar then prevChar = ""
      keepPrevChar = false

      var matched = false

      // escape
      val esc = tokenizer.escape(src)
      if esc != null then
        src = src.jsSlice(esc.raw.length)
        toks.push(esc)
        matched = true

      // tag
      if !matched then
        val tg = tokenizer.tag(src)
        if tg != null then
          src = src.jsSlice(tg.raw.length)
          toks.push(tg)
          matched = true

      // link
      if !matched then
        val lnk = tokenizer.link(src)
        if lnk != null then
          src = src.jsSlice(lnk.raw.length)
          toks.push(lnk)
          matched = true

      // reflink / nolink
      if !matched then
        val ref = tokenizer.reflink(src, links)
        if ref != null then
          src = src.jsSlice(ref.raw.length)
          val lastTok: Token | Null = if toks.length > 0 then toks(toks.length - 1) else null
          (ref, lastTok) match
            case (rt: TextToken, lt: TextToken) =>
              lt.raw += rt.raw
              lt.text += rt.text
            case _ =>
              toks.push(ref)
          matched = true

      // em & strong
      if !matched then
        val em = tokenizer.emStrong(src, maskedSrc, prevChar)
        if em != null then
          src = src.jsSlice(em.raw.length)
          toks.push(em)
          matched = true

      // codespan
      if !matched then
        val cs = tokenizer.codespan(src)
        if cs != null then
          src = src.jsSlice(cs.raw.length)
          toks.push(cs)
          matched = true

      // br
      if !matched then
        val brTok = tokenizer.br(src)
        if brTok != null then
          src = src.jsSlice(brTok.raw.length)
          toks.push(brTok)
          matched = true

      // del (gfm)
      if !matched then
        val dl = tokenizer.del(src, maskedSrc, prevChar)
        if dl != null then
          src = src.jsSlice(dl.raw.length)
          toks.push(dl)
          matched = true

      // autolink
      if !matched then
        val auto = tokenizer.autolink(src)
        if auto != null then
          src = src.jsSlice(auto.raw.length)
          toks.push(auto)
          matched = true

      // url (gfm)
      if !matched && !state.inLink then
        val u = tokenizer.url(src)
        if u != null then
          src = src.jsSlice(u.raw.length)
          toks.push(u)
          matched = true

      // inlineText
      if !matched then
        val it = tokenizer.inlineText(src)
        if it != null then
          src = src.jsSlice(it.raw.length)
          val lastChar = it.raw.jsSlice(-1)
          if lastChar != "_" then prevChar = lastChar
          keepPrevChar = true
          val lastTok: Token | Null = if toks.length > 0 then toks(toks.length - 1) else null
          lastTok match
            case lt: TextToken =>
              lt.raw += it.raw
              lt.text += it.text
            case _ =>
              toks.push(it)
          matched = true

      if !matched && src.length > 0 then
        throw new RuntimeException("Infinite loop on byte: " + src.charAt(0).toInt)

    toks

object Lexer:
  private val maskPunctOutsideCode: js.Function3[String, js.UndefOr[String], js.UndefOr[String], String] =
    (match0: String, code: js.UndefOr[String], punct: js.UndefOr[String]) =>
      if isTruthy(punct) then "+".jsRepeat(match0.length)
      else if isTruthy(code) then
        val c = code.asInstanceOf[String]
        c + match0.jsSlice(c.length, -c.length).jsReplaceRe(Rules.backtickGlobal, "+") + c
      else match0

  private val maskBlockSkip: js.Function3[String, js.UndefOr[String], js.UndefOr[String], String] =
    (match0: String, _link: js.UndefOr[String], context: js.UndefOr[String]) =>
      val offset = if isTruthy(context) then context.asInstanceOf[String].length else 0
      match0.jsSlice(0, offset) + "[" + "a".jsRepeat(match0.length - offset - 2) + "]"

  def lex(src: String, options: MarkedOptions): js.Array[Token] =
    new Lexer(options).lex(src)

  def lexInline(src: String, options: MarkedOptions): js.Array[Token] =
    new Lexer(options).inlineTokens(src)
