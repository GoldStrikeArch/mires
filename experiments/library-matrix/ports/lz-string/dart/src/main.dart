import 'dart:js_interop';
import 'dart:js_interop_unsafe';

extension type Input._(JSObject _) implements JSObject {
  external String get op;
  external String get format;
  external JSArray<JSNumber>? get data;
  external int? get repeat;
}

const base64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=';
const uri = r'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$';
int width(String format) => switch (format) {
  'utf16' => 15, 'base64' || 'uri' => 6, 'bytes' || 'raw' => 16,
  _ => throw ArgumentError('unknown format')
};

class CodeWidth {
  int bits, left;
  CodeWidth(this.bits, this.left);
  void advance() {
    if (--left == 0) { left = 1 << bits; bits++; }
  }
}

List<int> compress(String text, String format) {
  final dictionary = <String,int>{};
  final fresh = <String>{};
  final words = <int>[];
  final codes = CodeWidth(2, 2);
  final size = width(format);
  int word = 0, used = 0, next = 3;
  void write(int n, int value) {
    for (int i = 0; i < n; i++) {
      word = (word << 1) | (value & 1);
      value >>= 1;
      if (++used == size) { words.add(word); used = 0; word = 0; }
    }
  }
  void emit(String phrase) {
    if (fresh.remove(phrase)) {
      final c = phrase.codeUnitAt(0);
      write(codes.bits, c >= 256 ? 1 : 0);
      write(c >= 256 ? 16 : 8, c);
      codes.advance();
    } else { write(codes.bits, dictionary[phrase]!); }
    codes.advance();
  }
  var prefix = '';
  for (var i = 0; i < text.length; i++) {
    final c = text[i];
    if (!dictionary.containsKey(c)) { dictionary[c] = next++; fresh.add(c); }
    final combined = prefix + c;
    if (dictionary.containsKey(combined)) { prefix = combined; continue; }
    emit(prefix);
    if (combined != '__proto__') dictionary[combined] = next;
    next++;
    prefix = c;
  }
  if (prefix.isNotEmpty) emit(prefix);
  write(codes.bits, 2);
  write(size - used, 0);
  if (format == 'raw') return words;
  if (format == 'bytes') return [for (final w in words) ...[w >> 8, w & 255]];
  if (format == 'utf16') return [for (final w in words) w + 32, 32];
  final alphabet = format == 'base64' ? base64 : uri;
  final out = [for (final w in words) alphabet.codeUnitAt(w)];
  if (format == 'base64') { while (out.length % 4 != 0) { out.add(61); } }
  return out;
}

String? decompress(List<int> data, String format) {
  if (data.isEmpty) return null;
  final size = width(format);
  final alphabet = format == 'base64' ? base64 : uri;
  int getWord(int i) {
    final c = data[i];
    if (format == 'utf16') return c - 32;
    if (format == 'raw') return c;
    final n = alphabet.indexOf(String.fromCharCode(format == 'uri' && c == 32 ? 43 : c));
    return n < 0 ? 0 : n;
  }
  if (format == 'bytes' && data.length.isOdd) throw ArgumentError('odd byte count');
  final words = format == 'bytes' ? [for (int i=0; i<data.length; i+=2) data[i]*256 + data[i+1]]
      : List.generate(data.length, getWord);
  int index = 0, position = size;
  int read(int n) {
    var value = 0;
    for (int bit = 0; bit < n; bit++) {
      final word = index < words.length ? words[index] : 0;
      value |= ((word >> --position) & 1) << bit;
      if (position == 0) { position = size; index++; }
    }
    return value;
  }
  final header = read(2);
  if (header == 2) return '';
  String? previous = header == 3 ? null : String.fromCharCode(read(header == 0 ? 8 : 16));
  final dictionary = <String?>[null, null, null, previous];
  final out = StringBuffer(previous ?? '');
  final codes = CodeWidth(3, 4);
  while (index < words.length) {
    var code = read(codes.bits);
    if (code == 2) return out.toString();
    if (code <= 1) {
      final literal = String.fromCharCode(read(code == 0 ? 8 : 16));
      code = dictionary.length;
      dictionary.add(literal);
      codes.advance();
    }
    final String? entry;
    if (code < dictionary.length) { entry = dictionary[code]; }
    else if (code == dictionary.length) { entry = previous! + previous[0]; }
    else { return null; }
    if (entry == null) return null;
    out.write(entry);
    dictionary.add((previous ?? 'undefined') + entry[0]);
    previous = entry;
    codes.advance();
  }
  return '';
}

JSArray<JSNumber>? run(Input input) {
  JSArray<JSNumber> array(List<int> values) {
    final result = JSArray<JSNumber>.withLength(values.length);
    for (var i = 0; i < values.length; i++) result[i] = values[i].toJS;
    return result;
  }
  final value = input.data;
  if (value == null) return array([]);
  final data = [for (final n in value.toDart) n.toDartInt];
  if (input.op == 'decompress') {
    final text = decompress(data, input.format);
    return text == null ? null : array(text.codeUnits);
  }
  final text = String.fromCharCodes(data) * (input.repeat ?? 1);
  final packed = compress(text, input.format);
  if (input.op == 'compress') return array(packed);
  if (input.op != 'roundtrip' || decompress(packed, input.format) != text) throw StateError('roundtrip');
  return array([packed.length, text.length]);
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
