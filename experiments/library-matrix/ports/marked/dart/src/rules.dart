/// Compiled grammar rules matching `marked@18.1.1` `src/rules.ts` verbatim.
///
/// All immutable compiled `RegExp` tables are grouped into a single module-level
/// `final` instance [markedRules], matching `marked`'s own module-level tables
/// and avoiding repeated lazy-initializer checks in dart2js.
final markedRules = MarkedRules._init();

final class _RuleEditor {
  _RuleEditor(
    this._source,
    this._caretRe, {
    bool caseSensitive = true,
    bool multiLine = false,
    bool unicode = false,
  }) : _caseSensitive = caseSensitive,
       _multiLine = multiLine,
       _unicode = unicode;

  String _source;
  final RegExp _caretRe;
  final bool _caseSensitive;
  final bool _multiLine;
  final bool _unicode;

  String _clean(String val) =>
      val.replaceAllMapped(_caretRe, (m) => m[1]!);

  _RuleEditor replaceFirst(String name, String val) {
    _source = _source.replaceFirst(name, _clean(val));
    return this;
  }

  _RuleEditor replaceAll(String name, String val) {
    _source = _source.replaceAll(name, _clean(val));
    return this;
  }

  RegExp getRegex() => RegExp(
    _source,
    caseSensitive: _caseSensitive,
    multiLine: _multiLine,
    unicode: _unicode,
  );
}

final class OtherRules {
  const OtherRules({
    required this.codeRemoveIndent,
    required this.outputLinkReplace,
    required this.indentCodeCompensation,
    required this.beginningSpace,
    required this.endingSpaceTabChar,
    required this.leadingSpaceTab,
    required this.multipleSpaceGlobal,
    required this.doubleBlankLine,
    required this.blockquoteStart,
    required this.blockquoteSetextReplace,
    required this.blockquoteSetextReplace2,
    required this.listReplaceNesting,
    required this.listIsTask,
    required this.listReplaceTask,
    required this.listTaskCheckbox,
    required this.anyLine,
    required this.hrefBrackets,
    required this.tableDelimiter,
    required this.tableAlignChars,
    required this.tableRowBlankLine,
    required this.tableAlignRight,
    required this.tableAlignCenter,
    required this.tableAlignLeft,
    required this.startATag,
    required this.endATag,
    required this.startPreScriptTag,
    required this.endPreScriptTag,
    required this.pedanticHrefTitle,
    required this.unicodeAlphaNumeric,
    required this.numericCharacterReference,
    required this.numericCharacterReferenceOrAmpersand,
    required this.carriageReturn,
    required this.spaceLine,
    required this.notSpaceStart,
    required this.nextBulletCache,
    required this.hrCache,
    required this.fencesBeginCache,
    required this.headingBeginCache,
    required this.htmlBeginCache,
    required this.blockquoteBeginCache,
  });

  final RegExp codeRemoveIndent;
  final RegExp outputLinkReplace;
  final RegExp indentCodeCompensation;
  final RegExp beginningSpace;
  final RegExp endingSpaceTabChar;
  final RegExp leadingSpaceTab;
  final RegExp multipleSpaceGlobal;
  final RegExp doubleBlankLine;
  final RegExp blockquoteStart;
  final RegExp blockquoteSetextReplace;
  final RegExp blockquoteSetextReplace2;
  final RegExp listReplaceNesting;
  final RegExp listIsTask;
  final RegExp listReplaceTask;
  final RegExp listTaskCheckbox;
  final RegExp anyLine;
  final RegExp hrefBrackets;
  final RegExp tableDelimiter;
  final RegExp tableAlignChars;
  final RegExp tableRowBlankLine;
  final RegExp tableAlignRight;
  final RegExp tableAlignCenter;
  final RegExp tableAlignLeft;
  final RegExp startATag;
  final RegExp endATag;
  final RegExp startPreScriptTag;
  final RegExp endPreScriptTag;
  final RegExp pedanticHrefTitle;
  final RegExp unicodeAlphaNumeric;
  final RegExp numericCharacterReference;
  final RegExp numericCharacterReferenceOrAmpersand;
  final RegExp carriageReturn;
  final RegExp spaceLine;
  final RegExp notSpaceStart;

  final List<RegExp> nextBulletCache;
  final List<RegExp> hrCache;
  final List<RegExp> fencesBeginCache;
  final List<RegExp> headingBeginCache;
  final List<RegExp> htmlBeginCache;
  final List<RegExp> blockquoteBeginCache;

  RegExp listItemRegex(String bull) =>
      RegExp('^( {0,3}$bull)((?:[\\t ][^\\n]*)?(?:\\n|\$))');

  static int _indentIndex(int indent) {
    final idx = indent - 1;
    return idx < 0 ? 0 : (idx > 3 ? 3 : idx);
  }

  RegExp nextBulletRegex(int indent) => nextBulletCache[_indentIndex(indent)];
  RegExp hrRegex(int indent) => hrCache[_indentIndex(indent)];
  RegExp fencesBeginRegex(int indent) => fencesBeginCache[_indentIndex(indent)];
  RegExp headingBeginRegex(int indent) =>
      headingBeginCache[_indentIndex(indent)];
  RegExp htmlBeginRegex(int indent) => htmlBeginCache[_indentIndex(indent)];
  RegExp blockquoteBeginRegex(int indent) =>
      blockquoteBeginCache[_indentIndex(indent)];
}

