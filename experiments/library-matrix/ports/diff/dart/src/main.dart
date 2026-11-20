import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

class Run {
  int kind;
  int count;
  int prev;
  Run(this.kind, this.count, this.prev);
}

class Path {
  int old;
  int last;
  Path(this.old, this.last);
}

class Change {
  int kind;
  int count;
  String value;
  Change(this.kind, this.count, this.value);
}

class Hunk {
  int oldStart;
  int oldLines;
  int newStart;
  int newLines;
  List<String> lines;
  Hunk(this.oldStart, this.oldLines, this.newStart, this.newLines, this.lines);
}

class Patch {
  JSAny? meta;
  List<Hunk> hunks;
  Patch(this.meta, this.hunks);
}

@JS('Object.keys')
external JSArray<JSString> _keys(JSObject o);
@JS('Object.create')
external JSObject _dict(JSAny? proto);
@JS('Array.isArray')
external bool isArray(JSAny? v);
@JS('JSON.stringify')
external String _json(JSAny? v, [JSAny? replacer, int? indent]);
@JS('undefined')
external JSAny? undefinedValueRaw;
int smaller(int a, int b) => a < b ? a : b;
int bigger(int a, int b) => a > b ? a : b;
int divide(int a, int b) => a ~/ b;
void flip<T>(List<T> a) {
  for (var i = 0; i < a.length ~/ 2; i++) {
    final j = a.length - i - 1;
    final t = a[i];
    a[i] = a[j];
    a[j] = t;
  }
}

void pop<T>(List<T> a) {
  a.removeLast();
}

List<int> utf8(String s) => const Utf8Encoder().convert(s);
String decodeUtf8(List<int> b) =>
    const Utf8Decoder(allowMalformed: true).convert(b);
String cut(String s, int a, int b) => s.substring(a, b);
int code(String s, int i) => s.codeUnitAt(i);
String unit(int c) => String.fromCharCode(c);
bool starts(String s, String p) => s.startsWith(p);
bool ends(String s, String p) => s.endsWith(p);
bool contains(String s, String p) => s.contains(p);
String replaceAll(String s, String a, String b) => s.replaceAll(a, b);
String lower(String s) => s.toLowerCase();
int indexOf(String s, String p) => s.indexOf(p);
int lastIndex(String s, String p) => s.lastIndexOf(p);
List<String> split(String s, String p) => s.split(p);
String strAt(String s, int i) => s.split('|')[i];
String join(List<String> a, String sep) => a.join(sep);
String dec(int n) => n.toString();
int parseInt(String s) => int.parse(s);
String compact(JSAny? v) => _json(v);
String pretty(JSAny? v) => _json(v, null, 2);
JSAny? dictionary() => _dict(null);
JSAny? obj() => JSObject();
JSAny? arr(int n) => JSArray<JSAny?>.withLength(n);
bool defined(JSAny? v) => !v.isUndefinedOrNull;
JSAny? get(JSAny? v, String k) =>
    defined(v) ? (v as JSObject)[k] : undefinedValueRaw;
void put(JSAny? v, String k, JSAny? x) {
  (v as JSObject).setProperty(k.toJS, x);
}

void assign(JSAny? v, int i, JSAny? x) {
  (v as JSArray<JSAny?>)[i] = x;
}

JSAny? item(JSAny? v, int i) => (v as JSArray<JSAny?>)[i];
List<String> names(JSAny? v) =>
    _keys(v as JSObject).toDart.map((s) => s.toDart).toList();
void sortNames(List<String> a) => a.sort();
int size(JSAny? v) => (v as JSArray<JSAny?>).length;
bool flag(JSAny? v, String k) {
  final b = get(v, k);
  return defined(b) && (b as JSBoolean).toDart;
}

JSAny? undefinedValue() => undefinedValueRaw;
String text(JSAny? v) => (v as JSString).toDart;
JSAny? string(String s) => s.toJS;
int integer(JSAny? v) => (v as JSNumber).toDartInt;
JSAny? numValue(int n) => n.toJS;
JSAny? boolVal(bool b) => b.toJS;
bool isString(JSAny? v) => v.typeofEquals('string');
bool isObject(JSAny? v) => defined(v) && v.typeofEquals('object');
void ensure(bool b) {
  if (!b) throw FormatException('invalid patch');
}

// The edit history is an arena of linked runs, shared between neighbouring diagonals.
int record(List<Run> arena, int last, int kind, int count, bool separate) {
  if (count == 0) {
    return last;
  }
  int prev = last;
  int n = count;
  if (separate) {
    for (var q = 0; q < count; q++) {
      arena.add(Run(kind, 1, prev));
      prev = arena.length - 1;
    }
    return prev;
  }
  if (prev != 0 && arena[prev].kind == kind) {
    n += arena[prev].count;
    prev = arena[prev].prev;
  }
  arena.add(Run(kind, n, prev));
  return arena.length - 1;
}

