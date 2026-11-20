import 'helpers.dart';
import 'lexer.dart';
import 'options.dart';
import 'rules.dart';
import 'tokens.dart';

Token? _outputLink(
  String cap0,
  String cap1,
  LinkDef link,
  String raw,
  Lexer lexer,
  ActiveRules rules,
) {
  final href = link.href;
  final title = (link.title != null && link.title!.isNotEmpty)
      ? link.title
      : null;
  final text = cap1.replaceAllMapped(
    rules.other.outputLinkReplace,
    (m) => m[1]!,
  );
  final isImage = cap0.isNotEmpty && cap0.codeUnitAt(0) == 0x21 /* ! */;

  final outerInLink = lexer.state.inLink;
  final outerLinkEmitted = lexer.state.linkEmitted;
  final outerInRawBlock = lexer.state.inRawBlock;
  lexer.state.inLink = true;
  lexer.state.linkEmitted = false;
  final tokens = lexer.inlineTokens(text);
  final textHasLink = lexer.state.linkEmitted;
  lexer.state.linkEmitted = outerLinkEmitted;
  lexer.state.inLink = outerInLink;

  if (!isImage) {
    if (textHasLink) {
      lexer.state.inRawBlock = outerInRawBlock;
      return null;
    }
    lexer.state.linkEmitted = true;
    return LinkToken(
      raw: raw,
      href: href,
      title: title,
      text: text,
      tokens: tokens,
    );
  }

  return ImageToken(
    raw: raw,
    href: href,
    title: title,
    text: text,
    tokens: tokens,
  );
}

String _indentCodeCompensation(String raw, String text, ActiveRules rules) {
  final matchIndentToCode = rules.other.indentCodeCompensation.firstMatch(raw);
  if (matchIndentToCode == null) {
    return text;
  }

  final indentLen = matchIndentToCode[1]!.length;
  final lines = text.split('\n');
  for (var i = 0; i < lines.length; i++) {
    final node = lines[i];
    final matchIndentInNode = rules.other.beginningSpace.firstMatch(node);
    if (matchIndentInNode == null) continue;
    final nodeIndentLen = matchIndentInNode[0]!.length;
    final remove = nodeIndentLen < indentLen ? nodeIndentLen : indentLen;
    lines[i] = node.substring(remove);
  }
  return lines.join('\n');
}

bool _isLabelEndInsideToken(
  String src,
  String label,
  int labelStart,
  ActiveRules rules,
) {
  if (!label.contains('<')) {
    return false;
  }

  final len = label.length;
  for (var i = 0; i < len; i++) {
    final c = label.codeUnitAt(i);
    if (c == 0x5C /* \ */) {
      i++;
      continue;
    }

    if (c == 0x60 /* ` */) {
      final code = rules.inline.code.firstMatch(label.substring(i));
      if (code != null) {
        i += code[0]!.length - 1;
        continue;
      }
    }

    if (c != 0x3C /* < */) {
      continue;
    }

    final tokenSrc = src.substring(labelStart + i);
    final token =
        rules.inline.tag.firstMatch(tokenSrc) ??
        rules.inline.autolink.firstMatch(tokenSrc);
    if (token == null) {
      continue;
    }

    final tokenLen = token[0]!.length;
    if (tokenLen > len - i) {
      return true;
    }
    i += tokenLen - 1;
  }
  return false;
}

int _firstAfterUpTo3Spaces(String s) {
  final len = s.length;
  var i = 0;
  while (i < 3 && i < len && s.codeUnitAt(i) == 0x20) {
    i++;
  }
  return i < len ? s.codeUnitAt(i) : -1;
}

int _firstAfterAnySpaces(String s) {
  final len = s.length;
  var i = 0;
  while (i < len && s.codeUnitAt(i) == 0x20) {
    i++;
  }
  return i < len ? s.codeUnitAt(i) : -1;
}

bool _isUrlStartChar(int c) =>
    (c >= 0x61 && c <= 0x7A) ||
    (c >= 0x41 && c <= 0x5A) ||
    (c >= 0x30 && c <= 0x39) ||
    c == 0x2E ||
    c == 0x5F ||
    c == 0x2B ||
    c == 0x2D;

/// Faithful port of `marked@18.1.1` `_Tokenizer`.
final class Tokenizer {
  Tokenizer(this.options);

  MarkedOptions options;
  late ActiveRules rules;
  late Lexer lexer;

  SpaceToken? space(String src) {
    if (src.isEmpty) return null;
    final c = src.codeUnitAt(0);
    if (c != 0x20 && c != 0x09 && c != 0x0A) return null;
    final cap = rules.block.newline.firstMatch(src);
    if (cap != null) {
      final raw = cap[0]!;
      if (raw.isNotEmpty) {
        return SpaceToken(raw: raw);
      }
    }
    return null;
  }

