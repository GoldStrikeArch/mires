// jmespath's Pratt parser, method by method. Nodes are plain JS objects with jmespath's property
// order, since `compile` returns them and `&expr` passes them around as values.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'errors.dart';
import 'host.dart';
import 'lexer.dart';

Node _node(String type, [Map<String, JSAny?> properties = const {}]) {
  final node = JSObject();
  node['type'] = type.toJS;
  properties.forEach((key, value) => node[key] = value);
  return node as Node;
}

Node _identity() => _node('Identity');

Node _branch(String type, List<Node?> children) => _node(type, {'children': children.toJS});

extension on TokenType {
  int get power => bindingPower!;

  /// `rbp < bindingPower[type]`, false where jmespath's table has no entry.
  bool bindsTighterThan(int rbp) => switch (bindingPower) {
    final power? => rbp < power,
    null => false,
  };
}

class Parser {
  List<Token> _tokens = const [];
  int _index = 0;

  Node? parse(String expression) {
    _tokens = Lexer().tokenize(expression)..add(Token(TokenType.eof, ''.toJS, expression.length));
    _index = 0;
    final ast = _expression(0);
    if (_lookahead(0) != TokenType.eof) {
      throw syntaxError('Unexpected token type: ${_lookahead(0).label}');
    }
    return ast;
  }

  Node? _expression(int rbp) {
    final leftToken = _lookaheadToken(0);
    _advance();
    var left = _nud(leftToken);
    var currentToken = _lookahead(0);
    while (currentToken.bindsTighterThan(rbp)) {
      _advance();
      left = _led(currentToken, left);
      currentToken = _lookahead(0);
    }
    return left;
  }

  TokenType _lookahead(int number) => _tokens[_index + number].type;

  Token _lookaheadToken(int number) => _tokens[_index + number];

  void _advance() => _index++;

  Node? _nud(Token token) {
    switch (token.type) {
      case TokenType.literal:
        return _node('Literal', {'value': token.value});
      case TokenType.unquotedIdentifier:
        return _node('Field', {'name': token.value});
      case TokenType.quotedIdentifier:
        final node = _node('Field', {'name': token.value});
        if (_lookahead(0) == TokenType.lparen) throw syntaxError('Quoted identifier not allowed for function names.');
        return node;
      case TokenType.not:
        return _branch('NotExpression', [_expression(TokenType.not.power)]);
      case TokenType.star:
        final right = _lookahead(0) == TokenType.rbracket ? _identity() : _parseProjectionRHS(TokenType.star.power);
        return _branch('ValueProjection', [_identity(), right]);
      case TokenType.filter:
        return _led(token.type, _identity());
      case TokenType.lbrace:
        return _parseMultiselectHash();
      case TokenType.flatten:
        final left = _branch('Flatten', [_identity()]);
        return _branch('Projection', [left, _parseProjectionRHS(TokenType.flatten.power)]);
      case TokenType.lbracket:
        if (_lookahead(0) == TokenType.number || _lookahead(0) == TokenType.colon) {
          return _projectIfSlice(_identity(), _parseIndexExpression());
        } else if (_lookahead(0) == TokenType.star && _lookahead(1) == TokenType.rbracket) {
          _advance();
          _advance();
          return _branch('Projection', [_identity(), _parseProjectionRHS(TokenType.star.power)]);
        }
        return _parseMultiselectList();
      case TokenType.current:
        return _node('Current');
      case TokenType.expref:
        return _branch('ExpressionReference', [_expression(TokenType.expref.power)]);
      case TokenType.lparen:
        final args = _parseArguments(commas: false);
        // `args[0]`: undefined for `()`.
        return args.isEmpty ? undefined as Node? : args[0];
      default:
        throw _errorToken(token);
    }
  }

  Node? _led(TokenType tokenName, Node? left) {
    switch (tokenName) {
      case TokenType.dot:
        final rbp = TokenType.dot.power;
        if (_lookahead(0) != TokenType.star) {
          return _branch('Subexpression', [left, _parseDotRHS(rbp)]);
        }
        _advance();
        return _branch('ValueProjection', [left, _parseProjectionRHS(rbp)]);
      case TokenType.pipe:
        return _branch('Pipe', [left, _expression(TokenType.pipe.power)]);
      case TokenType.or:
        return _branch('OrExpression', [left, _expression(TokenType.or.power)]);
      case TokenType.and:
        return _branch('AndExpression', [left, _expression(TokenType.and.power)]);
      case TokenType.lparen:
        // `left.name`: a TypeError (syntax) when there is no left node, as after `()`.
        if (left == null) throw syntaxError('Function call without a name');
        final name = left.name;
        return _node('Function', {'name': name, 'children': _parseArguments(commas: true).toJS});
      case TokenType.filter:
        final condition = _expression(0);
        _match(TokenType.rbracket);
        final right = _lookahead(0) == TokenType.flatten ? _identity() : _parseProjectionRHS(TokenType.filter.power);
        return _branch('FilterProjection', [left, right, condition]);
      case TokenType.flatten:
        return _branch('Projection', [
          _branch('Flatten', [left]),
          _parseProjectionRHS(TokenType.flatten.power),
        ]);
      case TokenType.eq || TokenType.ne || TokenType.gt || TokenType.gte || TokenType.lt || TokenType.lte:
        return _parseComparator(left, tokenName);
      case TokenType.lbracket:
        final token = _lookaheadToken(0);
        if (token.type == TokenType.number || token.type == TokenType.colon) {
          return _projectIfSlice(left, _parseIndexExpression());
        }
        _match(TokenType.star);
        _match(TokenType.rbracket);
        return _branch('Projection', [left, _parseProjectionRHS(TokenType.star.power)]);
      default:
        throw _errorToken(_lookaheadToken(0));
    }
  }

