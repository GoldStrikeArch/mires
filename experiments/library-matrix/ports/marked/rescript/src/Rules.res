// Grammar rules for marked@18.1.1 (ported faithfully from `src/rules.ts`).
// All regex tables are immutable module-level constants compiled once at module load,
// with zero mutable cross-call state (the 4 indent-clamped variants for each list-interrupt
// regex are built eagerly at module init instead of lazily cached).

// Zero-cost JS RegExp / String interop helpers (avoiding Primitive_option boxing on hot exec calls).
@send external execNull: (RegExp.t, string) => Null.t<RegExp.Result.t> = "exec"
@get_index external groupOpt: (RegExp.Result.t, int) => option<string> = ""
@get_index external groupUnsafe: (RegExp.Result.t, int) => string = ""

@send external replaceBy1: (string, RegExp.t, string => string) => string = "replace"
@send
external replaceBy0OffsetInput: (string, RegExp.t, (string, int, string) => string) => string =
  "replace"
@send
external replaceBy2Opt: (
  string,
  RegExp.t,
  (string, option<string>, option<string>) => string,
) => string = "replace"

let caretRe = /(^|[^\[])\^/g

type editor = {
  mutable source: string,
  opt: string,
}

let edit = (source: string, ~opt: string=""): editor => {source, opt}
let editRe = (re: RegExp.t, ~opt: string=""): editor => {source: re->RegExp.source, opt}

let replaceStr = (ed: editor, name: string, val: string): editor => {
  let valSource = val->String.replaceRegExp(caretRe, "$1")
  ed.source = ed.source->String.replace(name, valSource)
  ed
}

let replaceStrRe = (ed: editor, name: string, val: RegExp.t): editor =>
  replaceStr(ed, name, val->RegExp.source)

let replaceRe = (ed: editor, name: RegExp.t, val: string): editor => {
  let valSource = val->String.replaceRegExp(caretRe, "$1")
  ed.source = ed.source->String.replaceRegExp(name, valSource)
  ed
}

let replaceReRe = (ed: editor, name: RegExp.t, val: RegExp.t): editor =>
  replaceRe(ed, name, val->RegExp.source)

let getRegex = (ed: editor): RegExp.t => RegExp.fromString(ed.source, ~flags=ed.opt)

let supportsLookbehind: bool = try {
  ignore(RegExp.fromString("(?<=1)(?<!1)"))
  true
} catch {
| _ => false
}

let _tag =
  "address|article|aside|base|basefont|blockquote|body|caption" ++
  "|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption" ++
  "|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe" ++
  "|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option" ++
  "|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title" ++
  "|tr|track|ul"

let clampIndentIndex = (indent: int): int => {
  let idx = indent - 1
  if idx < 0 {
    0
  } else if idx > 3 {
    3
  } else {
    idx
  }
}

let makeIndentTable = (create: int => RegExp.t): array<RegExp.t> => [
  create(0),
  create(1),
  create(2),
  create(3),
]

let nextBulletTable = makeIndentTable(indent =>
  RegExp.fromString(
    "^ {0," ++ Int.toString(indent) ++ "}(?:[*+-]|\\d{1,9}[.)])((?:[ \t][^\\n]*)?(?:\\n|$))",
  )
)

let hrTable = makeIndentTable(indent =>
  RegExp.fromString(
    "^ {0," ++
    Int.toString(indent) ++
    "}((?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\\*[ \t]*){3,})(?:\\n+|$)",
  )
)

let fencesBeginTable = makeIndentTable(indent =>
  RegExp.fromString("^ {0," ++ Int.toString(indent) ++ "}(?:```|~~~)")
)

let headingBeginTable = makeIndentTable(indent =>
  RegExp.fromString("^ {0," ++ Int.toString(indent) ++ "}#")
)

let htmlBeginTable = makeIndentTable(indent =>
  RegExp.fromString(
    "^ {0," ++
    Int.toString(indent) ++
    "}(?:</?(?:" ++
    _tag ++
    ")(?: +|$|/?>)|<(?:(?:script|pre|style|textarea)(?:[\\s>]|$)|!--)|<\\?|<![A-Za-z]|<!\\[CDATA\\[)",
    ~flags="i",
  )
)

let blockquoteBeginTable = makeIndentTable(indent =>
  RegExp.fromString("^ {0," ++ Int.toString(indent) ++ "}>")
)

let listItemRegex = (bull: string): RegExp.t =>
  RegExp.fromString("^( {0,3}" ++ bull ++ ")((?:[\t ][^\\n]*)?(?:\\n|$))")

let nextBulletRegex = (indent: int): RegExp.t =>
  nextBulletTable->Array.getUnsafe(clampIndentIndex(indent))

let hrRegex = (indent: int): RegExp.t => hrTable->Array.getUnsafe(clampIndentIndex(indent))

let fencesBeginRegex = (indent: int): RegExp.t =>
  fencesBeginTable->Array.getUnsafe(clampIndentIndex(indent))

let headingBeginRegex = (indent: int): RegExp.t =>
  headingBeginTable->Array.getUnsafe(clampIndentIndex(indent))

let htmlBeginRegex = (indent: int): RegExp.t =>
  htmlBeginTable->Array.getUnsafe(clampIndentIndex(indent))

let blockquoteBeginRegex = (indent: int): RegExp.t =>
  blockquoteBeginTable->Array.getUnsafe(clampIndentIndex(indent))

// Other helper regexes (immutable constants)
let codeRemoveIndent = /^(?: {0,3}\t| {1,4})/gm
let outputLinkReplace = /\\([\[\]])/g
let indentCodeCompensation = /^(\s+)(?:```)/
let beginningSpace = /^\s+/
let endingHash = /#$/
let startingSpaceChar = /^ /
let endingSpaceChar = / $/
let endingSpaceTabChar = /[ \t]$/
let nonSpaceChar = /[^ ]/
let newLineCharGlobal = /\n/g
let tabCharGlobal = /\t/g
let leadingSpaceTab = /^[ \t]+/
let multipleSpaceGlobal = /\s+/g
let blankLine = /^[ \t]*$/
let doubleBlankLine = /\n[ \t]*\n[ \t]*$/
let blockquoteStart = /^ {0,3}>/
let blockquoteSetextReplace = /\n {0,3}((?:=+|-+) *)(?=\n|$)/g
let blockquoteSetextReplace2 = /^ {0,3}>[ \t]?/gm
let listReplaceNesting = /^ {1,4}(?=( {4})*[^ ])/g
let listIsTask = /^\[[ xX]\] +\S/
let listReplaceTask = /^\[[ xX]\] +/
let listTaskCheckbox = /\[[ xX]\]/
let anyLine = /\n.*\n/
let hrefBrackets = /^<(.*)>$/
let tableDelimiter = /[:|]/
let tableAlignChars = /^\||\| *$/g
let tableRowBlankLine = /\n[ \t]*$/
let tableAlignRight = /^ *-+: *$/
let tableAlignCenter = /^ *:-+: *$/
let tableAlignLeft = /^ *:-+ *$/
let startATag = /^<a /i
let endATag = /^<\/a>/i
let startPreScriptTag = /^<(pre|code|kbd|script)(\s|>)/i
let endPreScriptTag = /^<\/(pre|code|kbd|script)(\s|>)/i
let startAngleBracket = /^</
let endAngleBracket = />$/
let pedanticHrefTitle = /^([^'"]*[^\s])\s+(['"])(.*)\2/
let unicodeAlphaNumeric = /[\p{L}\p{N}]/u
let numericCharacterReference = /&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));/
let numericCharacterReferenceOrAmpersand =
  /&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/g
let escapeTest = /[&<>"']/
let escapeReplace = /[&<>"']/g
let escapeTestNoEncode = /[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/
let escapeReplaceNoEncode = /[<>"']|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)/g
let percentDecode = /%25/g
let findPipe = /\|/g
let slashPipe = /\\\|/g
let carriageReturn = /\r\n|\r/g
let spaceLine = /^ +$/gm
let notSpaceStart = /^\S*/
let endingNewline = /\n$/
let backtickGlobal = /`/g

/**
 * Block-Level Grammar
 */
let newline = /^(?:[ \t]*(?:\n|$))+/
let blockCode = /^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+/
let fences =
  /^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)/
let hr = /^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)/
let heading = /^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)/
let bullet = / {0,3}(?:[*+-]|\d{1,9}[.)])/
let lheadingCore =
  /^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |fences|blockquote|heading|hr|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)/

