// jsdiff 9.0.0 diff/base.js with the character, line, JSON and array diffs.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math';

import 'host.dart';

/// The synchronous options of the contract; patch creation passes none of them.
final class DiffOptions {
  const DiffOptions({
    this.ignoreCase = false,
    this.oneChangePerToken = false,
    this.stripTrailingCr = false,
    this.ignoreWhitespace = false,
    this.newlineIsToken = false,
    this.ignoreNewlineAtEof = false,
  });

  final bool ignoreCase;
  final bool oneChangePerToken;
  final bool stripTrailingCr;
  final bool ignoreWhitespace;
  final bool newlineIsToken;
  final bool ignoreNewlineAtEof;
}

/// A change object: [count] tokens kept, added or removed, and their joined [value].
final class Change<V> {
  Change(this.count, this.added, this.removed, this.value);

  final int count;
  final bool added;
  final bool removed;
  V value;
}

/// An edit-history node; each path shares its predecessors with its neighbours.
final class _Component {
  _Component(this.count, this.added, this.removed, this.previous);

  final int count;
  final bool added;
  final bool removed;
  final _Component? previous;
}

final class _Path {
  _Path(this.oldPos, this.lastComponent);

  int oldPos;
  _Component? lastComponent;
}

/// base.js: Myers' O(ND) search over tokens of type [T], joined into values of type [V].
abstract class Diff<T, V> {
  bool get useLongestToken => false;

  List<T> removeEmpty(List<T> tokens) => tokens;

  bool tokensEqual(T left, T right, DiffOptions options) => left == right;

  V join(List<T> tokens);

  List<Change<V>> postProcess(List<Change<V>> changes, DiffOptions options) => changes;

  /// The longer of an old and a new token that compare equal (diffJson's dangling commas).
  T longest(T oldToken, T newToken) => newToken;

  List<Change<V>> diffTokens(List<T> oldTokens, List<T> newTokens, DiffOptions options) {
    final oldNonEmpty = removeEmpty(oldTokens);
    final newNonEmpty = removeEmpty(newTokens);
    return postProcess(_buildValues(_search(oldNonEmpty, newNonEmpty, options), newNonEmpty, oldNonEmpty), options);
  }

  _Component? _search(List<T> oldTokens, List<T> newTokens, DiffOptions options) {
    final newLen = newTokens.length;
    final oldLen = oldTokens.length;
    final maxEditLength = newLen + oldLen;
    // The best path on each diagonal, which runs from -maxEditLength - 1 to maxEditLength + 1.
    final bestPath = List<_Path?>.filled(2 * maxEditLength + 3, null);
    int at(int diagonal) => diagonal + maxEditLength + 1;
    final origin = _Path(-1, null);
    bestPath[at(0)] = origin;
    var newPos = _extractCommon(origin, newTokens, oldTokens, 0, options);
    if (origin.oldPos + 1 >= oldLen && newPos + 1 >= newLen) return origin.lastComponent;
    // Once a path reaches the right or bottom edge, the diagonals beyond it are pointless. No
    // diagonal lies outside [-maxEditLength, maxEditLength], so those bounds limit nothing.
    var minDiagonalToConsider = -maxEditLength;
    var maxDiagonalToConsider = maxEditLength;
    for (var editLength = 1; editLength <= maxEditLength; editLength++) {
      var diagonalPath = max(minDiagonalToConsider, -editLength);
      while (diagonalPath <= min(maxDiagonalToConsider, editLength)) {
        final removePath = bestPath[at(diagonalPath - 1)];
        final addPath = bestPath[at(diagonalPath + 1)];
        // No one else is going to use this value.
        if (removePath != null) bestPath[at(diagonalPath - 1)] = null;
        // The new position after an insertion must lie inside the new tokens.
        final canAdd = addPath != null && addPath.oldPos - diagonalPath >= 0 && addPath.oldPos - diagonalPath < newLen;
        final canRemove = removePath != null && removePath.oldPos + 1 < oldLen;
        if (!canAdd && !canRemove) {
          bestPath[at(diagonalPath)] = null;
          diagonalPath += 2;
          continue;
        }
        // Branch from the path that is furthest along the old tokens (one that cannot remove can add).
        final basePath = !canRemove || (canAdd && removePath.oldPos < addPath.oldPos)
            ? _addToPath(addPath!, true, false, 0, options)
            : _addToPath(removePath, false, true, 1, options);
        newPos = _extractCommon(basePath, newTokens, oldTokens, diagonalPath, options);
        if (basePath.oldPos + 1 >= oldLen && newPos + 1 >= newLen) return basePath.lastComponent;
        bestPath[at(diagonalPath)] = basePath;
        if (basePath.oldPos + 1 >= oldLen) maxDiagonalToConsider = min(maxDiagonalToConsider, diagonalPath - 1);
        if (newPos + 1 >= newLen) minDiagonalToConsider = max(minDiagonalToConsider, diagonalPath + 1);
        diagonalPath += 2;
      }
    }
    throw StateError('the edit graph has no path');
  }