  /// The argument loops of `(…)` and of a function call. An argument that starts with `@` is the
  /// bare Current node, so `abs(@.a)` fails on the `.` as in jmespath. Calls may separate arguments
  /// with commas; parentheses may not.
  List<Node?> _parseArguments({required bool commas}) {
    final args = <Node?>[];
    while (_lookahead(0) != TokenType.rparen) {
      final Node? expression;
      if (_lookahead(0) == TokenType.current) {
        _advance();
        expression = _node('Current');
      } else {
        expression = _expression(0);
      }
      if (commas && _lookahead(0) == TokenType.comma) _match(TokenType.comma);
      args.add(expression);
    }
    _match(TokenType.rparen);
    return args;
  }

  void _match(TokenType tokenType) {
    if (_lookahead(0) != tokenType) throw syntaxError('Expected ${tokenType.label}, got: ${_lookahead(0).label}');
    _advance();
  }

  JmesPathException _errorToken(Token token) => syntaxError('Invalid token (${token.type.label})');

  Node _parseIndexExpression() {
    if (_lookahead(0) == TokenType.colon || _lookahead(1) == TokenType.colon) return _parseSliceExpression();
    final node = _node('Index', {'value': _lookaheadToken(0).value});
    _advance();
    _match(TokenType.rbracket);
    return node;
  }

  Node _projectIfSlice(Node? left, Node right) {
    final indexExpr = _branch('IndexExpression', [left, right]);
    if (stringValue(right.type) == 'Slice') {
      return _branch('Projection', [indexExpr, _parseProjectionRHS(TokenType.star.power)]);
    }
    return indexExpr;
  }

  /// `[start:stop:step]`, each part optional; a later number in one part replaces an earlier one.
  Node _parseSliceExpression() {
    final parts = <JSAny?>[null, null, null];
    var index = 0;
    var currentToken = _lookahead(0);
    while (currentToken != TokenType.rbracket && index < 3) {
      if (currentToken == TokenType.colon) {
        index++;
        _advance();
      } else if (currentToken == TokenType.number) {
        parts[index] = _lookaheadToken(0).value;
        _advance();
      } else {
        throw syntaxError('Syntax error, unexpected token: ${currentToken.label}');
      }
      currentToken = _lookahead(0);
    }
    _match(TokenType.rbracket);
    return _node('Slice', {'children': parts.toJS});
  }

  Node _parseComparator(Node? left, TokenType comparator) {
    final right = _expression(comparator.power);
    return _node('Comparator', {
      'name': comparator.label.toJS,
      'children': [left, right].toJS,
    });
  }

  /// Undefined (no error) when nothing that can follow a dot follows it, as in jmespath.
  Node? _parseDotRHS(int rbp) {
    switch (_lookahead(0)) {
      case TokenType.unquotedIdentifier || TokenType.quotedIdentifier || TokenType.star:
        return _expression(rbp);
      case TokenType.lbracket:
        _match(TokenType.lbracket);
        return _parseMultiselectList();
      case TokenType.lbrace:
        _match(TokenType.lbrace);
        return _parseMultiselectHash();
      default:
        return undefined as Node?;
    }
  }

  Node? _parseProjectionRHS(int rbp) {
    final next = _lookahead(0);
    final power = next.bindingPower;
    if (power != null && power < 10) return _identity();
    if (next == TokenType.lbracket || next == TokenType.filter) return _expression(rbp);
    if (next == TokenType.dot) {
      _match(TokenType.dot);
      return _parseDotRHS(rbp);
    }
    throw syntaxError('Sytanx error, unexpected token: ${next.label}');
  }

  /// `[a, b]`; the comma between elements is optional, a trailing one is an error.
  Node _parseMultiselectList() {
    final expressions = <Node?>[];
    while (_lookahead(0) != TokenType.rbracket) {
      expressions.add(_expression(0));
      if (_lookahead(0) == TokenType.comma) {
        _match(TokenType.comma);
        if (_lookahead(0) == TokenType.rbracket) throw syntaxError('Unexpected token Rbracket');
      }
    }
    _match(TokenType.rbracket);
    return _branch('MultiSelectList', expressions);
  }

  /// `{k: v, …}`; as in jmespath, the comma between pairs is optional too.
  Node _parseMultiselectHash() {
    final pairs = <Node?>[];
    while (true) {
      final keyToken = _lookaheadToken(0);
      if (keyToken.type != TokenType.unquotedIdentifier && keyToken.type != TokenType.quotedIdentifier) {
        throw syntaxError('Expecting an identifier token, got: ${keyToken.type.label}');
      }
      _advance();
      _match(TokenType.colon);
      final value = _expression(0);
      pairs.add(_node('KeyValuePair', {'name': keyToken.value, 'value': value}));
      if (_lookahead(0) == TokenType.comma) {
        _match(TokenType.comma);
      } else if (_lookahead(0) == TokenType.rbrace) {
        _match(TokenType.rbrace);
        break;
      }
    }
    return _branch('MultiSelectHash', pairs);
  }
}
