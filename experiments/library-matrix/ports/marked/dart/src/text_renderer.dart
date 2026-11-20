import 'tokens.dart';

/// Faithful port of `marked@18.1.1` `_TextRenderer` (used to flatten image alt text).
final class TextRenderer {
  const TextRenderer();

  String strong(StrongToken token) => token.text;
  String em(EmToken token) => token.text;
  String codespan(CodespanToken token) => token.text;
  String del(DelToken token) => token.text;
  String html(HtmlToken token) => token.text;
  String text(TextToken token) => token.text;
  String escape(EscapeToken token) => token.text;
  String link(LinkToken token) => token.text;
  String image(ImageToken token) => token.text;
  String br(BrToken token) => '';
  String checkbox(CheckboxToken token) => token.raw;
}
