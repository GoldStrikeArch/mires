package marked

import kotlin.js.RegExp

private class RuleEditor(
    private var source: String,
    private val opt: String = "",
) {
    fun replace(name: String, value: String): RuleEditor {
        val valSource = value.asJs().replaceRe(CARET_RE, "$1")
        source = source.asJs().replaceStr(name, valSource)
        return this
    }

    fun replace(name: String, value: RegExp): RuleEditor = replace(name, value.source)

    fun replace(name: RegExp, value: String): RuleEditor {
        val valSource = value.asJs().replaceRe(CARET_RE, "$1")
        source = source.asJs().replaceRe(name, valSource)
        return this
    }

    fun replace(name: RegExp, value: RegExp): RuleEditor = replace(name, value.source)

    fun getRegex(): RegExp = RegExp(source, opt)
}

private fun edit(regex: String, opt: String = ""): RuleEditor = RuleEditor(regex, opt)

private fun edit(regex: RegExp, opt: String = ""): RuleEditor = RuleEditor(regex.source, opt)

private fun buildIndentTable(create: (Int) -> RegExp): Array<RegExp> =
    arrayOf(create(0), create(1), create(2), create(3))

private fun lookupIndent(table: Array<RegExp>, indent: Int): RegExp {
    val idx = when {
        indent <= 1 -> 0
        indent >= 4 -> 3
        else -> indent - 1
    }
    return table[idx]
}

private val CARET_RE: RegExp = RegExp("""(^|[^\[])\^""", "g")

private val SUPPORTS_LOOKBEHIND: Boolean = try {
    RegExp("(?<=1)(?<!1)")
    true
} catch (_: Throwable) {
    false
}

private const val BLOCK_TAG: String =
    "address|article|aside|base|basefont|blockquote|body|caption" +
        "|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption" +
        "|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe" +
        "|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option" +
        "|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title" +
        "|tr|track|ul"

class OtherRules {
    val codeRemoveIndent: RegExp = RegExp("""^(?: {0,3}\t| {1,4})""", "gm")
    val outputLinkReplace: RegExp = RegExp("""\\([\[\]])""", "g")
    val indentCodeCompensation: RegExp = RegExp("""^(\s+)(?:```)""")
    val beginningSpace: RegExp = RegExp("""^\s+""")
    val endingHash: RegExp = RegExp("""#$""")
    val startingSpaceChar: RegExp = RegExp("""^ """)
    val endingSpaceChar: RegExp = RegExp(""" $""")
    val endingSpaceTabChar: RegExp = RegExp("""[ \t]$""")
    val nonSpaceChar: RegExp = RegExp("""[^ ]""")
    val newLineCharGlobal: RegExp = RegExp("""\n""", "g")
    val tabCharGlobal: RegExp = RegExp("""\t""", "g")
    val leadingSpaceTab: RegExp = RegExp("""^[ \t]+""")
    val multipleSpaceGlobal: RegExp = RegExp("""\s+""", "g")
    val blankLine: RegExp = RegExp("""^[ \t]*$""")
    val doubleBlankLine: RegExp = RegExp("""\n[ \t]*\n[ \t]*$""")
    val blockquoteStart: RegExp = RegExp("""^ {0,3}>""")
    val blockquoteSetextReplace: RegExp = RegExp("""\n {0,3}((?:=+|-+) *)(?=\n|$)""", "g")
    val blockquoteSetextReplace2: RegExp = RegExp("""^ {0,3}>[ \t]?""", "gm")
    val listReplaceNesting: RegExp = RegExp("""^ {1,4}(?=( {4})*[^ ])""", "g")
    val listIsTask: RegExp = RegExp("""^\[[ xX]\] +\S""")
    val listReplaceTask: RegExp = RegExp("""^\[[ xX]\] +""")
    val listTaskCheckbox: RegExp = RegExp("""\[[ xX]\]""")
    val anyLine: RegExp = RegExp("""\n.*\n""")
    val hrefBrackets: RegExp = RegExp("""^<(.*)>$""")
    val tableDelimiter: RegExp = RegExp("""[:|]""")
    val tableAlignChars: RegExp = RegExp("""^\||\| *$""", "g")
    val tableRowBlankLine: RegExp = RegExp("""\n[ \t]*$""")
    val tableAlignRight: RegExp = RegExp("""^ *-+: *$""")
    val tableAlignCenter: RegExp = RegExp("""^ *:-+: *$""")
    val tableAlignLeft: RegExp = RegExp("""^ *:-+ *$""")
    val startATag: RegExp = RegExp("""^<a """, "i")
    val endATag: RegExp = RegExp("""^<\/a>""", "i")
    val startPreScriptTag: RegExp = RegExp("""^<(pre|code|kbd|script)(\s|>)""", "i")
    val endPreScriptTag: RegExp = RegExp("""^<\/(pre|code|kbd|script)(\s|>)""", "i")
    val startAngleBracket: RegExp = RegExp("""^<""")
    val endAngleBracket: RegExp = RegExp(""">$""")
    val pedanticHrefTitle: RegExp = RegExp("""^([^'"]*[^\s])\s+(['"])(.*)\2""")
    val unicodeAlphaNumeric: RegExp = RegExp("""[\p{L}\p{N}]""", "u")
    val numericCharacterReference: RegExp = RegExp("""&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));""")
    val numericCharacterReferenceOrAmpersand: RegExp =
        RegExp("""&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)""", "g")
    val escapeTest: RegExp = RegExp("""[&<>"']""")
    val escapeReplace: RegExp = RegExp("""[&<>"']""", "g")
    val escapeTestNoEncode: RegExp = RegExp("""[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)""")
    val escapeReplaceNoEncode: RegExp = RegExp("""[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)""", "g")
    val caret: RegExp = CARET_RE
    val percentDecode: RegExp = RegExp("""%25""", "g")
    val findPipe: RegExp = RegExp("""\|""", "g")
    val splitPipe: RegExp = RegExp(""" \|""")
    val slashPipe: RegExp = RegExp("""\\\|""", "g")
    val carriageReturn: RegExp = RegExp("""\r\n|\r""", "g")
    val spaceLine: RegExp = RegExp("""^ +$""", "gm")
    val notSpaceStart: RegExp = RegExp("""^\S*""")
    val endingNewline: RegExp = RegExp("""\n$""")
    val backtickGlobal: RegExp = RegExp("`", "g")