int advance(
  Path p,
  int k,
  List<String> a,
  List<String> b,
  List<Run> arena,
  bool separate,
) {
  final int initial = p.old;
  int j = initial - k;
  while (p.old + 1 < a.length && j + 1 < b.length && a[p.old + 1] == b[j + 1]) {
    p.old += 1;
    j += 1;
  }
  p.last = record(arena, p.last, 0, p.old - initial, separate);
  return j;
}

List<Run> finish(List<Run> arena, int last) {
  int i = last;
  final List<Run> spans = <Run>[];
  while (i > 0) {
    spans.add(arena[i]);
    i = arena[i].prev;
  }
  flip(spans);
  return spans;
}

List<Run> myers(List<String> a, List<String> b, bool separate) {
  final int limit = a.length + b.length;
  final int base = limit + 1;
  final List<Path> best = <Path>[];
  for (var i = 0; i < base * 2 + 1; i++) {
    best.add(Path(-2, 0));
  }
  final List<Run> arena = <Run>[];
  arena.add(Run(0, 0, 0));
  final Path first = Path(-1, 0);
  final int origin = advance(first, 0, a, b, arena, separate);
  if (first.old + 1 == a.length && origin + 1 == b.length) {
    return finish(arena, first.last);
  }
  best[base] = first;
  int lo = -limit;
  int hi = limit;
  for (var d = 1; d < limit + 1; d++) {
    int k = bigger(lo, -d);
    final int stop = smaller(hi, d);
    while (k <= stop) {
      final int i = base + k;
      final Path rem = best[i - 1];
      final Path ins = best[i + 1];
      best[i - 1] = Path(-2, 0);
      final bool canR = rem.old >= -1 && rem.old + 1 < a.length;
      final bool canA =
          ins.old >= -1 && ins.old - k >= 0 && ins.old - k < b.length;
      if (canR || canA) {
        final bool take = !canR || (canA && rem.old < ins.old);
        final Path path = Path(
          (take ? ins.old : rem.old + 1),
          record(
            arena,
            (take ? ins.last : rem.last),
            (take ? 2 : 1),
            1,
            separate,
          ),
        );
        final int j = advance(path, k, a, b, arena, separate);
        if (path.old + 1 == a.length && j + 1 == b.length) {
          return finish(arena, path.last);
        }
        best[i] = path;
        if (path.old + 1 == a.length) {
          hi = smaller(hi, k - 1);
        }
        if (j + 1 == b.length) {
          lo = bigger(lo, k + 1);
        }
      } else {
        best[i] = Path(-2, 0);
      }
      k += 2;
    }
  }
  return <Run>[];
}

bool ws(int c) {
  return c >= 9 && c <= 13 ||
      c == 32 ||
      c == 160 ||
      c == 0x1680 ||
      c >= 0x2000 && c <= 0x200a ||
      c == 0x2028 ||
      c == 0x2029 ||
      c == 0x202f ||
      c == 0x205f ||
      c == 0x3000 ||
      c == 0xfeff;
}

bool word(int c) {
  return c >= 65 && c <= 90 ||
      c >= 97 && c <= 122 ||
      c >= 48 && c <= 57 ||
      c == 95 ||
      c == 0xad ||
      c >= 0xc0 && c <= 0xd6 ||
      c >= 0xd8 && c <= 0xf6 ||
      c >= 0xf8 && c <= 0x2c6 ||
      c >= 0x2c8 && c <= 0x2d7 ||
      c >= 0x2de && c <= 0x2ff ||
      c >= 0x1e00 && c <= 0x1eff;
}

String leading(String s) {
  int i = 0;
  while (i < s.length && ws(code(s, i))) {
    i += 1;
  }
  return cut(s, 0, i);
}

String trailing(String s) {
  int i = s.length;
  while (i > 0 && ws(code(s, i - 1))) {
    i -= 1;
  }
  return cut(s, i, s.length);
}

String trim(String s) {
  final int start = leading(s).length;
  return cut(s, start, bigger(start, s.length - trailing(s).length));
}

List<String> tokenize(String op, String s, JSAny? options) {
  final List<String> out = <String>[];
  int i = 0;
  int start = 0;
  String prev = "";
  if (op == "diffLines" || op == "diffJson") {
    for (var j = 0; j < s.length; j++) {
      if (code(s, j) == 10) {
        final int end = (j > start && code(s, j - 1) == 13 ? j - 1 : j);
        if (flag(options, "newlineIsToken")) {
          if (end > start) {
            out.add(cut(s, start, end));
          }
          out.add(
            (flag(options, "stripTrailingCr") ? "\n" : cut(s, end, j + 1)),
          );
        } else {
          out.add(
            (flag(options, "stripTrailingCr") && end < j
                ? cut(s, start, end) + "\n"
                : cut(s, start, j + 1)),
          );
        }
        start = j + 1;
      }
    }
    if (start < s.length) {
      out.add(cut(s, start, s.length));
    }
    return out;
  }
  while (i < s.length) {
    final int c = code(s, i);
    start = i;
    i +=
        (c >= 0xd800 &&
            c <= 0xdbff &&
            i + 1 < s.length &&
            code(s, i + 1) >= 0xdc00 &&
            code(s, i + 1) <= 0xdfff
        ? 2
        : 1);
    if (op == "diffWords") {
      if (ws(c)) {
        while (i < s.length && ws(code(s, i))) {
          i += 1;
        }
        prev = cut(s, start, i);
        if (out.length == 0) {
          out.add(prev);
        } else {
          out[out.length - 1] = out[out.length - 1] + prev;
        }
      } else {
        if (word(c)) {
          while (i < s.length && word(code(s, i))) {
            i += 1;
          }
        }
        final String t = cut(s, start, i);
        if (prev != "" && out.length == 1 && out[0] == prev) {
          out[0] = prev + t;
        } else {
          out.add(prev + t);
        }
        prev = "";
      }
    } else {
      out.add(cut(s, start, i));
    }
  }
  return out;
}