  CodeToken? code(String src) {
    if (src.isEmpty) return null;
    final c = src.codeUnitAt(0);
    if (c != 0x20 && c != 0x09) return null;
    final cap = rules.block.code.firstMatch(src);
    if (cap != null) {
      final cap0 = cap[0]!;
      final raw = options.pedantic ? cap0 : trimTrailingBlankLines(cap0);
      final text = raw.replaceAll(rules.other.codeRemoveIndent, '');
      return CodeToken(
        raw: raw,
        codeBlockStyle: 'indented',
        text: text,
      );
    }
    return null;
  }

  CodeToken? fences(String src) {
    final fencesRe = rules.block.fences;
    if (fencesRe == null) return null;
    final c = _firstAfterUpTo3Spaces(src);
    if (c != 0x60 && c != 0x7E) return null;
    final cap = fencesRe.firstMatch(src);
    if (cap != null) {
      final raw = cap[0]!;
      final text = _indentCodeCompensation(raw, cap[3] ?? '', rules);
      final rawLang = cap[2]!;
      final lang = rawLang.isNotEmpty
          ? jsTrim(rawLang).replaceAllMapped(
              rules.inline.anyPunctuation,
              (m) => m[1]!,
            )
          : rawLang;
      return CodeToken(
        raw: raw,
        lang: lang,
        text: text,
      );
    }
    return null;
  }

  HeadingToken? heading(String src) {
    final c = options.pedantic
        ? (src.isEmpty ? -1 : src.codeUnitAt(0))
        : _firstAfterUpTo3Spaces(src);
    if (c != 0x23) return null;
    final cap = rules.block.heading.firstMatch(src);
    if (cap != null) {
      var text = jsTrim(cap[2]!);

      if (text.isNotEmpty && text.codeUnitAt(text.length - 1) == 0x23 /* # */) {
        final trimmed = rtrim(text, 0x23);
        if (options.pedantic) {
          text = jsTrim(trimmed);
        } else if (trimmed.isEmpty ||
            rules.other.endingSpaceTabChar.hasMatch(trimmed)) {
          text = jsTrim(trimmed);
        }
      }

      return HeadingToken(
        raw: rtrim(cap[0]!, 0x0A),
        depth: cap[1]!.length,
        text: text,
        tokens: lexer.inline(text),
      );
    }
    return null;
  }

  HrToken? hr(String src) {
    final c = _firstAfterUpTo3Spaces(src);
    if (c != 0x2D && c != 0x5F && c != 0x2A) return null;
    final cap = rules.block.hr.firstMatch(src);
    if (cap != null) {
      return HrToken(raw: rtrim(cap[0]!, 0x0A));
    }
    return null;
  }

  BlockquoteToken? blockquote(String src) {
    if (_firstAfterUpTo3Spaces(src) != 0x3E) return null;
    final cap = rules.block.blockquote.firstMatch(src);
    if (cap == null) return null;

    var lines = rtrim(cap[0]!, 0x0A).split('\n');
    var raw = '';
    var text = '';
    final tokens = <Token>[];

    while (lines.isNotEmpty) {
      var inBlockquote = false;
      final currentLines = <String>[];

      var i = 0;
      for (; i < lines.length; i++) {
        if (rules.other.blockquoteStart.hasMatch(lines[i])) {
          currentLines.add(lines[i]);
          inBlockquote = true;
        } else if (!inBlockquote) {
          currentLines.add(lines[i]);
        } else {
          break;
        }
      }
      lines = lines.sublist(i);

      final currentRaw = currentLines.join('\n');
      final currentText = currentRaw
          .replaceAllMapped(
            rules.other.blockquoteSetextReplace,
            (m) => '\n    ${m[1]!}',
          )
          .replaceAll(rules.other.blockquoteSetextReplace2, '');
      raw = raw.isNotEmpty ? '$raw\n$currentRaw' : currentRaw;
      text = text.isNotEmpty ? '$text\n$currentText' : currentText;

      final top = lexer.state.top;
      lexer.state.top = true;
      lexer.blockTokens(currentText, tokens, true);
      lexer.state.top = top;

      if (lines.isEmpty) {
        break;
      }

      final lastToken = tokens.isNotEmpty ? tokens.last : null;

      if (lastToken is CodeToken ||
          lastToken is HeadingToken ||
          lastToken is HrToken ||
          lastToken is HtmlToken) {
        break;
      } else if (lastToken is BlockquoteToken) {
        final continuation = lines.join('\n');
        final newText =
            '${lastToken.raw}\n${continuation.replaceAll(rules.other.blockquoteSetextReplace2, '')}';
        final newToken = blockquote(newText)!;
        tokens[tokens.length - 1] = newToken;

        var leftover = jsSubstring(newText, newToken.raw.length);
        if (leftover.startsWith('\n')) leftover = leftover.substring(1);
        final leftoverCount = leftover.isNotEmpty
            ? leftover.split('\n').length
            : 0;
        final List<String> consumed;
        if (leftoverCount > 0) {
          final end = lines.length - leftoverCount;
          consumed = end > 0 ? lines.sublist(0, end) : const <String>[];
        } else {
          consumed = lines;
        }
        if (consumed.isNotEmpty) {
          raw = '$raw\n${consumed.join('\n')}';
        }
        text =
            jsSubstring(text, 0, text.length - lastToken.text.length) +
            newToken.text;
        break;
      } else if (lastToken is ListToken) {
        final newText = '${lastToken.raw}\n${lines.join('\n')}';
        final newToken = list(newText)!;
        tokens[tokens.length - 1] = newToken;

        raw =
            jsSubstring(raw, 0, raw.length - lastToken.raw.length) +
            newToken.raw;
        text =
            jsSubstring(text, 0, text.length - lastToken.raw.length) +
            newToken.raw;
        lines = jsSubstring(newText, newToken.raw.length).split('\n');
        continue;
      }
    }

    return BlockquoteToken(
      raw: raw,
      tokens: tokens,
      text: text,
    );
  }