    private val nextBulletTable: Array<RegExp> = buildIndentTable { indent ->
        RegExp("""^ {0,$indent}(?:[*+-]|\d{1,9}[.)])((?:[ \t][^\n]*)?(?:\n|$))""")
    }
    private val hrTable: Array<RegExp> = buildIndentTable { indent ->
        RegExp("""^ {0,$indent}((?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
    }
    private val fencesBeginTable: Array<RegExp> = buildIndentTable { indent ->
        RegExp("""^ {0,$indent}(?:```|~~~)""")
    }
    private val headingBeginTable: Array<RegExp> = buildIndentTable { indent ->
        RegExp("""^ {0,$indent}#""")
    }
    private val htmlBeginTable: Array<RegExp> = buildIndentTable { indent ->
        RegExp(
            """^ {0,$indent}(?:</?(?:$BLOCK_TAG)(?: +|$|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""",
            "i",
        )
    }
    private val blockquoteBeginTable: Array<RegExp> = buildIndentTable { indent ->
        RegExp("""^ {0,$indent}>""")
    }

    fun listItemRegex(bull: String): RegExp =
        RegExp("""^( {0,3}$bull)((?:[\t ][^\n]*)?(?:\n|$))""")

    fun nextBulletRegex(indent: Int): RegExp = lookupIndent(nextBulletTable, indent)
    fun hrRegex(indent: Int): RegExp = lookupIndent(hrTable, indent)
    fun fencesBeginRegex(indent: Int): RegExp = lookupIndent(fencesBeginTable, indent)
    fun headingBeginRegex(indent: Int): RegExp = lookupIndent(headingBeginTable, indent)
    fun htmlBeginRegex(indent: Int): RegExp = lookupIndent(htmlBeginTable, indent)
    fun blockquoteBeginRegex(indent: Int): RegExp = lookupIndent(blockquoteBeginTable, indent)
}

class BlockRules(
    val blockquote: RegExp,
    val code: RegExp,
    val def: RegExp,
    val fences: RegExp?,
    val heading: RegExp,
    val hr: RegExp,
    val html: RegExp,
    val lheading: RegExp,
    val list: RegExp,
    val newline: RegExp,
    val paragraph: RegExp,
    val table: RegExp?,
    val text: RegExp,
)

class InlineRules(
    val backpedal: RegExp?,
    val anyPunctuation: RegExp,
    val anyPunctuationOutsideCode: RegExp,
    val autolink: RegExp,
    val blockSkip: RegExp,
    val br: RegExp,
    val code: RegExp,
    val del: RegExp?,
    val delLDelim: RegExp?,
    val delRDelim: RegExp?,
    val emStrongLDelim: RegExp,
    val emStrongRDelimAst: RegExp,
    val emStrongRDelimUnd: RegExp,
    val escape: RegExp,
    val link: RegExp,
    val nolink: RegExp,
    val punctuation: RegExp,
    val reflink: RegExp,
    val reflinkSearch: RegExp,
    val tag: RegExp,
    val text: RegExp,
    val url: RegExp?,
)

class Rules(
    val other: OtherRules,
    val block: BlockRules,
    val inline: InlineRules,
)

val OTHER_RULES: OtherRules = OtherRules()

// Block-Level Grammar
private val B_NEWLINE: RegExp = RegExp("""^(?:[ \t]*(?:\n|$))+""")
private val B_CODE: RegExp = RegExp("""^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+""")
private val B_FENCES: RegExp =
    RegExp("""^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)""")
private val B_HR: RegExp = RegExp("""^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
private val B_HEADING: RegExp = RegExp("""^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)""")
private val B_BULLET: RegExp = RegExp(""" {0,3}(?:[*+-]|\d{1,9}[.)])""")
private val B_LHEADING_CORE: RegExp =
    RegExp("""^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |fences|blockquote|heading|hr|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)""")
private val B_LHEADING: RegExp = edit(B_LHEADING_CORE)
    .replace(RegExp("bull", "g"), B_BULLET)
    .replace(RegExp("blockCode", "g"), """(?: {4}| {0,3}\t)""")
    .replace(RegExp("fences", "g"), """ {0,3}(?:`{3,}|~{3,})""")
    .replace(RegExp("blockquote", "g"), """ {0,3}>""")
    .replace(RegExp("heading", "g"), """ {0,3}#{1,6}(?:\s|$)""")
    .replace(RegExp("hr", "g"), """ {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
    .replace(RegExp("html", "g"), """ {0,3}<[^\n>]+>\n""")
    .replace(RegExp("""\|table""", "g"), "")
    .getRegex()
private val B_LHEADING_GFM: RegExp = edit(B_LHEADING_CORE)
    .replace(RegExp("bull", "g"), B_BULLET)
    .replace(RegExp("blockCode", "g"), """(?: {4}| {0,3}\t)""")
    .replace(RegExp("fences", "g"), """ {0,3}(?:`{3,}|~{3,})""")
    .replace(RegExp("blockquote", "g"), """ {0,3}>""")
    .replace(RegExp("heading", "g"), """ {0,3}#{1,6}(?:\s|$)""")
    .replace(RegExp("hr", "g"), """ {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)""")
    .replace(RegExp("html", "g"), """ {0,3}<[^\n>]+>\n""")
    .replace(RegExp("table", "g"), """ {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n""")
    .getRegex()
private val B_PARAGRAPH_CORE: RegExp =
    RegExp("""^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)""")
private val B_TEXT: RegExp = RegExp("""^[^\n]+""")
private val B_LABEL: RegExp = RegExp("""(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+""")
private val B_DEF: RegExp = edit("""^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)""")
    .replace("label", B_LABEL)
    .replace("title", """(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))""")
    .getRegex()
private val B_LIST: RegExp = edit("""^(bull)([ \t][^\n]*?)?(?:\n|$)""")
    .replace(RegExp("bull", "g"), B_BULLET)
    .getRegex()
private val B_COMMENT: RegExp = RegExp("""<!--(?:-?>|[\s\S]*?(?:-->|$))""")
private val B_HTML: RegExp = edit(
    """^ {0,3}(?:""" +
        """<(script|pre|style|textarea)[\s>][\s\S]*?(?:</\1>[^\n]*\n*|$)""" +
        """|comment[^\n]*(\n+|$)""" +
        """|<\?[\s\S]*?(?:\?>[^\n]*\n*|$)""" +
        """|<![A-Z][\s\S]*?(?:>[^\n]*\n*|$)""" +
        """|<!\[CDATA\[[\s\S]*?(?:\]\]>[^\n]*\n*|$)""" +
        """|</?(tag)(?: +|\n|/?>)[\s\S]*?(?:(?:\n[ \t]*)+\n|$)""" +
        """|<(?!(?:script|pre|style|textarea)(?![a-z0-9-]))([a-z][a-z0-9-]*)(?:attribute)*? */?>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)""" +
        """|</[a-z][a-z0-9-]*\s*>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)""" +
        """)""",
    "i",
)
    .replace("comment", B_COMMENT)
    .replace("tag", BLOCK_TAG)
    .replace("attribute", """ +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?""")
    .getRegex()

private fun createParagraph(listInterrupt: String): RegExp = edit(B_PARAGRAPH_CORE)
    .replace("hr", B_HR)
    .replace("heading", """ {0,3}#{1,6}(?:\s|$)""")
    .replace("|lheading", "")
    .replace("|table", "")
    .replace("blockquote", """ {0,3}>""")
    .replace("fences", """ {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)""")
    .replace("list", listInterrupt)
    .replace("html", """ {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""")
    .replace("tag", BLOCK_TAG)
    .getRegex()

private val B_PARAGRAPH: RegExp = createParagraph(""" {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]""")
private val B_BLOCKQUOTE_PARAGRAPH: RegExp = createParagraph(""" {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)""")
private val B_BLOCKQUOTE: RegExp = edit("""^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+""")
    .replace("paragraph", B_BLOCKQUOTE_PARAGRAPH)
    .getRegex()

private val B_GFM_TABLE: RegExp = edit(
    """^ *([^\n ].*)\n""" +
        """ {0,3}((?:\| *)?:?-+:? *(?:\| *:?-+:? *)*(?:\| *)?)""" +
        """(?:\n((?:(?! *\n|hr|heading|blockquote|code|fences|list|html).*(?:\n|$))*)\n*|$)""",
)
    .replace("hr", B_HR)
    .replace("heading", """ {0,3}#{1,6}(?:\s|$)""")
    .replace("blockquote", """ {0,3}>""")
    .replace("code", """(?: {4}| {0,3}\t)[^\n]""")
    .replace("fences", """ {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)""")
    .replace("list", """ {0,3}(?:[*+-]|1[.)])[ \t]""")
    .replace("html", """ {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""")
    .replace("tag", BLOCK_TAG)
    .getRegex()

val BLOCK_NORMAL: BlockRules = BlockRules(
    blockquote = B_BLOCKQUOTE,
    code = B_CODE,
    def = B_DEF,
    fences = B_FENCES,
    heading = B_HEADING,
    hr = B_HR,
    html = B_HTML,
    lheading = B_LHEADING,
    list = B_LIST,
    newline = B_NEWLINE,
    paragraph = B_PARAGRAPH,
    table = null,
    text = B_TEXT,
)

val BLOCK_GFM: BlockRules = BlockRules(
    blockquote = B_BLOCKQUOTE,
    code = B_CODE,
    def = B_DEF,
    fences = B_FENCES,
    heading = B_HEADING,
    hr = B_HR,
    html = B_HTML,
    lheading = B_LHEADING_GFM,
    list = B_LIST,
    newline = B_NEWLINE,
    paragraph = edit(B_PARAGRAPH_CORE)
        .replace("hr", B_HR)
        .replace("heading", """ {0,3}#{1,6}(?:\s|$)""")
        .replace("|lheading", "")
        .replace("table", B_GFM_TABLE)
        .replace("blockquote", """ {0,3}>""")
        .replace("fences", """ {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)""")
        .replace("list", """ {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]""")
        .replace("html", """ {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)""")
        .replace("tag", BLOCK_TAG)
        .getRegex(),
    table = B_GFM_TABLE,
    text = B_TEXT,
)

val BLOCK_PEDANTIC: BlockRules = BlockRules(
    blockquote = B_BLOCKQUOTE,
    code = B_CODE,
    def = RegExp("""^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)"""),
    fences = null,
    heading = RegExp("""^(#{1,6})(.*)(?:\n+|$)"""),
    hr = B_HR,
    html = edit(
        """^ *(?:comment *(?:\n|\s*$)""" +
            """|<(tag)[\s\S]+?</\1> *(?:\n{2,}|\s*$)""" +
            """|<tag(?:"[^"]*"|'[^']*'|\s[^'"/>\s]*)*?/?> *(?:\n{2,}|\s*$))""",
    )
        .replace("comment", B_COMMENT)
        .replace(
            RegExp("tag", "g"),
            """(?!(?:a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub""" +
                """|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)""" +
                """\b)\w+(?!:|[^\w\s@]*@)\b""",
        )
        .getRegex(),
    lheading = RegExp("""^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)"""),
    list = B_LIST,
    newline = B_NEWLINE,
    paragraph = edit(B_PARAGRAPH_CORE)
        .replace("hr", B_HR)
        .replace("heading", " *#{1,6} *[^\n]")
        .replace("lheading", B_LHEADING)
        .replace("|table", "")
        .replace("blockquote", """ {0,3}>""")
        .replace("|fences", "")
        .replace("|list", "")
        .replace("|html", "")
        .replace("|tag", "")
        .getRegex(),
    table = null,
    text = B_TEXT,
)

// Inline-Level Grammar
private val I_ESCAPE: RegExp = RegExp("""^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])""")
private val I_CODE: RegExp = RegExp("""^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)""")
private val I_BR: RegExp = RegExp("""^( {2,}|\\)\n(?!\s*$)[ \t]*""")
private val I_TEXT: RegExp = RegExp("""^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))""")

private val I_PUNCT_PAT: String = """[\p{P}\p{S}]"""
private val I_PUNCT_SPACE_PAT: String = """[\s\p{P}\p{S}]"""
private val I_NOT_PUNCT_SPACE_PAT: String = """[^\s\p{P}\p{S}]"""
private val I_PUNCTUATION: RegExp = edit("""^((?![*_])punctSpace)""", "u")
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_PAT)
    .getRegex()

private val I_OPEN_QUOTE_PAT: String = """[\p{Pi}\p{Ps}"']"""
private val I_PUNCT_GFM_PAT: String = """(?!~)[\p{P}\p{S}]"""
private val I_PUNCT_SPACE_GFM_PAT: String = """(?!~)[\s\p{P}\p{S}]"""
private val I_NOT_PUNCT_SPACE_GFM_PAT: String = """(?:[^\s\p{P}\p{S}]|~)"""

private val I_BLOCK_SKIP: RegExp = edit("""link|precode-code|html""", "g")
    .replace("link", """\[(?:[^\[\]`]|(?<a>`+)[^`]+\k<a>(?!`))*?\]\((?:\\[\s\S]|[^\\\(\)]|\((?:\\[\s\S]|[^\\\(\)])*\))*\)""")
    .replace("precode-", if (SUPPORTS_LOOKBEHIND) "(?<!`)()" else "(^^|[^`])")
    .replace("code", """(?<b>`+)[^`]+\k<b>(?!`)""")
    .replace("html", """<(?! )[^<>]*?>""")
    .getRegex()

private val I_EM_STRONG_LDELIM_CORE: String = """^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?"""
private val I_EM_STRONG_LDELIM: RegExp = edit(I_EM_STRONG_LDELIM_CORE, "u")
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()
private val I_EM_STRONG_LDELIM_GFM: RegExp = edit(I_EM_STRONG_LDELIM_CORE, "u")
    .replace(RegExp("punct", "g"), I_PUNCT_GFM_PAT)
    .getRegex()
private val I_EM_STRONG_LDELIM_PEDANTIC: RegExp =
    edit("""^(?:\*+(?:((?!\*)(?!openQuote)punct)|([^\s*]))?)|^_+(?:((?!_)(?!openQuote)punct)|([^\s_]))?""", "u")
        .replace(RegExp("openQuote", "g"), I_OPEN_QUOTE_PAT)
        .replace(RegExp("punct", "g"), I_PUNCT_PAT)
        .getRegex()

private val I_EM_STRONG_RDELIM_AST_CORE: String =
    """^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)""" +
        """|[^*]+(?=[^*])""" +
        """|(?!\*)punct(\*+)(?=[\s]|$)""" +
        """|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)""" +
        """|(?!\*)punctSpace(\*+)(?=notPunctSpace)""" +
        """|[\s](\*+)(?!\*)(?=punct)""" +
        """|(?!\*)punct(\*+)(?!\*)(?=punct)""" +
        """|notPunctSpace(\*+)(?=notPunctSpace)"""

private val I_EM_STRONG_RDELIM_AST: RegExp = edit(I_EM_STRONG_RDELIM_AST_CORE, "gu")
    .replace(RegExp("notPunctSpace", "g"), I_NOT_PUNCT_SPACE_PAT)
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_PAT)
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_EM_STRONG_RDELIM_AST_GFM: RegExp = edit(I_EM_STRONG_RDELIM_AST_CORE, "gu")
    .replace(RegExp("notPunctSpace", "g"), I_NOT_PUNCT_SPACE_GFM_PAT)
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_GFM_PAT)
    .replace(RegExp("punct", "g"), I_PUNCT_GFM_PAT)
    .getRegex()

private val I_EM_STRONG_RDELIM_AST_PEDANTIC: RegExp = edit(
    """^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)""" +
        """|[^*]+(?=[^*])""" +
        """|(?!\*)punct(\*+)(?=[\s]|$)""" +
        """|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)""" +
        """|(?!\*)[\s](\*+)(?=notPunctSpace)""" +
        """|[\s](\*+)(?!\*)(?=punct)""" +
        """|(?!\*)punct(\*+)(?!\*)(?=punct)""" +
        """|(?:(?!\*)punct|notPunctSpace)(\*+)(?!\*)(?=notPunctSpace)""",
    "gu",
)
    .replace(RegExp("notPunctSpace", "g"), I_NOT_PUNCT_SPACE_PAT)
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_PAT)
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_EM_STRONG_RDELIM_UND: RegExp = edit(
    """^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)""" +
        """|[^_]+(?=[^_])""" +
        """|(?!_)punct(_+)(?=[\s]|$)""" +
        """|notPunctSpace(_+)(?!_)(?=punctSpace|$)""" +
        """|(?!_)punctSpace(_+)(?=notPunctSpace)""" +
        """|[\s](_+)(?!_)(?=punct)""" +
        """|(?!_)punct(_+)(?!_)(?=punct)""",
    "gu",
)
    .replace(RegExp("notPunctSpace", "g"), I_NOT_PUNCT_SPACE_PAT)
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_PAT)
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_EM_STRONG_RDELIM_UND_PEDANTIC: RegExp = edit(
    """^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)""" +
        """|[^_]+(?=[^_])""" +
        """|(?!_)punct(_+)(?=[\s]|$)""" +
        """|notPunctSpace(_+)(?!_)(?=punctSpace|$)""" +
        """|(?!_)[\s](_+)(?=notPunctSpace)""" +
        """|[\s](_+)(?!_)(?=punct)""" +
        """|(?!_)punct(_+)(?!_)(?=punct)""" +
        """|(?:(?!_)punct|notPunctSpace)(_+)(?!_)(?=notPunctSpace)""",
    "gu",
)
    .replace(RegExp("notPunctSpace", "g"), I_NOT_PUNCT_SPACE_PAT)
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_PAT)
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_DEL_LDELIM: RegExp = edit("""^~~?(?:((?!~)punct)|[^\s~])""", "u")
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_DEL_RDELIM: RegExp = edit(
    """^[^~]+(?=[^~])""" +
        """|(?!~)punct(~~?)(?=[\s]|$)""" +
        """|notPunctSpace(~~?)(?!~)(?=punctSpace|$)""" +
        """|(?!~)punctSpace(~~?)(?=notPunctSpace)""" +
        """|[\s](~~?)(?!~)(?=punct)""" +
        """|(?!~)punct(~~?)(?!~)(?=punct)""" +
        """|notPunctSpace(~~?)(?=notPunctSpace)""",
    "gu",
)
    .replace(RegExp("notPunctSpace", "g"), I_NOT_PUNCT_SPACE_PAT)
    .replace(RegExp("punctSpace", "g"), I_PUNCT_SPACE_PAT)
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_ANY_PUNCTUATION: RegExp = edit("""\\(punct)""", "gu")
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_ANY_PUNCTUATION_OUTSIDE_CODE: RegExp = edit("""(`+)(?:[^`]|[^`][\s\S]*?[^`])\1(?!`)|`+|\\(punct)""", "gu")
    .replace(RegExp("punct", "g"), I_PUNCT_PAT)
    .getRegex()

private val I_AUTOLINK: RegExp = edit("""^<(scheme:[^\s\x00-\x1f<>]*|email)>""")
    .replace("scheme", """[a-zA-Z][a-zA-Z0-9+.-]{1,31}""")
    .replace("email", """[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])""")
    .getRegex()

private val I_INLINE_COMMENT: RegExp = edit(B_COMMENT).replace("(?:-->|$)", "-->").getRegex()
private val I_TAG: RegExp = edit(
    """^comment""" +
        """|^</[a-zA-Z][a-zA-Z0-9-]*\s*>""" +
        """|^<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\s*/?>""" +
        """|^<\?[\s\S]*?\?>""" +
        """|^<![a-zA-Z]+\s[\s\S]*?>""" +
        """|^<!\[CDATA\[[\s\S]*?\]\]>""",
)
    .replace("comment", I_INLINE_COMMENT)
    .replace("attribute", """\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?""")
    .getRegex()

private val I_LABEL_BRACKETS: RegExp = RegExp("""\[(?:\\[\s\S]|[^\[\]\\])*\]""")
private val I_LABEL: RegExp = edit("""(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?""")
    .replace("brackets", I_LABEL_BRACKETS)
    .getRegex()

private val I_LINK: RegExp = edit("""^!?\[(label)\]\([ \t\n]*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?[ \t\n]*\)""")
    .replace("label", I_LABEL)
    .replace("href", """<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))""")
    .replace("title", """"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)""")
    .getRegex()