List<String> normal(String op, List<String> tokens, JSAny? opts) {
  final List<String> result = <String>[];
  for (var i = 0; i < tokens.length; i++) {
    String s = tokens[i];
    if (op == "diffWords" ||
        op == "diffLines" &&
            flag(opts, "ignoreWhitespace") &&
            (!flag(opts, "newlineIsToken") || !contains(s, "\n"))) {
      s = trim(s);
    } else {
      if (op == "diffLines" &&
          flag(opts, "ignoreNewlineAtEof") &&
          !flag(opts, "newlineIsToken") &&
          ends(s, "\n")) {
        s = cut(s, 0, s.length - 1);
      }
    }
    if (op == "diffJson") {
      s = replaceAll(replaceAll(s, ",\n", "\n"), ",\r", "\r");
    }
    result.add((flag(opts, "ignoreCase") ? lower(s) : s));
  }
  return result;
}

int common(String a, String b, bool back) {
  int n = 0;
  while (n < a.length &&
      n < b.length &&
      code(a, (back ? a.length - n - 1 : n)) ==
          code(b, (back ? b.length - n - 1 : n))) {
    n += 1;
  }
  return n;
}

int overlap(String a, String b) {
  int n = smaller(a.length, b.length);
  while (n > 0 && cut(a, a.length - n, a.length) != cut(b, 0, n)) {
    n -= 1;
  }
  return n;
}

void clean(List<Change> out, int start, int del, int ins, int end) {
  if (del >= 0 && ins >= 0) {
    final Change d = out[del];
    final Change a = out[ins];
    final String dp = leading(d.value);
    final String ap = leading(a.value);
    final String ds = trailing(d.value);
    final String asuf = trailing(a.value);
    if (start >= 0) {
      final int n = common(dp, ap, false);
      final Change s = out[start];
      s.value = cut(s.value, 0, s.value.length - ap.length) + cut(ap, 0, n);
      d.value = cut(d.value, n, d.value.length);
      a.value = cut(a.value, n, a.value.length);
    }
    if (end >= 0) {
      final int n = common(ds, asuf, true);
      final Change e = out[end];
      e.value =
          cut(asuf, asuf.length - n, asuf.length) +
          cut(e.value, asuf.length, e.value.length);
      d.value = cut(d.value, 0, d.value.length - n);
      a.value = cut(a.value, 0, a.value.length - n);
    }
  } else {
    if (ins >= 0) {
      if (start >= 0) {
        final Change a = out[ins];
        a.value = cut(a.value, leading(a.value).length, a.value.length);
      }
      if (end >= 0) {
        final Change e = out[end];
        e.value = cut(e.value, leading(e.value).length, e.value.length);
      }
    } else {
      if (del >= 0) {
        final Change d = out[del];
        if (start >= 0 && end >= 0) {
          final Change e = out[end];
          final Change s = out[start];
          final String w = leading(e.value);
          final int a = common(w, leading(d.value), false);
          final int b = common(cut(w, a, w.length), trailing(d.value), true);
          d.value = cut(d.value, a, d.value.length - b);
          e.value = cut(e.value, w.length - b, e.value.length);
          s.value = cut(s.value, 0, s.value.length - b);
        } else {
          if (start >= 0) {
            d.value = cut(
              d.value,
              overlap(trailing(out[start].value), leading(d.value)),
              d.value.length,
            );
          }
          if (end >= 0) {
            d.value = cut(
              d.value,
              0,
              d.value.length -
                  overlap(trailing(d.value), leading(out[end].value)),
            );
          }
        }
      }
    }
  }
}

List<Change> changes(String op, List<String> a, List<String> b, JSAny? opts) {
  final List<Run> spans = myers(
    normal(op, a, opts),
    normal(op, b, opts),
    flag(opts, "oneChangePerToken"),
  );
  final List<Change> out = <Change>[];
  int old = 0;
  int pos = 0;
  for (var i = 0; i < spans.length; i++) {
    final Run r = spans[i];
    String value = "";
    for (var j = 0; j < r.count; j++) {
      final String s =
          (r.kind == 1 ||
              (op == "diffJson" &&
                  r.kind == 0 &&
                  a[old + j].length > b[pos + j].length)
          ? a[old + j]
          : b[pos + j]);
      value += (op == "diffWords" && j > 0
          ? cut(s, leading(s).length, s.length)
          : s);
    }
    out.add(Change(r.kind, r.count, value));
    if (r.kind != 2) {
      old += r.count;
    }
    if (r.kind != 1) {
      pos += r.count;
    }
  }
  if (op == "diffWords" && !flag(opts, "oneChangePerToken")) {
    int start = -1;
    int del = -1;
    int ins = -1;
    for (var i = 0; i < out.length; i++) {
      if (out[i].kind == 2) {
        ins = i;
      } else {
        if (out[i].kind == 1) {
          del = i;
        } else {
          clean(out, start, del, ins, i);
          start = i;
          del = -1;
          ins = -1;
        }
      }
    }
    clean(out, start, del, ins, -1);
  }
  return out;
}