  ListToken? list(String src) {
    final c0 = _firstAfterUpTo3Spaces(src);
    if (c0 != 0x2A && c0 != 0x2B && c0 != 0x2D && (c0 < 0x30 || c0 > 0x39)) {
      return null;
    }
    var cap = rules.block.list.firstMatch(src);
    if (cap == null) return null;

    final listSrc = src;
    var bull = jsTrim(cap[1]!);
    final isOrdered = bull.length > 1;

    final listToken = ListToken(
      raw: '',
      ordered: isOrdered,
      start: isOrdered ? int.parse(bull.substring(0, bull.length - 1)) : 0,
      loose: false,
      items: <ListItemToken>[],
    );

    bull = isOrdered
        ? '\\d{1,9}\\${bull.substring(bull.length - 1)}'
        : '\\$bull';

    if (options.pedantic) {
      bull = isOrdered ? bull : '[*+-]';
    }

    final itemRegex = rules.other.listItemRegex(bull);
    var endsWithBlankLine = false;

    while (src.isNotEmpty) {
      var endEarly = false;
      var itemContents = '';
      cap = itemRegex.firstMatch(src);
      if (cap == null) {
        break;
      }

      if (rules.block.hr.hasMatch(src)) {
        break;
      }

      final itemStart = listSrc.length - src.length;
      src = src.substring(cap[0]!.length);

      final cap1 = cap[1]!;
      final cap2 = cap[2]!;
      final firstLine = firstLineOf(cap2);
      final bulletIndent = cap1.length;
      var line = options.pedantic
          ? expandTabs(firstLine, bulletIndent)
          : firstLine.replaceFirstMapped(
              rules.other.leadingSpaceTab,
              (m) => expandTabs(m[0]!, bulletIndent),
            );
      var nextLine = firstLineOf(src);
      var blankLine = jsTrim(line).isEmpty;

      var indent = 0;
      if (options.pedantic) {
        indent = 2;
        itemContents = jsTrimStart(line);
      } else if (blankLine) {
        indent = bulletIndent + 1;
      } else {
        indent = indexOfNonSpace(line);
        indent = indent > 4 ? 1 : indent;
        itemContents = jsSlice(line, indent);
        indent += bulletIndent;
      }

      if (blankLine && isBlankLine(nextLine)) {
        src = jsSubstring(src, nextLine.length + 1);
        endEarly = true;
      }

      if (!endEarly) {
        final nextBulletRegex = rules.other.nextBulletRegex(indent);
        final hrRegex = rules.other.hrRegex(indent);
        final fencesBeginRegex = rules.other.fencesBeginRegex(indent);
        final headingBeginRegex = rules.other.headingBeginRegex(indent);
        final htmlBeginRegex = rules.other.htmlBeginRegex(indent);
        final blockquoteBeginRegex = rules.other.blockquoteBeginRegex(indent);

        while (src.isNotEmpty) {
          final rawLine = firstLineOf(src);
          final String nextLineWithoutTabs;
          nextLine = rawLine;

          if (options.pedantic) {
            nextLine = nextLine.replaceAll(rules.other.listReplaceNesting, '  ');
            nextLineWithoutTabs = nextLine;
          } else {
            nextLineWithoutTabs = nextLine.replaceFirstMapped(
              rules.other.leadingSpaceTab,
              (m) => expandTabs(m[0]!),
            );
          }

          if (fencesBeginRegex.hasMatch(nextLine)) break;
          if (headingBeginRegex.hasMatch(nextLine)) break;
          if (htmlBeginRegex.hasMatch(nextLine)) break;
          if (blockquoteBeginRegex.hasMatch(nextLine)) break;
          if (nextBulletRegex.hasMatch(nextLine)) break;
          if (hrRegex.hasMatch(nextLine)) break;

          if (indexOfNonSpace(nextLineWithoutTabs) >= indent ||
              jsTrim(nextLine).isEmpty) {
            itemContents += '\n${jsSlice(nextLineWithoutTabs, indent)}';
          } else {
            if (blankLine) break;

            if (indexOfNonSpace(line.replaceAll('\t', '    ')) >= 4) break;
            if (fencesBeginRegex.hasMatch(line)) break;
            if (headingBeginRegex.hasMatch(line)) break;
            if (hrRegex.hasMatch(line)) break;

            itemContents += '\n$nextLine';
          }

          blankLine = jsTrim(nextLine).isEmpty;
          src = jsSubstring(src, rawLine.length + 1);
          line = jsSlice(nextLineWithoutTabs, indent);
        }
      }

      final raw = listSrc.substring(itemStart, listSrc.length - src.length);

      if (!listToken.loose) {
        if (endsWithBlankLine) {
          listToken.loose = true;
        } else if (rules.other.doubleBlankLine.hasMatch(raw)) {
          endsWithBlankLine = true;
        }
      }

      listToken.items.add(
        ListItemToken(
          raw: raw,
          task: options.gfm && rules.other.listIsTask.hasMatch(itemContents),
          loose: false,
          text: itemContents,
          tokens: <Token>[],
        ),
      );
    }

    if (listToken.items.isEmpty) {
      return null;
    }

    final lastItem = listToken.items.last;
    lastItem.raw = jsTrimEnd(lastItem.raw);
    lastItem.text = jsTrimEnd(lastItem.text);
    listToken.raw = jsTrimEnd(listSrc.substring(0, listSrc.length - src.length));

    // First pass: tokenize items and finalize list.loose from spacers.
    for (var idx = 0; idx < listToken.items.length; idx++) {
      final item = listToken.items[idx];
      lexer.state.top = false;
      item.tokens = lexer.blockTokens(item.text, <Token>[]);

      if (!listToken.loose) {
        for (var j = 0; j < item.tokens.length; j++) {
          final tok = item.tokens[j];
          if (tok is SpaceToken && rules.other.anyLine.hasMatch(tok.raw)) {
            listToken.loose = true;
            break;
          }
        }
      }
    }

    // Second pass: place task checkboxes using final list.loose.
    for (var idx = 0; idx < listToken.items.length; idx++) {
      final item = listToken.items[idx];
      final itemToken = item.tokens.isNotEmpty ? item.tokens[0] : null;
      if (item.task &&
          (itemToken is TextToken || itemToken is ParagraphToken)) {
        item.text = item.text.replaceFirst(rules.other.listReplaceTask, '');
        if (itemToken is TextToken) {
          itemToken.raw = itemToken.raw.replaceFirst(
            rules.other.listReplaceTask,
            '',
          );
          itemToken.text = itemToken.text.replaceFirst(
            rules.other.listReplaceTask,
            '',
          );
        } else if (itemToken is ParagraphToken) {
          itemToken.raw = itemToken.raw.replaceFirst(
            rules.other.listReplaceTask,
            '',
          );
          itemToken.text = itemToken.text.replaceFirst(
            rules.other.listReplaceTask,
            '',
          );
        }

        for (var i = lexer.inlineQueue.length - 1; i >= 0; i--) {
          final entry = lexer.inlineQueue[i];
          if (rules.other.listIsTask.hasMatch(entry.src)) {
            entry.src = entry.src.replaceFirst(rules.other.listReplaceTask, '');
            break;
          }
        }

        final taskRaw = rules.other.listTaskCheckbox.firstMatch(item.raw);
        if (taskRaw != null) {
          final boxStr = taskRaw[0]!;
          final checkboxToken = CheckboxToken(
            raw: '$boxStr ',
            checked: boxStr != '[ ]',
          );
          item.checked = checkboxToken.checked;
          if (listToken.loose) {
            final first = item.tokens.isNotEmpty ? item.tokens[0] : null;
            if (first is ParagraphToken) {
              first.raw = checkboxToken.raw + first.raw;
              first.text = checkboxToken.raw + first.text;
              first.tokens.insert(0, checkboxToken);
            } else if (first is TextToken && first.tokens != null) {
              first.raw = checkboxToken.raw + first.raw;
              first.text = checkboxToken.raw + first.text;
              first.tokens!.insert(0, checkboxToken);
            } else {
              item.tokens.insert(
                0,
                ParagraphToken(
                  raw: checkboxToken.raw,
                  text: checkboxToken.raw,
                  tokens: <Token>[checkboxToken],
                ),
              );
            }
          } else {
            item.tokens.insert(0, checkboxToken);
          }
        }
      } else if (item.task) {
        item.task = false;
      }
    }

    // Promote text tokens to paragraphs when list is loose.
    if (listToken.loose) {
      for (var idx = 0; idx < listToken.items.length; idx++) {
        final item = listToken.items[idx];
        item.loose = true;
        for (var j = 0; j < item.tokens.length; j++) {
          final tok = item.tokens[j];
          if (tok is TextToken) {
            item.tokens[j] = ParagraphToken(
              raw: tok.raw,
              text: tok.text,
              tokens: tok.tokens ?? <Token>[],
            );
          }
        }
      }
    }

    return listToken;
  }

