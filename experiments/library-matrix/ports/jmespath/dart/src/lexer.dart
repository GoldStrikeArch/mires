// jmespath's Lexer, method by method.

import 'dart:js_interop';

import 'errors.dart';
import 'host.dart';

/// Token types and jmespath's `bindingPower` table. Colon and Literal have no entry there, so their
/// power is null: `rbp < bindingPower[Colon]` is false, and so is `bindingPower[Literal] < 10` in
/// `_parseProjectionRHS`, which therefore rejects a literal after a projection.
enum TokenType {
  eof('EOF', 0),
  unquotedIdentifier('UnquotedIdentifier', 0),
  quotedIdentifier('QuotedIdentifier', 0),
  rbracket('Rbracket', 0),
  rparen('Rparen', 0),
  comma('Comma', 0),
  colon('Colon', null),
  rbrace('Rbrace', 0),
  number('Number', 0),
  current('Current', 0),
  expref('Expref', 0),
  pipe('Pipe', 1),
  or('Or', 2),
  and('And', 3),
  eq('EQ', 5),
  gt('GT', 5),
  lt('LT', 5),
  gte('GTE', 5),
  lte('LTE', 5),
  ne('NE', 5),
  flatten('Flatten', 9),
  star('Star', 20),
  filter('Filter', 21),
  dot('Dot', 40),
  not('Not', 45),
  lbrace('Lbrace', 50),
  lbracket('Lbracket', 55),
  lparen('Lparen', 60),
  literal('Literal', null);

  const TokenType(this.label, this.bindingPower);

  /// jmespath's TOK_* string; the comparator nodes carry it as their name.
  final String label;
  final int? bindingPower;
}

class Token {
  Token(this.type, this.value, this.start);
  final TokenType type;
  final JSAny? value;
  final int start;
}

bool _isAlpha(int c) => (c >= 0x61 && c <= 0x7a) || (c >= 0x41 && c <= 0x5a) || c == 0x5f; // a-z A-Z _

bool _isNum(int c) => (c >= 0x30 && c <= 0x39) || c == 0x2d; // 0-9 -

bool _isAlphaNum(int c) => _isAlpha(c) || (c >= 0x30 && c <= 0x39);

const _basicTokens = {
  '.': TokenType.dot,
  '*': TokenType.star,
  ',': TokenType.comma,
  ':': TokenType.colon,
  '{': TokenType.lbrace,
  '}': TokenType.rbrace,
  ']': TokenType.rbracket,
  '(': TokenType.lparen,
  ')': TokenType.rparen,
  '@': TokenType.current,
};

/// `String.prototype.trimLeft`'s set: ECMAScript WhiteSpace and LineTerminator. Dart's own
/// `trimLeft` differs: it also strips U+0085.
bool _isJsWhiteSpace(int c) => switch (c) {
  0x09 || 0x0a || 0x0b || 0x0c || 0x0d || 0x20 || 0xa0 || 0x1680 || 0x2028 || 0x2029 || 0x202f || 0x205f || 0x3000 ||
  0xfeff =>
    true,
  _ => c >= 0x2000 && c <= 0x200a,
};

String _trimLeft(String text) {
  var start = 0;
  while (start < text.length && _isJsWhiteSpace(text.codeUnitAt(start))) {
    start++;
  }
  return text.substring(start);
}

class Lexer {
  int _current = 0;

  List<Token> tokenize(String stream) {
    final tokens = <Token>[];
    _current = 0;
    while (_current < stream.length) {
      final start = _current;
      final ch = stream[_current];
      final c = stream.codeUnitAt(_current);
      final basic = _basicTokens[ch];
      if (_isAlpha(c)) {
        tokens.add(Token(TokenType.unquotedIdentifier, _consumeUnquotedIdentifier(stream).toJS, start));
      } else if (basic != null) {
        tokens.add(Token(basic, ch.toJS, start));
        _current++;
      } else if (_isNum(c)) {
        tokens.add(_consumeNumber(stream));
      } else if (ch == '[') {
        tokens.add(_consumeLBracket(stream));
      } else if (ch == '"') {
        tokens.add(Token(TokenType.quotedIdentifier, _consumeQuotedIdentifier(stream), start));
      } else if (ch == "'") {
        tokens.add(Token(TokenType.literal, _consumeRawStringLiteral(stream).toJS, start));
      } else if (ch == '`') {
        tokens.add(Token(TokenType.literal, _consumeLiteral(stream), start));
      } else if (ch == '<' || ch == '>' || ch == '=' || ch == '!') {
        tokens.add(_consumeOperator(stream));
      } else if (ch == ' ' || ch == '\t' || ch == '\n') {
        _current++;
      } else if (ch == '&' || ch == '|') {
        _current++;
        final doubled = _current < stream.length && stream[_current] == ch;
        if (doubled) _current++;
        tokens.add(ch == '&'
            ? (doubled ? Token(TokenType.and, '&&'.toJS, start) : Token(TokenType.expref, '&'.toJS, start))
            : (doubled ? Token(TokenType.or, '||'.toJS, start) : Token(TokenType.pipe, '|'.toJS, start)));
      } else {
        throw syntaxError('Unknown character:$ch');
      }
    }
    return tokens;
  }