Hunk fresh() {
  return Hunk(0, 0, 0, 0, <String>[]);
}

void emit(Hunk h, int k, List<String> src, int at, int n) {
  final String mark = (k == 2 ? "+" : (k == 1 ? "-" : " "));
  for (var i = at; i < at + n; i++) {
    final String s = src[i];
    if (ends(s, "\n")) {
      h.lines.add(mark + cut(s, 0, s.length - 1));
    } else {
      h.lines.add(mark + s);
      h.lines.add("\\ No newline at end of file");
    }
  }
  if (k != 2) {
    h.oldLines += n;
  }
  if (k != 1) {
    h.newLines += n;
  }
}

Patch create(JSAny? input, int ctx) {
  final JSAny? meta = obj();
  for (var j = 0; j < 4; j++) {
    final String k = strAt("oldFileName|newFileName|oldHeader|newHeader", j);
    put(meta, k, get(input, k));
  }
  final Patch p = Patch(meta, <Hunk>[]);
  final List<String> a = tokenize(
    "diffLines",
    text(get(input, "before")),
    undefinedValue(),
  );
  final List<String> b = tokenize(
    "diffLines",
    text(get(input, "after")),
    undefinedValue(),
  );
  final List<Run> spans = myers(a, b, false);
  int old = 0;
  int newer = 0;
  int kept = 0;
  Hunk h = fresh();
  for (var i = 0; i < spans.length; i++) {
    final Run r = spans[i];
    if (r.kind == 0) {
      if (h.oldStart > 0) {
        final bool close = i + 1 == spans.length || r.count > ctx * 2;
        emit(h, 0, b, newer, (close ? smaller(ctx, r.count) : r.count));
        if (close) {
          p.hunks.add(h);
          h = fresh();
        }
      }
      kept = r.count;
    } else {
      if (h.oldStart == 0) {
        final int k = smaller(ctx, kept);
        h.oldStart = old + 1 - k;
        h.newStart = newer + 1 - k;
        emit(h, 0, b, newer - k, k);
      }
      if (r.kind == 2) {
        emit(h, r.kind, b, newer, r.count);
      } else {
        emit(h, r.kind, a, old, r.count);
      }
    }
    if (r.kind != 2) {
      old += r.count;
    }
    if (r.kind != 1) {
      newer += r.count;
    }
  }
  if (h.oldStart > 0) {
    p.hunks.add(h);
  }
  return p;
}

String quotedName(String s) {
  final List<int> bytes = utf8(s);
  bool need = false;
  for (var i = 0; i < bytes.length; i++) {
    final int c = bytes[i];
    if (c < 32 || c > 126 || c == 34 || c == 92) {
      need = true;
    }
  }
  if (!need) {
    return s;
  }
  String out = "\"";
  for (var i = 0; i < bytes.length; i++) {
    final int c = bytes[i];
    if (c >= 7 && c <= 13) {
      out += "\\" + cut("abtnvfr", c - 7, c - 6);
    } else {
      if (c == 34 || c == 92) {
        out += "\\" + unit(c);
      } else {
        if (c < 32 || c > 126) {
          out +=
              "\\" +
              unit(48 + divide(c, 64)) +
              unit(48 + divide(c, 8) % 8) +
              unit(48 + c % 8);
        } else {
          out += unit(c);
        }
      }
    }
  }
  return out + "\"";
}

int endQuote(String s) {
  int i = 1;
  while (i < s.length) {
    final int c = code(s, i);
    if (c == 34) {
      return i;
    }
    if (c == 92) {
      i += 1;
    }
    i += 1;
  }
  return -1;
}

String unquote(String s) {
  if (!starts(s, "\"")) {
    return s;
  }
  final int stop = endQuote(s);
  if (stop < 0) {
    return s;
  }
  final List<int> bytes = <int>[];
  final out = StringBuffer();
  void flush() {
    if (bytes.isNotEmpty) {
      out.write(decodeUtf8(bytes));
      bytes.clear();
    }
  }
  int i = 1;
  while (i < stop) {
    final int c = code(s, i);
    i += 1;
    if (c != 92) {
      flush();
      out.writeCharCode(c);
    } else {
      final int v = code(s, i);
      i += 1;
      if (v >= 48 && v <= 55 && i + 1 < stop) {
        final int x = code(s, i) - 48;
        final int y = code(s, i + 1) - 48;
        if (x < 0 || x > 7 || y < 0 || y > 7) {
          return s;
        }
        bytes.add(((v - 48) * 64 + x * 8 + y) % 256);
        i += 2;
      } else {
        final int j = indexOf("abtnvfr", unit(v));
        if (j < 0 && v != 34 && v != 92) {
          return s;
        }
        flush();
        out.writeCharCode(j >= 0 ? j + 7 : v);
      }
    }
  }
  flush();
  return out.toString();
}