  _Path _addToPath(_Path path, bool added, bool removed, int oldPosInc, DiffOptions options) {
    final last = path.lastComponent;
    final component = last != null && !options.oneChangePerToken && last.added == added && last.removed == removed
        ? _Component(last.count + 1, added, removed, last.previous)
        : _Component(1, added, removed, last);
    return _Path(path.oldPos + oldPosInc, component);
  }

  int _extractCommon(_Path basePath, List<T> newTokens, List<T> oldTokens, int diagonalPath, DiffOptions options) {
    var oldPos = basePath.oldPos;
    var newPos = oldPos - diagonalPath;
    var commonCount = 0;
    while (newPos + 1 < newTokens.length &&
        oldPos + 1 < oldTokens.length &&
        tokensEqual(oldTokens[oldPos + 1], newTokens[newPos + 1], options)) {
      newPos++;
      oldPos++;
      commonCount++;
      if (options.oneChangePerToken) {
        basePath.lastComponent = _Component(1, false, false, basePath.lastComponent);
      }
    }
    if (commonCount > 0 && !options.oneChangePerToken) {
      basePath.lastComponent = _Component(commonCount, false, false, basePath.lastComponent);
    }
    basePath.oldPos = oldPos;
    return newPos;
  }

  List<Change<V>> _buildValues(_Component? lastComponent, List<T> newTokens, List<T> oldTokens) {
    final components = <_Component>[];
    for (var component = lastComponent; component != null; component = component.previous) {
      components.add(component);
    }
    var newPos = 0;
    var oldPos = 0;
    final changes = <Change<V>>[];
    for (final component in components.reversed) {
      final List<T> tokens;
      if (!component.removed) {
        final kept = newTokens.sublist(newPos, newPos + component.count);
        tokens = !component.added && useLongestToken
            ? [for (var i = 0; i < kept.length; i++) longest(oldTokens[oldPos + i], kept[i])]
            : kept;
        newPos += component.count;
        if (!component.added) oldPos += component.count;
      } else {
        tokens = oldTokens.sublist(oldPos, oldPos + component.count);
        oldPos += component.count;
      }
      changes.add(Change(component.count, component.added, component.removed, join(tokens)));
    }
    return changes;
  }
}

/// A diff over string tokens, which are dropped when empty and compared case-insensitively on request.
abstract class TextDiff extends Diff<String, String> {
  String castInput(JSAny? value) => (value as JSString).toDart;

  List<String> tokenize(String value, DiffOptions options);

  List<Change<String>> diff(JSAny? oldValue, JSAny? newValue, DiffOptions options) =>
      diffTokens(tokenize(castInput(oldValue), options), tokenize(castInput(newValue), options), options);

  @override
  List<String> removeEmpty(List<String> tokens) => [for (final token in tokens) if (token.isNotEmpty) token];

  // dart2js compiles toLowerCase to the host's.
  @override
  bool tokensEqual(String left, String right, DiffOptions options) =>
      left == right || (options.ignoreCase && left.toLowerCase() == right.toLowerCase());

  @override
  String join(List<String> tokens) => tokens.join();
}

/// character.js: tokens are code points (`Array.from`), so a lone surrogate is one token.
final class CharacterDiff extends TextDiff {
  @override
  List<String> tokenize(String value, DiffOptions options) => [for (final rune in value.runes) String.fromCharCode(rune)];
}