let lheading =
  editRe(lheadingCore)
  ->replaceReRe(/bull/g, bullet)
  ->replaceReRe(/blockCode/g, /(?: {4}| {0,3}\t)/)
  ->replaceReRe(/fences/g, / {0,3}(?:`{3,}|~{3,})/)
  ->replaceReRe(/blockquote/g, / {0,3}>/)
  ->replaceReRe(/heading/g, / {0,3}#{1,6}(?:\s|$)/)
  ->replaceReRe(/hr/g, / {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)/)
  ->replaceReRe(/html/g, / {0,3}<[^\n>]+>\n/)
  ->replaceRe(/\|table/g, "")
  ->getRegex

let lheadingGfm =
  editRe(lheadingCore)
  ->replaceReRe(/bull/g, bullet)
  ->replaceReRe(/blockCode/g, /(?: {4}| {0,3}\t)/)
  ->replaceReRe(/fences/g, / {0,3}(?:`{3,}|~{3,})/)
  ->replaceReRe(/blockquote/g, / {0,3}>/)
  ->replaceReRe(/heading/g, / {0,3}#{1,6}(?:\s|$)/)
  ->replaceReRe(/hr/g, / {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)/)
  ->replaceReRe(/html/g, / {0,3}<[^\n>]+>\n/)
  ->replaceReRe(/table/g, / {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n/)
  ->getRegex

let _paragraph = /^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)/
let blockText = /^[^\n]+/
let _blockLabel = /(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+/
let def =
  editRe(/^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)/)
  ->replaceStrRe("label", _blockLabel)
  ->replaceStrRe("title", /(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))/)
  ->getRegex

