import 'lexer.dart';
import 'options.dart';
import 'parser.dart';

/// Top-level `Marked` instance matching `marked@18.1.1` synchronous API subset.
final class Marked {
  const Marked([this.options = const MarkedOptions()]);

  final MarkedOptions options;

  String parse(String src) {
    final tokens = Lexer.lexStatic(src, options);
    return Parser.parseStatic(tokens, options);
  }

  String parseInline(String src) {
    final tokens = Lexer.lexInlineStatic(src, options);
    return Parser.parseInlineStatic(tokens, options);
  }
}