String formatPatch(Patch p) {
  final JSAny? old = get(p.meta, "oldFileName");
  final JSAny? newer = get(p.meta, "newFileName");
  String out = "";
  if (defined(old) && defined(newer) && text(old) == text(newer)) {
    out += "Index: " + text(old) + "\n";
  }
  out +=
      "===================================================================\n";
  if (defined(old) && defined(newer)) {
    for (var i = 0; i < 2; i++) {
      final String k = (i == 0 ? "old" : "new");
      final JSAny? h = get(p.meta, k + "Header");
      out +=
          (i == 0 ? "--- " : "+++ ") +
          quotedName(text(get(p.meta, k + "FileName"))) +
          (defined(h) && text(h) != "" ? "\t" + text(h) : "") +
          "\n";
    }
  }
  for (var i = 0; i < p.hunks.length; i++) {
    final Hunk h = p.hunks[i];
    out +=
        "@@ -" +
        dec(h.oldStart - (h.oldLines == 0 ? 1 : 0)) +
        "," +
        dec(h.oldLines) +
        " +" +
        dec(h.newStart - (h.newLines == 0 ? 1 : 0)) +
        "," +
        dec(h.newLines) +
        " @@\n";
    for (var j = 0; j < h.lines.length; j++) {
      out += h.lines[j] + "\n";
    }
  }
  return out;
}

JSAny? writePatch(Patch p, bool reverse) {
  final JSAny? all = arr(p.hunks.length);
  for (var i = 0; i < p.hunks.length; i++) {
    final Hunk h = p.hunks[i];
    final JSAny? x = obj();
    if (reverse) {
      put(x, "oldLines", numValue(h.oldLines));
      put(x, "oldStart", numValue(h.oldStart));
      put(x, "newLines", numValue(h.newLines));
      put(x, "newStart", numValue(h.newStart));
    } else {
      put(x, "oldStart", numValue(h.oldStart));
      put(x, "oldLines", numValue(h.oldLines));
      put(x, "newStart", numValue(h.newStart));
      put(x, "newLines", numValue(h.newLines));
    }
    final JSAny? data = arr(h.lines.length);
    for (var j = 0; j < h.lines.length; j++) {
      assign(data, j, string(h.lines[j]));
    }
    put(x, "lines", data);
    assign(all, i, x);
  }
  put(p.meta, "hunks", all);
  return p.meta;
}

Patch readPatch(JSAny? v) {
  final JSAny? meta = obj();
  final List<String> ks = names(v);
  for (var i = 0; i < ks.length; i++) {
    put(meta, ks[i], get(v, ks[i]));
  }
  final Patch p = Patch(meta, <Hunk>[]);
  final JSAny? raw = get(v, "hunks");
  for (var i = 0; i < size(raw); i++) {
    final JSAny? x = item(raw, i);
    final Hunk h = Hunk(
      integer(get(x, "oldStart")),
      integer(get(x, "oldLines")),
      integer(get(x, "newStart")),
      integer(get(x, "newLines")),
      <String>[],
    );
    final JSAny? a = get(x, "lines");
    for (var j = 0; j < size(a); j++) {
      h.lines.add(text(item(a, j)));
    }
    p.hunks.add(h);
  }
  return p;
}

bool fileLine(String s) {
  return s.length > 3 &&
      (starts(s, "---") || starts(s, "+++")) &&
      (code(s, 3) == 32 || code(s, 3) == 9);
}

bool header(String s) {
  return starts(s, "diff --git ") ||
      starts(s, "Index: ") ||
      starts(s, "diff -r ");
}

void fileMeta(JSAny? meta, String s) {
  final List<String> parts = split(trim(cut(s, 3, s.length)), "\t");
  final String side = (starts(s, "---") ? "old" : "new");
  final String n = parts[0];
  put(
    meta,
    side + "FileName",
    string((starts(n, "\"") ? unquote(n) : replaceAll(n, "\\\\", "\\"))),
  );
  put(meta, side + "Header", string((parts.length > 1 ? trim(parts[1]) : "")));
}

void gitNames(JSAny? meta, String s) {
  int splitAt = -1;
  if (starts(s, "\"")) {
    splitAt = endQuote(s) + 1;
  } else {
    final int q = indexOf(s, "\"");
    if (q > 0) {
      splitAt = q - 1;
    } else {
      final List<int> spots = <int>[];
      for (var i = 0; i < bigger(0, s.length - 2); i++) {
        if (cut(s, i, i + 3) == " b/") {
          spots.add(i);
        }
      }
      if (spots.length > 0 && starts(s, "a/")) {
        splitAt = spots[divide(spots.length, 2)];
      }
    }
  }
  if (splitAt > 0 && splitAt + 1 < s.length) {
    put(meta, "oldFileName", string(unquote(cut(s, 0, splitAt))));
    put(meta, "newFileName", string(unquote(cut(s, splitAt + 1, s.length))));
  }
}