  HtmlToken? html(String src) {
    final c = options.pedantic
        ? _firstAfterAnySpaces(src)
        : _firstAfterUpTo3Spaces(src);
    if (c != 0x3C) return null;
    final cap = rules.block.html.firstMatch(src);
    if (cap != null) {
      final raw = trimTrailingBlankLines(cap[0]!);
      final g1 = cap[1];
      return HtmlToken(
        block: true,
        raw: raw,
        pre: g1 == 'pre' || g1 == 'script' || g1 == 'style',
        text: raw,
      );
    }
    return null;
  }

  DefToken? def(String src) {
    final c = options.pedantic
        ? _firstAfterAnySpaces(src)
        : _firstAfterUpTo3Spaces(src);
    if (c != 0x5B) return null;
    final cap = rules.block.def.firstMatch(src);
    if (cap == null) return null;

    final cap2 = cap[2]!;
    if (!cap2.startsWith('<') && findClosingBracket(cap2, 0x28, 0x29) != -1) {
      return null;
    }

    final tag = normalizeLabel(
      cap[1]!,
    ).replaceAll(rules.other.multipleSpaceGlobal, ' ');
    final href = cap2.isNotEmpty
        ? cap2
              .replaceFirstMapped(rules.other.hrefBrackets, (m) => m[1]!)
              .replaceAllMapped(rules.inline.anyPunctuation, (m) => m[1]!)
        : '';
    final cap3 = cap[3];
    final title = (cap3 != null && cap3.isNotEmpty)
        ? cap3
              .substring(1, cap3.length - 1)
              .replaceAllMapped(rules.inline.anyPunctuation, (m) => m[1]!)
        : cap3;
    return DefToken(
      tag: tag,
      raw: rtrim(cap[0]!, 0x0A),
      href: href,
      title: title,
    );
  }