  String _consumeUnquotedIdentifier(String stream) {
    final start = _current;
    _current++;
    while (_current < stream.length && _isAlphaNum(stream.codeUnitAt(_current))) {
      _current++;
    }
    return stream.substring(start, _current);
  }

  /// Skips a quoted body, where a backslash escapes [quote] or another backslash.
  void _skipQuoted(String stream, String quote) {
    while (_current < stream.length && stream[_current] != quote) {
      final next = _current + 1 < stream.length ? stream[_current + 1] : null;
      _current += stream[_current] == r'\' && (next == r'\' || next == quote) ? 2 : 1;
    }
  }

  JSAny? _consumeQuotedIdentifier(String stream) {
    final start = _current;
    _current++;
    _skipQuoted(stream, '"');
    _current++;
    return _parseJson(stream.substring(start, _current < stream.length ? _current : stream.length));
  }

  /// An unterminated raw string runs to the end of the expression, as in jmespath.
  String _consumeRawStringLiteral(String stream) {
    final start = _current;
    _current++;
    _skipQuoted(stream, "'");
    _current++;
    final end = _current - 1 < stream.length ? _current - 1 : stream.length;
    return stream.substring(start + 1, end).replaceFirst(r"\'", "'");
  }

  Token _consumeNumber(String stream) {
    final start = _current;
    _current++;
    while (_current < stream.length && _isNum(stream.codeUnitAt(_current))) {
      _current++;
    }
    return Token(TokenType.number, parseInt(stream.substring(start, _current)).toJS, start);
  }

  Token _consumeLBracket(String stream) {
    final start = _current;
    _current++;
    final next = _current < stream.length ? stream[_current] : null;
    if (next == '?') {
      _current++;
      return Token(TokenType.filter, '[?'.toJS, start);
    } else if (next == ']') {
      _current++;
      return Token(TokenType.flatten, '[]'.toJS, start);
    }
    return Token(TokenType.lbracket, '['.toJS, start);
  }

  /// A lone `=` is the one character jmespath accepts here without making a token: it pushes
  /// `undefined`, and parsing fails on it with a TypeError, which is a syntax error. That error is
  /// raised here.
  Token _consumeOperator(String stream) {
    final start = _current;
    final startingChar = stream[start];
    _current++;
    final equalsFollows = _current < stream.length && stream[_current] == '=';
    if (equalsFollows) _current++;
    return switch (startingChar) {
      '!' => equalsFollows ? Token(TokenType.ne, '!='.toJS, start) : Token(TokenType.not, '!'.toJS, start),
      '<' => equalsFollows ? Token(TokenType.lte, '<='.toJS, start) : Token(TokenType.lt, '<'.toJS, start),
      '>' => equalsFollows ? Token(TokenType.gte, '>='.toJS, start) : Token(TokenType.gt, '>'.toJS, start),
      _ => equalsFollows ? Token(TokenType.eq, '=='.toJS, start) : throw syntaxError('Unknown operator: ='),
    };
  }

  JSAny? _consumeLiteral(String stream) {
    _current++;
    final start = _current;
    _skipQuoted(stream, '`');
    final literalString = _trimLeft(stream.substring(start, _current)).replaceFirst(r'\`', '`');
    final literal = _looksLikeJson(literalString) ? _parseJson(literalString) : _parseJson('"$literalString"');
    _current++;
    return literal;
  }

  /// `JSON.parse`, whose SyntaxError `category()` files as "syntax". Anything else it throws, such
  /// as stack exhaustion on a deep literal, propagates.
  JSAny? _parseJson(String text) {
    try {
      return parseJson(text);
    } on JSObject catch (error) {
      if (error.instanceOfString('SyntaxError')) throw syntaxError('Invalid JSON: $text');
      rethrow;
    }
  }

  bool _looksLikeJson(String literalString) {
    if (literalString.isEmpty) return false;
    if ('[{"'.contains(literalString[0])) return true;
    if (const ['true', 'false', 'null'].contains(literalString)) return true;
    if ('-0123456789'.contains(literalString[0])) {
      try {
        parseJson(literalString);
        return true;
      } catch (_) {
        return false;
      }
    }
    return false;
  }
}
