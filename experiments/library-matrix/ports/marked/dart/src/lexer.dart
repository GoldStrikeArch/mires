import 'helpers.dart';
import 'options.dart';
import 'rules.dart';
import 'tokenizer.dart';
import 'tokens.dart';

String _maskAnyPunctuationOutsideCode(Match m) {
  final match0 = m[0]!;
  final code = m[1];
  final punct = m[2];
  if (punct != null) {
    return '+' * match0.length;
  }
  if (code != null) {
    final inner = match0.substring(
      code.length,
      match0.length - code.length,
    );
    return '$code${inner.replaceAll('`', '+')}$code';
  }
  return match0;
}

String _maskBlockSkip(Match m) {
  final match0 = m[0]!;
  final context = m[2];
  final offset = context?.length ?? 0;
  return '${match0.substring(0, offset)}[${'a' * (match0.length - offset - 2)}]';
}

final class LexerState {
  bool inLink = false;
  bool inRawBlock = false;
  bool linkEmitted = false;
  bool linkParenPossible = true;
  bool top = true;
}

/// Faithful port of `marked@18.1.1` `_Lexer`.
final class Lexer {
  Lexer(this.options) : tokenizer = Tokenizer(options) {
    tokenizer.lexer = this;

    final allRules = markedRules;
    final BlockRules blockRules;
    final InlineRules inlineRules;
    if (options.pedantic) {
      blockRules = allRules.blockPedantic;
      inlineRules = allRules.inlinePedantic;
    } else if (options.gfm) {
      blockRules = allRules.blockGfm;
      inlineRules = options.breaks ? allRules.inlineBreaks : allRules.inlineGfm;
    } else {
      blockRules = allRules.blockNormal;
      inlineRules = allRules.inlineNormal;
    }

    tokenizer.rules = ActiveRules(
      other: allRules.other,
      block: blockRules,
      inline: inlineRules,
    );
  }

  final List<Token> tokens = <Token>[];
  final Map<String, LinkDef> links = <String, LinkDef>{};
  final MarkedOptions options;
  final LexerState state = LexerState();
  final List<InlineQueueEntry> inlineQueue = <InlineQueueEntry>[];
  final Tokenizer tokenizer;

  static List<Token> lexStatic(String src, MarkedOptions options) =>
      Lexer(options).lex(src);

  static List<Token> lexInlineStatic(String src, MarkedOptions options) =>
      Lexer(options).inlineTokens(src);

  List<Token> lex(String src) {
    final normalized = src.replaceAll(markedRules.other.carriageReturn, '\n');

    blockTokens(normalized, tokens);

    for (var i = 0; i < inlineQueue.length; i++) {
      final next = inlineQueue[i];
      inlineTokens(next.src, next.tokens);
    }
    inlineQueue.clear();

    return tokens;
  }

  List<Token> blockTokens(
    String src, [
    List<Token>? outTokens,
    bool lastParagraphClipped = false,
  ]) {
    final target = outTokens ?? <Token>[];
    tokenizer.lexer = this;
    if (options.pedantic) {
      src = src
          .replaceAll('\t', '    ')
          .replaceAll(markedRules.other.spaceLine, '');
    }

    var srcLength = src.length + 1;
    while (src.isNotEmpty) {
      if (src.length < srcLength) {
        srcLength = src.length;
      } else {
        _infiniteLoopError(src.codeUnitAt(0));
      }

      // newline
      final spaceTok = tokenizer.space(src);
      if (spaceTok != null) {
        src = src.substring(spaceTok.raw.length);
        if (spaceTok.raw.length == 1 && target.isNotEmpty) {
          target.last.raw += '\n';
        } else {
          target.add(spaceTok);
        }
        continue;
      }

      // code
      final codeTok = tokenizer.code(src);
      if (codeTok != null) {
        src = src.substring(codeTok.raw.length);
        final lastToken = target.isNotEmpty ? target.last : null;
        if (lastToken is ParagraphToken) {
          lastToken.raw +=
              (lastToken.raw.endsWith('\n') ? '' : '\n') + codeTok.raw;
          lastToken.text += '\n${codeTok.text}';
          inlineQueue.last.src = lastToken.text;
        } else if (lastToken is TextToken) {
          lastToken.raw +=
              (lastToken.raw.endsWith('\n') ? '' : '\n') + codeTok.raw;
          lastToken.text += '\n${codeTok.text}';
          inlineQueue.last.src = lastToken.text;
        } else {
          target.add(codeTok);
        }
        continue;
      }

      // fences
      final fencesTok = tokenizer.fences(src);
      if (fencesTok != null) {
        src = src.substring(fencesTok.raw.length);
        target.add(fencesTok);
        continue;
      }

      // heading
      final headingTok = tokenizer.heading(src);
      if (headingTok != null) {
        src = src.substring(headingTok.raw.length);
        target.add(headingTok);
        continue;
      }

      // hr
      final hrTok = tokenizer.hr(src);
      if (hrTok != null) {
        src = src.substring(hrTok.raw.length);
        target.add(hrTok);
        continue;
      }

      // blockquote
      final bqTok = tokenizer.blockquote(src);
      if (bqTok != null) {
        src = jsSubstring(src, bqTok.raw.length);
        target.add(bqTok);
        continue;
      }

      // list
      final listTok = tokenizer.list(src);
      if (listTok != null) {
        src = src.substring(listTok.raw.length);
        target.add(listTok);
        continue;
      }

      // html
      final htmlTok = tokenizer.html(src);
      if (htmlTok != null) {
        src = src.substring(htmlTok.raw.length);
        target.add(htmlTok);
        continue;
      }

      // def
      final defTok = tokenizer.def(src);
      if (defTok != null) {
        src = src.substring(defTok.raw.length);
        final lastToken = target.isNotEmpty ? target.last : null;
        if (lastToken is ParagraphToken) {
          lastToken.raw +=
              (lastToken.raw.endsWith('\n') ? '' : '\n') + defTok.raw;
          lastToken.text += '\n${defTok.raw}';
          inlineQueue.last.src = lastToken.text;
        } else if (lastToken is TextToken) {
          lastToken.raw +=
              (lastToken.raw.endsWith('\n') ? '' : '\n') + defTok.raw;
          lastToken.text += '\n${defTok.raw}';
          inlineQueue.last.src = lastToken.text;
        } else if (!links.containsKey(defTok.tag)) {
          links[defTok.tag] = LinkDef(href: defTok.href, title: defTok.title);
          target.add(defTok);
        }
        continue;
      }

      // table (gfm)
      final tableTok = tokenizer.table(src);
      if (tableTok != null) {
        src = src.substring(tableTok.raw.length);
        target.add(tableTok);
        continue;
      }

      // lheading
      final lheadingTok = tokenizer.lheading(src);
      if (lheadingTok != null) {
        src = src.substring(lheadingTok.raw.length);
        target.add(lheadingTok);
        continue;
      }

      // top-level paragraph
      if (state.top) {
        final paraTok = tokenizer.paragraph(src);
        if (paraTok != null) {
          final lastToken = target.isNotEmpty ? target.last : null;
          if (lastParagraphClipped && lastToken is ParagraphToken) {
            lastToken.raw +=
                (lastToken.raw.endsWith('\n') ? '' : '\n') + paraTok.raw;
            lastToken.text += '\n${paraTok.text}';
            inlineQueue.removeLast();
            inlineQueue.last.src = lastToken.text;
          } else {
            target.add(paraTok);
          }
          lastParagraphClipped = false;
          src = src.substring(paraTok.raw.length);
          continue;
        }
      }

      // text
      final textTok = tokenizer.text(src);
      if (textTok != null) {
        src = src.substring(textTok.raw.length);
        final lastToken = target.isNotEmpty ? target.last : null;
        if (lastToken is TextToken) {
          lastToken.raw +=
              (lastToken.raw.endsWith('\n') ? '' : '\n') + textTok.raw;
          lastToken.text += '\n${textTok.text}';
          inlineQueue.removeLast();
          inlineQueue.last.src = lastToken.text;
        } else {
          target.add(textTok);
        }
        continue;
      }

      if (src.isNotEmpty) {
        _infiniteLoopError(src.codeUnitAt(0));
      }
    }

    state.top = true;
    return target;
  }

  List<Token> inline(String src, [List<Token>? outTokens]) {
    final target = outTokens ?? <Token>[];
    inlineQueue.add(InlineQueueEntry(src, target));
    return target;
  }

  bool _linkInText(String text) {
    if (!text.contains('[')) {
      return false;
    }

    final linkRule = tokenizer.rules.inline.link;
    for (final match in tokenizer.rules.inline.blockSkip.allMatches(text)) {
      if (linkRule.hasMatch(match[0]!) &&
          (match.start == 0 || text.codeUnitAt(match.start - 1) != 0x21)) {
        return true;
      }
    }

    for (final match in tokenizer.rules.inline.reflinkSearch.allMatches(text)) {
      final match0 = match[0]!;
      final refStart = match0.lastIndexOf('[');
      if (match0.codeUnitAt(0) == 0x21 /* ! */ ||
          !links.containsKey(
            normalizeLabel(match0.substring(refStart + 1, match0.length - 1)),
          )) {
        continue;
      }
      if (refStart > 1 && _linkInText(match0.substring(1, refStart - 1))) {
        continue;
      }
      return true;
    }

    return false;
  }

  List<Token> inlineTokens(String src, [List<Token>? outTokens]) {
    final target = outTokens ?? <Token>[];
    tokenizer.lexer = this;
    final prevLinkParenPossible = state.linkParenPossible;
    state.linkParenPossible = prevLinkParenPossible && src.contains(')');
    try {
      return _inlineTokensInner(src, target);
    } finally {
      state.linkParenPossible = prevLinkParenPossible;
    }
  }

  String _computeMaskedSrc(String origSrc) {
    var maskedSrc = origSrc;

    // Mask out reflinks
    if (links.isNotEmpty && origSrc.contains('[')) {
      final reflinkSearch = tokenizer.rules.inline.reflinkSearch;
      String maskReflink(Match m) {
        final match0 = m[0]!;
        final refStart = match0.lastIndexOf('[');
        if (!links.containsKey(
          normalizeLabel(match0.substring(refStart + 1, match0.length - 1)),
        )) {
          return match0;
        }
        if (refStart > 1 && match0.codeUnitAt(0) != 0x21 /* ! */) {
          final text = match0.substring(1, refStart - 1);
          if (_linkInText(text)) {
            return '[${text.replaceAllMapped(reflinkSearch, maskReflink)}][${'a' * (match0.length - refStart - 2)}]';
          }
        }
        return '[${'a' * (match0.length - 2)}]';
      }

      maskedSrc = maskedSrc.replaceAllMapped(reflinkSearch, maskReflink);
    }

    // Mask out escaped characters outside code spans, and backticks inside code spans.
    if (maskedSrc.contains('\\') || maskedSrc.contains('`')) {
      maskedSrc = maskedSrc.replaceAllMapped(
        tokenizer.rules.inline.anyPunctuationOutsideCode,
        _maskAnyPunctuationOutsideCode,
      );
    }

    // Mask out other blocks (links, code spans, inline html)
    if (maskedSrc.contains('[') ||
        maskedSrc.contains('`') ||
        maskedSrc.contains('<')) {
      maskedSrc = maskedSrc.replaceAllMapped(
        tokenizer.rules.inline.blockSkip,
        _maskBlockSkip,
      );
    }

    return maskedSrc;
  }

  List<Token> _inlineTokensInner(String src, List<Token> target) {
    final origSrc = src;
    String? maskedSrc;

    var keepPrevChar = false;
    var prevChar = '';
    var srcLength = src.length + 1;

    while (src.isNotEmpty) {
      if (src.length < srcLength) {
        srcLength = src.length;
      } else {
        _infiniteLoopError(src.codeUnitAt(0));
      }

      if (!keepPrevChar) {
        prevChar = '';
      }
      keepPrevChar = false;

      final c0 = src.codeUnitAt(0);

      // escape
      if (c0 == 0x5C /* \ */) {
        final escTok = tokenizer.escape(src);
        if (escTok != null) {
          src = src.substring(escTok.raw.length);
          target.add(escTok);
          continue;
        }
      }

      // tag
      if (c0 == 0x3C /* < */) {
        final tagTok = tokenizer.tag(src);
        if (tagTok != null) {
          src = src.substring(tagTok.raw.length);
          target.add(tagTok);
          continue;
        }
      }

      // link, reflink, nolink
      if (c0 == 0x5B /* [ */ || c0 == 0x21 /* ! */) {
        final linkTok = tokenizer.link(src);
        if (linkTok != null) {
          src = src.substring(linkTok.raw.length);
          target.add(linkTok);
          continue;
        }

        final refTok = tokenizer.reflink(src, links);
        if (refTok != null) {
          src = src.substring(refTok.raw.length);
          final lastToken = target.isNotEmpty ? target.last : null;
          if (refTok is TextToken && lastToken is TextToken) {
            lastToken.raw += refTok.raw;
            lastToken.text += refTok.text;
          } else {
            target.add(refTok);
          }
          continue;
        }
      }

      // em & strong
      if (c0 == 0x2A /* * */ || c0 == 0x5F /* _ */) {
        final emTok = tokenizer.emStrong(
          src,
          maskedSrc ??= _computeMaskedSrc(origSrc),
          prevChar,
        );
        if (emTok != null) {
          src = src.substring(emTok.raw.length);
          target.add(emTok);
          continue;
        }
      }

      // code
      if (c0 == 0x60 /* ` */) {
        final codeTok = tokenizer.codespan(src);
        if (codeTok != null) {
          src = src.substring(codeTok.raw.length);
          target.add(codeTok);
          continue;
        }
      }

      // br
      if (c0 == 0x20 || c0 == 0x5C || c0 == 0x0A) {
        final brTok = tokenizer.br(src);
        if (brTok != null) {
          src = src.substring(brTok.raw.length);
          target.add(brTok);
          continue;
        }
      }

      // del (gfm)
      if (c0 == 0x7E /* ~ */ && tokenizer.rules.inline.delLDelim != null) {
        final delTok = tokenizer.del(
          src,
          maskedSrc ??= _computeMaskedSrc(origSrc),
          prevChar,
        );
        if (delTok != null) {
          src = src.substring(delTok.raw.length);
          target.add(delTok);
          continue;
        }
      }

      // autolink
      if (c0 == 0x3C /* < */) {
        final autoTok = tokenizer.autolink(src);
        if (autoTok != null) {
          src = src.substring(autoTok.raw.length);
          target.add(autoTok);
          continue;
        }
      }

      // url (gfm)
      if (!state.inLink) {
        final urlTok = tokenizer.url(src);
        if (urlTok != null) {
          src = src.substring(urlTok.raw.length);
          target.add(urlTok);
          continue;
        }
      }

      // text
      final textTok = tokenizer.inlineText(src);
      if (textTok != null) {
        src = src.substring(textTok.raw.length);
        final lastChar = textTok.raw.substring(textTok.raw.length - 1);
        if (lastChar != '_') {
          prevChar = lastChar;
        }
        keepPrevChar = true;
        final lastToken = target.isNotEmpty ? target.last : null;
        if (lastToken is TextToken) {
          lastToken.raw += textTok.raw;
          lastToken.text += textTok.text;
        } else {
          target.add(textTok);
        }
        continue;
      }

      if (src.isNotEmpty) {
        _infiniteLoopError(src.codeUnitAt(0));
      }
    }

    return target;
  }

  Never _infiniteLoopError(int byte) {
    throw StateError('Infinite loop on byte: $byte');
  }
}