let list = editRe(/^(bull)([ \t][^\n]*?)?(?:\n|$)/)->replaceReRe(/bull/g, bullet)->getRegex

let _comment = /<!--(?:-?>|[\s\S]*?(?:-->|$))/
let html =
  edit(
    "^ {0,3}(?:" ++
    "<(script|pre|style|textarea)[\\s>][\\s\\S]*?(?:</\\1>[^\\n]*\\n*|$)" ++
    "|comment[^\\n]*(\\n+|$)" ++
    "|<\\?[\\s\\S]*?(?:\\?>[^\\n]*\\n*|$)" ++
    "|<![A-Z][\\s\\S]*?(?:>[^\\n]*\\n*|$)" ++
    "|<!\\[CDATA\\[[\\s\\S]*?(?:\\]\\]>[^\\n]*\\n*|$)" ++
    "|</?(tag)(?: +|\\n|/?>)[\\s\\S]*?(?:(?:\\n[ \t]*)+\\n|$)" ++
    "|<(?!(?:script|pre|style|textarea)(?![a-z0-9-]))([a-z][a-z0-9-]*)(?:attribute)*? */?>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ \t]*)+\\n|$)" ++
    "|</[a-z][a-z0-9-]*\\s*>(?=[ \\t]*(?:\\n|$))[\\s\\S]*?(?:(?:\\n[ \t]*)+\\n|$)" ++
    ")",
    ~opt="i",
  )
  ->replaceStrRe("comment", _comment)
  ->replaceStr("tag", _tag)
  ->replaceStrRe(
    "attribute",
    / +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?/,
  )
  ->getRegex

