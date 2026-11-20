import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

typedef Tokens = List<Tok>;

class Tok {
  final String kind;
  final dynamic value;
  Tok(this.kind, this.value);
}

Tokens tokensBuffer() => [];
void addToken(Tokens tokens, String kind, dynamic value) =>
    tokens.add(Tok(kind, value));
final Object undefinedValue = Object();
dynamic nil() => null;
Exception syntaxError() => FormatException('syntax');
dynamic str(String s) => s;
int codeAt(String s, int i) => s.codeUnitAt(i);
String part(String s, int i, int j) => s.substring(i, j);
int strLen(String s) => s.length;
String slash() => '\\';
String replaceFirst(String s, String from, String to) =>
    s.replaceFirst(from, to);
dynamic decodeJson(String s) => jsonDecode(s);
dynamic parseInteger(String s) {
  final m = RegExp(r'^-?\d+').firstMatch(s);
  return m == null ? double.nan : double.parse(m[0]!);
}

dynamic literal(String input) {
  final s = input.trimLeft().replaceFirst("\\`", "`");
  if (s.startsWith('[') ||
      s.startsWith('{') ||
      s.startsWith('"') ||
      ['true', 'false', 'null'].contains(s))
    return decodeJson(s);
  if (RegExp(r'^[-0-9]').hasMatch(s)) {
    try {
      return decodeJson(s);
    } on FormatException {
      /* unquoted literal */
    }
  }
  return decodeJson('"' + s + '"');
}