final class BlockRules {
  const BlockRules({
    required this.blockquote,
    required this.code,
    required this.def,
    required this.fences,
    required this.heading,
    required this.hr,
    required this.html,
    required this.lheading,
    required this.list,
    required this.newline,
    required this.paragraph,
    required this.table,
    required this.text,
  });

  final RegExp blockquote;
  final RegExp code;
  final RegExp def;
  final RegExp? fences;
  final RegExp heading;
  final RegExp hr;
  final RegExp html;
  final RegExp lheading;
  final RegExp list;
  final RegExp newline;
  final RegExp paragraph;
  final RegExp? table;
  final RegExp text;
}

final class InlineRules {
  const InlineRules({
    required this.backpedal,
    required this.anyPunctuation,
    required this.anyPunctuationOutsideCode,
    required this.autolink,
    required this.blockSkip,
    required this.br,
    required this.code,
    required this.del,
    required this.delLDelim,
    required this.delRDelim,
    required this.emStrongLDelim,
    required this.emStrongRDelimAst,
    required this.emStrongRDelimUnd,
    required this.escape,
    required this.link,
    required this.nolink,
    required this.punctuation,
    required this.reflink,
    required this.reflinkSearch,
    required this.tag,
    required this.text,
    required this.url,
  });

  final RegExp? backpedal;
  final RegExp anyPunctuation;
  final RegExp anyPunctuationOutsideCode;
  final RegExp autolink;
  final RegExp blockSkip;
  final RegExp br;
  final RegExp code;
  final RegExp? del;
  final RegExp? delLDelim;
  final RegExp? delRDelim;
  final RegExp emStrongLDelim;
  final RegExp emStrongRDelimAst;
  final RegExp emStrongRDelimUnd;
  final RegExp escape;
  final RegExp link;
  final RegExp nolink;
  final RegExp punctuation;
  final RegExp reflink;
  final RegExp reflinkSearch;
  final RegExp tag;
  final RegExp text;
  final RegExp? url;
}

final class ActiveRules {
  const ActiveRules({
    required this.other,
    required this.block,
    required this.inline,
  });

  final OtherRules other;
  final BlockRules block;
  final InlineRules inline;
}

final class MarkedRules {
  MarkedRules._({
    required this.other,
    required this.blockNormal,
    required this.blockGfm,
    required this.blockPedantic,
    required this.inlineNormal,
    required this.inlineGfm,
    required this.inlineBreaks,
    required this.inlinePedantic,
  });

  final OtherRules other;
  final BlockRules blockNormal;
  final BlockRules blockGfm;
  final BlockRules blockPedantic;
  final InlineRules inlineNormal;
  final InlineRules inlineGfm;
  final InlineRules inlineBreaks;
  final InlineRules inlinePedantic;