let createParagraph = (listInterrupt: RegExp.t): RegExp.t =>
  editRe(_paragraph)
  ->replaceStrRe("hr", hr)
  ->replaceStr("heading", " {0,3}#{1,6}(?:\\s|$)")
  ->replaceStr("|lheading", "")
  ->replaceStr("|table", "")
  ->replaceStr("blockquote", " {0,3}>")
  ->replaceStr("fences", " {0,3}(?:`{3,}(?=[^`\\n]*(?:\\n|$))|~~~)[^\\n]*(?:\\n|$)")
  ->replaceStrRe("list", listInterrupt)
  ->replaceStr(
    "html",
    " {0,3}(?:</?(?:tag)(?: +|\\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\\s>]|$)|!--)|<\\?|<![A-Za-z]|<!\\[CDATA\\[)",
  )
  ->replaceStr("tag", _tag)
  ->getRegex

let paragraph = createParagraph(/ {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]/)
let blockquoteParagraph = createParagraph(/ {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)/)

let blockquote =
  editRe(/^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+/)
  ->replaceStrRe("paragraph", blockquoteParagraph)
  ->getRegex

let gfmTable =
  edit(
    "^ *([^\\n ].*)\\n" ++
    " {0,3}((?:\\| *)?:?-+:? *(?:\\| *:?-+:? *)*(?:\\| *)?)" ++
    "(?:\\n((?:(?! *\\n|hr|heading|blockquote|code|fences|list|html).*(?:\\n|$))*)\\n*|$)",
  )
  ->replaceStrRe("hr", hr)
  ->replaceStr("heading", " {0,3}#{1,6}(?:\\s|$)")
  ->replaceStr("blockquote", " {0,3}>")
  ->replaceStr("code", "(?: {4}| {0,3}\t)[^\\n]")
  ->replaceStr("fences", " {0,3}(?:`{3,}(?=[^`\\n]*(?:\\n|$))|~~~)[^\\n]*(?:\\n|$)")
  ->replaceStr("list", " {0,3}(?:[*+-]|1[.)])[ \\t]")
  ->replaceStr(
    "html",
    " {0,3}(?:</?(?:tag)(?: +|\\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\\s>]|$)|!--)|<\\?|<![A-Za-z]|<!\\[CDATA\\[)",
  )
  ->replaceStr("tag", _tag)
  ->getRegex

type blockRules = {
  blockquote: RegExp.t,
  code: RegExp.t,
  def: RegExp.t,
  fences: option<RegExp.t>,
  heading: RegExp.t,
  hr: RegExp.t,
  html: RegExp.t,
  lheading: RegExp.t,
  list: RegExp.t,
  newline: RegExp.t,
  paragraph: RegExp.t,
  table: option<RegExp.t>,
  text: RegExp.t,
}

let blockNormal: blockRules = {
  blockquote,
  code: blockCode,
  def,
  fences: Some(fences),
  heading,
  hr,
  html,
  lheading,
  list,
  newline,
  paragraph,
  table: None,
  text: blockText,
}

let blockGfm: blockRules = {
  ...blockNormal,
  lheading: lheadingGfm,
  table: Some(gfmTable),
  paragraph: editRe(_paragraph)
  ->replaceStrRe("hr", hr)
  ->replaceStr("heading", " {0,3}#{1,6}(?:\\s|$)")
  ->replaceStr("|lheading", "")
  ->replaceStrRe("table", gfmTable)
  ->replaceStr("blockquote", " {0,3}>")
  ->replaceStr("fences", " {0,3}(?:`{3,}(?=[^`\\n]*(?:\\n|$))|~~~)[^\\n]*(?:\\n|$)")
  ->replaceStr("list", " {0,3}(?:[*+-]|1[.)])[ \\t]+[^ \\t\\n]")
  ->replaceStr(
    "html",
    " {0,3}(?:</?(?:tag)(?: +|\\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\\s>]|$)|!--)|<\\?|<![A-Za-z]|<!\\[CDATA\\[)",
  )
  ->replaceStr("tag", _tag)
  ->getRegex,
}

