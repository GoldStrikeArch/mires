// Exact block, inline, and helper RegExp grammars of marked 18.1.1 (src/rules.ts).
// Uses host js.RegExp natively so Unicode \p{...} property escapes, lookbehind, and
// named backreferences run directly on the JS engine under Scala.js's ES2015 link target.

package marked

import scala.scalajs.js
import JsInterop.*

final class BlockRules(
    val blockquote: js.RegExp,
    val code: js.RegExp,
    val defRule: js.RegExp,
    val fences: js.RegExp | Null,
    val heading: js.RegExp,
    val hr: js.RegExp,
    val html: js.RegExp,
    val lheading: js.RegExp,
    val list: js.RegExp,
    val newline: js.RegExp,
    val paragraph: js.RegExp,
    val table: js.RegExp | Null,
    val text: js.RegExp,
)

final class InlineRules(
    val backpedal: js.RegExp | Null,
    val anyPunctuation: js.RegExp,
    val anyPunctuationOutsideCode: js.RegExp,
    val autolink: js.RegExp,
    val blockSkip: js.RegExp,
    val br: js.RegExp,
    val code: js.RegExp,
    val del: js.RegExp | Null,
    val delLDelim: js.RegExp | Null,
    val delRDelim: js.RegExp | Null,
    val emStrongLDelim: js.RegExp,
    val emStrongRDelimAst: js.RegExp,
    val emStrongRDelimUnd: js.RegExp,
    val escape: js.RegExp,
    val link: js.RegExp,
    val nolink: js.RegExp,
    val punctuation: js.RegExp,
    val reflink: js.RegExp,
    val reflinkSearch: js.RegExp,
    val tag: js.RegExp,
    val text: js.RegExp,
    val url: js.RegExp | Null,
)

final class RuleSet(
    val block: BlockRules,
    val inline: InlineRules,
)

