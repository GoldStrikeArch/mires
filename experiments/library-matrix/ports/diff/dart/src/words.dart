// jsdiff 9.0.0 diff/word.js and the whitespace helpers of util/string.js.

import 'diff.dart';
import 'host.dart';

// Latin letters with diacritics and the spacing modifier letters count as word characters.
const _extendedWordChars =
    r'a-zA-Z0-9_\u{AD}\u{C0}-\u{D6}\u{D8}-\u{F6}\u{F8}-\u{2C6}\u{2C8}-\u{2D7}\u{2DE}-\u{2FF}\u{1E00}-\u{1EFF}';

final class WordDiff extends TextDiff {
  // On the web a Dart RegExp is the host's; `unicode` is jsdiff's `u` flag.
  static final _tokenizeIncludingWhitespace =
      RegExp('[$_extendedWordChars]+|\\s+|[^$_extendedWordChars]', unicode: true);
  static final _whitespace = RegExp(r'\s');

  @override
  List<String> tokenize(String value, DiffOptions options) {
    // Whitespace runs are joined to the neighbouring word or punctuation tokens.
    final tokens = <String>[];
    String? prevPart;
    for (final match in _tokenizeIncludingWhitespace.allMatches(value)) {
      final part = match[0]!;
      if (_whitespace.hasMatch(part)) {
        tokens.add(prevPart == null ? part : tokens.removeLast() + part);
      } else if (prevPart != null && _whitespace.hasMatch(prevPart)) {
        tokens.add(tokens.last == prevPart ? tokens.removeLast() + part : prevPart + part);
      } else {
        tokens.add(part);
      }
      prevPart = part;
    }
    return tokens;
  }

  @override
  bool tokensEqual(String left, String right, DiffOptions options) => options.ignoreCase
      ? jsTrim(left.toLowerCase()) == jsTrim(right.toLowerCase())
      : jsTrim(left) == jsTrim(right);

  // Tokens that are joined appeared consecutively, so each but the first drops its leading whitespace.
  @override
  String join(List<String> tokens) =>
      [for (var i = 0; i < tokens.length; i++) i == 0 ? tokens[i] : tokens[i].substring(leadingWs(tokens[i]).length)].join();

  @override
  List<Change<String>> postProcess(List<Change<String>> changes, DiffOptions options) {
    if (options.oneChangePerToken) return changes;
    Change<String>? lastKeep;
    // The insertion and deletion since the last kept change; there is at most one of each.
    Change<String>? insertion;
    Change<String>? deletion;
    for (final change in changes) {
      if (change.added) {
        insertion = change;
      } else if (change.removed) {
        deletion = change;
      } else {
        if (insertion != null || deletion != null) _dedupeWhitespace(lastKeep, deletion, insertion, change);
        lastKeep = change;
        insertion = null;
        deletion = null;
      }
    }
    if (insertion != null || deletion != null) _dedupeWhitespace(lastKeep, deletion, insertion, null);
    return changes;
  }
}

