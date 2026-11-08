library;

import 'dart:js_interop';

import 'host.dart';

bool includes(JSAny? haystack, JSAny? needle) {
  final length = jsLength(haystack);
  for (var i = 0; i < length; i++) {
    final item = jsIndex(haystack, i);
    if (identical(item, needle) || item == needle) {
      if (typof(item) == typof(needle) && jsString(item) == jsString(needle)) {
        // Strict-ish equality: prefer JS ===
      }
    }
  }
  return _jsIncludes(haystack, needle);
}

bool _jsIncludes(JSAny? haystack, JSAny? needle) {
  final length = jsLength(haystack);
  for (var i = 0; i < length; i++) {
    if (_strictEquals(jsIndex(haystack, i), needle)) return true;
  }
  return false;
}

bool _strictEquals(JSAny? a, JSAny? b) {
  if (a.isUndefined && b.isUndefined) return true;
  if (a.isNull && b.isNull) return true;
  if (a.isUndefinedOrNull || b.isUndefinedOrNull) return false;
  if (a is JSString && b is JSString) return a.toDart == b.toDart;
  if (a is JSNumber && b is JSNumber) return a.toDartDouble == b.toDartDouble;
  return a == b;
}

bool excludes(JSAny? haystack, JSAny? needle) => !includes(haystack, needle);

JSAny? toInt(JSAny? value) => fromDart(jsParseInt(value));

JSAny? toFloat(JSAny? value) => fromDart(jsParseFloat(value));

JSAny? replaceNaN(JSAny? value, JSAny? replacement) {
  if (jsIsNaN(value)) return replacement;
  return value;
}

int countTruthy(Iterable<bool> values) {
  var sum = 0;
  for (final value in values) {
    if (value) sum++;
  }
  return sum;
}

int countTruthyJs(JSAny? values) {
  final length = jsLength(values);
  var sum = 0;
  for (var i = 0; i < length; i++) {
    if (jsTruthy(jsIndex(values, i))) sum++;
  }
  return sum;
}

double roundTo(double value, [double base = 1]) {
  if (mathAbs(base) >= 1) {
    return mathRound(value / base) * base;
  }
  final counterBase = 1 / base;
  return mathRound(value * counterBase) / counterBase;
}

(String?, Map<String, List<String>>) parseSimpleCssSelector(String selector) {
  const errorMessage = "Unexpected syntax";
  final tagMatch = RegExp(r'^\s*([a-z-]*)(.*)$', caseSensitive: false)
      .firstMatch(selector);
  if (tagMatch == null) {
    throw StateError("$errorMessage '$selector'");
  }
  final tag = tagMatch.group(1);
  final attributes = <String, List<String>>{};
  final partsRegex = RegExp(r'([.:#][\w-]+|\[.+?\])', caseSensitive: false);
  void addAttribute(String name, String value) {
    attributes.putIfAbsent(name, () => []).add(value);
  }

  for (final match in partsRegex.allMatches(tagMatch.group(2) ?? '')) {
    final part = match.group(0)!;
    switch (part[0]) {
      case '.':
        addAttribute('class', part.substring(1));
      case '#':
        addAttribute('id', part.substring(1));
      case '[':
        final attributeMatch = RegExp(
          r'^\[([\w-]+)([~|^$*]?=("(.*?)"|([\w-]+)))?(\s+[is])?\]$',
        ).firstMatch(part);
        if (attributeMatch != null) {
          addAttribute(
            attributeMatch.group(1)!,
            attributeMatch.group(4) ?? attributeMatch.group(5) ?? '',
          );
        } else {
          throw StateError("$errorMessage '$selector'");
        }
      default:
        throw StateError("$errorMessage '$selector'");
    }
  }
  return (tag == null || tag.isEmpty ? null : tag, attributes);
}

String escapeComponentKey(String key) {
  return key.replaceAllMapped(RegExp(r'([:|\\])'), (match) => '\\${match[1]}');
}