  TableToken? table(String src) {
    final tableRe = rules.block.table;
    if (tableRe == null || src.indexOf('\n') == -1) return null;
    final cap = tableRe.firstMatch(src);
    if (cap == null) return null;

    final cap2 = cap[2]!;
    if (!rules.other.tableDelimiter.hasMatch(cap2)) {
      return null;
    }

    final headers = splitCells(cap[1]!);
    final aligns = cap2.replaceAll(rules.other.tableAlignChars, '').split('|');
    final cap3 = cap[3];
    final rows = (cap3 != null && jsTrim(cap3).isNotEmpty)
        ? cap3.replaceFirst(rules.other.tableRowBlankLine, '').split('\n')
        : const <String>[];

    if (headers.length != aligns.length) {
      return null;
    }

    final alignList = <String?>[];
    for (var i = 0; i < aligns.length; i++) {
      final a = aligns[i];
      if (rules.other.tableAlignRight.hasMatch(a)) {
        alignList.add('right');
      } else if (rules.other.tableAlignCenter.hasMatch(a)) {
        alignList.add('center');
      } else if (rules.other.tableAlignLeft.hasMatch(a)) {
        alignList.add('left');
      } else {
        alignList.add(null);
      }
    }

    final headerCells = <TableCellToken>[];
    for (var i = 0; i < headers.length; i++) {
      headerCells.add(
        TableCellToken(
          text: headers[i],
          tokens: lexer.inline(headers[i]),
          header: true,
          align: alignList[i],
        ),
      );
    }

    final rowList = <List<TableCellToken>>[];
    final colCount = headerCells.length;
    for (var r = 0; r < rows.length; r++) {
      final cells = splitCells(rows[r], colCount);
      final rowCells = <TableCellToken>[];
      for (var c = 0; c < cells.length; c++) {
        rowCells.add(
          TableCellToken(
            text: cells[c],
            tokens: lexer.inline(cells[c]),
            header: false,
            align: alignList[c],
          ),
        );
      }
      rowList.add(rowCells);
    }

    return TableToken(
      raw: rtrim(cap[0]!, 0x0A),
      header: headerCells,
      align: alignList,
      rows: rowList,
    );
  }

  HeadingToken? lheading(String src) {
    if (src.indexOf('\n') == -1) return null;
    final cap = rules.block.lheading.firstMatch(src);
    if (cap != null) {
      final text = jsTrim(cap[1]!);
      return HeadingToken(
        raw: rtrim(cap[0]!, 0x0A),
        depth: cap[2]!.codeUnitAt(0) == 0x3D /* = */ ? 1 : 2,
        text: text,
        tokens: lexer.inline(text),
      );
    }
    return null;
  }