object Rules:
  private inline def re(pattern: String): js.RegExp = new js.RegExp(pattern)
  private inline def re(pattern: String, flags: String): js.RegExp = new js.RegExp(pattern, flags)

  private final class Editor(private var source: String, private val opt: String):
    private def stripCaret(valSource: String): String =
      valSource.jsReplaceRe(caret, "$1")

    def replace(name: String, valStr: String): Editor =
      source = source.jsReplaceStr(name, stripCaret(valStr))
      this

    def replace(name: String, valRe: js.RegExp): Editor =
      source = source.jsReplaceStr(name, stripCaret(valRe.source))
      this

    def replaceRe(name: js.RegExp, valStr: String): Editor =
      source = source.jsReplaceRe(name, stripCaret(valStr))
      this

    def replaceRe(name: js.RegExp, valRe: js.RegExp): Editor =
      source = source.jsReplaceRe(name, stripCaret(valRe.source))
      this

    def getRegex(): js.RegExp =
      if opt.isEmpty then new js.RegExp(source) else new js.RegExp(source, opt)

  private def edit(pattern: String, opt: String = ""): Editor = new Editor(pattern, opt)
  private def edit(regex: js.RegExp, opt: String): Editor = new Editor(regex.source, opt)
  private def edit(regex: js.RegExp): Editor = new Editor(regex.source, "")

  private val supportsLookbehind: Boolean =
    try
      new js.RegExp("(?<=1)(?<!1)")
      true
    catch
      case _: Throwable => false

  private val _tag: String =
    "address|article|aside|base|basefont|blockquote|body|caption" +
      "|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption" +
      "|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe" +
      "|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option" +
      "|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title" +
      "|tr|track|ul"

  // Helper regexes (`other` in rules.ts)
  val codeRemoveIndent: js.RegExp = re("""^(?: {0,3}\t| {1,4})""", "gm")
  val outputLinkReplace: js.RegExp = re("""\\([\[\]])""", "g")
  val indentCodeCompensation: js.RegExp = re("""^(\s+)(?:```)""")
  val beginningSpace: js.RegExp = re("""^\s+""")
  val endingHash: js.RegExp = re("""#$""")
  val startingSpaceChar: js.RegExp = re("""^ """)
  val endingSpaceChar: js.RegExp = re(""" $""")
  val endingSpaceTabChar: js.RegExp = re("""[ \t]$""")
  val nonSpaceChar: js.RegExp = re("""[^ ]""")
  val newLineCharGlobal: js.RegExp = re("""\n""", "g")
  val tabCharGlobal: js.RegExp = re("""\t""", "g")
  val leadingSpaceTab: js.RegExp = re("""^[ \t]+""")
  val multipleSpaceGlobal: js.RegExp = re("""\s+""", "g")
  val blankLine: js.RegExp = re("""^[ \t]*$""")
  val doubleBlankLine: js.RegExp = re("""\n[ \t]*\n[ \t]*$""")
  val blockquoteStart: js.RegExp = re("""^ {0,3}>""")
  val blockquoteSetextReplace: js.RegExp = re("""\n {0,3}((?:=+|-+) *)(?=\n|$)""", "g")
  val blockquoteSetextReplace2: js.RegExp = re("""^ {0,3}>[ \t]?""", "gm")
  val listReplaceNesting: js.RegExp = re("""^ {1,4}(?=( {4})*[^ ])""", "g")
  val listIsTask: js.RegExp = re("""^\[[ xX]\] +\S""")
  val listReplaceTask: js.RegExp = re("""^\[[ xX]\] +""")
  val listTaskCheckbox: js.RegExp = re("""\[[ xX]\]""")
  val anyLine: js.RegExp = re("""\n.*\n""")
  val hrefBrackets: js.RegExp = re("""^<(.*)>$""")
  val tableDelimiter: js.RegExp = re("""[:|]""")
  val tableAlignChars: js.RegExp = re("""^\||\| *$""", "g")
  val tableRowBlankLine: js.RegExp = re("""\n[ \t]*$""")
  val tableAlignRight: js.RegExp = re("""^ *-+: *$""")
  val tableAlignCenter: js.RegExp = re("""^ *:-+: *$""")
  val tableAlignLeft: js.RegExp = re("""^ *:-+ *$""")
  val startATag: js.RegExp = re("""^<a """, "i")
  val endATag: js.RegExp = re("""^<\/a>""", "i")
  val startPreScriptTag: js.RegExp = re("""^<(pre|code|kbd|script)(\s|>)""", "i")
  val endPreScriptTag: js.RegExp = re("""^<\/(pre|code|kbd|script)(\s|>)""", "i")
  val startAngleBracket: js.RegExp = re("""^<""")
  val endAngleBracket: js.RegExp = re(""">$""")
  val pedanticHrefTitle: js.RegExp = re("""^([^'"]*[^\s])\s+(['"])(.*)\2""")
  val unicodeAlphaNumeric: js.RegExp = re("""[\p{L}\p{N}]""", "u")
  val numericCharacterReference: js.RegExp = re("""&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));""")
  val numericCharacterReferenceOrAmpersand: js.RegExp =
    re("""&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)""", "g")
  val escapeTest: js.RegExp = re("""[&<>"']""")
  val escapeReplace: js.RegExp = re("""[&<>"']""", "g")
  val escapeTestNoEncode: js.RegExp = re("""[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)""")
  val escapeReplaceNoEncode: js.RegExp = re("""[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)""", "g")
  val caret: js.RegExp = re("""(^|[^\[])\^""", "g")
  val percentDecode: js.RegExp = re("""%25""", "g")
  val findPipe: js.RegExp = re("""\|""", "g")
  val splitPipe: js.RegExp = re(""" \|""")
  val slashPipe: js.RegExp = re("""\\\|""", "g")
  val carriageReturn: js.RegExp = re("""\r\n|\r""", "g")
  val spaceLine: js.RegExp = re("""^ +$""", "gm")
  val notSpaceStart: js.RegExp = re("""^\S*""")
  val endingNewline: js.RegExp = re("""\n$""")
  val leadingNewline: js.RegExp = re("""^\n""")
  val backtickGlobal: js.RegExp = re("""`""", "g")

  def listItemRegex(bull: String): js.RegExp =
    re("^( {0,3}" + bull + ")((?:[\t ][^\\n]*)?(?:\\n|$))")

  private inline def clampIndent(indent: Int): Int =
    if indent <= 1 then 0 else if indent >= 4 then 3 else indent - 1

  private def buildIndentTable(f: Int => js.RegExp): Array[js.RegExp] =
    val arr = new Array[js.RegExp](4)
    arr(0) = f(0)
    arr(1) = f(1)
    arr(2) = f(2)
    arr(3) = f(3)
    arr

  private val nextBulletTable: Array[js.RegExp] = buildIndentTable(i =>
    re("^ {0," + i + "}(?:[*+-]|\\d{1,9}[.)])((?:[ \t][^\\n]*)?(?:\\n|$))"),
  )
  private val hrTable: Array[js.RegExp] = buildIndentTable(i =>
    re("^ {0," + i + "}((?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\\*[ \t]*){3,})(?:\\n+|$)"),
  )
  private val fencesBeginTable: Array[js.RegExp] = buildIndentTable(i =>
    re("^ {0," + i + "}(?:```|~~~)"),
  )
  private val headingBeginTable: Array[js.RegExp] = buildIndentTable(i =>
    re("^ {0," + i + "}#"),
  )
  private val htmlBeginTable: Array[js.RegExp] = buildIndentTable(i =>
    re(
      "^ {0," + i + "}(?:</?(?:" + _tag + ")(?: +|$|/?>)|<(?:(?:script|pre|style|textarea)(?:[\\s>]|$)|!--)|<\\?|<![A-Za-z]|<!\\[CDATA\\[)",
      "i",
    ),
  )
  private val blockquoteBeginTable: Array[js.RegExp] = buildIndentTable(i =>
    re("^ {0," + i + "}>"),
  )

  def nextBulletRegex(indent: Int): js.RegExp = nextBulletTable(clampIndent(indent))
  def hrRegex(indent: Int): js.RegExp = hrTable(clampIndent(indent))
  def fencesBeginRegex(indent: Int): js.RegExp = fencesBeginTable(clampIndent(indent))
  def headingBeginRegex(indent: Int): js.RegExp = headingBeginTable(clampIndent(indent))
  def htmlBeginRegex(indent: Int): js.RegExp = htmlBeginTable(clampIndent(indent))
  def blockquoteBeginRegex(indent: Int): js.RegExp = blockquoteBeginTable(clampIndent(indent))

  // Block-Level Grammar
  private val newlineRe: js.RegExp = re("""^(?:[ \t]*(?:\n|$))+""")
  private val blockCodeRe: js.RegExp = re("""^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+""")
  private val fencesRe: js.RegExp =
    re("""^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)""")
  private val hrRe: js.RegExp = re("""^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
  private val headingRe: js.RegExp = re("""^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)""")
  private val bulletRe: js.RegExp = re(""" {0,3}(?:[*+-]|\d{1,9}[.)])""")
  private val lheadingCore: js.RegExp =
    re("""^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |fences|blockquote|heading|hr|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)""")

  private val lheadingRe: js.RegExp = edit(lheadingCore)
    .replaceRe(re("bull", "g"), bulletRe)
    .replaceRe(re("blockCode", "g"), """(?: {4}| {0,3}\t)""")
    .replaceRe(re("fences", "g"), """ {0,3}(?:`{3,}|~{3,})""")
    .replaceRe(re("blockquote", "g"), """ {0,3}>""")
    .replaceRe(re("heading", "g"), """ {0,3}#{1,6}(?:\s|$)""")
    .replaceRe(re("hr", "g"), """ {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
    .replaceRe(re("html", "g"), """ {0,3}<[^\n>]+>\n""")
    .replaceRe(re("""\|table""", "g"), "")
    .getRegex()

  private val lheadingGfmRe: js.RegExp = edit(lheadingCore)
    .replaceRe(re("bull", "g"), bulletRe)
    .replaceRe(re("blockCode", "g"), """(?: {4}| {0,3}\t)""")
    .replaceRe(re("fences", "g"), """ {0,3}(?:`{3,}|~{3,})""")
    .replaceRe(re("blockquote", "g"), """ {0,3}>""")
    .replaceRe(re("heading", "g"), """ {0,3}#{1,6}(?:\s|$)""")
    .replaceRe(re("hr", "g"), """ {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
    .replaceRe(re("html", "g"), """ {0,3}<[^\n>]+>\n""")
    .replaceRe(re("table", "g"), """ {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n""")
    .getRegex()

  private val _paragraphRe: js.RegExp =
    re("""^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)""")
  private val blockTextRe: js.RegExp = re("""^[^\n]+""")
  private val _blockLabelRe: js.RegExp = re("""(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+""")

  private val defRe: js.RegExp =
    edit("""^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)""")
      .replace("label", _blockLabelRe)
      .replace("title", """(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))""")
      .getRegex()

  private val listRe: js.RegExp = edit("""^(bull)([ \t][^\n]*?)?(?:\n|$)""")
    .replaceRe(re("bull", "g"), bulletRe)
    .getRegex()

  private val _commentRe: js.RegExp = re("""<!--(?:-?>|[\s\S]*?(?:-->|$))""")

  private val htmlRe: js.RegExp = edit(
    "^ {0,3}(?:" +
      "<(script|pre|style|textarea)[\\s>][\\s\\S]*?(?:</\\1>[^\\n]*\\n*|$)" +
      "|comment[^\\n]*(\\n+|$)" +
      "|<\\?[\\s\\S]*?(?:\\?>[^\\n]*\\n*|$)" +
      "|<![A-Z][\\s\\S]*?(?:>[^\\n]*\\n*|$)" +
      "|<!\\[CDATA\\[[\\s\\S]*?(?:\\]\\]>[^\\n]*\\n*|$)" +
      "|</?(tag)(?: +|\\n|/?>)[\\s\\S]*?(?:(?:\\n[ \t]*)+\\n|$)" +
      "|<(?!(?:script|pre|style|textarea)(?![a-z0-9-]))([a-z][a-z0-9-]*)(?:attribute)*? */?>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ \t]*)+\\n|$)" +
      "|</[a-z][a-z0-9-]*\\s*>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ \t]*)+\\n|$)" +
      ")",
    "i",
  )
    .replace("comment", _commentRe)
    .replace("tag", _tag)
    .replace("attribute", """ +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?""")
    .getRegex()

  private def createParagraph(listInterrupt: String): js.RegExp = edit(_paragraphRe)
    .replace("hr", hrRe)
    .replace("heading", """ {0,3}#{1,6}(?:\s|$)""")
    .replace("|lheading", "")
    .replace("|table", "")
    .replace("blockquote", " {0,3}>")
    .replace("fences", """ {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)""")
    .replace("list", listInterrupt)
    .replace(
      "html",
      """ {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""",
    )
    .replace("tag", _tag)
    .getRegex()

  private val paragraphRe: js.RegExp = createParagraph(""" {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]""")
  private val blockquoteParagraphRe: js.RegExp = createParagraph(""" {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)""")

  private val blockquoteRe: js.RegExp = edit("""^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+""")
    .replace("paragraph", blockquoteParagraphRe)
    .getRegex()

  val blockNormal: BlockRules = new BlockRules(
    blockquote = blockquoteRe,
    code = blockCodeRe,
    defRule = defRe,
    fences = fencesRe,
    heading = headingRe,
    hr = hrRe,
    html = htmlRe,
    lheading = lheadingRe,
    list = listRe,
    newline = newlineRe,
    paragraph = paragraphRe,
    table = null,
    text = blockTextRe,
  )

  private val gfmTableRe: js.RegExp = edit(
    "^ *([^\\n ].*)\\n" +
      " {0,3}((?:\\| *)?:?-+:? *(?:\\| *:?-+:? *)*(?:\\| *)?)" +
      "(?:\\n((?:(?! *\\n|hr|heading|blockquote|code|fences|list|html).*(?:\\n|$))*)\\n*|$)",
  )
    .replace("hr", hrRe)
    .replace("heading", """ {0,3}#{1,6}(?:\s|$)""")
    .replace("blockquote", " {0,3}>")
    .replace("code", "(?: {4}| {0,3}\t)[^\\n]")
    .replace("fences", """ {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)""")
    .replace("list", """ {0,3}(?:[*+-]|1[.)])[ \t]""")
    .replace(
      "html",
      """ {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""",
    )
    .replace("tag", _tag)
    .getRegex()

  val blockGfm: BlockRules = new BlockRules(
    blockquote = blockquoteRe,
    code = blockCodeRe,
    defRule = defRe,
    fences = fencesRe,
    heading = headingRe,
    hr = hrRe,
    html = htmlRe,
    lheading = lheadingGfmRe,
    list = listRe,
    newline = newlineRe,
    paragraph = edit(_paragraphRe)
      .replace("hr", hrRe)
      .replace("heading", """ {0,3}#{1,6}(?:\s|$)""")
      .replace("|lheading", "")
      .replace("table", gfmTableRe)
      .replace("blockquote", " {0,3}>")
      .replace("fences", """ {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)""")
      .replace("list", """ {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]""")
      .replace(
        "html",
        """ {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""",
      )
      .replace("tag", _tag)
      .getRegex(),
    table = gfmTableRe,
    text = blockTextRe,
  )

  val blockPedantic: BlockRules = new BlockRules(
    blockquote = blockquoteRe,
    code = blockCodeRe,
    defRule = re("""^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)"""),
    fences = null,
    heading = re("""^(#{1,6})(.*)(?:\n+|$)"""),
    hr = hrRe,
    html = edit(
      "^ *(?:comment *(?:\\n|\\s*$)" +
        "|<(tag)[\\s\\S]+?</\\1> *(?:\\n{2,}|\\s*$)" +
        "|<tag(?:\"[^\"]*\"|'[^']*'|\\s[^'\"/>\\s]*)*?/?> *(?:\\n{2,}|\\s*$))",
    )
      .replace("comment", _commentRe)
      .replaceRe(
        re("tag", "g"),
        "(?!(?:" +
          "a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub" +
          "|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)" +
          "\\b)\\w+(?!:|[^\\w\\s@]*@)\\b",
      )
      .getRegex(),
    lheading = re("""^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)"""),
    list = listRe,
    newline = newlineRe,
    paragraph = edit(_paragraphRe)
      .replace("hr", hrRe)
      .replace("heading", " *#{1,6} *[^\n]")
      .replace("lheading", lheadingRe)
      .replace("|table", "")
      .replace("blockquote", " {0,3}>")
      .replace("|fences", "")
      .replace("|list", "")
      .replace("|html", "")
      .replace("|tag", "")
      .getRegex(),
    table = null,
    text = blockTextRe,
  )

  // Inline-Level Grammar
  private val escapeRe: js.RegExp = re("""^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])""")
  private val inlineCodeRe: js.RegExp = re("""^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)""")
  private val brRe: js.RegExp = re("""^( {2,}|\\)\n(?!\s*$)[ \t]*""")
  private val inlineTextRe: js.RegExp =
    re("""^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))""")

  private val _punctuation: String = """[\p{P}\p{S}]"""
  private val _punctuationOrSpace: String = """[\s\p{P}\p{S}]"""
  private val _notPunctuationOrSpace: String = """[^\s\p{P}\p{S}]"""
  private val punctuationRe: js.RegExp = edit("""^((?![*_])punctSpace)""", "u")
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpace)
    .getRegex()

  private val _openQuote: String = """[\p{Pi}\p{Ps}"']"""

  private val _punctuationGfmStrongEm: String = """(?!~)[\p{P}\p{S}]"""
  private val _punctuationOrSpaceGfmStrongEm: String = """(?!~)[\s\p{P}\p{S}]"""
  private val _notPunctuationOrSpaceGfmStrongEm: String = """(?:[^\s\p{P}\p{S}]|~)"""

  private val blockSkipRe: js.RegExp = edit("link|precode-code|html", "g")
    .replace(
      "link",
      """\[(?:[^\[\]`]|(?<a>`+)[^`]+\k<a>(?!`))*?\]\((?:\\[\s\S]|[^\\\(\)]|\((?:\\[\s\S]|[^\\\(\)])*\))*\)""",
    )
    .replace("precode-", if supportsLookbehind then "(?<!`)()" else "(^^|[^`])")
    .replace("code", """(?<b>`+)[^`]+\k<b>(?!`)""")
    .replace("html", """<(?! )[^<>]*?>""")
    .getRegex()

  private val emStrongLDelimCore: String =
    """^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?"""

  private val emStrongLDelimRe: js.RegExp = edit(emStrongLDelimCore, "u")
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val emStrongLDelimGfmRe: js.RegExp = edit(emStrongLDelimCore, "u")
    .replaceRe(re("punct", "g"), _punctuationGfmStrongEm)
    .getRegex()

  private val emStrongLDelimPedanticCore: String =
    """^(?:\*+(?:((?!\*)(?!openQuote)punct)|([^\s*]))?)|^_+(?:((?!_)(?!openQuote)punct)|([^\s_]))?"""

  private val emStrongLDelimPedanticRe: js.RegExp = edit(emStrongLDelimPedanticCore, "u")
    .replaceRe(re("openQuote", "g"), _openQuote)
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val emStrongRDelimAstCore: String =
    "^[^_*]*?__[^_*]*?\\*[^_*]*?(?=__)" +
      "|[^*]+(?=[^*])" +
      "|(?!\\*)punct(\\*+)(?=[\\s]|$)" +
      "|notPunctSpace(\\*+)(?!\\*)(?=punctSpace|$)" +
      "|(?!\\*)punctSpace(\\*+)(?=notPunctSpace)" +
      "|[\\s](\\*+)(?!\\*)(?=punct)" +
      "|(?!\\*)punct(\\*+)(?!\\*)(?=punct)" +
      "|notPunctSpace(\\*+)(?=notPunctSpace)"

  private val emStrongRDelimAstRe: js.RegExp = edit(emStrongRDelimAstCore, "gu")
    .replaceRe(re("notPunctSpace", "g"), _notPunctuationOrSpace)
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpace)
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val emStrongRDelimAstGfmRe: js.RegExp = edit(emStrongRDelimAstCore, "gu")
    .replaceRe(re("notPunctSpace", "g"), _notPunctuationOrSpaceGfmStrongEm)
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpaceGfmStrongEm)
    .replaceRe(re("punct", "g"), _punctuationGfmStrongEm)
    .getRegex()

  private val emStrongRDelimAstPedanticCore: String =
    "^[^_*]*?__[^_*]*?\\*[^_*]*?(?=__)" +
      "|[^*]+(?=[^*])" +
      "|(?!\\*)punct(\\*+)(?=[\\s]|$)" +
      "|notPunctSpace(\\*+)(?!\\*)(?=punctSpace|$)" +
      "|(?!\\*)[\\s](\\*+)(?=notPunctSpace)" +
      "|[\\s](\\*+)(?!\\*)(?=punct)" +
      "|(?!\\*)punct(\\*+)(?!\\*)(?=punct)" +
      "|(?:(?!\\*)punct|notPunctSpace)(\\*+)(?!\\*)(?=notPunctSpace)"

  private val emStrongRDelimAstPedanticRe: js.RegExp = edit(emStrongRDelimAstPedanticCore, "gu")
    .replaceRe(re("notPunctSpace", "g"), _notPunctuationOrSpace)
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpace)
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val emStrongRDelimUndRe: js.RegExp = edit(
    "^[^_*]*?\\*\\*[^_*]*?_[^_*]*?(?=\\*\\*)" +
      "|[^_]+(?=[^_])" +
      "|(?!_)punct(_+)(?=[\\s]|$)" +
      "|notPunctSpace(_+)(?!_)(?=punctSpace|$)" +
      "|(?!_)punctSpace(_+)(?=notPunctSpace)" +
      "|[\\s](_+)(?!_)(?=punct)" +
      "|(?!_)punct(_+)(?!_)(?=punct)",
    "gu",
  )
    .replaceRe(re("notPunctSpace", "g"), _notPunctuationOrSpace)
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpace)
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val emStrongRDelimUndPedanticCore: String =
    "^[^_*]*?\\*\\*[^_*]*?_[^_*]*?(?=\\*\\*)" +
      "|[^_]+(?=[^_])" +
      "|(?!_)punct(_+)(?=[\\s]|$)" +
      "|notPunctSpace(_+)(?!_)(?=punctSpace|$)" +
      "|(?!_)[\\s](_+)(?=notPunctSpace)" +
      "|[\\s](_+)(?!_)(?=punct)" +
      "|(?!_)punct(_+)(?!_)(?=punct)" +
      "|(?:(?!_)punct|notPunctSpace)(_+)(?!_)(?=notPunctSpace)"

  private val emStrongRDelimUndPedanticRe: js.RegExp = edit(emStrongRDelimUndPedanticCore, "gu")
    .replaceRe(re("notPunctSpace", "g"), _notPunctuationOrSpace)
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpace)
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val delLDelimRe: js.RegExp = edit("""^~~?(?:((?!~)punct)|[^\s~])""", "u")
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val delRDelimCore: String =
    "^[^~]+(?=[^~])" +
      "|(?!~)punct(~~?)(?=[\\s]|$)" +
      "|notPunctSpace(~~?)(?!~)(?=punctSpace|$)" +
      "|(?!~)punctSpace(~~?)(?=notPunctSpace)" +
      "|[\\s](~~?)(?!~)(?=punct)" +
      "|(?!~)punct(~~?)(?!~)(?=punct)" +
      "|notPunctSpace(~~?)(?=notPunctSpace)"

  private val delRDelimRe: js.RegExp = edit(delRDelimCore, "gu")
    .replaceRe(re("notPunctSpace", "g"), _notPunctuationOrSpace)
    .replaceRe(re("punctSpace", "g"), _punctuationOrSpace)
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val anyPunctuationRe: js.RegExp = edit("""\\(punct)""", "gu")
    .replaceRe(re("punct", "g"), _punctuation)
    .getRegex()

  private val anyPunctuationOutsideCodeRe: js.RegExp =
    edit("""(`+)(?:[^`]|[^`][\s\S]*?[^`])\1(?!`)|`+|\\(punct)""", "gu")
      .replaceRe(re("punct", "g"), _punctuation)
      .getRegex()

  private val autolinkRe: js.RegExp = edit("""^<(scheme:[^\s\x00-\x1f<>]*|email)>""")
    .replace("scheme", """[a-zA-Z][a-zA-Z0-9+.-]{1,31}""")
    .replace(
      "email",
      """[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])""",
    )
    .getRegex()

  private val _inlineCommentRe: js.RegExp = edit(_commentRe).replace("(?:-->|$)", "-->").getRegex()
  private val tagRe: js.RegExp = edit(
    "^comment" +
      "|^</[a-zA-Z][a-zA-Z0-9-]*\\s*>" +
      "|^<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\\s*/?>" +
      "|^<\\?[\\s\\S]*?\\?>" +
      "|^<![a-zA-Z]+\\s[\\s\\S]*?>" +
      "|^<!\\[CDATA\\[[\\s\\S]*?\\]\\]>",
  )
    .replace("comment", _inlineCommentRe)
    .replace("attribute", """\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?""")
    .getRegex()

  private val _inlineLabelBracketsRe: js.RegExp = re("""\[(?:\\[\s\S]|[^\[\]\\])*\]""")
  private val _inlineLabelRe: js.RegExp =
    edit("""(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?""")
      .replace("brackets", _inlineLabelBracketsRe)
      .getRegex()

  private val linkRe: js.RegExp =
    edit("""^!?\[(label)\]\([ \t\n]*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?[ \t\n]*\)""")
      .replace("label", _inlineLabelRe)
      .replace("href", """<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))""")
      .replace("title", "\"" + """(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)""")
      .getRegex()

  private val reflinkRe: js.RegExp = edit("""^!?\[(label)\]\[(ref)\]""")
    .replace("label", _inlineLabelRe)
    .replace("ref", _blockLabelRe)
    .getRegex()

  private val nolinkRe: js.RegExp = edit("""^!?\[(ref)\](?:\[\])?""")
    .replace("ref", _blockLabelRe)
    .getRegex()

  private val _boundedBlockLabelRe: js.RegExp = re("""(?!\s*\])(?:\\[\s\S]|[^\[\]\\]){1,999}""")
  private val _boundedInlineLabelRe: js.RegExp =
    edit("""(?:[^\[\]\\`]*(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\]))){0,999}?[^\[\]\\`]*?""")
      .replace("brackets", _inlineLabelBracketsRe)
      .getRegex()

  private val reflinkSearchRe: js.RegExp = edit("""reflink|nolink(?!\()""", "g")
    .replace(
      "reflink",
      edit("""^!?\[(label)\]\[(ref)\]""")
        .replace("label", _boundedInlineLabelRe)
        .replace("ref", _boundedBlockLabelRe)
        .getRegex(),
    )
    .replace(
      "nolink",
      edit("""^!?\[(ref)\](?:\[\])?""")
        .replace("ref", _boundedBlockLabelRe)
        .getRegex(),
    )
    .getRegex()

  private val _caseInsensitiveProtocol: String = """[hH][tT][tT][pP][sS]?|[fF][tT][pP]"""
  private val _extendedEmail: String =
    """[A-Za-z0-9._+-]+@[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])"""
  private val _extendedEmailProtocolRe: js.RegExp =
    edit("""(?:mailto:email|xmpp:email(?:\/[A-Za-z0-9@.]+)?)""")
      .replaceRe(re("email", "g"), _extendedEmail)
      .getRegex()

  val inlineNormal: InlineRules = new InlineRules(
    backpedal = null,
    anyPunctuation = anyPunctuationRe,
    anyPunctuationOutsideCode = anyPunctuationOutsideCodeRe,
    autolink = autolinkRe,
    blockSkip = blockSkipRe,
    br = brRe,
    code = inlineCodeRe,
    del = null,
    delLDelim = null,
    delRDelim = null,
    emStrongLDelim = emStrongLDelimRe,
    emStrongRDelimAst = emStrongRDelimAstRe,
    emStrongRDelimUnd = emStrongRDelimUndRe,
    escape = escapeRe,
    link = linkRe,
    nolink = nolinkRe,
    punctuation = punctuationRe,
    reflink = reflinkRe,
    reflinkSearch = reflinkSearchRe,
    tag = tagRe,
    text = inlineTextRe,
    url = null,
  )

  val inlinePedantic: InlineRules = new InlineRules(
    backpedal = null,
    anyPunctuation = anyPunctuationRe,
    anyPunctuationOutsideCode = anyPunctuationOutsideCodeRe,
    autolink = autolinkRe,
    blockSkip = blockSkipRe,
    br = brRe,
    code = inlineCodeRe,
    del = null,
    delLDelim = null,
    delRDelim = null,
    emStrongLDelim = emStrongLDelimPedanticRe,
    emStrongRDelimAst = emStrongRDelimAstPedanticRe,
    emStrongRDelimUnd = emStrongRDelimUndPedanticRe,
    escape = escapeRe,
    link = edit("""^!?\[(label)\]\((.*?)\)""").replace("label", _inlineLabelRe).getRegex(),
    nolink = nolinkRe,
    punctuation = punctuationRe,
    reflink = edit("""^!?\[(label)\]\s*\[([^\]]*)\]""").replace("label", _inlineLabelRe).getRegex(),
    reflinkSearch = reflinkSearchRe,
    tag = tagRe,
    text = inlineTextRe,
    url = null,
  )

  private val gfmUrlRe: js.RegExp =
    edit("""^emailProtocol|^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email""")
      .replace("emailProtocol", _extendedEmailProtocolRe)
      .replace("protocol", _caseInsensitiveProtocol)
      .replace("email", """[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])""")
      .getRegex()

  private val gfmBackpedalRe: js.RegExp =
    re("""(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+""")

  private val gfmDelRe: js.RegExp =
    re("""^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)""")

  private val gfmInlineTextRe: js.RegExp =
    edit(
      """^(?:[^a-zA-Z0-9](?=emailProtocol)|(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9](?=emailProtocol)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@))))""",
    )
      .replace("protocol", _caseInsensitiveProtocol)
      .replaceRe(re("emailProtocol", "g"), """(?:mailto|xmpp):""")
      .getRegex()

  val inlineGfm: InlineRules = new InlineRules(
    backpedal = gfmBackpedalRe,
    anyPunctuation = anyPunctuationRe,
    anyPunctuationOutsideCode = anyPunctuationOutsideCodeRe,
    autolink = autolinkRe,
    blockSkip = blockSkipRe,
    br = brRe,
    code = inlineCodeRe,
    del = gfmDelRe,
    delLDelim = delLDelimRe,
    delRDelim = delRDelimRe,
    emStrongLDelim = emStrongLDelimGfmRe,
    emStrongRDelimAst = emStrongRDelimAstGfmRe,
    emStrongRDelimUnd = emStrongRDelimUndRe,
    escape = escapeRe,
    link = linkRe,
    nolink = nolinkRe,
    punctuation = punctuationRe,
    reflink = reflinkRe,
    reflinkSearch = reflinkSearchRe,
    tag = tagRe,
    text = gfmInlineTextRe,
    url = gfmUrlRe,
  )

  val inlineBreaks: InlineRules = new InlineRules(
    backpedal = gfmBackpedalRe,
    anyPunctuation = anyPunctuationRe,
    anyPunctuationOutsideCode = anyPunctuationOutsideCodeRe,
    autolink = autolinkRe,
    blockSkip = blockSkipRe,
    br = edit(brRe).replace("{2,}", "*").getRegex(),
    code = inlineCodeRe,
    del = gfmDelRe,
    delLDelim = delLDelimRe,
    delRDelim = delRDelimRe,
    emStrongLDelim = emStrongLDelimGfmRe,
    emStrongRDelimAst = emStrongRDelimAstGfmRe,
    emStrongRDelimUnd = emStrongRDelimUndRe,
    escape = escapeRe,
    link = linkRe,
    nolink = nolinkRe,
    punctuation = punctuationRe,
    reflink = reflinkRe,
    reflinkSearch = reflinkSearchRe,
    tag = tagRe,
    text = edit(gfmInlineTextRe)
      .replace("""\b_""", """\b_| {2,}\n""")
      .replaceRe(re("""\{2,\}""", "g"), "*")
      .getRegex(),
    url = gfmUrlRe,
  )

  val normalSet: RuleSet = new RuleSet(blockNormal, inlineNormal)
  val gfmSet: RuleSet = new RuleSet(blockGfm, inlineGfm)
  val breaksSet: RuleSet = new RuleSet(blockGfm, inlineBreaks)
  val pedanticSet: RuleSet = new RuleSet(blockPedantic, inlinePedantic)

  def select(options: MarkedOptions): RuleSet =
    if options.pedantic then pedanticSet
    else if options.gfm then
      if options.breaks then breaksSet else gfmSet
    else normalSet
