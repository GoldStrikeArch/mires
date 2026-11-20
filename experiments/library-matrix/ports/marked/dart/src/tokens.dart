/// Token types for `marked@18.1.1` block and inline lexers/parsers.
///
/// Uses a sealed hierarchy of concrete non-generic final classes so dart2js
/// compiles pattern switches to monomorphic `instanceof` checks without RTI
/// `as`-cast overhead.
sealed class Token {
  Token(this.raw);

  String raw;
}

final class SpaceToken extends Token {
  SpaceToken({required String raw}) : super(raw);
}

final class CodeToken extends Token {
  CodeToken({
    required String raw,
    required this.text,
    this.lang,
    this.codeBlockStyle,
    this.escaped = false,
  }) : super(raw);

  String text;
  String? lang;
  String? codeBlockStyle;
  bool escaped;
}

final class HeadingToken extends Token {
  HeadingToken({
    required String raw,
    required this.depth,
    required this.text,
    required this.tokens,
  }) : super(raw);

  int depth;
  String text;
  List<Token> tokens;
}

final class HrToken extends Token {
  HrToken({required String raw}) : super(raw);
}

final class BlockquoteToken extends Token {
  BlockquoteToken({
    required String raw,
    required this.text,
    required this.tokens,
  }) : super(raw);

  String text;
  List<Token> tokens;
}

final class ListToken extends Token {
  ListToken({
    required String raw,
    required this.ordered,
    required this.start,
    required this.loose,
    required this.items,
  }) : super(raw);

  bool ordered;

  /// Numeric start value when [ordered] is true; `0` otherwise (unused when unordered).
  int start;
  bool loose;
  List<ListItemToken> items;
}

final class ListItemToken extends Token {
  ListItemToken({
    required String raw,
    required this.task,
    required this.loose,
    required this.text,
    required this.tokens,
    this.checked,
  }) : super(raw);

  bool task;
  bool? checked;
  bool loose;
  String text;
  List<Token> tokens;
}

final class CheckboxToken extends Token {
  CheckboxToken({
    required String raw,
    required this.checked,
  }) : super(raw);

  bool checked;
}

final class HtmlToken extends Token {
  HtmlToken({
    required String raw,
    required this.text,
    required this.block,
    this.pre = false,
    this.inLink = false,
    this.inRawBlock = false,
  }) : super(raw);

  String text;
  bool block;
  bool pre;
  bool inLink;
  bool inRawBlock;
}

final class DefToken extends Token {
  DefToken({
    required String raw,
    required this.tag,
    required this.href,
    required this.title,
  }) : super(raw);

  String tag;
  String href;
  String? title;
}

final class TableCellToken {
  TableCellToken({
    required this.text,
    required this.tokens,
    required this.header,
    required this.align,
  });

  String text;
  List<Token> tokens;
  bool header;
  String? align;
}

final class TableToken extends Token {
  TableToken({
    required String raw,
    required this.header,
    required this.align,
    required this.rows,
  }) : super(raw);

  List<TableCellToken> header;
  List<String?> align;
  List<List<TableCellToken>> rows;
}

final class ParagraphToken extends Token {
  ParagraphToken({
    required String raw,
    required this.text,
    required this.tokens,
  }) : super(raw);

  String text;
  List<Token> tokens;
}

final class TextToken extends Token {
  TextToken({
    required String raw,
    required this.text,
    this.tokens,
    this.escaped = false,
  }) : super(raw);

  String text;
  List<Token>? tokens;
  bool escaped;
}

final class EscapeToken extends Token {
  EscapeToken({
    required String raw,
    required this.text,
  }) : super(raw);

  String text;
}

final class LinkToken extends Token {
  LinkToken({
    required String raw,
    required this.href,
    required this.title,
    required this.text,
    required this.tokens,
    this.autolink = false,
  }) : super(raw);

  String href;
  String? title;
  String text;
  List<Token> tokens;
  bool autolink;
}

final class ImageToken extends Token {
  ImageToken({
    required String raw,
    required this.href,
    required this.title,
    required this.text,
    required this.tokens,
  }) : super(raw);

  String href;
  String? title;
  String text;
  List<Token> tokens;
}

final class StrongToken extends Token {
  StrongToken({
    required String raw,
    required this.text,
    required this.tokens,
  }) : super(raw);

  String text;
  List<Token> tokens;
}

final class EmToken extends Token {
  EmToken({
    required String raw,
    required this.text,
    required this.tokens,
  }) : super(raw);

  String text;
  List<Token> tokens;
}

final class CodespanToken extends Token {
  CodespanToken({
    required String raw,
    required this.text,
  }) : super(raw);

  String text;
}

final class BrToken extends Token {
  BrToken({required String raw}) : super(raw);
}

final class DelToken extends Token {
  DelToken({
    required String raw,
    required this.text,
    required this.tokens,
  }) : super(raw);

  String text;
  List<Token> tokens;
}

final class LinkDef {
  const LinkDef({required this.href, required this.title});

  final String href;
  final String? title;
}

final class InlineQueueEntry {
  InlineQueueEntry(this.src, this.tokens);

  String src;
  final List<Token> tokens;
}