  ParagraphToken? paragraph(String src) {
    final cap = rules.block.paragraph.firstMatch(src);
    if (cap != null) {
      final cap1 = cap[1]!;
      final text =
          cap1.isNotEmpty && cap1.codeUnitAt(cap1.length - 1) == 0x0A
          ? cap1.substring(0, cap1.length - 1)
          : cap1;
      return ParagraphToken(
        raw: cap[0]!,
        text: text,
        tokens: lexer.inline(text),
      );
    }
    return null;
  }

  TextToken? text(String src) {
    final cap = rules.block.text.firstMatch(src);
    if (cap != null) {
      final raw = cap[0]!;
      return TextToken(
        raw: raw,
        text: raw,
        tokens: lexer.inline(raw),
      );
    }
    return null;
  }

  EscapeToken? escape(String src) {
    if (src.isEmpty || src.codeUnitAt(0) != 0x5C) return null;
    final cap = rules.inline.escape.firstMatch(src);
    if (cap != null) {
      return EscapeToken(
        raw: cap[0]!,
        text: cap[1]!,
      );
    }
    return null;
  }

  HtmlToken? tag(String src) {
    if (src.isEmpty || src.codeUnitAt(0) != 0x3C) return null;
    final cap = rules.inline.tag.firstMatch(src);
    if (cap != null) {
      final raw = cap[0]!;
      if (!lexer.state.inLink && rules.other.startATag.hasMatch(raw)) {
        lexer.state.inLink = true;
      } else if (lexer.state.inLink && rules.other.endATag.hasMatch(raw)) {
        lexer.state.inLink = false;
      }
      if (!lexer.state.inRawBlock &&
          rules.other.startPreScriptTag.hasMatch(raw)) {
        lexer.state.inRawBlock = true;
      } else if (lexer.state.inRawBlock &&
          rules.other.endPreScriptTag.hasMatch(raw)) {
        lexer.state.inRawBlock = false;
      }

      return HtmlToken(
        raw: raw,
        inLink: lexer.state.inLink,
        inRawBlock: lexer.state.inRawBlock,
        block: false,
        text: raw,
      );
    }
    return null;
  }

  Token? link(String src) {
    if (!lexer.state.linkParenPossible || src.isEmpty) {
      return null;
    }
    final c = src.codeUnitAt(0);
    if (c != 0x5B && c != 0x21) return null;
    final cap = rules.inline.link.firstMatch(src);
    if (cap == null) return null;

    var cap0 = cap[0]!;
    final cap1 = cap[1]!;
    var cap2 = cap[2]!;
    var cap3 = cap.groupCount >= 3 ? (cap[3] ?? '') : '';

    final labelStart =
        cap0.isNotEmpty && cap0.codeUnitAt(0) == 0x21 /* ! */ ? 2 : 1;
    if (!options.pedantic &&
        _isLabelEndInsideToken(src, cap1, labelStart, rules)) {
      return null;
    }

    final trimmedUrl = jsTrim(cap2);
    if (!options.pedantic && trimmedUrl.startsWith('<')) {
      if (!trimmedUrl.endsWith('>')) {
        return null;
      }
      final rtrimSlash = rtrim(
        trimmedUrl.substring(0, trimmedUrl.length - 1),
        0x5C,
      );
      if ((trimmedUrl.length - rtrimSlash.length).isEven) {
        return null;
      }
    } else {
      final lastParenIndex = findClosingBracket(cap2, 0x28, 0x29);
      if (lastParenIndex == -2) {
        return null;
      }
      if (lastParenIndex > -1) {
        final start = cap0.startsWith('!') ? 5 : 4;
        final linkLen = start + cap1.length + lastParenIndex;
        cap2 = cap2.substring(0, lastParenIndex);
        cap0 = jsTrim(jsSubstring(cap0, 0, linkLen));
        cap3 = '';
      }
    }

    var href = cap2;
    var title = '';
    if (options.pedantic) {
      final linkMatch = rules.other.pedanticHrefTitle.firstMatch(href);
      if (linkMatch != null) {
        href = linkMatch[1]!;
        title = linkMatch[3]!;
      }
    } else {
      title = cap3.isNotEmpty ? jsSlice(cap3, 1, -1) : '';
    }

    href = jsTrim(href);
    if (href.startsWith('<')) {
      if (options.pedantic && !trimmedUrl.endsWith('>')) {
        href = href.substring(1);
      } else {
        href = jsSlice(href, 1, -1);
      }
    }

    return _outputLink(
      cap0,
      cap1,
      LinkDef(
        href: href.isNotEmpty
            ? href.replaceAllMapped(rules.inline.anyPunctuation, (m) => m[1]!)
            : href,
        title: title.isNotEmpty
            ? title.replaceAllMapped(rules.inline.anyPunctuation, (m) => m[1]!)
            : title,
      ),
      cap0,
      lexer,
      rules,
    );
  }