let blockPedantic: blockRules = {
  ...blockNormal,
  html: edit(
    "^ *(?:comment *(?:\\n|\\s*$)" ++
    "|<(tag)[\\s\\S]+?</\\1> *(?:\\n{2,}|\\s*$)" ++
    "|<tag(?:\"[^\"]*\"|'[^']*'|\\s[^'\"/>\\s]*)*?/?> *(?:\\n{2,}|\\s*$))",
  )
  ->replaceStrRe("comment", _comment)
  ->replaceRe(
    /tag/g,
    "(?!(?:" ++
    "a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub" ++
    "|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)" ++
    "\\b)\\w+(?!:|[^\\w\\s@]*@)\\b",
  )
  ->getRegex,
  def: /^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)/,
  heading: /^(#{1,6})(.*)(?:\n+|$)/,
  fences: None,
  lheading: /^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)/,
  paragraph: editRe(_paragraph)
  ->replaceStrRe("hr", hr)
  ->replaceStr("heading", " *#{1,6} *[^\n]")
  ->replaceStrRe("lheading", lheading)
  ->replaceStr("|table", "")
  ->replaceStr("blockquote", " {0,3}>")
  ->replaceStr("|fences", "")
  ->replaceStr("|list", "")
  ->replaceStr("|html", "")
  ->replaceStr("|tag", "")
  ->getRegex,
}

/**
 * Inline-Level Grammar
 */
let escape = /^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])/
let inlineCode = /^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)/
let br = /^( {2,}|\\)\n(?!\s*$)[ \t]*/
let inlineText = /^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))/

let _punctuation = /[\p{P}\p{S}]/u
let _punctuationOrSpace = /[\s\p{P}\p{S}]/u
let _notPunctuationOrSpace = /[^\s\p{P}\p{S}]/u
let punctuation =
  editRe(/^((?![*_])punctSpace)/, ~opt="u")
  ->replaceReRe(/punctSpace/g, _punctuationOrSpace)
  ->getRegex

