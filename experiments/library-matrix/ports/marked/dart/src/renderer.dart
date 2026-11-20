import 'helpers.dart';
import 'options.dart';
import 'parser.dart';
import 'rules.dart';
import 'tokens.dart';

/// Faithful port of `marked@18.1.1` `_Renderer`.
final class Renderer {
  Renderer(this.options);

  final MarkedOptions options;
  late Parser parser;

  String space(SpaceToken token) => '';

  String code(CodeToken token) {
    final lang = token.lang ?? '';
    final langString = markedRules.other.notSpaceStart.firstMatch(lang)?[0] ?? '';

    final rawText = token.text;
    final codeStr = rawText.isNotEmpty
        ? '${rawText.endsWith('\n') ? rawText.substring(0, rawText.length - 1) : rawText}\n'
        : '';
    final rendered = token.escaped ? codeStr : escapeHtmlEntities(codeStr, true);

    if (langString.isEmpty) {
      return '<pre><code>$rendered</code></pre>\n';
    }
    return '<pre><code class="language-${escapeHtmlEntities(langString)}">$rendered</code></pre>\n';
  }

  String blockquote(BlockquoteToken token) =>
      '<blockquote>\n${parser.parse(token.tokens)}</blockquote>\n';

  String html(HtmlToken token) => token.text;

  String def(DefToken token) => '';

  String heading(HeadingToken token) =>
      '<h${token.depth}>${parser.parseInline(token.tokens)}</h${token.depth}>\n';

  String hr(HrToken token) => '<hr>\n';

  String list(ListToken token) {
    final body = StringBuffer();
    for (var j = 0; j < token.items.length; j++) {
      body.write(listitem(token.items[j]));
    }
    final type = token.ordered ? 'ol' : 'ul';
    final startAttr = (token.ordered && token.start != 1)
        ? ' start="${token.start}"'
        : '';
    return '<$type$startAttr>\n$body</$type>\n';
  }

  String listitem(ListItemToken item) =>
      '<li>${parser.parse(item.tokens)}</li>\n';

  String checkbox(CheckboxToken token) =>
      '<input ${token.checked ? 'checked="" ' : ''}disabled="" type="checkbox"> ';

  String paragraph(ParagraphToken token) =>
      '<p>${parser.parseInline(token.tokens)}</p>\n';

  String table(TableToken token) {
    final headerCells = StringBuffer();
    for (var j = 0; j < token.header.length; j++) {
      headerCells.write(tablecell(token.header[j]));
    }
    final header = tablerow(headerCells.toString());

    final bodyBuf = StringBuffer();
    for (var j = 0; j < token.rows.length; j++) {
      final row = token.rows[j];
      final rowCells = StringBuffer();
      for (var k = 0; k < row.length; k++) {
        rowCells.write(tablecell(row[k]));
      }
      bodyBuf.write(tablerow(rowCells.toString()));
    }
    final body = bodyBuf.isNotEmpty ? '<tbody>$bodyBuf</tbody>' : '';

    return '<table>\n<thead>\n$header</thead>\n$body</table>\n';
  }

  String tablerow(String text) => '<tr>\n$text</tr>\n';

  String tablecell(TableCellToken token) {
    final content = parser.parseInline(token.tokens);
    final type = token.header ? 'th' : 'td';
    final align = token.align;
    final tag = align != null ? '<$type align="$align">' : '<$type>';
    return '$tag$content</$type>\n';
  }

  String strong(StrongToken token) =>
      '<strong>${parser.parseInline(token.tokens)}</strong>';

  String em(EmToken token) => '<em>${parser.parseInline(token.tokens)}</em>';

  String codespan(CodespanToken token) =>
      '<code>${escapeHtmlEntities(token.text, true)}</code>';

  String br(BrToken token) => '<br>';

  String del(DelToken token) =>
      '<del>${parser.parseInline(token.tokens)}</del>';

  String link(LinkToken token) {
    final parsedText = token.autolink
        ? escapeHtmlEntities(token.text, true)
        : parser.parseInline(token.tokens);
    final cleanHref = cleanUrl(token.href);
    if (cleanHref == null) {
      return parsedText;
    }
    final href = escapeHtmlEntities(cleanHref, token.autolink);
    final title = token.title;
    if (title != null && title.isNotEmpty) {
      return '<a href="$href" title="${escapeHtmlEntities(title)}">$parsedText</a>';
    }
    return '<a href="$href">$parsedText</a>';
  }

  String image(ImageToken token) {
    final text = parser.parseInlineText(token.tokens);
    final cleanHref = cleanUrl(token.href);
    if (cleanHref == null) {
      return escapeHtmlEntities(text);
    }
    final title = token.title;
    if (title != null && title.isNotEmpty) {
      return '<img src="${escapeHtmlEntities(cleanHref)}" alt="${escapeHtmlEntities(text)}" title="${escapeHtmlEntities(title)}">';
    }
    return '<img src="${escapeHtmlEntities(cleanHref)}" alt="${escapeHtmlEntities(text)}">';
  }

  String text(TextToken token) {
    final childTokens = token.tokens;
    if (childTokens != null) {
      return parser.parseInline(childTokens);
    }
    return token.escaped ? token.text : escapeHtmlEntities(token.text);
  }

  String escape(EscapeToken token) => escapeHtmlEntities(token.text);
}