void extended(JSAny? meta, String s) {
  for (var i = 0; i < 8; i++) {
    final String tag = strAt(
      "rename from |rename to |copy from |copy to |new file mode |deleted file mode |old mode |new mode ",
      i,
    );
    if (starts(s, tag)) {
      final String v = cut(s, tag.length, s.length);
      if (i < 4) {
        put(
          meta,
          (i % 2 == 0 ? "oldFileName" : "newFileName"),
          string((i % 2 == 0 ? "a/" : "b/") + unquote(v)),
        );
        put(meta, (i < 2 ? "isRename" : "isCopy"), boolVal(true));
      } else {
        if (i < 6) {
          put(meta, (i == 4 ? "isCreate" : "isDelete"), boolVal(true));
        }
        put(meta, (i == 5 || i == 6 ? "oldMode" : "newMode"), string(v));
      }
      return;
    }
  }
  if (starts(s, "Binary files ")) {
    put(meta, "isBinary", boolVal(true));
  }
}

List<int> parseRange(String s) {
  final List<String> cols = split(cut(s, 1, s.length), ",");
  final int n = (cols.length > 1 ? parseInt(cols[1]) : 1);
  final List<int> result = <int>[];
  result.add(parseInt(cols[0]) + (n == 0 ? 1 : 0));
  result.add(n);
  return result;
}

List<Patch> parsePatch(String s) {
  final List<String> body = split(s, "\n");
  final List<Patch> out = <Patch>[];
  int i = 0;
  while (i < body.length) {
    final Patch p = Patch(obj(), <Hunk>[]);
    put(p.meta, "hunks", undefinedValue());
    bool seen = false;
    while (i < body.length &&
        !fileLine(body[i]) &&
        !starts(body[i], "@@ ") &&
        !(seen && header(body[i]))) {
      final String l = body[i];
      if (header(l)) {
        seen = true;
        if (starts(l, "diff --git ")) {
          put(p.meta, "isGit", boolVal(true));
          gitNames(p.meta, cut(l, 11, l.length));
        } else {
          put(
            p.meta,
            "index",
            string(
              (starts(l, "Index: ")
                  ? trim(cut(l, 7, l.length))
                  : cut(l, lastIndex(l, " ") + 1, l.length)),
            ),
          );
        }
      } else {
        if (flag(p.meta, "isGit")) {
          extended(p.meta, l);
        }
      }
      i += 1;
    }
    for (var q = 0; q < 2; q++) {
      if (i < body.length && fileLine(body[i])) {
        fileMeta(p.meta, body[i]);
        i += 1;
      }
    }
    ensure(
      defined(get(p.meta, "oldFileName")) ==
          defined(get(p.meta, "newFileName")),
    );
    while (i < body.length &&
        !header(body[i]) &&
        !fileLine(body[i]) &&
        !starts(
          body[i],
          "===================================================================",
        )) {
      final String l = body[i];
      i += 1;
      if (starts(l, "@@ ")) {
        final List<String> cols = split(l, " ");
        ensure(
          cols.length >= 4 &&
              starts(cols[1], "-") &&
              starts(cols[2], "+") &&
              cols[3] == "@@",
        );
        final List<int> o = parseRange(cols[1]);
        final List<int> n = parseRange(cols[2]);
        final Hunk h = Hunk(o[0], o[1], n[0], n[1], <String>[]);
        int old = 0;
        int newer = 0;
        while (i < body.length &&
            (old < h.oldLines || newer < h.newLines || starts(body[i], "\\"))) {
          final String line = body[i];
          ensure(line != "" || i + 1 < body.length);
          final int mark = (line == "" ? 32 : code(line, 0));
          ensure(mark == 32 || mark == 43 || mark == 45 || mark == 92);
          old += (mark == 32 || mark == 45 ? 1 : 0);
          newer += (mark == 32 || mark == 43 ? 1 : 0);
          h.lines.add(line);
          i += 1;
        }
        if (old == 0 && h.oldLines == 1) {
          h.oldLines = 0;
        }
        if (newer == 0 && h.newLines == 1) {
          h.newLines = 0;
        }
        ensure(old == h.oldLines && newer == h.newLines);
        ensure(
          i >= body.length ||
              body[i] == "" ||
              !contains("+ -", cut(body[i], 0, 1)) ||
              fileLine(body[i]),
        );
        p.hunks.add(h);
      }
    }
    out.add(p);
  }
  return out;
}