let _openQuote = /[\p{Pi}\p{Ps}"']/u

let _punctuationGfmStrongEm = /(?!~)[\p{P}\p{S}]/u
let _punctuationOrSpaceGfmStrongEm = /(?!~)[\s\p{P}\p{S}]/u
let _notPunctuationOrSpaceGfmStrongEm = /(?:[^\s\p{P}\p{S}]|~)/u

let blockSkip =
  editRe(/link|precode-code|html/, ~opt="g")
  ->replaceStr(
    "link",
    "\\[(?:[^\\[\\]`]|(?<a>`+)[^`]+\\k<a>(?!`))*?\\]\\((?:\\\\[\\s\\S]|[^\\\\\\(\\)]|\\((?:\\\\[\\s\\S]|[^\\\\\\(\\)])*\\))*\\)",
  )
  ->replaceStr("precode-", supportsLookbehind ? "(?<!`)()" : "(^^|[^`])")
  ->replaceStr("code", "(?<b>`+)[^`]+\\k<b>(?!`)")
  ->replaceStrRe("html", /<(?! )[^<>]*?>/)
  ->getRegex

let emStrongLDelimCore = /^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?/

let emStrongLDelim =
  editRe(emStrongLDelimCore, ~opt="u")->replaceReRe(/punct/g, _punctuation)->getRegex

let emStrongLDelimGfm =
  editRe(emStrongLDelimCore, ~opt="u")->replaceReRe(/punct/g, _punctuationGfmStrongEm)->getRegex

let emStrongLDelimPedanticCore =
  /^(?:\*+(?:((?!\*)(?!openQuote)punct)|([^\s*]))?)|^_+(?:((?!_)(?!openQuote)punct)|([^\s_]))?/

let emStrongLDelimPedantic =
  editRe(emStrongLDelimPedanticCore, ~opt="u")
  ->replaceReRe(/openQuote/g, _openQuote)
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let emStrongRDelimAstCore =
  "^[^_*]*?__[^_*]*?\\*[^_*]*?(?=__)" ++
  "|[^*]+(?=[^*])" ++
  "|(?!\\*)punct(\\*+)(?=[\\s]|$)" ++
  "|notPunctSpace(\\*+)(?!\\*)(?=punctSpace|$)" ++
  "|(?!\\*)punctSpace(\\*+)(?=notPunctSpace)" ++
  "|[\\s](\\*+)(?!\\*)(?=punct)" ++
  "|(?!\\*)punct(\\*+)(?!\\*)(?=punct)" ++
  "|notPunctSpace(\\*+)(?=notPunctSpace)"

let emStrongRDelimAst =
  edit(emStrongRDelimAstCore, ~opt="gu")
  ->replaceReRe(/notPunctSpace/g, _notPunctuationOrSpace)
  ->replaceReRe(/punctSpace/g, _punctuationOrSpace)
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let emStrongRDelimAstGfm =
  edit(emStrongRDelimAstCore, ~opt="gu")
  ->replaceReRe(/notPunctSpace/g, _notPunctuationOrSpaceGfmStrongEm)
  ->replaceReRe(/punctSpace/g, _punctuationOrSpaceGfmStrongEm)
  ->replaceReRe(/punct/g, _punctuationGfmStrongEm)
  ->getRegex

let emStrongRDelimAstPedanticCore =
  "^[^_*]*?__[^_*]*?\\*[^_*]*?(?=__)" ++
  "|[^*]+(?=[^*])" ++
  "|(?!\\*)punct(\\*+)(?=[\\s]|$)" ++
  "|notPunctSpace(\\*+)(?!\\*)(?=punctSpace|$)" ++
  "|(?!\\*)[\\s](\\*+)(?=notPunctSpace)" ++
  "|[\\s](\\*+)(?!\\*)(?=punct)" ++
  "|(?!\\*)punct(\\*+)(?!\\*)(?=punct)" ++
  "|(?:(?!\\*)punct|notPunctSpace)(\\*+)(?!\\*)(?=notPunctSpace)"

let emStrongRDelimAstPedantic =
  edit(emStrongRDelimAstPedanticCore, ~opt="gu")
  ->replaceReRe(/notPunctSpace/g, _notPunctuationOrSpace)
  ->replaceReRe(/punctSpace/g, _punctuationOrSpace)
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let emStrongRDelimUnd =
  edit(
    "^[^_*]*?\\*\\*[^_*]*?_[^_*]*?(?=\\*\\*)" ++
    "|[^_]+(?=[^_])" ++
    "|(?!_)punct(_+)(?=[\\s]|$)" ++
    "|notPunctSpace(_+)(?!_)(?=punctSpace|$)" ++
    "|(?!_)punctSpace(_+)(?=notPunctSpace)" ++
    "|[\\s](_+)(?!_)(?=punct)" ++
    "|(?!_)punct(_+)(?!_)(?=punct)",
    ~opt="gu",
  )
  ->replaceReRe(/notPunctSpace/g, _notPunctuationOrSpace)
  ->replaceReRe(/punctSpace/g, _punctuationOrSpace)
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let emStrongRDelimUndPedanticCore =
  "^[^_*]*?\\*\\*[^_*]*?_[^_*]*?(?=\\*\\*)" ++
  "|[^_]+(?=[^_])" ++
  "|(?!_)punct(_+)(?=[\\s]|$)" ++
  "|notPunctSpace(_+)(?!_)(?=punctSpace|$)" ++
  "|(?!_)[\\s](_+)(?=notPunctSpace)" ++
  "|[\\s](_+)(?!_)(?=punct)" ++
  "|(?!_)punct(_+)(?!_)(?=punct)" ++
  "|(?:(?!_)punct|notPunctSpace)(_+)(?!_)(?=notPunctSpace)"

let emStrongRDelimUndPedantic =
  edit(emStrongRDelimUndPedanticCore, ~opt="gu")
  ->replaceReRe(/notPunctSpace/g, _notPunctuationOrSpace)
  ->replaceReRe(/punctSpace/g, _punctuationOrSpace)
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let delLDelim =
  editRe(/^~~?(?:((?!~)punct)|[^\s~])/, ~opt="u")->replaceReRe(/punct/g, _punctuation)->getRegex

let delRDelimCore =
  "^[^~]+(?=[^~])" ++
  "|(?!~)punct(~~?)(?=[\\s]|$)" ++
  "|notPunctSpace(~~?)(?!~)(?=punctSpace|$)" ++
  "|(?!~)punctSpace(~~?)(?=notPunctSpace)" ++
  "|[\\s](~~?)(?!~)(?=punct)" ++
  "|(?!~)punct(~~?)(?!~)(?=punct)" ++
  "|notPunctSpace(~~?)(?=notPunctSpace)"

let delRDelim =
  edit(delRDelimCore, ~opt="gu")
  ->replaceReRe(/notPunctSpace/g, _notPunctuationOrSpace)
  ->replaceReRe(/punctSpace/g, _punctuationOrSpace)
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let anyPunctuation =
  editRe(/\\(punct)/, ~opt="gu")->replaceReRe(/punct/g, _punctuation)->getRegex

let anyPunctuationOutsideCode =
  editRe(/(`+)(?:[^`]|[^`][\s\S]*?[^`])\1(?!`)|`+|\\(punct)/, ~opt="gu")
  ->replaceReRe(/punct/g, _punctuation)
  ->getRegex

let autolink =
  editRe(/^<(scheme:[^\s\x00-\x1f<>]*|email)>/)
  ->replaceStrRe("scheme", /[a-zA-Z][a-zA-Z0-9+.-]{1,31}/)
  ->replaceStrRe(
    "email",
    /[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])/,
  )
  ->getRegex