private val I_REFLINK: RegExp = edit("""^!?\[(label)\]\[(ref)\]""")
    .replace("label", I_LABEL)
    .replace("ref", B_LABEL)
    .getRegex()

private val I_NOLINK: RegExp = edit("""^!?\[(ref)\](?:\[\])?""")
    .replace("ref", B_LABEL)
    .getRegex()

private val I_BOUNDED_BLOCK_LABEL: RegExp = RegExp("""(?!\s*\])(?:\\[\s\S]|[^\[\]\\]){1,999}""")
private val I_BOUNDED_INLINE_LABEL: RegExp =
    edit("""(?:[^\[\]\\`]*(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\]))){0,999}?[^\[\]\\`]*?""")
        .replace("brackets", I_LABEL_BRACKETS)
        .getRegex()

private val I_REFLINK_SEARCH: RegExp = edit("""reflink|nolink(?!\()""", "g")
    .replace(
        "reflink",
        edit("""^!?\[(label)\]\[(ref)\]""")
            .replace("label", I_BOUNDED_INLINE_LABEL)
            .replace("ref", I_BOUNDED_BLOCK_LABEL)
            .getRegex(),
    )
    .replace(
        "nolink",
        edit("""^!?\[(ref)\](?:\[\])?""")
            .replace("ref", I_BOUNDED_BLOCK_LABEL)
            .getRegex(),
    )
    .getRegex()