/// line.js `tokenize`: lines with their terminators, or the terminators as separate tokens.
List<String> lineTokens(String value, DiffOptions options) {
  final text = options.stripTrailingCr ? value.replaceAll('\r\n', '\n') : value;
  // text.split(/(\n|\r\n)/): each \n is a separator, together with a \r before it.
  final linesAndNewlines = <String>[];
  var start = 0;
  for (var i = 0; i < text.length; i++) {
    if (text.codeUnitAt(i) != 0x0a) continue;
    final separator = i > start && text.codeUnitAt(i - 1) == 0x0d ? i - 1 : i;
    linesAndNewlines
      ..add(text.substring(start, separator))
      ..add(text.substring(separator, i + 1));
    start = i + 1;
  }
  linesAndNewlines.add(text.substring(start));
  // Ignore the final empty token that occurs if the string ends with a new line.
  if (linesAndNewlines.last.isEmpty) linesAndNewlines.removeLast();
  final retLines = <String>[];
  for (var i = 0; i < linesAndNewlines.length; i++) {
    if (i.isOdd && !options.newlineIsToken) {
      retLines.last += linesAndNewlines[i];
    } else {
      retLines.add(linesAndNewlines[i]);
    }
  }
  return retLines;
}

final class LineDiff extends TextDiff {
  @override
  List<String> tokenize(String value, DiffOptions options) => lineTokens(value, options);

  @override
  bool tokensEqual(String left, String right, DiffOptions options) {
    // With newlineIsToken, a newline token is not trimmed down to an empty line.
    String normalised(String token) {
      if (options.ignoreWhitespace) {
        return !options.newlineIsToken || !token.contains('\n') ? jsTrim(token) : token;
      }
      if (options.ignoreNewlineAtEof && !options.newlineIsToken && token.endsWith('\n')) {
        return token.substring(0, token.length - 1);
      }
      return token;
    }

    return super.tokensEqual(normalised(left), normalised(right), options);
  }
}

/// json.js: a pretty-printed canonical document, diffed by lines.
final class JsonDiff extends TextDiff {
  static final _danglingComma = RegExp(',([\r\n])');

  @override
  bool get useLongestToken => true;

  @override
  String castInput(JSAny? value) =>
      value.typeofEquals('string') ? (value as JSString).toDart : jsonStringify(canonicalize(value), null, '  ');

  @override
  List<String> tokenize(String value, DiffOptions options) => lineTokens(value, options);

  String _withoutDanglingComma(String token) => token.replaceAllMapped(_danglingComma, (match) => match[1]!);

  @override
  bool tokensEqual(String left, String right, DiffOptions options) =>
      super.tokensEqual(_withoutDanglingComma(left), _withoutDanglingComma(right), options);

  @override
  String longest(String oldToken, String newToken) => oldToken.length > newToken.length ? oldToken : newToken;
}

/// `canonicalize` for JSON documents: arrays in order, object keys sorted and assigned to a plain
/// object, so a "__proto__" key sets the prototype and is not serialized. An object with a truthy
/// `toJSON` member has it called, which throws, since JSON cannot make it callable. JSON input has
/// no cycles, and the default replacer keeps every value.
JSAny? canonicalize(JSAny? value) {
  if (value.isA<JSArray>()) {
    return [for (final element in (value as JSArray<JSAny?>).toDart) canonicalize(element)].toJS;
  }
  if (value != null && value.typeofEquals('object')) {
    final object = value as JSObject;
    if (truthy(object['toJSON'])) throw ArgumentError('obj.toJSON is not a function');
    final keys = [for (final key in objectKeys(object).toDart) key.toDart]..sort();
    final canonical = JSObject();
    for (final key in keys) {
      canonical[key] = canonicalize(object[key]);
    }
    return canonical;
  }
  return value;
}

/// array.js: tokens are the JSON values themselves, compared with `===`, and with ignoreCase also by
/// `toLowerCase()`, which the contract applies to strings only.
final class ArrayDiff extends Diff<JSAny?, JSArray<JSAny?>> {
  @override
  bool tokensEqual(JSAny? left, JSAny? right, DiffOptions options) =>
      left.strictEquals(right).toDart ||
      (options.ignoreCase && (left as JSString).toDart.toLowerCase() == (right as JSString).toDart.toLowerCase());

  @override
  JSArray<JSAny?> join(List<JSAny?> tokens) => tokens.toJS;
}