let _inlineComment = editRe(_comment)->replaceStr("(?:-->|$)", "-->")->getRegex
let tag =
  edit(
    "^comment" ++
    "|^</[a-zA-Z][a-zA-Z0-9-]*\\s*>" ++
    "|^<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\\s*/?>" ++
    "|^<\\?[\\s\\S]*?\\?>" ++
    "|^<![a-zA-Z]+\\s[\\s\\S]*?>" ++
    "|^<!\\[CDATA\\[[\\s\\S]*?\\]\\]>",
  )
  ->replaceStrRe("comment", _inlineComment)
  ->replaceStrRe(
    "attribute",
    /\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?/,
  )
  ->getRegex

let _inlineLabelBrackets = /\[(?:\\[\s\S]|[^\[\]\\])*\]/
let _inlineLabel =
  editRe(/(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?/)
  ->replaceStrRe("brackets", _inlineLabelBrackets)
  ->getRegex

let link =
  editRe(/^!?\[(label)\]\([ \t\n]*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?[ \t\n]*\)/)
  ->replaceStrRe("label", _inlineLabel)
  ->replaceStrRe("href", /<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))/)
  ->replaceStrRe("title", /"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)/)
  ->getRegex

let reflink =
  editRe(/^!?\[(label)\]\[(ref)\]/)
  ->replaceStrRe("label", _inlineLabel)
  ->replaceStrRe("ref", _blockLabel)
  ->getRegex

let nolink = editRe(/^!?\[(ref)\](?:\[\])?/)->replaceStrRe("ref", _blockLabel)->getRegex

let _boundedBlockLabel = /(?!\s*\])(?:\\[\s\S]|[^\[\]\\]){1,999}/
let _boundedInlineLabel =
  editRe(/(?:[^\[\]\\`]*(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\]))){0,999}?[^\[\]\\`]*?/)
  ->replaceStrRe("brackets", _inlineLabelBrackets)
  ->getRegex

let reflinkSearch =
  edit("reflink|nolink(?!\\()", ~opt="g")
  ->replaceStrRe(
    "reflink",
    editRe(/^!?\[(label)\]\[(ref)\]/)
    ->replaceStrRe("label", _boundedInlineLabel)
    ->replaceStrRe("ref", _boundedBlockLabel)
    ->getRegex,
  )
  ->replaceStrRe(
    "nolink",
    editRe(/^!?\[(ref)\](?:\[\])?/)->replaceStrRe("ref", _boundedBlockLabel)->getRegex,
  )
  ->getRegex

let _caseInsensitiveProtocol = /[hH][tT][tT][pP][sS]?|[fF][tT][pP]/
let _extendedEmail =
  /[A-Za-z0-9._+-]+@[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])/
let _extendedEmailProtocol =
  editRe(/(?:mailto:email|xmpp:email(?:\/[A-Za-z0-9@.]+)?)/)
  ->replaceReRe(/email/g, _extendedEmail)
  ->getRegex