/// dedupeWhitespaceInChangeObjects: whitespace that trails one change object and leads the next is
/// kept once, on the side where both texts have it.
void _dedupeWhitespace(
  Change<String>? startKeep,
  Change<String>? deletion,
  Change<String>? insertion,
  Change<String>? endKeep,
) {
  if (deletion != null && insertion != null) {
    final oldWsPrefix = leadingWs(deletion.value);
    final oldWsSuffix = trailingWs(deletion.value);
    final newWsPrefix = leadingWs(insertion.value);
    final newWsSuffix = trailingWs(insertion.value);
    if (startKeep != null) {
      final commonWsPrefix = _longestCommonPrefix(oldWsPrefix, newWsPrefix);
      startKeep.value = _replaceSuffix(startKeep.value, newWsPrefix, commonWsPrefix);
      deletion.value = _removePrefix(deletion.value, commonWsPrefix);
      insertion.value = _removePrefix(insertion.value, commonWsPrefix);
    }
    if (endKeep != null) {
      final commonWsSuffix = _longestCommonSuffix(oldWsSuffix, newWsSuffix);
      endKeep.value = _replacePrefix(endKeep.value, newWsSuffix, commonWsSuffix);
      deletion.value = _removeSuffix(deletion.value, commonWsSuffix);
      insertion.value = _removeSuffix(insertion.value, commonWsSuffix);
    }
  } else if (insertion != null) {
    // The whitespace is all from the new text: each object keeps its trailing whitespace.
    if (startKeep != null) insertion.value = insertion.value.substring(leadingWs(insertion.value).length);
    if (endKeep != null) endKeep.value = endKeep.value.substring(leadingWs(endKeep.value).length);
  } else if (deletion != null && startKeep != null && endKeep != null) {
    final newWsFull = leadingWs(endKeep.value);
    final delWsStart = leadingWs(deletion.value);
    final delWsEnd = trailingWs(deletion.value);
    // Whitespace right after startKeep in both texts goes to startKeep.
    final newWsStart = _longestCommonPrefix(newWsFull, delWsStart);
    deletion.value = _removePrefix(deletion.value, newWsStart);
    // Whitespace right before endKeep in both texts, not given to startKeep, goes to endKeep.
    final newWsEnd = _longestCommonSuffix(_removePrefix(newWsFull, newWsStart), delWsEnd);
    deletion.value = _removeSuffix(deletion.value, newWsEnd);
    endKeep.value = _replacePrefix(endKeep.value, newWsFull, newWsEnd);
    // Any whitespace from the new text not yet assigned goes to the start.
    startKeep.value = _replaceSuffix(startKeep.value, newWsFull, newWsFull.substring(0, newWsFull.length - newWsEnd.length));
  } else if (deletion != null && endKeep != null) {
    // At the start of the text: drop the deletion's trailing whitespace that endKeep repeats.
    deletion.value = _removeSuffix(deletion.value, _maximumOverlap(trailingWs(deletion.value), leadingWs(endKeep.value)));
  } else if (deletion != null && startKeep != null) {
    // At the end of the text: drop the deletion's leading whitespace that startKeep repeats.
    deletion.value = _removePrefix(deletion.value, _maximumOverlap(trailingWs(startKeep.value), leadingWs(deletion.value)));
  }
}

String _longestCommonPrefix(String a, String b) {
  var i = 0;
  while (i < a.length && i < b.length && a.codeUnitAt(i) == b.codeUnitAt(i)) {
    i++;
  }
  return a.substring(0, i);
}

String _longestCommonSuffix(String a, String b) {
  var i = 0;
  while (i < a.length && i < b.length && a.codeUnitAt(a.length - 1 - i) == b.codeUnitAt(b.length - 1 - i)) {
    i++;
  }
  return a.substring(a.length - i);
}

String _replacePrefix(String string, String oldPrefix, String newPrefix) {
  if (!string.startsWith(oldPrefix)) throw StateError("string doesn't start with prefix; this is a bug");
  return newPrefix + string.substring(oldPrefix.length);
}

String _replaceSuffix(String string, String oldSuffix, String newSuffix) {
  if (!string.endsWith(oldSuffix)) throw StateError("string doesn't end with suffix; this is a bug");
  return string.substring(0, string.length - oldSuffix.length) + newSuffix;
}

String _removePrefix(String string, String oldPrefix) => _replacePrefix(string, oldPrefix, '');

String _removeSuffix(String string, String oldSuffix) => _replaceSuffix(string, oldSuffix, '');

/// The longest suffix of [a] that is a prefix of [b], by Knuth-Morris-Pratt (overlapCount).
String _maximumOverlap(String a, String b) {
  final startA = a.length > b.length ? a.length - b.length : 0;
  final endB = a.length < b.length ? a.length : b.length;
  final map = List<int>.filled(endB, 0);
  var k = 0;
  for (var j = 1; j < endB; j++) {
    map[j] = b.codeUnitAt(j) == b.codeUnitAt(k) ? map[k] : k;
    while (k > 0 && b.codeUnitAt(j) != b.codeUnitAt(k)) {
      k = map[k];
    }
    if (b.codeUnitAt(j) == b.codeUnitAt(k)) k++;
  }
  k = 0;
  for (var i = startA; i < a.length; i++) {
    while (k > 0 && a.codeUnitAt(i) != b.codeUnitAt(k)) {
      k = map[k];
    }
    if (a.codeUnitAt(i) == b.codeUnitAt(k)) k++;
  }
  return b.substring(0, k);
}