JSAny? reversePatch(JSAny? v) {
  final Patch p = readPatch(v);
  final bool copy = flag(v, "isCopy");
  if (copy) {
    final JSAny? m = obj();
    final List<String> ks = names(v);
    for (var i = 0; i < ks.length; i++) {
      final String k = ks[i];
      if (k != "isCreate" && k != "isCopy" && k != "isRename") {
        put(m, k, get(v, k));
      }
    }
    p.meta = m;
  }
  for (var i = 0; i < 8; i++) {
    final String k = strAt(
      "oldFileName|oldHeader|newFileName|newHeader|oldMode|newMode|isCreate|isDelete",
      i,
    );
    final String other = strAt(
      "newFileName|newHeader|oldFileName|oldHeader|newMode|oldMode|isDelete|isCreate",
      i,
    );
    JSAny? v1 = get(v, other);
    if (flag(v, "isGit") && ends(k, "FileName") && defined(v1)) {
      final String s = text(v1);
      if (starts(s, "a/") || starts(s, "b/")) {
        v1 = string((starts(s, "a/") ? "b/" : "a/") + cut(s, 2, s.length));
      }
    }
    if (!copy || k != "isCreate") {
      put(p.meta, k, v1);
    }
  }
  if (copy) {
    put(p.meta, "newFileName", string("/dev/null"));
    put(p.meta, "newHeader", undefinedValue());
    put(p.meta, "isDelete", boolVal(true));
    p.hunks = <Hunk>[];
  }
  for (var i = 0; i < p.hunks.length; i++) {
    final Hunk h = p.hunks[i];
    final int oldStart = h.oldStart;
    final int oldLines = h.oldLines;
    h.oldStart = h.newStart;
    h.oldLines = h.newLines;
    h.newStart = oldStart;
    h.newLines = oldLines;
    for (var j = 0; j < h.lines.length; j++) {
      final String l = h.lines[j];
      if (starts(l, "+") || starts(l, "-")) {
        h.lines[j] = (starts(l, "+") ? "-" : "+") + cut(l, 1, l.length);
      }
    }
  }
  return writePatch(p, true);
}

void convert(String source, Patch p) {
  final bool win = contains(source, "\r\n");
  final bool unix = contains(replaceAll(source, "\r\n", ""), "\n");
  if (win == unix) {
    return;
  }
  bool pw = false;
  bool pu = false;
  for (var i = 0; i < p.hunks.length; i++) {
    final List<String> lines = p.hunks[i].lines;
    for (var j = 0; j < lines.length; j++) {
      if (!starts(lines[j], "\\")) {
        if (ends(lines[j], "\r")) {
          pw = true;
        } else {
          if (j + 1 >= lines.length || !starts(lines[j + 1], "\\")) {
            pu = true;
          }
        }
      }
    }
  }
  for (var i = 0; i < p.hunks.length; i++) {
    final List<String> lines = p.hunks[i].lines;
    for (var j = 0; j < lines.length; j++) {
      final String l = lines[j];
      if (win &&
          !pw &&
          !starts(l, "\\") &&
          (j + 1 >= lines.length || !starts(lines[j + 1], "\\"))) {
        lines[j] = l + "\r";
      }
      if (unix && pw && !pu && ends(l, "\r")) {
        lines[j] = cut(l, 0, l.length - 1);
      }
    }
  }
}

bool fits(Hunk h, List<String> lines, int at) {
  int i = at;
  for (var j = 0; j < h.lines.length; j++) {
    final String l = h.lines[j];
    if (!starts(l, "+") && !starts(l, "\\")) {
      if (i >= lines.length ||
          lines[i] != (l == "" ? "" : cut(l, 1, l.length))) {
        return false;
      }
      i += 1;
    }
  }
  return true;
}

JSAny? applyPatch(String source, Patch p) {
  if (p.hunks.length == 0) {
    return string(source);
  }
  convert(source, p);
  final List<String> data = split(source, "\n");
  bool ae = false;
  bool re = false;
  int prev = 0;
  final List<String> last = p.hunks[p.hunks.length - 1].lines;
  for (var i = 0; i < last.length; i++) {
    final String line = last[i];
    if (starts(line, "\\")) {
      ae = ae || prev == 45;
      re = re || prev == 43;
    }
    prev = (line == "" ? 32 : code(line, 0));
  }
  final bool eof = data[data.length - 1] == "";
  if (re && ae && eof || re && !ae && !eof || !re && ae && eof) {
    return boolVal(false);
  }
  if (re && !ae) {
    pop(data);
  }
  if (!re && ae) {
    data.add("");
  }
  int minLine = 0;
  int offset = 0;
  final List<String> res = <String>[];
  for (var i = 0; i < p.hunks.length; i++) {
    final Hunk h = p.hunks[i];
    final int maxLine = data.length - h.oldLines;
    if (maxLine < minLine) {
      return boolVal(false);
    }
    final int first = bigger(
      minLine,
      smaller(maxLine, h.oldStart + offset - 1),
    );
    int pos = -1;
    int d = 0;
    while (pos < 0 && d <= bigger(first - minLine, maxLine - first)) {
      if (first + d <= maxLine && fits(h, data, first + d)) {
        pos = first + d;
      } else {
        if (d > 0 && first - d >= minLine && fits(h, data, first - d)) {
          pos = first - d;
        }
      }
      d += 1;
    }
    if (pos < 0) {
      return boolVal(false);
    }
    for (var j = minLine; j < pos; j++) {
      res.add(data[j]);
    }
    int cursor = pos;
    int ctx = 0;
    for (var j = 0; j < h.lines.length; j++) {
      final String l = h.lines[j];
      final int op = (l == "" ? 32 : code(l, 0));
      if (op == 43) {
        res.add(cut(l, 1, l.length));
        ctx = 0;
      }
      if (op == 45) {
        cursor += 1;
        ctx = 0;
      }
      if (op == 32) {
        res.add(data[cursor]);
        cursor += 1;
        ctx += 1;
      }
    }
    for (var j = 0; j < ctx; j++) {
      pop(res);
    }
    minLine = cursor - ctx;
    offset = pos + 1 - h.oldStart;
  }
  for (var j = minLine; j < data.length; j++) {
    res.add(data[j]);
  }
  return string(join(res, "\n"));
}