private val I_CASE_INSENSITIVE_PROTOCOL: String = """[hH][tT][tT][pP][sS]?|[fF][tT][pP]"""
private val I_EXTENDED_EMAIL: String = """[A-Za-z0-9._+-]+@[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])"""
private val I_EXTENDED_EMAIL_PROTOCOL: RegExp = edit("""(?:mailto:email|xmpp:email(?:\/[A-Za-z0-9@.]+)?)""")
    .replace(RegExp("email", "g"), I_EXTENDED_EMAIL)
    .getRegex()

val INLINE_NORMAL: InlineRules = InlineRules(
    backpedal = null,
    anyPunctuation = I_ANY_PUNCTUATION,
    anyPunctuationOutsideCode = I_ANY_PUNCTUATION_OUTSIDE_CODE,
    autolink = I_AUTOLINK,
    blockSkip = I_BLOCK_SKIP,
    br = I_BR,
    code = I_CODE,
    del = null,
    delLDelim = null,
    delRDelim = null,
    emStrongLDelim = I_EM_STRONG_LDELIM,
    emStrongRDelimAst = I_EM_STRONG_RDELIM_AST,
    emStrongRDelimUnd = I_EM_STRONG_RDELIM_UND,
    escape = I_ESCAPE,
    link = I_LINK,
    nolink = I_NOLINK,
    punctuation = I_PUNCTUATION,
    reflink = I_REFLINK,
    reflinkSearch = I_REFLINK_SEARCH,
    tag = I_TAG,
    text = I_TEXT,
    url = null,
)