  Token? reflink(String src, Map<String, LinkDef> links) {
    if (src.isEmpty) return null;
    final c = src.codeUnitAt(0);
    if (c != 0x5B && c != 0x21) return null;
    final cap =
        rules.inline.reflink.firstMatch(src) ??
        rules.inline.nolink.firstMatch(src);
    if (cap == null) return null;

    final cap0 = cap[0]!;
    final cap1 = cap[1]!;
    final labelStart =
        cap0.isNotEmpty && cap0.codeUnitAt(0) == 0x21 /* ! */ ? 2 : 1;
    if (!options.pedantic &&
        _isLabelEndInsideToken(src, cap1, labelStart, rules)) {
      return null;
    }

    final g2 = cap.groupCount >= 2 ? cap[2] : null;
    final rawLinkStr = (g2 != null && g2.isNotEmpty) ? g2 : cap1;
    final linkString = rawLinkStr.replaceAll(
      rules.other.multipleSpaceGlobal,
      ' ',
    );
    final linkDef = links[normalizeLabel(linkString)];
    if (linkDef == null) {
      final ch = cap0.substring(0, 1);
      return TextToken(raw: ch, text: ch);
    }
    return _outputLink(cap0, cap1, linkDef, cap0, lexer, rules);
  }

  Token? emStrong(String src, String maskedSrc, [String prevChar = '']) {
    if (src.isEmpty) return null;
    final c = src.codeUnitAt(0);
    if (c != 0x2A && c != 0x5F) return null;
    final lMatch = rules.inline.emStrongLDelim.firstMatch(src);
    if (lMatch == null) return null;
    final g1 = lMatch[1];
    final g2 = lMatch[2];
    final g3 = lMatch[3];
    final g4 = lMatch[4];
    if (g1 == null && g2 == null && g3 == null && g4 == null) return null;

    if (g4 != null &&
        prevChar.isNotEmpty &&
        rules.other.unicodeAlphaNumeric.hasMatch(prevChar)) {
      return null;
    }

    final nextChar = g1 ?? g3 ?? '';
    if (nextChar.isEmpty ||
        prevChar.isEmpty ||
        rules.inline.punctuation.hasMatch(prevChar)) {
      final m0 = lMatch[0]!;
      final afterChar = (g1 ?? g2 ?? g3 ?? g4)!;
      final lLength = m0.length - afterChar.length;
      var delimTotal = lLength;
      var midDelimTotal = 0;

      final delimCode = m0.codeUnitAt(0);
      final midRun =
          prevChar.length == 1 && prevChar.codeUnitAt(0) == delimCode;
      final endReg = delimCode == 0x2A /* * */
          ? rules.inline.emStrongRDelimAst
          : rules.inline.emStrongRDelimUnd;

      final sliceStart = maskedSrc.length - src.length + lLength;
      final clippedMasked = sliceStart <= 0
          ? maskedSrc
          : (sliceStart >= maskedSrc.length
                ? ''
                : maskedSrc.substring(sliceStart));

      for (final rMatch in endReg.allMatches(clippedMasked)) {
        final rg1 = rMatch[1];
        final rg2 = rMatch[2];
        final rg3 = rMatch[3];
        final rg4 = rMatch[4];
        final rg5 = rMatch[5];
        final rg6 = rMatch.groupCount >= 6 ? rMatch[6] : null;
        final rDelim = rg1 ?? rg2 ?? rg3 ?? rg4 ?? rg5 ?? rg6;
        if (rDelim == null) continue;

        var rLength = rDelim.length;

        if (rg3 != null || rg4 != null) {
          delimTotal += rLength;
          continue;
        } else if (rg5 != null || rg6 != null) {
          if (lLength % 3 != 0 && (lLength + rLength) % 3 == 0) {
            midDelimTotal += rLength;
            continue;
          }
          if (midRun) {
            break;
          }
        }

        delimTotal -= rLength;
        if (delimTotal > 0) continue;

        final adj = rLength + delimTotal + midDelimTotal;
        if (adj < rLength) rLength = adj;
        final lastCharLength = firstCodePointUtf16Length(rMatch[0]!);
        final rawEnd = lLength + rMatch.start + lastCharLength + rLength;
        final raw = rawEnd >= src.length ? src : src.substring(0, rawEnd);

        final minLen = lLength < rLength ? lLength : rLength;
        if (minLen.isOdd) {
          final text = raw.substring(1, raw.length - 1);
          return EmToken(
            raw: raw,
            text: text,
            tokens: lexer.inlineTokens(text),
          );
        }

        final text = raw.substring(2, raw.length - 2);
        return StrongToken(
          raw: raw,
          text: text,
          tokens: lexer.inlineTokens(text),
        );
      }
    }
    return null;
  }

