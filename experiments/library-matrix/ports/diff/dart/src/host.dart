// The host facilities the port calls: Object.keys, JSON.stringify, the undefined value and
// TextDecoder. jsdiff decodes a run of octal escapes with TextDecoder, which replaces each maximal
// ill-formed subpart with U+FFFD and consumes a leading BOM; the port calls it rather than
// assuming dart:convert's decoder does both.
//
// ECMAScript's `\s` and `trim` are written out here: Dart's String.trim also strips U+0085.

import 'dart:js_interop';

@JS('Object.keys')
external JSArray<JSString> objectKeys(JSObject object);

@JS('JSON.stringify')
external String jsonStringify(JSAny? value, JSAny? replacer, String space);

@JS('undefined')
external JSAny? get jsUndefined;

/// JavaScript truthiness: `Boolean(value)`.
@JS('Boolean')
external bool truthy(JSAny? value);

extension type TextDecoder._(JSObject _) implements JSObject {
  external TextDecoder();
  external String decode(JSUint8Array bytes);
}

/// The class `\s`: ECMAScript white space and line terminators, one UTF-16 unit at a time.
bool isJsWhitespace(int unit) =>
    (unit >= 0x09 && unit <= 0x0d) ||
    unit == 0x20 ||
    unit == 0xa0 ||
    unit == 0x1680 ||
    (unit >= 0x2000 && unit <= 0x200a) ||
    unit == 0x2028 ||
    unit == 0x2029 ||
    unit == 0x202f ||
    unit == 0x205f ||
    unit == 0x3000 ||
    unit == 0xfeff;

/// The whitespace units at the start of [s].
String leadingWs(String s) {
  var i = 0;
  while (i < s.length && isJsWhitespace(s.codeUnitAt(i))) {
    i++;
  }
  return s.substring(0, i);
}

/// The whitespace units at the end of [s].
String trailingWs(String s) {
  var i = s.length;
  while (i > 0 && isJsWhitespace(s.codeUnitAt(i - 1))) {
    i--;
  }
  return s.substring(i);
}

/// `String.prototype.trim`.
String jsTrim(String s) {
  final start = leadingWs(s).length;
  return start == s.length ? '' : s.substring(start, s.length - trailingWs(s).length);
}