val INLINE_PEDANTIC: InlineRules = InlineRules(
    backpedal = null,
    anyPunctuation = I_ANY_PUNCTUATION,
    anyPunctuationOutsideCode = I_ANY_PUNCTUATION_OUTSIDE_CODE,
    autolink = I_AUTOLINK,
    blockSkip = I_BLOCK_SKIP,
    br = I_BR,
    code = I_CODE,
    del = null,
    delLDelim = null,
    delRDelim = null,
    emStrongLDelim = I_EM_STRONG_LDELIM_PEDANTIC,
    emStrongRDelimAst = I_EM_STRONG_RDELIM_AST_PEDANTIC,
    emStrongRDelimUnd = I_EM_STRONG_RDELIM_UND_PEDANTIC,
    escape = I_ESCAPE,
    link = edit("""^!?\[(label)\]\((.*?)\)""")
        .replace("label", I_LABEL)
        .getRegex(),
    nolink = I_NOLINK,
    punctuation = I_PUNCTUATION,
    reflink = edit("""^!?\[(label)\]\s*\[([^\]]*)\]""")
        .replace("label", I_LABEL)
        .getRegex(),
    reflinkSearch = I_REFLINK_SEARCH,
    tag = I_TAG,
    text = I_TEXT,
    url = null,
)