  CodespanToken? codespan(String src) {
    if (src.isEmpty || src.codeUnitAt(0) != 0x60) return null;
    final cap = rules.inline.code.firstMatch(src);
    if (cap != null) {
      var text = cap[2]!.replaceAll('\n', ' ');
      final hasNonSpaceChars = indexOfNonSpace(text) != -1;
      final hasSpaceCharsOnBothEnds =
          text.isNotEmpty &&
          text.codeUnitAt(0) == 0x20 &&
          text.codeUnitAt(text.length - 1) == 0x20;
      if (hasNonSpaceChars && hasSpaceCharsOnBothEnds) {
        text = text.substring(1, text.length - 1);
      }
      return CodespanToken(raw: cap[0]!, text: text);
    }
    return null;
  }

  BrToken? br(String src) {
    if (src.isEmpty) return null;
    final c = src.codeUnitAt(0);
    if (c != 0x20 && c != 0x5C && c != 0x0A) return null;
    final cap = rules.inline.br.firstMatch(src);
    if (cap != null) {
      return BrToken(raw: cap[0]!);
    }
    return null;
  }

  DelToken? del(String src, String maskedSrc, [String prevChar = '']) {
    final delLDelim = rules.inline.delLDelim;
    if (delLDelim == null || src.isEmpty || src.codeUnitAt(0) != 0x7E) {
      return null;
    }
    final lMatch = delLDelim.firstMatch(src);
    if (lMatch == null) return null;

    final nextChar = lMatch[1] ?? '';
    if (nextChar.isEmpty ||
        prevChar.isEmpty ||
        rules.inline.punctuation.hasMatch(prevChar)) {
      final m0 = lMatch[0]!;
      final lLength =
          (m0.length > 1 && m0.codeUnitAt(1) == 0x7E /* ~ */) ? 2 : 1;
      var delimTotal = lLength;
      final endReg = rules.inline.delRDelim!;

      final sliceStart = maskedSrc.length - src.length + lLength;
      final clippedMasked = sliceStart <= 0
          ? maskedSrc
          : (sliceStart >= maskedSrc.length
                ? ''
                : maskedSrc.substring(sliceStart));

      for (final rMatch in endReg.allMatches(clippedMasked)) {
        final rg1 = rMatch[1];
        final rg2 = rMatch[2];
        final rg3 = rMatch[3];
        final rg4 = rMatch[4];
        final rg5 = rMatch[5];
        final rg6 = rMatch[6];
        final rDelim = rg1 ?? rg2 ?? rg3 ?? rg4 ?? rg5 ?? rg6;
        if (rDelim == null) continue;

        var rLength = rDelim.length;
        if (rLength != lLength) continue;

        if (rg3 != null || rg4 != null) {
          delimTotal += rLength;
          continue;
        }

        delimTotal -= rLength;
        if (delimTotal > 0) continue;

        final adj = rLength + delimTotal;
        if (adj < rLength) rLength = adj;
        final lastCharLength = firstCodePointUtf16Length(rMatch[0]!);
        final rawEnd = lLength + rMatch.start + lastCharLength + rLength;
        final raw = rawEnd >= src.length ? src : src.substring(0, rawEnd);
        final text = raw.substring(lLength, raw.length - lLength);
        return DelToken(
          raw: raw,
          text: text,
          tokens: lexer.inlineTokens(text),
        );
      }
    }
    return null;
  }

  LinkToken? autolink(String src) {
    if (src.isEmpty || src.codeUnitAt(0) != 0x3C) return null;
    final cap = rules.inline.autolink.firstMatch(src);
    if (cap != null) {
      final text = cap[1]!;
      final href = cap[2] == '@' ? 'mailto:$text' : text;
      return LinkToken(
        raw: cap[0]!,
        text: text,
        href: href,
        title: null,
        autolink: true,
        tokens: <Token>[TextToken(raw: text, text: text)],
      );
    }
    return null;
  }

  LinkToken? url(String src) {
    final urlRe = rules.inline.url;
    if (urlRe == null || src.isEmpty || !_isUrlStartChar(src.codeUnitAt(0))) {
      return null;
    }
    final cap = urlRe.firstMatch(src);
    if (cap == null) return null;

    var cap0 = cap[0]!;
    final String text;
    final String href;
    if (cap[2] == '@') {
      text = cap0;
      href = 'mailto:$text';
    } else {
      final backpedal = rules.inline.backpedal!;
      String prevCapZero;
      do {
        prevCapZero = cap0;
        cap0 = backpedal.firstMatch(cap0)?[0] ?? '';
      } while (prevCapZero != cap0);
      text = cap0;
      href = cap[1] == 'www.' ? 'http://$cap0' : cap0;
    }

    return LinkToken(
      raw: cap0,
      text: text,
      href: href,
      title: null,
      autolink: true,
      tokens: <Token>[TextToken(raw: text, text: text)],
    );
  }

  TextToken? inlineText(String src) {
    final cap = rules.inline.text.firstMatch(src);
    if (cap != null) {
      final raw = cap[0]!;
      final escaped = lexer.state.inRawBlock;
      return TextToken(
        raw: raw,
        text: escaped ? raw : decodeNumericCharacterReferences(raw),
        escaped: escaped,
      );
    }
    return null;
  }
}