type inlineRules = {
  _backpedal: option<RegExp.t>,
  anyPunctuation: RegExp.t,
  anyPunctuationOutsideCode: RegExp.t,
  autolink: RegExp.t,
  blockSkip: RegExp.t,
  br: RegExp.t,
  code: RegExp.t,
  del: option<RegExp.t>,
  delLDelim: option<RegExp.t>,
  delRDelim: option<RegExp.t>,
  emStrongLDelim: RegExp.t,
  emStrongRDelimAst: RegExp.t,
  emStrongRDelimUnd: RegExp.t,
  escape: RegExp.t,
  link: RegExp.t,
  nolink: RegExp.t,
  punctuation: RegExp.t,
  reflink: RegExp.t,
  reflinkSearch: RegExp.t,
  tag: RegExp.t,
  text: RegExp.t,
  url: option<RegExp.t>,
}

let inlineNormal: inlineRules = {
  _backpedal: None,
  anyPunctuation,
  anyPunctuationOutsideCode,
  autolink,
  blockSkip,
  br,
  code: inlineCode,
  del: None,
  delLDelim: None,
  delRDelim: None,
  emStrongLDelim,
  emStrongRDelimAst,
  emStrongRDelimUnd,
  escape,
  link,
  nolink,
  punctuation,
  reflink,
  reflinkSearch,
  tag,
  text: inlineText,
  url: None,
}

let inlinePedantic: inlineRules = {
  ...inlineNormal,
  emStrongLDelim: emStrongLDelimPedantic,
  emStrongRDelimAst: emStrongRDelimAstPedantic,
  emStrongRDelimUnd: emStrongRDelimUndPedantic,
  link: editRe(/^!?\[(label)\]\((.*?)\)/)->replaceStrRe("label", _inlineLabel)->getRegex,
  reflink: editRe(/^!?\[(label)\]\s*\[([^\]]*)\]/)->replaceStrRe("label", _inlineLabel)->getRegex,
}

let inlineGfm: inlineRules = {
  ...inlineNormal,
  emStrongRDelimAst: emStrongRDelimAstGfm,
  emStrongLDelim: emStrongLDelimGfm,
  delLDelim: Some(delLDelim),
  delRDelim: Some(delRDelim),
  url: Some(
    editRe(/^emailProtocol|^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email/)
    ->replaceStrRe("emailProtocol", _extendedEmailProtocol)
    ->replaceStrRe("protocol", _caseInsensitiveProtocol)
    ->replaceStrRe(
      "email",
      /[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])/,
    )
    ->getRegex,
  ),
  _backpedal: Some(/(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+/),
  del: Some(/^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)/),
  text: editRe(
    /^(?:[^a-zA-Z0-9](?=emailProtocol)|(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9](?=emailProtocol)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@))))/,
  )
  ->replaceStrRe("protocol", _caseInsensitiveProtocol)
  ->replaceReRe(/emailProtocol/g, /(?:mailto|xmpp):/)
  ->getRegex,
}

let inlineBreaks: inlineRules = {
  ...inlineGfm,
  br: editRe(br)->replaceStr("{2,}", "*")->getRegex,
  text: editRe(inlineGfm.text)
  ->replaceStr("\\b_", "\\b_| {2,}\\n")
  ->replaceRe(/\{2,\}/g, "*")
  ->getRegex,
}

type rules = {
  block: blockRules,
  inline: inlineRules,
}

let rulesNormal: rules = {block: blockNormal, inline: inlineNormal}
let rulesGfm: rules = {block: blockGfm, inline: inlineGfm}
let rulesBreaks: rules = {block: blockGfm, inline: inlineBreaks}
let rulesPedantic: rules = {block: blockPedantic, inline: inlinePedantic}

let selectRules = (~gfm: bool, ~breaks: bool, ~pedantic: bool): rules =>
  if pedantic {
    rulesPedantic
  } else if gfm {
    if breaks {
      rulesBreaks
    } else {
      rulesGfm
    }
  } else {
    rulesNormal
  }