bool alpha(int c) {
  return (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c == 95;
}

bool digit(int c) {
  return c >= 48 && c <= 57;
}

Tokens lex(String source) {
  final Tokens result = tokensBuffer();
  final int length = strLen(source);
  int i = 0;
  while (i < length) {
    final int start = i;
    final int c = codeAt(source, i);
    i += 1;
    int following = 0;
    if (i < length) {
      following = codeAt(source, i);
    }
    String k = "";
    dynamic value = undefinedValue;
    if (alpha(c)) {
      while (i < length &&
          (alpha(codeAt(source, i)) || digit(codeAt(source, i)))) {
        i += 1;
      }
      value = str(part(source, start, i));
      k = "Field";
    } else if (c == 45 || digit(c)) {
      while (i < length &&
          (digit(codeAt(source, i)) || codeAt(source, i) == 45)) {
        i += 1;
      }
      value = parseInteger(part(source, start, i));
      k = "Number";
    } else if (c == 34 || c == 39 || c == 96) {
      while (i < length && codeAt(source, i) != c) {
        if (codeAt(source, i) == 92 &&
            i + 1 < length &&
            (codeAt(source, i + 1) == c || codeAt(source, i + 1) == 92)) {
          i += 1;
        }
        i += 1;
      }
      final String inner = part(source, start + 1, i);
      if (i < length) {
        i += 1;
      }
      if (c == 34) {
        value = decodeJson(part(source, start, i));
        k = "Quoted";
      } else if (c == 39) {
        value = str(replaceFirst(inner, slash() + "'", "'"));
        k = "Literal";
      } else {
        value = literal(inner);
        k = "Literal";
      }
    } else if (c == 32 || c == 9 || c == 10) {
      k = "Skip";
    } else {
      final String pair = doubleToken(c, following);
      if (pair != "") {
        i += 1;
        k = pair;
      } else {
        k = singleToken(c);
      }
    }
    if (k != "Skip") {
      addToken(result, k, value);
    }
  }
  addToken(result, "EOF", undefinedValue);
  return result;
}

String doubleToken(int c, int next) {
  if (c == 91 && next == 63) {
    return "Filter";
  }
  if (c == 91 && next == 93) {
    return "Flatten";
  }
  if (c == 38 && next == 38) {
    return "And";
  }
  if (c == 124 && next == 124) {
    return "Or";
  }
  if (c == 33 && next == 61) {
    return "NE";
  }
  if (c == 61 && next == 61) {
    return "EQ";
  }
  if (c == 60 && next == 61) {
    return "LTE";
  }
  if (c == 62 && next == 61) {
    return "GTE";
  }
  return "";
}

String singleToken(int c) {
  if (c == 60) {
    return "LT";
  }
  if (c == 62) {
    return "GT";
  }
  if (c == 33) {
    return "Not";
  }
  if (c == 38) {
    return "Expref";
  }
  if (c == 124) {
    return "Pipe";
  }
  if (c == 91) {
    return "Lbracket";
  }
  if (c == 93) {
    return "Rbracket";
  }
  if (c == 123) {
    return "Lbrace";
  }
  if (c == 125) {
    return "Rbrace";
  }
  if (c == 40) {
    return "Lparen";
  }
  if (c == 41) {
    return "Rparen";
  }
  if (c == 42) {
    return "Star";
  }
  if (c == 64) {
    return "Current";
  }
  if (c == 44) {
    return "Comma";
  }
  if (c == 46) {
    return "Dot";
  }
  if (c == 58) {
    return "Colon";
  }
  throw syntaxError();
}

class Parser {
  final Tokens tokens;
  int index = 0;
  Parser(String source) : tokens = lex(source);
  Tok tokenAt(int i) =>
      i < tokens.length ? tokens[i] : Tok('EOF', undefinedValue);
  String current(int offset) {
    return tokenAt(index + offset).kind;
  }

  Tok next() {
    final Tok result = tokenAt(index);
    if (result.kind == "EOF") {
      throw syntaxError();
    }
    index += 1;
    return result;
  }

  void take(String kind) {
    if (next().kind != kind) {
      throw syntaxError();
    }
  }

  int power(String k) {
    if (k == "Pipe") {
      return 1;
    }
    if (k == "Or") {
      return 2;
    }
    if (k == "And") {
      return 3;
    }
    if (k == "EQ" ||
        k == "NE" ||
        k == "GT" ||
        k == "LT" ||
        k == "GTE" ||
        k == "LTE") {
      return 5;
    }
    if (k == "Flatten") {
      return 9;
    }
    if (k == "Star") {
      return 20;
    }
    if (k == "Filter") {
      return 21;
    }
    if (k == "Dot") {
      return 40;
    }
    if (k == "Not") {
      return 45;
    }
    if (k == "Lbrace") {
      return 50;
    }
    if (k == "Lbracket") {
      return 55;
    }
    if (k == "Lparen") {
      return 60;
    }
    return 0;
  }

  dynamic parse() {
    final dynamic ast = expression(0);
    if (current(0) != "EOF") {
      throw syntaxError();
    }
    return ast;
  }

  dynamic expression(int rbp) {
    dynamic left = applyPrefix(next());
    while (power(current(0)) > rbp) {
      final String op = next().kind;
      left = applyInfix(op, left);
    }
    return left;
  }

  dynamic projection(int rbp) {
    final String k = current(0);
    if (power(k) < 10) {
      return node("Identity");
    }
    if (k == "Lbracket" || k == "Filter") {
      return expression(rbp);
    }
    if (k == "Dot") {
      next();
      return dot(rbp);
    }
    throw syntaxError();
  }

  dynamic dot(int rbp) {
    final String k = current(0);
    if (k == "Field" || k == "Quoted" || k == "Star") {
      return expression(rbp);
    }
    if (k == "Lbracket") {
      next();
      return listExpression();
    }
    if (k == "Lbrace") {
      next();
      return hashExpression();
    }
    return undefinedValue;
  }

  dynamic listExpression() {
    final dynamic items = arr();
    while (current(0) != "Rbracket") {
      append(items, expression(0));
      if (current(0) == "Comma") {
        next();
        if (current(0) == "Rbracket") {
          throw syntaxError();
        }
      }
    }
    take("Rbracket");
    return branch("MultiSelectList", items);
  }

  dynamic hashExpression() {
    final dynamic items = arr();
    while (true) {
      final Tok key = next();
      if (key.kind != "Field" && key.kind != "Quoted") {
        throw syntaxError();
      }
      take("Colon");
      final dynamic pair = named("KeyValuePair", key.value);
      put(pair, "value", expression(0));
      append(items, pair);
      if (current(0) == "Comma") {
        next();
      } else if (current(0) == "Rbrace") {
        next();
        return branch("MultiSelectHash", items);
      }
    }
    throw syntaxError();
  }

  dynamic bracket(dynamic left) {
    if (current(0) == "Colon" || current(1) == "Colon") {
      final dynamic parts = three(nil(), nil(), nil());
      int i = 0;
      while (current(0) != "Rbracket" && i < 3) {
        final Tok token = next();
        if (token.kind == "Colon") {
          i += 1;
        } else if (token.kind == "Number") {
          setIndex(parts, i, token.value);
        } else {
          throw syntaxError();
        }
      }
      take("Rbracket");
      final dynamic ix = branch(
        "IndexExpression",
        two(left, branch("Slice", parts)),
      );
      return branch("Projection", two(ix, projection(20)));
    }
    final Tok token = next();
    take("Rbracket");
    return branch("IndexExpression", two(left, leaf("Index", token.value)));
  }

  dynamic arguments(bool commas) {
    final dynamic args = arr();
    while (current(0) != "Rparen") {
      if (current(0) == "Current") {
        next();
        append(args, node("Current"));
      } else {
        append(args, expression(0));
      }
      if (commas && current(0) == "Comma") {
        next();
      }
    }
    take("Rparen");
    return args;
  }

  dynamic applyPrefix(Tok t) {
    final String k = t.kind;
    if (k == "Literal") {
      return leaf("Literal", t.value);
    }
    if (k == "Quoted" && current(0) == "Lparen") {
      throw syntaxError();
    }
    if (k == "Quoted" || k == "Field") {
      return named("Field", t.value);
    }
    if (k == "Not") {
      return branch("NotExpression", one(expression(45)));
    }
    if (k == "Current") {
      return node("Current");
    }
    if (k == "Expref") {
      return branch("ExpressionReference", one(expression(0)));
    }
    if (k == "Star") {
      return branch("ValueProjection", two(node("Identity"), projection(20)));
    }
    if (k == "Filter" || k == "Flatten") {
      return applyInfix(k, node("Identity"));
    }
    if (k == "Lbrace") {
      return hashExpression();
    }
    if (k == "Lbracket") {
      if (current(0) == "Colon" || current(0) == "Number") {
        return bracket(node("Identity"));
      }
      if (current(0) == "Star" && current(1) == "Rbracket") {
        index += 2;
        return branch("Projection", two(node("Identity"), projection(20)));
      }
      return listExpression();
    }
    if (k == "Lparen") {
      return at(arguments(false), 0);
    }
    throw syntaxError();
  }

  dynamic applyInfix(String k, dynamic left) {
    if (k == "Dot") {
      if (current(0) == "Star") {
        next();
        return branch("ValueProjection", two(left, projection(40)));
      }
      return branch("Subexpression", two(left, dot(40)));
    }
    if (k == "Or" || k == "And") {
      return branch(k + "Expression", two(left, expression(power(k))));
    }
    if (k == "Pipe") {
      return branch(k, two(left, expression(1)));
    }
    if (k == "Lparen") {
      final dynamic f = named("Function", field(left, "name"));
      put(f, "children", arguments(true));
      return f;
    }
    if (k == "Filter") {
      final dynamic condition = expression(0);
      take("Rbracket");
      dynamic rhs = node("Identity");
      if (current(0) != "Flatten") {
        rhs = projection(21);
      }
      return branch("FilterProjection", three(left, rhs, condition));
    }
    if (k == "Flatten") {
      return branch(
        "Projection",
        two(branch("Flatten", one(left)), projection(9)),
      );
    }
    if (power(k) == 5) {
      final dynamic cmp = named("Comparator", str(k));
      put(cmp, "children", two(left, expression(5)));
      return cmp;
    }
    if (k == "Lbracket") {
      if (current(0) == "Colon" || current(0) == "Number") {
        return bracket(left);
      }
      take("Star");
      take("Rbracket");
      return branch("Projection", two(left, projection(20)));
    }
    throw syntaxError();
  }
}

class QueryError implements Exception {
  final String category;
  QueryError(this.category);
}

QueryError invalid(String category) => QueryError(category);
dynamic arr() => <dynamic>[];
dynamic obj() => <String, dynamic>{};
bool missing(dynamic v) => identical(v, undefinedValue);
bool isNil(dynamic v) => v == null;
bool flag(dynamic v) => v as bool;
dynamic boolValue(bool v) => v;
dynamic intValue(int v) => v;
dynamic numeric(double v) => v;
dynamic infinity(bool positive) =>
    positive ? double.infinity : double.negativeInfinity;
bool nan(double v) => v.isNaN;
String stringValue(dynamic v) {
  if (missing(v)) return 'undefined';
  if (v is List) return v.map((x) => x == null ? '' : stringValue(x)).join(',');
  if (v is Map) return '[object Object]';
  return v.toString();
}

int integer(dynamic v) => toNumber(v).toInt();
@JS('Number')
external double parseJsNumber(JSString value);

double toNumber(dynamic v) {
  if (v == null || v == false) return 0;
  if (v == true) return 1;
  if (v is num) return v.toDouble();
  final text = stringValue(v).trim();
  if (text.isEmpty) return 0;
  return parseJsNumber(text.toJS);
}

bool same(dynamic a, dynamic b) => a == b;
int clamp(int i, int lo, int hi) => i.clamp(lo, hi);
dynamic field(dynamic v, String k) {
  if (v is Map) return v.containsKey(k) ? v[k] : undefinedValue;
  if (v is List || v is String) {
    final i = int.tryParse(k);
    return i == null ? undefinedValue : at(v, i);
  }
  throw syntaxError();
}

dynamic at(dynamic v, int i) => i >= 0 && i < v.length ? v[i] : undefinedValue;
int size(dynamic v) => v.length as int;
void put(dynamic v, String k, dynamic x) {
  // Upstream assigns hash and merge results into an ordinary JS object.
  // Assigning this inherited setter does not create an own data field.
  if (k != '__proto__') v[k] = x;
}

void append(dynamic v, dynamic x) {
  v.add(x);
}

void setIndex(dynamic v, int i, dynamic x) {
  v[i] = x;
}

dynamic keys(dynamic v) => v is Map
    ? v.keys.toList()
    : v is List || v is String
    ? List.generate(size(v), (i) => i.toString())
    : [];
String kind(dynamic v) {
  if (v == null) return 'null';
  if (v is String) return 'string';
  if (v is num) return 'number';
  if (v is bool) return 'boolean';
  if (v is List) return 'array';
  if (v is Map) return v['jmespathType'] == 'Expref' ? 'expref' : 'object';
  return 'undefined';
}

int compare(dynamic a, dynamic b) {
  final x = a is List || a is Map ? stringValue(a) : a;
  final y = b is List || b is Map ? stringValue(b) : b;
  if (x is String && y is String) return x.compareTo(y).sign;
  final u = toNumber(x), v = toNumber(y);
  return u < v
      ? -1
      : u > v
      ? 1
      : u == v
      ? 0
      : 2;
}

extension type HostString(JSString _) implements JSString {
  external int localeCompare(JSString other);
}

int localeOrder(dynamic a, dynamic b) =>
    HostString((a as String).toJS).localeCompare((b as String).toJS);
@JS('JSON.stringify')
external JSString encodeJs(JSAny? value);
dynamic jsonText(dynamic a) => encodeJs(toHost(a)).toDart;
dynamic sortByExpression(dynamic a, dynamic b) {
  if (size(a) == 0) return [];
  final entries = [
    for (int i = 0; i < size(a); i++) (evaluate(b, at(a, i)), i, at(a, i)),
  ];
  final type = kind(entries.first.$1);
  if ((type != 'number' && type != 'string') ||
      entries.any((e) => kind(e.$1) != type))
    throw invalid('invalid-type');
  entries.sort((a, b) {
    final cmp = compare(a.$1, b.$1);
    return cmp == 0 ? a.$2 - b.$2 : cmp;
  });
  return [for (final e in entries) e.$3];
}

dynamic primitiveFunction(String name, dynamic a, dynamic b) {
  switch (name) {
    case 'abs':
      return toNumber(a).abs();
    case 'ceil':
      return toNumber(a).ceilToDouble();
    case 'floor':
      return toNumber(a).floorToDouble();
    case 'contains':
      return a is String ? a.contains(stringValue(b)) : (a as List).contains(b);
    case 'starts_with':
      return (a as String).lastIndexOf(b as String) == 0;
    case 'ends_with':
      return (a as String).indexOf(
            b as String,
            (a.length - b.length).clamp(0, a.length),
          ) >=
          0;
    case 'join':
      return (b as List).join(a as String);
    case 'sort':
      return [...a]..sort((x, y) => stringValue(x).compareTo(stringValue(y)));
    case 'reverse':
      return a is String
          ? String.fromCharCodes(a.codeUnits.reversed)
          : (a as List).reversed.toList();
    default:
      throw invalid('unknown-function');
  }
}

dynamic one(dynamic a) {
  final dynamic xs = arr();
  append(xs, a);
  return xs;
}

dynamic two(dynamic a, dynamic b) {
  final dynamic xs = one(a);
  append(xs, b);
  return xs;
}

dynamic three(dynamic a, dynamic b, dynamic c) {
  final dynamic xs = two(a, b);
  append(xs, c);
  return xs;
}

dynamic node(String t) {
  final dynamic n = obj();
  put(n, "type", str(t));
  return n;
}

dynamic named(String t, dynamic name) {
  final dynamic n = node(t);
  put(n, "name", name);
  return n;
}

dynamic leaf(String t, dynamic value) {
  final dynamic n = node(t);
  put(n, "value", value);
  return n;
}

dynamic branch(String t, dynamic xs) {
  final dynamic n = node(t);
  put(n, "children", xs);
  return n;
}

dynamic child(dynamic n, int i) {
  return at(field(n, "children"), i);
}

bool truth(dynamic v) {
  final String t = kind(v);
  if (t == "null") {
    return false;
  }
  if (t == "boolean") {
    return flag(v);
  }
  if (t == "array" || t == "string") {
    return size(v) != 0;
  }
  if (t == "object" || t == "expref") {
    return size(keys(v)) != 0;
  }
  return true;
}

bool equal(dynamic a, dynamic b) {
  if (same(a, b)) {
    return true;
  }
  final String t = kind(a);
  if (t != kind(b)) {
    return false;
  }
  if (t == "array") {
    if (size(a) != size(b)) {
      return false;
    }
    int i = 0;
    while (i < size(a)) {
      if (!equal(at(a, i), at(b, i))) {
        return false;
      }
      i += 1;
    }
    return true;
  }
  if (t == "object") {
    final dynamic ks = keys(a);
    if (size(ks) != size(keys(b))) {
      return false;
    }
    int i = 0;
    while (i < size(ks)) {
      final String key = stringValue(at(ks, i));
      if (!equal(field(a, key), field(b, key))) {
        return false;
      }
      i += 1;
    }
    return true;
  }
  return false;
}

int cap(dynamic v, int defaultValue, int step, int length) {
  if (isNil(v)) {
    return defaultValue;
  }
  int y = integer(v);
  if (y < 0) {
    y += length;
  }
  if (step < 0) {
    return clamp(y, -1, length - 1);
  }
  return clamp(y, 0, length);
}

dynamic evaluate(dynamic n, dynamic v) {
  final String k = stringValue(field(n, "type"));
  if (k == "Field") {
    if (kind(v) != "object" && kind(v) != "expref") {
      return nil();
    }
    final dynamic result = field(v, stringValue(field(n, "name")));
    if (missing(result)) {
      return nil();
    }
    return result;
  }
  if (k == "Current" || k == "Identity") {
    return v;
  }
  if (k == "Literal") {
    return field(n, "value");
  }
  if (k == "Pipe" || k == "Subexpression" || k == "IndexExpression") {
    return evaluate(child(n, 1), evaluate(child(n, 0), v));
  }
  if (k == "Index") {
    if (kind(v) != "array") {
      return nil();
    }
    final dynamic ix = field(n, "value");
    if (nan(toNumber(ix))) {
      return nil();
    }
    int i = integer(ix);
    if (i < 0) {
      i += size(v);
    }
    if (i < 0 || i >= size(v)) {
      return nil();
    }
    return at(v, i);
  }
  if (k == "Slice") {
    if (kind(v) != "array") {
      return nil();
    }
    final dynamic p = child(n, 2);
    int step = 1;
    if (!isNil(p)) {
      step = integer(p);
    }
    if (step == 0) {
      throw invalid("invalid-value");
    }
    final int end = size(v);
    int startDefault = 0;
    int stopDefault = end;
    if (step < 0) {
      startDefault = end - 1;
      stopDefault = -1;
    }
    int i = cap(child(n, 0), startDefault, step, end);
    final int stop = cap(child(n, 1), stopDefault, step, end);
    final dynamic out = arr();
    while ((step > 0 && i < stop) || (step < 0 && i > stop)) {
      append(out, at(v, i));
      i += step;
    }
    return out;
  }
  if (k == "Projection" ||
      k == "ValueProjection" ||
      k == "FilterProjection" ||
      k == "Flatten") {
    dynamic base = evaluate(child(n, 0), v);
    if (k == "ValueProjection") {
      if (kind(base) != "object" && kind(base) != "expref") {
        return nil();
      }
      base = objectValues(base);
    }
    if (kind(base) != "array") {
      return nil();
    }
    final dynamic out = arr();
    int i = 0;
    while (i < size(base)) {
      final dynamic item = at(base, i);
      if (k == "Flatten") {
        if (kind(item) == "array") {
          int j = 0;
          while (j < size(item)) {
            append(out, at(item, j));
            j += 1;
          }
        } else {
          append(out, item);
        }
      } else if (k != "FilterProjection" ||
          truth(evaluate(child(n, 2), item))) {
        final dynamic x = evaluate(child(n, 1), item);
        if (!isNil(x)) {
          append(out, x);
        }
      }
      i += 1;
    }
    return out;
  }
  if (k == "Comparator") {
    final dynamic a = evaluate(child(n, 0), v);
    final dynamic b = evaluate(child(n, 1), v);
    final String op = stringValue(field(n, "name"));
    if (op == "EQ") {
      return boolValue(equal(a, b));
    }
    if (op == "NE") {
      return boolValue(!equal(a, b));
    }
    final int ord = compare(a, b);
    return boolValue(
      (ord == -1 && (op == "LT" || op == "LTE")) ||
          (ord == 1 && (op == "GT" || op == "GTE")) ||
          (ord == 0 && (op == "LTE" || op == "GTE")),
    );
  }
  if (k == "NotExpression") {
    return boolValue(!truth(evaluate(child(n, 0), v)));
  }
  if (k == "AndExpression" || k == "OrExpression") {
    final dynamic left = evaluate(child(n, 0), v);
    if (truth(left) == (k == "AndExpression")) {
      return evaluate(child(n, 1), v);
    }
    return left;
  }
  if (k == "ExpressionReference") {
    final dynamic result = child(n, 0);
    put(result, "jmespathType", str("Expref"));
    return result;
  }
  final dynamic cs = field(n, "children");
  if (k == "Function") {
    final dynamic args = arr();
    int i = 0;
    while (i < size(cs)) {
      append(args, evaluate(at(cs, i), v));
      i += 1;
    }
    return callFunction(stringValue(field(n, "name")), args);
  }
  if (k == "MultiSelectList" || k == "MultiSelectHash") {
    if (isNil(v)) {
      return nil();
    }
    dynamic out = arr();
    if (k == "MultiSelectHash") {
      out = obj();
    }
    int i = 0;
    while (i < size(cs)) {
      final dynamic c = at(cs, i);
      if (k == "MultiSelectHash") {
        put(out, stringValue(field(c, "name")), evaluate(field(c, "value"), v));
      } else {
        append(out, evaluate(c, v));
      }
      i += 1;
    }
    return out;
  }
  throw syntaxError();
}

dynamic objectValues(dynamic v) {
  final dynamic ks = keys(v);
  final dynamic result = arr();
  int i = 0;
  while (i < size(ks)) {
    append(result, field(v, stringValue(at(ks, i))));
    i += 1;
  }
  return result;
}

bool matchesType(dynamic v, String t) {
  if (t == "any") {
    return true;
  }
  if (t == "numbers" || t == "strings") {
    if (kind(v) != "array") {
      return false;
    }
    int i = 0;
    while (i < size(v)) {
      if (kind(at(v, i)) + "s" != t) {
        return false;
      }
      i += 1;
    }
    return true;
  }
  return kind(v) == t;
}

dynamic callFunction(String name, dynamic args) {
  checkArguments(name, args);
  final dynamic a = at(args, 0);
  final dynamic b = at(args, 1);
  if (name == "type") {
    return str(kind(a));
  }
  if (name == "length") {
    if (kind(a) == "object") {
      return intValue(size(keys(a)));
    }
    return intValue(size(a));
  }
  if (name == "keys") {
    return keys(a);
  }
  if (name == "values") {
    return objectValues(a);
  }
  if (name == "to_array") {
    if (kind(a) == "array") {
      return a;
    }
    return one(a);
  }
  if (name == "to_number") {
    if (kind(a) == "number") {
      return a;
    }
    if (kind(a) == "string" && !nan(toNumber(a))) {
      return numeric(toNumber(a));
    }
    return nil();
  }
  if (name == "to_string") {
    if (kind(a) == "string") {
      return a;
    }
    return jsonText(a);
  }
  if (name == "not_null") {
    int i = 0;
    while (i < size(args)) {
      if (!isNil(at(args, i))) {
        return at(args, i);
      }
      i += 1;
    }
    return nil();
  }
  if (name == "sum" || name == "avg") {
    double sum = 0.0;
    int i = 0;
    while (i < size(a)) {
      sum += toNumber(at(a, i));
      i += 1;
    }
    if (name == "avg") {
      return numeric(sum / size(a));
    }
    return numeric(sum);
  }
  if (name == "min" || name == "max") {
    if (size(a) == 0) {
      return nil();
    }
    dynamic best = at(a, 0);
    int i = 1;
    while (i < size(a)) {
      final dynamic item = at(a, i);
      int ord = compare(item, best);
      if (kind(item) == "string") {
        ord = localeOrder(item, best);
      }
      if ((name == "min" && ord < 0) || (name == "max" && ord > 0)) {
        best = item;
      }
      i += 1;
    }
    return best;
  }
  if (name == "max_by" || name == "min_by") {
    dynamic extreme = infinity(name == "min_by");
    dynamic result = undefinedValue;
    int i = 0;
    while (i < size(a)) {
      final dynamic item = at(a, i);
      final dynamic key = evaluate(b, item);
      final String t = kind(key);
      if (t != "number" && t != "string") {
        throw invalid("invalid-type");
      }
      final int ord = compare(key, extreme);
      if ((ord == -1 && name == "min_by") || (ord == 1 && name == "max_by")) {
        extreme = key;
        result = item;
      }
      i += 1;
    }
    return result;
  }
  if (name == "merge") {
    final dynamic out = obj();
    int i = 0;
    while (i < size(args)) {
      final dynamic item = at(args, i);
      if (!isNil(item)) {
        final dynamic ks = keys(item);
        int j = 0;
        while (j < size(ks)) {
          final String key = stringValue(at(ks, j));
          put(out, key, field(item, key));
          j += 1;
        }
      }
      i += 1;
    }
    return out;
  }
  if (name == "map") {
    final dynamic out = arr();
    int i = 0;
    while (i < size(b)) {
      append(out, evaluate(a, at(b, i)));
      i += 1;
    }
    return out;
  }
  if (name == "sort_by") {
    return sortByExpression(a, b);
  }
  return primitiveFunction(name, a, b);
}

void checkArguments(String name, dynamic args) {
  if (name == "abs" || name == "ceil" || name == "floor") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "number"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "avg" || name == "sum") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "numbers"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "contains") {
    if (size(args) < 2 ||
        (size(args) > 2 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "string") ||
        matchesType(at(args, 0), "array"))) {
      throw invalid("invalid-type");
    }
    if (!(matchesType(at(args, 1), "any"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "starts_with" || name == "ends_with") {
    if (size(args) < 2 ||
        (size(args) > 2 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "string"))) {
      throw invalid("invalid-type");
    }
    if (!(matchesType(at(args, 1), "string"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "length") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "object") ||
        matchesType(at(args, 0), "string") ||
        matchesType(at(args, 0), "array"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "map") {
    if (size(args) < 2 ||
        (size(args) > 2 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "expref"))) {
      throw invalid("invalid-type");
    }
    if (!(matchesType(at(args, 1), "array"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "max" || name == "min" || name == "sort") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "numbers") ||
        matchesType(at(args, 0), "strings"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "keys" || name == "values" || name == "merge") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "object"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "max_by" || name == "min_by" || name == "sort_by") {
    if (size(args) < 2 ||
        (size(args) > 2 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "array"))) {
      throw invalid("invalid-type");
    }
    if (!(matchesType(at(args, 1), "expref"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "join") {
    if (size(args) < 2 ||
        (size(args) > 2 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "string"))) {
      throw invalid("invalid-type");
    }
    if (!(matchesType(at(args, 1), "strings"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "reverse") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "array") ||
        matchesType(at(args, 0), "string"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  if (name == "to_array" ||
      name == "to_number" ||
      name == "to_string" ||
      name == "type" ||
      name == "not_null") {
    if (size(args) < 1 ||
        (size(args) > 1 && name != "merge" && name != "not_null")) {
      throw invalid("invalid-arity");
    }
    if (!(matchesType(at(args, 0), "any"))) {
      throw invalid("invalid-type");
    }
    return;
  }
  throw invalid("unknown-function");
}

@JS('Object.defineProperty')
external JSObject defineProperty(
  JSObject target,
  JSString key,
  JSObject descriptor,
);

JSAny? toHost(dynamic v) {
  if (v == null || missing(v)) return null;
  if (v is String) return v.toJS;
  if (v is bool) return v.toJS;
  if (v is num) return v.toJS;
  final JSObject out = v is List
      ? JSArray<JSAny?>.withLength(v.length)
      : JSObject();
  void set(String key, dynamic value) {
    if (missing(value) || key == "__proto__") {
      final desc = JSObject();
      desc['enumerable'] = true.toJS;
      desc['configurable'] = true.toJS;
      desc['writable'] = true.toJS;
      if (!missing(value)) desc['value'] = toHost(value);
      defineProperty(out, key.toJS, desc);
    } else {
      out[key] = toHost(value);
    }
  }

  if (v is List) {
    for (var i = 0; i < v.length; i++) set(i.toString(), v[i]);
  } else {
    (v as Map).forEach((k, val) => set(k as String, val));
  }
  return out;
}

@JS('Object.keys')
external JSArray<JSString> ownKeys(JSObject value);

dynamic fromHost(JSAny? value) {
  if (value == null) return null;
  if (value.isA<JSString>()) return (value as JSString).toDart;
  if (value.isA<JSNumber>()) return (value as JSNumber).toDartDouble;
  if (value.isA<JSBoolean>()) return (value as JSBoolean).toDart;
  if (value.isA<JSArray>()) {
    final a = value as JSArray<JSAny?>;
    return [for (var i = 0; i < a.length; i++) fromHost(a[i])];
  }
  final record = value as JSObject;
  return {
    for (final key in ownKeys(record).toDart)
      key.toDart: fromHost(record.getProperty(key)),
  };
}

JSAny? run(JSObject input) {
  final data = fromHost(input['data']);
  final queries = fromHost(input['queries']) as List;
  final result = [];
  for (final q in queries) {
    try {
      final ast = Parser(q['expression'] as String).parse();
      result.add({'value': q['op'] == 'compile' ? ast : evaluate(ast, data)});
    } on QueryError catch (e) {
      result.add({'error': e.category});
    } catch (_) {
      result.add({'error': 'syntax'});
    }
  }
  return toHost(result);
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