  factory MarkedRules._init() {
    final caretRe = RegExp(r'(^|[^\[])\^');

    _RuleEditor edit(
      String source, {
      bool caseSensitive = true,
      bool multiLine = false,
      bool unicode = false,
    }) => _RuleEditor(
      source,
      caretRe,
      caseSensitive: caseSensitive,
      multiLine: multiLine,
      unicode: unicode,
    );

    const tagNames =
        'address|article|aside|base|basefont|blockquote|body|caption'
        '|center|col|colgroup|dd|details|dialog|dir|div|dl|dt|fieldset|figcaption'
        '|figure|footer|form|frame|frameset|h[1-6]|head|header|hr|html|iframe'
        '|legend|li|link|main|menu|menuitem|meta|nav|noframes|ol|optgroup|option'
        '|p|param|search|section|summary|table|tbody|td|tfoot|th|thead|title'
        '|tr|track|ul';

    final other = OtherRules(
      codeRemoveIndent: RegExp(r'^(?: {0,3}\t| {1,4})', multiLine: true),
      outputLinkReplace: RegExp(r'\\([\[\]])'),
      indentCodeCompensation: RegExp(r'^(\s+)(?:```)'),
      beginningSpace: RegExp(r'^\s+'),
      endingSpaceTabChar: RegExp(r'[ \t]$'),
      leadingSpaceTab: RegExp(r'^[ \t]+'),
      multipleSpaceGlobal: RegExp(r'\s+'),
      doubleBlankLine: RegExp(r'\n[ \t]*\n[ \t]*$'),
      blockquoteStart: RegExp(r'^ {0,3}>'),
      blockquoteSetextReplace: RegExp(r'\n {0,3}((?:=+|-+) *)(?=\n|$)'),
      blockquoteSetextReplace2: RegExp(r'^ {0,3}>[ \t]?', multiLine: true),
      listReplaceNesting: RegExp(r'^ {1,4}(?=( {4})*[^ ])'),
      listIsTask: RegExp(r'^\[[ xX]\] +\S'),
      listReplaceTask: RegExp(r'^\[[ xX]\] +'),
      listTaskCheckbox: RegExp(r'\[[ xX]\]'),
      anyLine: RegExp(r'\n.*\n'),
      hrefBrackets: RegExp(r'^<(.*)>$'),
      tableDelimiter: RegExp(r'[:|]'),
      tableAlignChars: RegExp(r'^\||\| *$'),
      tableRowBlankLine: RegExp(r'\n[ \t]*$'),
      tableAlignRight: RegExp(r'^ *-+: *$'),
      tableAlignCenter: RegExp(r'^ *:-+: *$'),
      tableAlignLeft: RegExp(r'^ *:-+ *$'),
      startATag: RegExp(r'^<a ', caseSensitive: false),
      endATag: RegExp(r'^</a>', caseSensitive: false),
      startPreScriptTag: RegExp(
        r'^<(pre|code|kbd|script)(\s|>)',
        caseSensitive: false,
      ),
      endPreScriptTag: RegExp(
        r'^</(pre|code|kbd|script)(\s|>)',
        caseSensitive: false,
      ),
      pedanticHrefTitle: RegExp(r'''^([^'"]*[^\s])\s+(['"])(.*)\2'''),
      unicodeAlphaNumeric: RegExp(r'[\p{L}\p{N}]', unicode: true),
      numericCharacterReference: RegExp(
        r'&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));',
      ),
      numericCharacterReferenceOrAmpersand: RegExp(
        r'&#(?:(\d{1,7})|[Xx]([A-Fa-f0-9]{1,6}));|&(?!(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);)',
      ),
      carriageReturn: RegExp(r'\r\n|\r'),
      spaceLine: RegExp(r'^ +$', multiLine: true),
      notSpaceStart: RegExp(r'^\S*'),
      nextBulletCache: List<RegExp>.generate(
        4,
        (indent) => RegExp(
          '^ {0,$indent}(?:[*+-]|\\d{1,9}[.)])((?:[ \t][^\\n]*)?(?:\\n|\$))',
        ),
        growable: false,
      ),
      hrCache: List<RegExp>.generate(
        4,
        (indent) => RegExp(
          '^ {0,$indent}((?:-[ \t]*){3,}|(?:_[ \t]*){3,}|(?:\\*[ \t]*){3,})(?:\\n+|\$)',
        ),
        growable: false,
      ),
      fencesBeginCache: List<RegExp>.generate(
        4,
        (indent) => RegExp('^ {0,$indent}(?:```|~~~)'),
        growable: false,
      ),
      headingBeginCache: List<RegExp>.generate(
        4,
        (indent) => RegExp('^ {0,$indent}#'),
        growable: false,
      ),
      htmlBeginCache: List<RegExp>.generate(
        4,
        (indent) => RegExp(
          '^ {0,$indent}(?:</?(?:$tagNames)(?: +|\$|/?>)|<(?:(?:script|pre|style|textarea)(?:[\\s>]|\$)|!--)|<\\?|<![A-Za-z]|<!\\[CDATA\\[)',
          caseSensitive: false,
        ),
        growable: false,
      ),
      blockquoteBeginCache: List<RegExp>.generate(
        4,
        (indent) => RegExp('^ {0,$indent}>'),
        growable: false,
      ),
    );

    // Block-level grammar
    const newlineSrc = r'^(?:[ \t]*(?:\n|$))+';
    const blockCodeSrc =
        r'^((?: {4}| {0,3}\t)[^\n]+(?:\n(?:[ \t]*(?:\n|$))*)?)+';
    const fencesSrc =
        r'^ {0,3}(`{3,}(?=[^`\n]*(?:\n|$))|~{3,})([^\n]*)(?:\n|$)(?:|([\s\S]*?)(?:\n|$))(?: {0,3}\1[~`]* *(?=\n|$)|$)';
    const hrSrc =
        r'^ {0,3}((?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)';
    const headingSrc = r'^ {0,3}(#{1,6})(?=\s|$)(.*)(?:\n+|$)';
    const bulletSrc = r' {0,3}(?:[*+-]|\d{1,9}[.)])';
    const lheadingCoreSrc =
        r'^(?!bull |blockCode|fences|blockquote|heading|html|table)((?:.|\n(?!\s*?\n|bull |fences|blockquote|heading|hr|html|table))+?)\n {0,3}(=+|-+) *(?:\n+|$)';

    final newline = RegExp(newlineSrc);
    final blockCode = RegExp(blockCodeSrc);
    final fences = RegExp(fencesSrc);
    final hr = RegExp(hrSrc);
    final heading = RegExp(headingSrc);

    final lheading = edit(lheadingCoreSrc)
        .replaceAll('bull', bulletSrc)
        .replaceAll('blockCode', r'(?: {4}| {0,3}\t)')
        .replaceAll('fences', r' {0,3}(?:`{3,}|~{3,})')
        .replaceAll('blockquote', r' {0,3}>')
        .replaceAll('heading', r' {0,3}#{1,6}(?:\s|$)')
        .replaceAll(
          'hr',
          r' {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)',
        )
        .replaceAll('html', r' {0,3}<[^\n>]+>\n')
        .replaceAll('|table', '')
        .getRegex();

    final lheadingGfm = edit(lheadingCoreSrc)
        .replaceAll('bull', bulletSrc)
        .replaceAll('blockCode', r'(?: {4}| {0,3}\t)')
        .replaceAll('fences', r' {0,3}(?:`{3,}|~{3,})')
        .replaceAll('blockquote', r' {0,3}>')
        .replaceAll('heading', r' {0,3}#{1,6}(?:\s|$)')
        .replaceAll(
          'hr',
          r' {0,3}(?:(?:-[\t ]*){3,}|(?:_[ \t]*){3,}|(?:\*[ \t]*){3,})(?:\n+|$)',
        )
        .replaceAll('html', r' {0,3}<[^\n>]+>\n')
        .replaceAll('table', r' {0,3}\|?(?:[:\- ]*\|)+[\:\- ]*\n')
        .getRegex();

    const paragraphSrc =
        r'^([^\n]+(?:\n(?!hr|heading|lheading|blockquote|fences|list|html|table|[ \t]+\n)[^\n]+)*)';
    final blockText = RegExp(r'^[^\n]+');
    const blockLabelSrc = r'(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+';

    final def = edit(
      r'^ {0,3}\[(label)\]: *(?:\n[ \t]*)?([^<\s][^\s]*|<.*?>)(?:(?: +(?:\n[ \t]*)?| *\n[ \t]*)(title))? *(?:\n+|$)',
    )
        .replaceFirst('label', blockLabelSrc)
        .replaceFirst(
          'title',
          r'''(?:"(?:\\"?|[^"\\])*"|'[^'\n]*(?:\n[^'\n]+)*\n?'|\([^()]*\))''',
        )
        .getRegex();

    final list = edit(r'^(bull)([ \t][^\n]*?)?(?:\n|$)')
        .replaceAll('bull', bulletSrc)
        .getRegex();

    const commentSrc = r'<!--(?:-?>|[\s\S]*?(?:-->|$))';
    const htmlAttrSrc =
        r''' +[a-zA-Z:_][\w.:-]*(?: *= *"[^"\n]*"| *= *'[^'\n]*'| *= *[^\s"'=<>`]+)?''';

    final html = edit(
      '^ {0,3}(?:'
          r'<(script|pre|style|textarea)[\s>][\s\S]*?(?:</\1>[^\n]*\n*|$)'
          r'|comment[^\n]*(\n+|$)'
          r'|<\?[\s\S]*?(?:\?>[^\n]*\n*|$)'
          r'|<![A-Z][\s\S]*?(?:>[^\n]*\n*|$)'
          r'|<!\[CDATA\[[\s\S]*?(?:\]\]>[^\n]*\n*|$)'
          r'|</?(tag)(?: +|\n|/?>)[\s\S]*?(?:(?:\n[ \t]*)+\n|$)'
          r'|<(?!(?:script|pre|style|textarea)(?![a-z0-9-]))([a-z][a-z0-9-]*)(?:attribute)*? */?>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)'
          r'|</[a-z][a-z0-9-]*\s*>(?=[ \t]*(?:\n|$))[\s\S]*?(?:(?:\n[ \t]*)+\n|$)'
          ')',
      caseSensitive: false,
    )
        .replaceFirst('comment', commentSrc)
        .replaceFirst('tag', tagNames)
        .replaceFirst('attribute', htmlAttrSrc)
        .getRegex();

    RegExp createParagraph(String listInterrupt) => edit(paragraphSrc)
        .replaceFirst('hr', hrSrc)
        .replaceFirst('heading', r' {0,3}#{1,6}(?:\s|$)')
        .replaceFirst('|lheading', '')
        .replaceFirst('|table', '')
        .replaceFirst('blockquote', r' {0,3}>')
        .replaceFirst(
          'fences',
          r' {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)',
        )
        .replaceFirst('list', listInterrupt)
        .replaceFirst(
          'html',
          r' {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)',
        )
        .replaceFirst('tag', tagNames)
        .getRegex();

    final paragraph = createParagraph(r' {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]');
    final blockquoteParagraph =
        createParagraph(r' {0,3}(?:[*+-]|\d{1,9}[.)])(?:[ \t]|\n|$)');

    final blockquote = edit(r'^( {0,3}> ?(paragraph|[^\n]*)(?:\n|$))+')
        .replaceFirst('paragraph', blockquoteParagraph.pattern)
        .getRegex();

    final blockNormal = BlockRules(
      blockquote: blockquote,
      code: blockCode,
      def: def,
      fences: fences,
      heading: heading,
      hr: hr,
      html: html,
      lheading: lheading,
      list: list,
      newline: newline,
      paragraph: paragraph,
      table: null,
      text: blockText,
    );

    final gfmTable = edit(
      r'^ *([^\n ].*)\n'
      r' {0,3}((?:\| *)?:?-+:? *(?:\| *:?-+:? *)*(?:\| *)?)'
      r'(?:\n((?:(?! *\n|hr|heading|blockquote|code|fences|list|html).*(?:\n|$))*)\n*|$)',
    )
        .replaceFirst('hr', hrSrc)
        .replaceFirst('heading', r' {0,3}#{1,6}(?:\s|$)')
        .replaceFirst('blockquote', r' {0,3}>')
        .replaceFirst('code', '(?: {4}| {0,3}\t)[^\\n]')
        .replaceFirst(
          'fences',
          r' {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)',
        )
        .replaceFirst('list', r' {0,3}(?:[*+-]|1[.)])[ \t]')
        .replaceFirst(
          'html',
          r' {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)',
        )
        .replaceFirst('tag', tagNames)
        .getRegex();

    final blockGfm = BlockRules(
      blockquote: blockquote,
      code: blockCode,
      def: def,
      fences: fences,
      heading: heading,
      hr: hr,
      html: html,
      lheading: lheadingGfm,
      list: list,
      newline: newline,
      paragraph: edit(paragraphSrc)
          .replaceFirst('hr', hrSrc)
          .replaceFirst('heading', r' {0,3}#{1,6}(?:\s|$)')
          .replaceFirst('|lheading', '')
          .replaceFirst('table', gfmTable.pattern)
          .replaceFirst('blockquote', r' {0,3}>')
          .replaceFirst(
            'fences',
            r' {0,3}(?:`{3,}(?=[^`\n]*(?:\n|$))|~~~)[^\n]*(?:\n|$)',
          )
          .replaceFirst('list', r' {0,3}(?:[*+-]|1[.)])[ \t]+[^ \t\n]')
          .replaceFirst(
            'html',
            r' {0,3}(?:</?(?:tag)(?: +|\n|/?>)|<(?:(?:script|pre|style|textarea)(?:[\s>]|$)|!--)|<\?|<![A-Za-z]|<!\[CDATA\[)',
          )
          .replaceFirst('tag', tagNames)
          .getRegex(),
      table: gfmTable,
      text: blockText,
    );

    final blockPedantic = BlockRules(
      blockquote: blockquote,
      code: blockCode,
      def: RegExp(
        r'^ *\[([^\]]+)\]: *<?([^\s>]+)>?(?: +(["(][^\n]+[")]))? *(?:\n+|$)',
      ),
      fences: null,
      heading: RegExp(r'^(#{1,6})(.*)(?:\n+|$)'),
      hr: hr,
      html: edit(
        r'^ *(?:comment *(?:\n|\s*$)'
        r'|<(tag)[\s\S]+?</\1> *(?:\n{2,}|\s*$)'
        r'''|<tag(?:"[^"]*"|'[^']*'|\s[^'"/>\s]*)*?/?> *(?:\n{2,}|\s*$))''',
      )
          .replaceFirst('comment', commentSrc)
          .replaceAll(
            'tag',
            r'(?!(?:'
                r'a|em|strong|small|s|cite|q|dfn|abbr|data|time|code|var|samp|kbd|sub'
                r'|sup|i|b|u|mark|ruby|rt|rp|bdi|bdo|span|br|wbr|ins|del|img)'
                r'\b)\w+(?!:|[^\w\s@]*@)\b',
          )
          .getRegex(),
      lheading: RegExp(r'^(.+?)\n {0,3}(=+|-+) *(?:\n+|$)'),
      list: list,
      newline: newline,
      paragraph: edit(paragraphSrc)
          .replaceFirst('hr', hrSrc)
          .replaceFirst('heading', ' *#{1,6} *[^\n]')
          .replaceFirst('lheading', lheading.pattern)
          .replaceFirst('|table', '')
          .replaceFirst('blockquote', r' {0,3}>')
          .replaceFirst('|fences', '')
          .replaceFirst('|list', '')
          .replaceFirst('|html', '')
          .replaceFirst('|tag', '')
          .getRegex(),
      table: null,
      text: blockText,
    );

    // Inline-level grammar
    final escape = RegExp(
      r'''^\\([!"#$%&'()*+,\-./:;<=>?@\[\]\\^_`{|}~])''',
    );
    final inlineCode = RegExp(r'^(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)');
    const brSrc = r'^( {2,}|\\)\n(?!\s*$)[ \t]*';
    final br = RegExp(brSrc);
    final inlineText = RegExp(
      r'^(`+|[^`])(?:(?= {2,}\n)|[\s\S]*?(?:(?=[\\<!\[`*_]|\b_|$)|[^ ](?= {2,}\n)))',
    );

    const punctSrc = r'[\p{P}\p{S}]';
    const punctSpaceSrc = r'[\s\p{P}\p{S}]';
    const notPunctSpaceSrc = r'[^\s\p{P}\p{S}]';
    final punctuation = edit(r'^((?![*_])punctSpace)', unicode: true)
        .replaceAll('punctSpace', punctSpaceSrc)
        .getRegex();

    const openQuoteSrc = r'''[\p{Pi}\p{Ps}"']''';

    const punctGfmSrc = r'(?!~)[\p{P}\p{S}]';
    const punctSpaceGfmSrc = r'(?!~)[\s\p{P}\p{S}]';
    const notPunctSpaceGfmSrc = r'(?:[^\s\p{P}\p{S}]|~)';

    // Lookbehind is always supported on V8/SpiderMonkey/JSC (Node/Chrome/Firefox/Safari).
    final blockSkip = edit('link|precode-code|html')
        .replaceFirst(
          'link',
          r'\[(?:[^\[\]`]|(?<a>`+)[^`]+\k<a>(?!`))*?\]\((?:\\[\s\S]|[^\\\(\)]|\((?:\\[\s\S]|[^\\\(\)])*\))*\)',
        )
        .replaceFirst('precode-', r'(?<!`)()')
        .replaceFirst('code', r'(?<b>`+)[^`]+\k<b>(?!`)')
        .replaceFirst('html', r'<(?! )[^<>]*?>')
        .getRegex();

    const emStrongLDelimCoreSrc =
        r'^(?:\*+(?:((?!\*)punct)|([^\s*]))?)|^_+(?:((?!_)punct)|([^\s_]))?';
    final emStrongLDelim = edit(emStrongLDelimCoreSrc, unicode: true)
        .replaceAll('punct', punctSrc)
        .getRegex();
    final emStrongLDelimGfm = edit(emStrongLDelimCoreSrc, unicode: true)
        .replaceAll('punct', punctGfmSrc)
        .getRegex();

    const emStrongLDelimPedanticCoreSrc =
        r'^(?:\*+(?:((?!\*)(?!openQuote)punct)|([^\s*]))?)|^_+(?:((?!_)(?!openQuote)punct)|([^\s_]))?';
    final emStrongLDelimPedantic =
        edit(emStrongLDelimPedanticCoreSrc, unicode: true)
            .replaceAll('openQuote', openQuoteSrc)
            .replaceAll('punct', punctSrc)
            .getRegex();

    const emStrongRDelimAstCoreSrc =
        r'^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)'
        r'|[^*]+(?=[^*])'
        r'|(?!\*)punct(\*+)(?=[\s]|$)'
        r'|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)'
        r'|(?!\*)punctSpace(\*+)(?=notPunctSpace)'
        r'|[\s](\*+)(?!\*)(?=punct)'
        r'|(?!\*)punct(\*+)(?!\*)(?=punct)'
        r'|notPunctSpace(\*+)(?=notPunctSpace)';

    final emStrongRDelimAst = edit(emStrongRDelimAstCoreSrc, unicode: true)
        .replaceAll('notPunctSpace', notPunctSpaceSrc)
        .replaceAll('punctSpace', punctSpaceSrc)
        .replaceAll('punct', punctSrc)
        .getRegex();

    final emStrongRDelimAstGfm = edit(emStrongRDelimAstCoreSrc, unicode: true)
        .replaceAll('notPunctSpace', notPunctSpaceGfmSrc)
        .replaceAll('punctSpace', punctSpaceGfmSrc)
        .replaceAll('punct', punctGfmSrc)
        .getRegex();

    const emStrongRDelimAstPedanticCoreSrc =
        r'^[^_*]*?__[^_*]*?\*[^_*]*?(?=__)'
        r'|[^*]+(?=[^*])'
        r'|(?!\*)punct(\*+)(?=[\s]|$)'
        r'|notPunctSpace(\*+)(?!\*)(?=punctSpace|$)'
        r'|(?!\*)[\s](\*+)(?=notPunctSpace)'
        r'|[\s](\*+)(?!\*)(?=punct)'
        r'|(?!\*)punct(\*+)(?!\*)(?=punct)'
        r'|(?:(?!\*)punct|notPunctSpace)(\*+)(?!\*)(?=notPunctSpace)';

    final emStrongRDelimAstPedantic =
        edit(emStrongRDelimAstPedanticCoreSrc, unicode: true)
            .replaceAll('notPunctSpace', notPunctSpaceSrc)
            .replaceAll('punctSpace', punctSpaceSrc)
            .replaceAll('punct', punctSrc)
            .getRegex();

    final emStrongRDelimUnd = edit(
      r'^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)'
      r'|[^_]+(?=[^_])'
      r'|(?!_)punct(_+)(?=[\s]|$)'
      r'|notPunctSpace(_+)(?!_)(?=punctSpace|$)'
      r'|(?!_)punctSpace(_+)(?=notPunctSpace)'
      r'|[\s](_+)(?!_)(?=punct)'
      r'|(?!_)punct(_+)(?!_)(?=punct)',
      unicode: true,
    )
        .replaceAll('notPunctSpace', notPunctSpaceSrc)
        .replaceAll('punctSpace', punctSpaceSrc)
        .replaceAll('punct', punctSrc)
        .getRegex();

    const emStrongRDelimUndPedanticCoreSrc =
        r'^[^_*]*?\*\*[^_*]*?_[^_*]*?(?=\*\*)'
        r'|[^_]+(?=[^_])'
        r'|(?!_)punct(_+)(?=[\s]|$)'
        r'|notPunctSpace(_+)(?!_)(?=punctSpace|$)'
        r'|(?!_)[\s](_+)(?=notPunctSpace)'
        r'|[\s](_+)(?!_)(?=punct)'
        r'|(?!_)punct(_+)(?!_)(?=punct)'
        r'|(?:(?!_)punct|notPunctSpace)(_+)(?!_)(?=notPunctSpace)';

    final emStrongRDelimUndPedantic =
        edit(emStrongRDelimUndPedanticCoreSrc, unicode: true)
            .replaceAll('notPunctSpace', notPunctSpaceSrc)
            .replaceAll('punctSpace', punctSpaceSrc)
            .replaceAll('punct', punctSrc)
            .getRegex();

    final delLDelim = edit(r'^~~?(?:((?!~)punct)|[^\s~])', unicode: true)
        .replaceAll('punct', punctSrc)
        .getRegex();

    const delRDelimCoreSrc =
        r'^[^~]+(?=[^~])'
        r'|(?!~)punct(~~?)(?=[\s]|$)'
        r'|notPunctSpace(~~?)(?!~)(?=punctSpace|$)'
        r'|(?!~)punctSpace(~~?)(?=notPunctSpace)'
        r'|[\s](~~?)(?!~)(?=punct)'
        r'|(?!~)punct(~~?)(?!~)(?=punct)'
        r'|notPunctSpace(~~?)(?=notPunctSpace)';

    final delRDelim = edit(delRDelimCoreSrc, unicode: true)
        .replaceAll('notPunctSpace', notPunctSpaceSrc)
        .replaceAll('punctSpace', punctSpaceSrc)
        .replaceAll('punct', punctSrc)
        .getRegex();

    final anyPunctuation = edit(r'\\(punct)', unicode: true)
        .replaceAll('punct', punctSrc)
        .getRegex();

    final anyPunctuationOutsideCode = edit(
      r'(`+)(?:[^`]|[^`][\s\S]*?[^`])\1(?!`)|`+|\\(punct)',
      unicode: true,
    ).replaceAll('punct', punctSrc).getRegex();

    final autolink = edit(r'^<(scheme:[^\s\x00-\x1f<>]*|email)>')
        .replaceFirst('scheme', r'[a-zA-Z][a-zA-Z0-9+.-]{1,31}')
        .replaceFirst(
          'email',
          r'''[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+(@)[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+(?![-_])''',
        )
        .getRegex();

    final inlineComment = edit(commentSrc)
        .replaceFirst('(?:-->|\$)', '-->')
        .getRegex();

    final tag = edit(
      r'^comment'
      r'|^</[a-zA-Z][a-zA-Z0-9-]*\s*>'
      r'|^<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\s*/?>'
      r'|^<\?[\s\S]*?\?>'
      r'|^<![a-zA-Z]+\s[\s\S]*?>'
      r'|^<!\[CDATA\[[\s\S]*?\]\]>',
    )
        .replaceFirst('comment', inlineComment.pattern)
        .replaceFirst(
          'attribute',
          r'''\s+[a-zA-Z:_][\w.:-]*(?:\s*=\s*"[^"]*"|\s*=\s*'[^']*'|\s*=\s*[^\s"'=<>`]+)?''',
        )
        .getRegex();

    const inlineLabelBracketsSrc = r'\[(?:\\[\s\S]|[^\[\]\\])*\]';
    final inlineLabel = edit(
      r'(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\])|[^\[\]\\`])*?',
    ).replaceFirst('brackets', inlineLabelBracketsSrc).getRegex();

    final link = edit(
      r'^!?\[(label)\]\([ \t\n]*(href)(?:(?:[ \t]+(?:\n[ \t]*)?|\n[ \t]*)(title))?[ \t\n]*\)',
    )
        .replaceFirst('label', inlineLabel.pattern)
        .replaceFirst('href', r'<(?:\\.|[^\n<>\\])+>|[^ \t\n\x00-\x1f]+|(?=\))')
        .replaceFirst(
          'title',
          r'''"(?:\\"?|[^"\\])*"|'(?:\\'?|[^'\\])*'|\((?:\\\)?|[^)\\])*\)''',
        )
        .getRegex();

    final reflink = edit(r'^!?\[(label)\]\[(ref)\]')
        .replaceFirst('label', inlineLabel.pattern)
        .replaceFirst('ref', blockLabelSrc)
        .getRegex();

    final nolink = edit(r'^!?\[(ref)\](?:\[\])?')
        .replaceFirst('ref', blockLabelSrc)
        .getRegex();

    const boundedBlockLabelSrc = r'(?!\s*\])(?:\\[\s\S]|[^\[\]\\]){1,999}';
    final boundedInlineLabel = edit(
      r'(?:[^\[\]\\`]*(?:\[(?:brackets|\\[\s\S]|[^\[\]\\])*\]|\\[\s\S]|`+(?!`)[^`]*?`+(?!`)|``+(?=\]))){0,999}?[^\[\]\\`]*?',
    ).replaceFirst('brackets', inlineLabelBracketsSrc).getRegex();

    final reflinkSearch = edit(r'reflink|nolink(?!\()')
        .replaceFirst(
          'reflink',
          edit(r'^!?\[(label)\]\[(ref)\]')
              .replaceFirst('label', boundedInlineLabel.pattern)
              .replaceFirst('ref', boundedBlockLabelSrc)
              .getRegex()
              .pattern,
        )
        .replaceFirst(
          'nolink',
          edit(r'^!?\[(ref)\](?:\[\])?')
              .replaceFirst('ref', boundedBlockLabelSrc)
              .getRegex()
              .pattern,
        )
        .getRegex();

    const caseInsensitiveProtocolSrc = r'[hH][tT][tT][pP][sS]?|[fF][tT][pP]';
    const extendedEmailSrc =
        r'[A-Za-z0-9._+-]+@[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])';
    final extendedEmailProtocol = edit(
      r'(?:mailto:email|xmpp:email(?:\/[A-Za-z0-9@.]+)?)',
    ).replaceAll('email', extendedEmailSrc).getRegex();

    final inlineNormal = InlineRules(
      backpedal: null,
      anyPunctuation: anyPunctuation,
      anyPunctuationOutsideCode: anyPunctuationOutsideCode,
      autolink: autolink,
      blockSkip: blockSkip,
      br: br,
      code: inlineCode,
      del: null,
      delLDelim: null,
      delRDelim: null,
      emStrongLDelim: emStrongLDelim,
      emStrongRDelimAst: emStrongRDelimAst,
      emStrongRDelimUnd: emStrongRDelimUnd,
      escape: escape,
      link: link,
      nolink: nolink,
      punctuation: punctuation,
      reflink: reflink,
      reflinkSearch: reflinkSearch,
      tag: tag,
      text: inlineText,
      url: null,
    );

    final inlinePedantic = InlineRules(
      backpedal: null,
      anyPunctuation: anyPunctuation,
      anyPunctuationOutsideCode: anyPunctuationOutsideCode,
      autolink: autolink,
      blockSkip: blockSkip,
      br: br,
      code: inlineCode,
      del: null,
      delLDelim: null,
      delRDelim: null,
      emStrongLDelim: emStrongLDelimPedantic,
      emStrongRDelimAst: emStrongRDelimAstPedantic,
      emStrongRDelimUnd: emStrongRDelimUndPedantic,
      escape: escape,
      link: edit(r'^!?\[(label)\]\((.*?)\)')
          .replaceFirst('label', inlineLabel.pattern)
          .getRegex(),
      nolink: nolink,
      punctuation: punctuation,
      reflink: edit(r'^!?\[(label)\]\s*\[([^\]]*)\]')
          .replaceFirst('label', inlineLabel.pattern)
          .getRegex(),
      reflinkSearch: reflinkSearch,
      tag: tag,
      text: inlineText,
      url: null,
    );

    final gfmUrl = edit(
      r'^emailProtocol|^((?:protocol):\/\/|www\.)(?:[a-zA-Z0-9\-]+\.?)+[^\s<]*|^email',
    )
        .replaceFirst('emailProtocol', extendedEmailProtocol.pattern)
        .replaceFirst('protocol', caseInsensitiveProtocolSrc)
        .replaceFirst(
          'email',
          r'[A-Za-z0-9._+-]+(@)[a-zA-Z0-9-_]+(?:\.[a-zA-Z0-9-_]*[a-zA-Z0-9])+(?![\w-])',
        )
        .getRegex();

    final gfmBackpedal = RegExp(
      r'''(?:[^?!.,:;*_'"~()&]+|\([^)]*\)|&(?![a-zA-Z0-9]+;$)|[?!.,:;*_'"~)]+(?!$))+''',
    );

    final gfmDel = RegExp(
      r'^(~~?)(?=[^\s~])((?:\\[\s\S]|[^\\])*?(?:\\[\s\S]|[^\s~\\]))\1(?=[^~]|$)',
    );

    final gfmText = edit(
      r'''^(?:[^a-zA-Z0-9](?=emailProtocol)|(`+|~+|[^`~])(?:(?=[`~])|(?= {2,}\n)|(?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@)|[\s\S]*?(?:(?=[\\<!\[`*~_]|\b_|protocol:\/\/|www\.|$)|[^ ](?= {2,}\n)|[^a-zA-Z0-9](?=emailProtocol)|[^a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-](?=[a-zA-Z0-9.!#$%&'*+\/=?_`{\|}~-]+@))))''',
    )
        .replaceFirst('protocol', caseInsensitiveProtocolSrc)
        .replaceAll('emailProtocol', r'(?:mailto|xmpp):')
        .getRegex();

    final inlineGfm = InlineRules(
      backpedal: gfmBackpedal,
      anyPunctuation: anyPunctuation,
      anyPunctuationOutsideCode: anyPunctuationOutsideCode,
      autolink: autolink,
      blockSkip: blockSkip,
      br: br,
      code: inlineCode,
      del: gfmDel,
      delLDelim: delLDelim,
      delRDelim: delRDelim,
      emStrongLDelim: emStrongLDelimGfm,
      emStrongRDelimAst: emStrongRDelimAstGfm,
      emStrongRDelimUnd: emStrongRDelimUnd,
      escape: escape,
      link: link,
      nolink: nolink,
      punctuation: punctuation,
      reflink: reflink,
      reflinkSearch: reflinkSearch,
      tag: tag,
      text: gfmText,
      url: gfmUrl,
    );

    final inlineBreaks = InlineRules(
      backpedal: gfmBackpedal,
      anyPunctuation: anyPunctuation,
      anyPunctuationOutsideCode: anyPunctuationOutsideCode,
      autolink: autolink,
      blockSkip: blockSkip,
      br: edit(brSrc).replaceFirst('{2,}', '*').getRegex(),
      code: inlineCode,
      del: gfmDel,
      delLDelim: delLDelim,
      delRDelim: delRDelim,
      emStrongLDelim: emStrongLDelimGfm,
      emStrongRDelimAst: emStrongRDelimAstGfm,
      emStrongRDelimUnd: emStrongRDelimUnd,
      escape: escape,
      link: link,
      nolink: nolink,
      punctuation: punctuation,
      reflink: reflink,
      reflinkSearch: reflinkSearch,
      tag: tag,
      text: edit(gfmText.pattern)
          .replaceFirst(r'\b_', r'\b_| {2,}\n')
          .replaceAll('{2,}', '*')
          .getRegex(),
      url: gfmUrl,
    );

    return MarkedRules._(
      other: other,
      blockNormal: blockNormal,
      blockGfm: blockGfm,
      blockPedantic: blockPedantic,
      inlineNormal: inlineNormal,
      inlineGfm: inlineGfm,
      inlineBreaks: inlineBreaks,
      inlinePedantic: inlinePedantic,
    );
  }
}
