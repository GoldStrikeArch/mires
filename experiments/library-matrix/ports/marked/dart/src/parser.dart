import 'options.dart';
import 'renderer.dart';
import 'text_renderer.dart';
import 'tokens.dart';

/// Faithful port of `marked@18.1.1` `_Parser`.
final class Parser {
  Parser(this.options) : renderer = Renderer(options) {
    renderer.parser = this;
  }

  final MarkedOptions options;
  final Renderer renderer;
  final TextRenderer textRenderer = const TextRenderer();

  static String parseStatic(List<Token> tokens, MarkedOptions options) =>
      Parser(options).parse(tokens);

  static String parseInlineStatic(List<Token> tokens, MarkedOptions options) =>
      Parser(options).parseInline(tokens);

  String parse(List<Token> tokens) {
    renderer.parser = this;
    final len = tokens.length;
    if (len == 0) return '';
    if (len == 1) return _renderBlockToken(tokens[0]);
    final out = StringBuffer();
    for (var i = 0; i < len; i++) {
      out.write(_renderBlockToken(tokens[i]));
    }
    return out.toString();
  }

  String _renderBlockToken(Token token) {
    switch (token) {
      case ParagraphToken():
        return renderer.paragraph(token);
      case TextToken():
        return renderer.text(token);
      case SpaceToken():
        return renderer.space(token);
      case HeadingToken():
        return renderer.heading(token);
      case ListToken():
        return renderer.list(token);
      case CodeToken():
        return renderer.code(token);
      case TableToken():
        return renderer.table(token);
      case BlockquoteToken():
        return renderer.blockquote(token);
      case HtmlToken():
        return renderer.html(token);
      case HrToken():
        return renderer.hr(token);
      case CheckboxToken():
        return renderer.checkbox(token);
      case DefToken():
        return renderer.def(token);
      default:
        throw StateError('Token with "${token.runtimeType}" type was not found.');
    }
  }

  String parseInline(List<Token> tokens) {
    renderer.parser = this;
    final len = tokens.length;
    if (len == 0) return '';
    if (len == 1) return _renderInlineToken(tokens[0]);
    final out = StringBuffer();
    for (var i = 0; i < len; i++) {
      out.write(_renderInlineToken(tokens[i]));
    }
    return out.toString();
  }

  String _renderInlineToken(Token token) {
    switch (token) {
      case TextToken():
        return renderer.text(token);
      case CodespanToken():
        return renderer.codespan(token);
      case LinkToken():
        return renderer.link(token);
      case StrongToken():
        return renderer.strong(token);
      case EmToken():
        return renderer.em(token);
      case BrToken():
        return renderer.br(token);
      case DelToken():
        return renderer.del(token);
      case EscapeToken():
        return renderer.escape(token);
      case HtmlToken():
        return renderer.html(token);
      case ImageToken():
        return renderer.image(token);
      case CheckboxToken():
        return renderer.checkbox(token);
      default:
        throw StateError('Token with "${token.runtimeType}" type was not found.');
    }
  }

  String parseInlineText(List<Token> tokens) {
    final len = tokens.length;
    if (len == 0) return '';
    final out = StringBuffer();
    for (var i = 0; i < len; i++) {
      final token = tokens[i];
      switch (token) {
        case TextToken():
          out.write(textRenderer.text(token));
        case EscapeToken():
          out.write(textRenderer.escape(token));
        case HtmlToken():
          out.write(textRenderer.html(token));
        case LinkToken():
          out.write(textRenderer.link(token));
        case ImageToken():
          out.write(textRenderer.image(token));
        case CheckboxToken():
          out.write(textRenderer.checkbox(token));
        case StrongToken():
          out.write(textRenderer.strong(token));
        case EmToken():
          out.write(textRenderer.em(token));
        case CodespanToken():
          out.write(textRenderer.codespan(token));
        case BrToken():
          out.write(textRenderer.br(token));
        case DelToken():
          out.write(textRenderer.del(token));
        default:
          throw StateError('Token with "${token.runtimeType}" type was not found.');
      }
    }
    return out.toString();
  }
}