JSAny? canonical(JSAny? v) {
  if (isArray(v)) {
    final JSAny? a = arr(size(v));
    for (var i = 0; i < size(v); i++) {
      assign(a, i, canonical(item(v, i)));
    }
    return a;
  }
  if (isObject(v)) {
    final JSAny? m = dictionary();
    final List<String> ks = names(v);
    sortNames(ks);
    for (var i = 0; i < ks.length; i++) {
      put(m, ks[i], canonical(get(v, ks[i])));
    }
    return m;
  }
  return v;
}

String doc(JSAny? v, String op) {
  if (op == "diffJson" && !isString(v)) {
    return pretty(canonical(v));
  }
  return text(v);
}

JSAny? writeChange(int kind, int n, JSAny? v) {
  final JSAny? x = obj();
  put(x, "count", numValue(n));
  put(x, "added", boolVal(kind == 2));
  put(x, "removed", boolVal(kind == 1));
  put(x, "value", v);
  return x;
}

List<String> arrayKeys(JSAny? v) {
  final List<String> keys = <String>[];
  for (var i = 0; i < size(v); i++) {
    keys.add(compact(item(v, i)));
  }
  return keys;
}

JSAny? execute(JSAny? input) {
  final String op = text(get(input, "op"));
  final JSAny? a = get(input, "before");
  final JSAny? b = get(input, "after");
  final JSAny? opt = get(input, "options");
  final JSAny? arg = get(input, "patch");
  if (op == "diffArrays") {
    final List<Run> spans = myers(
      arrayKeys(a),
      arrayKeys(b),
      flag(opt, "oneChangePerToken"),
    );
    final JSAny? all = arr(spans.length);
    int old = 0;
    int newer = 0;
    for (var i = 0; i < spans.length; i++) {
      final Run r = spans[i];
      final JSAny? v = arr(r.count);
      for (var j = 0; j < r.count; j++) {
        assign(v, j, (r.kind == 1 ? item(a, old + j) : item(b, newer + j)));
      }
      assign(all, i, writeChange(r.kind, r.count, v));
      old += (r.kind == 2 ? 0 : r.count);
      newer += (r.kind == 1 ? 0 : r.count);
    }
    return all;
  }
  if (op == "diffChars" ||
      op == "diffWords" ||
      op == "diffLines" ||
      op == "diffJson") {
    final List<Change> rows = changes(
      op,
      tokenize(op, doc(a, op), opt),
      tokenize(op, doc(b, op), opt),
      opt,
    );
    final JSAny? all = arr(rows.length);
    for (var i = 0; i < rows.length; i++) {
      final Change c = rows[i];
      assign(all, i, writeChange(c.kind, c.count, string(c.value)));
    }
    return all;
  }
  if (op == "structuredPatch" || op == "createTwoFilesPatch") {
    final JSAny? ctx = get(opt, "context");
    final Patch p = create(input, (defined(ctx) ? integer(ctx) : 4));
    return (op == "structuredPatch"
        ? writePatch(p, false)
        : string(formatPatch(p)));
  }
  if (op == "parsePatch") {
    final List<Patch> ps = parsePatch(text(arg));
    final JSAny? all = arr(ps.length);
    for (var i = 0; i < ps.length; i++) {
      assign(all, i, writePatch(ps[i], false));
    }
    return all;
  }
  if (op == "reversePatch") {
    if (isArray(arg)) {
      final int n = size(arg);
      final JSAny? all = arr(n);
      for (var i = 0; i < n; i++) {
        assign(all, n - i - 1, reversePatch(item(arg, i)));
      }
      return all;
    }
    return reversePatch(arg);
  }
  if (op == "applyPatch") {
    if (isString(arg)) {
      final List<Patch> ps = parsePatch(text(arg));
      ensure(ps.length == 1);
      return applyPatch(text(a), ps[0]);
    }
    if (isArray(arg)) {
      ensure(size(arg) == 1);
      return applyPatch(text(a), readPatch(item(arg, 0)));
    }
    return applyPatch(text(a), readPatch(arg));
  }
  ensure(false);
  return undefinedValue();
}

void main() {
  final exports = JSObject();
  exports['run'] = execute.toJS;
  globalContext['__matrixExports'] = exports;
}