private val I_GFM_URL: RegExp = edit("""^emailProtocol|^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email""")
    .replace("emailProtocol", I_EXTENDED_EMAIL_PROTOCOL)
    .replace("protocol", I_CASE_INSENSITIVE_PROTOCOL)
    .replace("email", """[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])""")
    .getRegex()

private val I_GFM_BACKPEDAL: RegExp =
    RegExp("""(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+""")

private val I_GFM_DEL: RegExp =
    RegExp("""^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)""")

private val I_GFM_TEXT: RegExp = edit(
    """^(?:[^a-zA-Z0-9](?=emailProtocol)|(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9](?=emailProtocol)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@))))""",
)
    .replace("protocol", I_CASE_INSENSITIVE_PROTOCOL)
    .replace(RegExp("emailProtocol", "g"), """(?:mailto|xmpp):""")
    .getRegex()

val INLINE_GFM: InlineRules = InlineRules(
    backpedal = I_GFM_BACKPEDAL,
    anyPunctuation = I_ANY_PUNCTUATION,
    anyPunctuationOutsideCode = I_ANY_PUNCTUATION_OUTSIDE_CODE,
    autolink = I_AUTOLINK,
    blockSkip = I_BLOCK_SKIP,
    br = I_BR,
    code = I_CODE,
    del = I_GFM_DEL,
    delLDelim = I_DEL_LDELIM,
    delRDelim = I_DEL_RDELIM,
    emStrongLDelim = I_EM_STRONG_LDELIM_GFM,
    emStrongRDelimAst = I_EM_STRONG_RDELIM_AST_GFM,
    emStrongRDelimUnd = I_EM_STRONG_RDELIM_UND,
    escape = I_ESCAPE,
    link = I_LINK,
    nolink = I_NOLINK,
    punctuation = I_PUNCTUATION,
    reflink = I_REFLINK,
    reflinkSearch = I_REFLINK_SEARCH,
    tag = I_TAG,
    text = I_GFM_TEXT,
    url = I_GFM_URL,
)

val INLINE_BREAKS: InlineRules = InlineRules(
    backpedal = I_GFM_BACKPEDAL,
    anyPunctuation = I_ANY_PUNCTUATION,
    anyPunctuationOutsideCode = I_ANY_PUNCTUATION_OUTSIDE_CODE,
    autolink = I_AUTOLINK,
    blockSkip = I_BLOCK_SKIP,
    br = edit(I_BR).replace("{2,}", "*").getRegex(),
    code = I_CODE,
    del = I_GFM_DEL,
    delLDelim = I_DEL_LDELIM,
    delRDelim = I_DEL_RDELIM,
    emStrongLDelim = I_EM_STRONG_LDELIM_GFM,
    emStrongRDelimAst = I_EM_STRONG_RDELIM_AST_GFM,
    emStrongRDelimUnd = I_EM_STRONG_RDELIM_UND,
    escape = I_ESCAPE,
    link = I_LINK,
    nolink = I_NOLINK,
    punctuation = I_PUNCTUATION,
    reflink = I_REFLINK,
    reflinkSearch = I_REFLINK_SEARCH,
    tag = I_TAG,
    text = edit(I_GFM_TEXT)
        .replace("""\b_""", """\b_| {2,}\n""")
        .replace(RegExp("""\{2,\}""", "g"), "*")
        .getRegex(),
    url = I_GFM_URL,
)

val RULES_NORMAL: Rules = Rules(OTHER_RULES, BLOCK_NORMAL, INLINE_NORMAL)
val RULES_GFM: Rules = Rules(OTHER_RULES, BLOCK_GFM, INLINE_GFM)
val RULES_BREAKS: Rules = Rules(OTHER_RULES, BLOCK_GFM, INLINE_BREAKS)
val RULES_PEDANTIC: Rules = Rules(OTHER_RULES, BLOCK_PEDANTIC, INLINE_PEDANTIC)
