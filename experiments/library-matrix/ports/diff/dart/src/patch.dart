// jsdiff 9.0.0 patch/create.js, parse.js, apply.js, reverse.js and line-endings.js.

import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'diff.dart';
import 'host.dart';

/// A hunk. Its ranges are JavaScript numbers: a parsed header can exceed int32 or be NaN.
final class Hunk {
  Hunk(this.oldStart, this.oldLines, this.newStart, this.newLines, [List<String>? lines]) : lines = lines ?? [];

  double oldStart;
  double oldLines;
  double newStart;
  double newLines;
  final List<String> lines;

  JSObject toJSObject() => JSObject()
    ..['oldStart'] = oldStart.toJS
    ..['oldLines'] = oldLines.toJS
    ..['newStart'] = newStart.toJS
    ..['newLines'] = newLines.toJS
    ..['lines'] = [for (final line in lines) line.toJS].toJS;
}

/// A file patch. [properties] holds its metadata in the order jsdiff first assigns each property,
/// which is the key order of the object it returns; `hunks` stands for [hunks].
final class FilePatch {
  FilePatch([List<Hunk>? hunks]) : hunks = hunks ?? [];

  final List<Hunk> hunks;
  final properties = <String, JSAny?>{};

  JSObject toJSObject() {
    final out = JSObject();
    properties.forEach((key, value) {
      out[key] = key == 'hunks' ? [for (final hunk in hunks) hunk.toJSObject()].toJS : value;
    });
    return out;
  }
}

// ---- structuredPatch and formatPatch ----

FilePatch structuredPatch(
  JSAny? oldFileName,
  JSAny? newFileName,
  String oldStr,
  String newStr,
  JSAny? oldHeader,
  JSAny? newHeader,
  int context,
) {
  // Step 1: hunks of lines that keep their newlines; an empty entry at the end closes the last one.
  final diff = [
    for (final change in LineDiff().diff(oldStr.toJS, newStr.toJS, const DiffOptions()))
      (added: change.added, removed: change.removed, lines: _splitLines(change.value)),
    (added: false, removed: false, lines: <String>[]),
  ];
  final hunks = <Hunk>[];
  var oldRangeStart = 0;
  var newRangeStart = 0;
  var curRange = <String>[];
  var oldLine = 1;
  var newLine = 1;
  for (var i = 0; i < diff.length; i++) {
    final current = diff[i];
    final lines = current.lines;
    if (current.added || current.removed) {
      // If we have previous context, start with that.
      if (oldRangeStart == 0) {
        oldRangeStart = oldLine;
        newRangeStart = newLine;
        if (i > 0) {
          final previous = diff[i - 1].lines;
          curRange = context > 0
              ? [for (final line in previous.sublist(previous.length > context ? previous.length - context : 0)) ' $line']
              : [];
          oldRangeStart -= curRange.length;
          newRangeStart -= curRange.length;
        }
      }
      curRange.addAll([for (final line in lines) (current.added ? '+' : '-') + line]);
      if (current.added) {
        newLine += lines.length;
      } else {
        oldLine += lines.length;
      }
    } else {
      if (oldRangeStart != 0) {
        if (lines.length <= context * 2 && i < diff.length - 2) {
          // Overlapping: the context joins the next change.
          curRange.addAll([for (final line in lines) ' $line']);
        } else {
          final contextSize = lines.length < context ? lines.length : context;
          curRange.addAll([for (final line in lines.take(contextSize)) ' $line']);
          hunks.add(Hunk(
            oldRangeStart.toDouble(),
            (oldLine - oldRangeStart + contextSize).toDouble(),
            newRangeStart.toDouble(),
            (newLine - newRangeStart + contextSize).toDouble(),
            curRange,
          ));
          oldRangeStart = 0;
          newRangeStart = 0;
          curRange = [];
        }
      }
      oldLine += lines.length;
      newLine += lines.length;
    }
  }
  // Step 2: drop each line's newline, or mark that it has none.
  for (final hunk in hunks) {
    for (var i = 0; i < hunk.lines.length; i++) {
      if (hunk.lines[i].endsWith('\n')) {
        hunk.lines[i] = hunk.lines[i].substring(0, hunk.lines[i].length - 1);
      } else {
        hunk.lines.insert(i + 1, r'\ No newline at end of file');
        i++;
      }
    }
  }
  return FilePatch(hunks)
    ..properties['oldFileName'] = oldFileName
    ..properties['newFileName'] = newFileName
    ..properties['oldHeader'] = oldHeader
    ..properties['newHeader'] = newHeader
    ..properties['hunks'] = null;
}

/// The lines of [text], each with its newline where it has one.
List<String> _splitLines(String text) {
  final result = [for (final line in text.split('\n')) '$line\n'];
  if (text.endsWith('\n')) {
    result.removeLast();
  } else {
    result.last = result.last.substring(0, result.last.length - 1);
  }
  return result;
}

/// C-style quoting of a name, over its UTF-8 bytes, as Git and GNU diffutils write it. utf8 encodes
/// an isolated surrogate as U+FFFD, as TextEncoder does.
String _quoteFileNameIfNeeded(String s) {
  if (!s.codeUnits.any((c) => c < 0x20 || c > 0x7e || c == 0x22 || c == 0x5c)) return s;
  final result = StringBuffer('"');
  for (final b in utf8.encode(s)) {
    result.write(switch (b) {
      0x07 => r'\a',
      0x08 => r'\b',
      0x09 => r'\t',
      0x0a => r'\n',
      0x0b => r'\v',
      0x0c => r'\f',
      0x0d => r'\r',
      0x22 => r'\"',
      0x5c => r'\\',
      >= 0x20 && <= 0x7e => String.fromCharCode(b),
      _ => '\\${b.toRadixString(8).padLeft(3, '0')}',
    });
  }
  result.write('"');
  return result.toString();
}

/// formatPatch with every header, for a patch from [structuredPatch].
String formatPatch(FilePatch patch) {
  final ret = <String>[];
  final oldFileName = patch.properties['oldFileName'];
  final newFileName = patch.properties['newFileName'];
  final oldName = oldFileName.isA<JSString>() ? (oldFileName as JSString).toDart : null;
  final newName = newFileName.isA<JSString>() ? (newFileName as JSString).toDart : null;
  if (oldName != null && oldName == newName) ret.add('Index: $oldName');
  ret.add(_underline);
  String header(JSAny? value) => value.isA<JSString>() && (value as JSString).toDart.isNotEmpty ? '\t${value.toDart}' : '';
  if (oldName != null && newName != null) {
    ret
      ..add('--- ${_quoteFileNameIfNeeded(oldName)}${header(patch.properties['oldHeader'])}')
      ..add('+++ ${_quoteFileNameIfNeeded(newName)}${header(patch.properties['newHeader'])}');
  }
  for (final hunk in patch.hunks) {
    // The start printed for an empty range is one lower.
    final oldStart = hunk.oldLines == 0 ? hunk.oldStart - 1 : hunk.oldStart;
    final newStart = hunk.newLines == 0 ? hunk.newStart - 1 : hunk.newStart;
    ret
      ..add('@@ -${oldStart.toInt()},${hunk.oldLines.toInt()} +${newStart.toInt()},${hunk.newLines.toInt()} @@')
      ..addAll(hunk.lines);
  }
  return '${ret.join('\n')}\n';
}

// ---- parsePatch ----

const _underline = '===================================================================';

// parse.js's line patterns; on the web a Dart RegExp is the host's.
final _gitDiffHeader = RegExp(r'^diff --git ');
final _indexHeader = RegExp(r'^Index:\s');
final _hgHeader = RegExp(r'^diff(?: -r \w+)+\s');
final _headerPrefix = RegExp(r'^(?:Index:|diff(?: -r \w+)+)\s+');
final _fileHeader = RegExp(r'^(---|\+\+\+)\s');
final _fileHeaderPrefix = RegExp(r'^(---|\+\+\+)\s+');
final _hunkHeader = RegExp(r'^@@\s');
final _chunkHeader = RegExp(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@');
final _hunkBodyLine = RegExp(r'^[+ -]');
final _renameFrom = RegExp(r'^rename from (.*)');
final _renameTo = RegExp(r'^rename to (.*)');
final _copyFrom = RegExp(r'^copy from (.*)');
final _copyTo = RegExp(r'^copy to (.*)');
final _newFileMode = RegExp(r'^new file mode (\d+)');
final _deletedFileMode = RegExp(r'^deleted file mode (\d+)');
final _oldMode = RegExp(r'^old mode (\d+)');
final _newMode = RegExp(r'^new mode (\d+)');
final _binaryFiles = RegExp(r'^Binary files ');

bool _isGitDiffHeader(String line) => _gitDiffHeader.hasMatch(line);

/// A line that starts the next file: `diff --git`, `Index:` or Mercurial's `diff -r`.
bool _isDiffHeader(String line) => _isGitDiffHeader(line) || _indexHeader.hasMatch(line) || _hgHeader.hasMatch(line);

bool _isFileHeader(String line) => _fileHeader.hasMatch(line);

bool _isHunkHeader(String line) => _hunkHeader.hasMatch(line);

bool _isOctal(String s, int i) => i < s.length && s.codeUnitAt(i) >= 0x30 && s.codeUnitAt(i) <= 0x37;

/// A C-style quoted name as Git and GNU diff write it: the unescaped name and the length of the
/// quoted text, or null for an unterminated quote or an unknown escape.
({String fileName, int rawLength})? _parseQuotedFileName(String s) {
  if (!s.startsWith('"')) return null;
  final result = StringBuffer();
  var j = 1;
  while (j < s.length) {
    if (s[j] == '"') return (fileName: result.toString(), rawLength: j + 1);
    if (s[j] == r'\' && j + 1 < s.length) {
      j++;
      switch (s[j]) {
        case 'a':
          result.write('\x07');
        case 'b':
          result.write('\b');
        case 'f':
          result.write('\f');
        case 'n':
          result.write('\n');
        case 'r':
          result.write('\r');
        case 't':
          result.write('\t');
        case 'v':
          result.write('\v');
        case r'\' || '"':
          result.write(s[j]);
        case '0' || '1' || '2' || '3' || '4' || '5' || '6' || '7':
          // Octal escapes are raw bytes: a run of them is decoded together as UTF-8.
          if (!_isOctal(s, j + 1) || !_isOctal(s, j + 2)) return null;
          final bytes = [int.parse(s.substring(j, j + 3), radix: 8)];
          j += 3;
          while (j + 1 < s.length && s[j] == r'\' && _isOctal(s, j + 1)) {
            if (!_isOctal(s, j + 2) || !_isOctal(s, j + 3)) return null;
            bytes.add(int.parse(s.substring(j + 1, j + 4), radix: 8));
            j += 4;
          }
          // Uint8List keeps the low byte of each, as Uint8Array does.
          result.write(TextDecoder().decode(Uint8List.fromList(bytes).toJS));
          continue;
        default:
          return null;
      }
    } else {
      result.write(s[j]);
    }
    j++;
  }
  return null;
}

String _unquoteIfQuoted(String s) => s.startsWith('"') ? _parseQuotedFileName(s)?.fileName ?? s : s;

/// The names in a `diff --git` line, or null when they cannot be told apart.
(String, String)? _parseGitDiffHeader(String line) {
  final rest = line.substring('diff --git '.length);
  if (rest.startsWith('"')) {
    final oldPath = _parseQuotedFileName(rest);
    if (oldPath == null) return null;
    // Skip the one unit after the quote, assumed to be a space.
    final afterOld = rest.substring(oldPath.rawLength + 1 < rest.length ? oldPath.rawLength + 1 : rest.length);
    if (!afterOld.startsWith('"')) return (oldPath.fileName, afterOld);
    final newPath = _parseQuotedFileName(afterOld);
    return newPath == null ? null : (oldPath.fileName, newPath.fileName);
  }
  final quoteIdx = rest.indexOf('"');
  if (quoteIdx > 0) {
    final newPath = _parseQuotedFileName(rest.substring(quoteIdx));
    return newPath == null ? null : (rest.substring(0, quoteIdx - 1), newPath.fileName);
  }
  if (rest.startsWith('a/')) {
    // Split on the middle " b/", which is right whenever both names are the same.
    final splits = <int>[];
    for (var idx = rest.indexOf(' b/', 1); idx != -1; idx = rest.indexOf(' b/', idx + 1)) {
      splits.add(idx);
    }
    if (splits.isNotEmpty) {
      final mid = splits[splits.length ~/ 2];
      return (rest.substring(0, mid), rest.substring(mid + 1));
    }
  }
  return null;
}

/// parsePatch: the unified diff text as file patches.
List<FilePatch> parsePatch(String uniDiff) => _PatchParser(uniDiff.split('\n')).parse();

final class _PatchParser {
  _PatchParser(this._diffstr);

  final List<String> _diffstr;
  var _i = 0;
  final _list = <FilePatch>[];

  List<FilePatch> parse() {
    while (_i < _diffstr.length) {
      _parseIndex();
    }
    return _list;
  }

  void _parseIndex() {
    final index = FilePatch();
    index.properties['hunks'] = null;
    _list.add(index);
    // Diff metadata, up to the file or hunk headers.
    var seenDiffHeader = false;
    while (_i < _diffstr.length) {
      final line = _diffstr[_i];
      if (_isFileHeader(line) || _isHunkHeader(line)) break;
      if (_isGitDiffHeader(line)) {
        // A second diff header belongs to the next file.
        if (seenDiffHeader) return;
        seenDiffHeader = true;
        index.properties['isGit'] = true.toJS;
        if (_parseGitDiffHeader(line) case (final oldName, final newName)) {
          index.properties['oldFileName'] = oldName.toJS;
          index.properties['newFileName'] = newName.toJS;
        }
        // Git extended headers.
        for (_i++; _i < _diffstr.length; _i++) {
          final extLine = _diffstr[_i];
          if (_isFileHeader(extLine) || _isHunkHeader(extLine) || _isDiffHeader(extLine)) break;
          _extendedHeader(index, extLine);
        }
        continue;
      } else if (_isDiffHeader(line)) {
        if (seenDiffHeader) return;
        seenDiffHeader = true;
        // The index is the rest of an `Index:` or Mercurial header, trimmed.
        if (_headerPrefix.firstMatch(line) case final headerMatch?) {
          index.properties['index'] = jsTrim(line.substring(headerMatch[0]!.length)).toJS;
        }
      }
      _i++;
    }
    _parseFileHeader(index);
    _parseFileHeader(index);
    // One file header without the other is a malformed patch.
    final oldFileName = index.properties['oldFileName'];
    final newFileName = index.properties['newFileName'];
    if ((oldFileName == null) != (newFileName == null)) {
      throw FormatException('Missing ${oldFileName != null ? '"+++ ..."' : '"--- ..."'} file header');
    }
    while (_i < _diffstr.length) {
      final line = _diffstr[_i];
      if (_isDiffHeader(line) || _isFileHeader(line) || line.startsWith(_underline)) {
        break;
      } else if (_isHunkHeader(line)) {
        index.hunks.add(_parseHunk());
      } else {
        // Blank lines and other content between or after hunks are skipped.
        _i++;
      }
    }
  }

  /// Git extended headers: each pattern is tried, and at most one matches.
  void _extendedHeader(FilePatch index, String extLine) {
    final properties = index.properties;
    if (_renameFrom.firstMatch(extLine) case final match?) {
      properties['oldFileName'] = 'a/${_unquoteIfQuoted(match[1]!)}'.toJS;
      properties['isRename'] = true.toJS;
    }
    if (_renameTo.firstMatch(extLine) case final match?) {
      properties['newFileName'] = 'b/${_unquoteIfQuoted(match[1]!)}'.toJS;
      properties['isRename'] = true.toJS;
    }
    if (_copyFrom.firstMatch(extLine) case final match?) {
      properties['oldFileName'] = 'a/${_unquoteIfQuoted(match[1]!)}'.toJS;
      properties['isCopy'] = true.toJS;
    }
    if (_copyTo.firstMatch(extLine) case final match?) {
      properties['newFileName'] = 'b/${_unquoteIfQuoted(match[1]!)}'.toJS;
      properties['isCopy'] = true.toJS;
    }
    if (_newFileMode.firstMatch(extLine) case final match?) {
      properties['isCreate'] = true.toJS;
      properties['newMode'] = match[1]!.toJS;
    }
    if (_deletedFileMode.firstMatch(extLine) case final match?) {
      properties['isDelete'] = true.toJS;
      properties['oldMode'] = match[1]!.toJS;
    }
    if (_oldMode.firstMatch(extLine) case final match?) properties['oldMode'] = match[1]!.toJS;
    if (_newMode.firstMatch(extLine) case final match?) properties['newMode'] = match[1]!.toJS;
    if (_binaryFiles.hasMatch(extLine)) properties['isBinary'] = true.toJS;
  }

  /// The `---` or `+++` line at the cursor, if there is one.
  void _parseFileHeader(FilePatch index) {
    if (_i >= _diffstr.length) return;
    final line = _diffstr[_i];
    final fileHeaderMatch = _fileHeaderPrefix.firstMatch(line);
    if (fileHeaderMatch == null) return;
    final data = jsTrim(line.substring(3)).split('\t');
    final header = jsTrim(data.length > 1 ? data[1] : '');
    final fileName = data[0].startsWith('"') ? _unquoteIfQuoted(data[0]) : data[0].replaceAll(r'\\', r'\');
    final side = fileHeaderMatch[1] == '---' ? 'old' : 'new';
    index.properties['${side}FileName'] = fileName.toJS;
    index.properties['${side}Header'] = header.toJS;
    _i++;
  }

  /// The hunk at the cursor. A header the range pattern does not match gives NaN starts.
  Hunk _parseHunk() {
    final chunkHeaderIndex = _i;
    final range = _chunkHeader.firstMatch(_diffstr[_i++]);
    double? number(int group) => range?[group] == null ? null : double.parse(range![group]!);
    final hunk = Hunk(number(1) ?? double.nan, number(2) ?? 1.0, number(3) ?? double.nan, number(4) ?? 1.0);
    // Unified diff quirk: the start of an empty range is one lower.
    if (hunk.oldLines == 0) hunk.oldStart += 1;
    if (hunk.newLines == 0) hunk.newStart += 1;
    var addCount = 0;
    var removeCount = 0;
    for (; _i < _diffstr.length && (removeCount < hunk.oldLines || addCount < hunk.newLines || _diffstr[_i].startsWith(r'\')); _i++) {
      final line = _diffstr[_i];
      // An empty line is context, except the last one (whose `[0]` is undefined).
      final operation = line.isEmpty && _i != _diffstr.length - 1 ? ' ' : (line.isEmpty ? null : line[0]);
      switch (operation) {
        case '+':
          addCount++;
        case '-':
          removeCount++;
        case ' ':
          addCount++;
          removeCount++;
        case r'\':
          break;
        default:
          throw FormatException('Hunk at line ${chunkHeaderIndex + 1} contained invalid line $line');
      }
      hunk.lines.add(line);
    }
    // The empty block count case.
    if (addCount == 0 && hunk.newLines == 1) hunk.newLines = 0;
    if (removeCount == 0 && hunk.oldLines == 1) hunk.oldLines = 0;
    if (addCount != hunk.newLines) throw FormatException('Added line count did not match for hunk at line ${chunkHeaderIndex + 1}');
    if (removeCount != hunk.oldLines) throw FormatException('Removed line count did not match for hunk at line ${chunkHeaderIndex + 1}');
    // A body line right after the counted ones means the counts were wrong, unless it is a file header.
    if (_i < _diffstr.length && _diffstr[_i].isNotEmpty && _hunkBodyLine.hasMatch(_diffstr[_i]) && !_isFileHeader(_diffstr[_i])) {
      throw FormatException('Hunk at line ${chunkHeaderIndex + 1} has more lines than expected');
    }
    return hunk;
  }
}

// ---- applyPatch ----

/// applyPatch: the patched text, or null when a hunk does not fit.
String? applyPatch(String source, List<FilePatch> patches) {
  if (patches.length > 1) throw ArgumentError('applyPatch only works with a single input.');
  if (patches.isEmpty) throw ArgumentError('applyPatch needs a patch');
  return _applyStructuredPatch(source, patches.single.hunks);
}

bool _hasOnlyWinLineEndings(String s) {
  if (!s.contains('\r\n') || s.startsWith('\n')) return false;
  for (var i = 1; i < s.length; i++) {
    if (s.codeUnitAt(i) == 0x0a && s.codeUnitAt(i - 1) != 0x0d) return false;
  }
  return true;
}

bool _hasOnlyUnixLineEndings(String s) => !s.contains('\r\n') && s.contains('\n');

bool _beforeMarker(List<String> lines, int i) => i + 1 < lines.length && lines[i + 1].startsWith(r'\');

bool _isUnix(List<Hunk> hunks) =>
    !hunks.any((hunk) => hunk.lines.any((line) => !line.startsWith(r'\') && line.endsWith('\r')));

bool _isWin(List<Hunk> hunks) =>
    hunks.any((hunk) => hunk.lines.any((line) => line.endsWith('\r'))) &&
    hunks.every((hunk) => hunk.lines.indexed
        .every((entry) => entry.$2.startsWith(r'\') || entry.$2.endsWith('\r') || _beforeMarker(hunk.lines, entry.$1)));

List<Hunk> _mapLines(List<Hunk> hunks, String Function(List<String> lines, int i) transform) => [
      for (final hunk in hunks)
        Hunk(hunk.oldStart, hunk.oldLines, hunk.newStart, hunk.newLines,
            [for (var i = 0; i < hunk.lines.length; i++) transform(hunk.lines, i)]),
    ];

List<Hunk> _unixToWin(List<Hunk> hunks) => _mapLines(hunks, (lines, i) {
      final line = lines[i];
      return line.startsWith(r'\') || line.endsWith('\r') || _beforeMarker(lines, i) ? line : '$line\r';
    });

List<Hunk> _winToUnix(List<Hunk> hunks) =>
    _mapLines(hunks, (lines, i) => lines[i].endsWith('\r') ? lines[i].substring(0, lines[i].length - 1) : lines[i]);

/// distance-iterator.js: positions after [_start], alternating forward and backward, within the
/// limits. Positions are JavaScript numbers, so a NaN start yields nothing, as in npm.
final class _DistanceIterator {
  _DistanceIterator(this._start, this._minLine, this._maxLine);

  final double _start;
  final double _minLine;
  final double _maxLine;
  var _wantForward = true;
  var _backwardExhausted = false;
  var _forwardExhausted = false;
  double _localOffset = 1;

  double? next() {
    if (_wantForward && !_forwardExhausted) {
      if (_backwardExhausted) {
        _localOffset++;
      } else {
        _wantForward = false;
      }
      // Fit after the offset location, unless that is beyond the text.
      if (_start + _localOffset <= _maxLine) return _start + _localOffset;
      _forwardExhausted = true;
    }
    if (!_backwardExhausted) {
      if (!_forwardExhausted) _wantForward = true;
      // Fit before the offset location, unless that is before the text.
      if (_minLine <= _start - _localOffset) return _start - _localOffset++;
      _backwardExhausted = true;
      return next();
    }
    return null;
  }
}

String? _applyStructuredPatch(String source, List<Hunk> patchHunks) {
  final hunks = _hasOnlyWinLineEndings(source) && _isUnix(patchHunks)
      ? _unixToWin(patchHunks)
      : _hasOnlyUnixLineEndings(source) && _isWin(patchHunks)
          ? _winToUnix(patchHunks)
          : patchHunks;
  final lines = source.split('\n');
  if (hunks.isEmpty) return source;
  // A change to the newline at the end of the file is in the last hunk.
  var prevLine = '';
  var removeEOFNL = false;
  var addEOFNL = false;
  for (final line in hunks.last.lines) {
    if (line.startsWith(r'\')) {
      if (prevLine.startsWith('+')) {
        removeEOFNL = true;
      } else if (prevLine.startsWith('-')) {
        addEOFNL = true;
      }
    }
    prevLine = line;
  }
  if (removeEOFNL) {
    if (addEOFNL) {
      // The last line changes but has no newline on either side: the source must not have one.
      if (lines.last == '') return null;
    } else if (lines.last == '') {
      lines.removeLast();
    } else {
      return null;
    }
  } else if (addEOFNL) {
    if (lines.last != '') {
      lines.add('');
    } else {
      return null;
    }
  }

  /// `lines[i]`, where a position outside the source, or NaN, reads as undefined.
  String? lineAt(double i) => i >= 0 && i < lines.length ? lines[i.toInt()] : null;

  /// applyHunk with no errors allowed: every removed and context line must equal the source line
  /// at its position. Trailing context is given back, so the next hunk may start inside it.
  ({List<String> patchedLines, double oldLineLastI})? applyHunk(List<String> hunkLines, double position) {
    var toPos = position;
    final patchedLines = <String>[];
    var nConsecutiveOldContextLines = 0;
    for (final hunkLine in hunkLines) {
      final operation = hunkLine.isNotEmpty ? hunkLine[0] : ' ';
      final content = hunkLine.isNotEmpty ? hunkLine.substring(1) : hunkLine;
      switch (operation) {
        case '-':
          if (lineAt(toPos) != content) return null;
          toPos++;
          nConsecutiveOldContextLines = 0;
        case '+':
          patchedLines.add(content);
          nConsecutiveOldContextLines = 0;
        case ' ':
          nConsecutiveOldContextLines++;
          if (lineAt(toPos) != content) return null;
          patchedLines.add(content);
          toPos++;
      }
    }
    patchedLines.length -= nConsecutiveOldContextLines;
    return (patchedLines: patchedLines, oldLineLastI: toPos - nConsecutiveOldContextLines - 1);
  }

  // Positions are JavaScript numbers: a NaN start fits only a hunk with no removed or context
  // line, and then every later position is NaN too. A position outside the source reads as
  // undefined, which joins as an empty line.
  final resultLines = <String>[];
  void copy(double from, double until) {
    for (var i = from; i < until; i++) {
      resultLines.add(lineAt(i) ?? '');
    }
  }

  var minLine = 0.0;
  var prevHunkOffset = 0.0;
  for (final hunk in hunks) {
    final maxLine = lines.length - hunk.oldLines;
    // The declared position first, then the positions around it.
    var toPos = hunk.oldStart + prevHunkOffset - 1;
    final iterator = _DistanceIterator(toPos, minLine, maxLine);
    var hunkResult = applyHunk(hunk.lines, toPos);
    while (hunkResult == null) {
      final next = iterator.next();
      if (next == null) return null;
      toPos = next;
      hunkResult = applyHunk(hunk.lines, toPos);
    }
    copy(minLine, toPos);
    resultLines.addAll(hunkResult.patchedLines);
    // Later hunks may not fit over text this one patched.
    minLine = hunkResult.oldLineLastI + 1;
    prevHunkOffset = toPos + 1 - hunk.oldStart;
  }
  copy(minLine, lines.length.toDouble());
  return resultLines.join('\n');
}

// ---- reversePatch, over the structured patch's own JSON values ----

@JS('Reflect.deleteProperty')
external bool _deleteProperty(JSObject target, String key);

JSAny? _swapPrefix(JSAny? fileName) {
  if (!fileName.isA<JSString>()) return fileName;
  final name = (fileName as JSString).toDart;
  if (name.startsWith('a/')) return 'b/${name.substring(2)}'.toJS;
  if (name.startsWith('b/')) return 'a/${name.substring(2)}'.toJS;
  return fileName;
}

String _reverseLine(String line) => line.startsWith('-')
    ? '+${line.substring(1)}'
    : line.startsWith('+')
        ? '-${line.substring(1)}'
        : line;

/// reversePatch: a copy of each structured patch with its sides swapped, the list reversed.
JSAny reversePatch(JSAny structuredPatch) {
  if (structuredPatch.isA<JSArray>()) {
    return [for (final patch in (structuredPatch as JSArray<JSAny>).toDart.reversed) reversePatch(patch)].toJS;
  }
  final patch = structuredPatch as JSObject;
  final isGit = truthy(patch['isGit']);
  JSAny? name(String key) => isGit ? _swapPrefix(patch[key]) : patch[key];
  // Object.assign({}, patch, {...}): the patch's own keys first, then the new ones.
  final reversed = JSObject();
  for (final key in objectKeys(patch).toDart) {
    reversed[key.toDart] = patch[key.toDart];
  }
  reversed
    ..['oldFileName'] = name('newFileName')
    ..['oldHeader'] = patch['newHeader']
    ..['newFileName'] = name('oldFileName')
    ..['newHeader'] = patch['oldHeader']
    ..['oldMode'] = patch['newMode']
    ..['newMode'] = patch['oldMode']
    ..['isCreate'] = patch['isDelete']
    ..['isDelete'] = patch['isCreate']
    ..['hunks'] = [
      for (final hunk in (patch['hunks'] as JSArray<JSObject>).toDart)
        JSObject()
          ..['oldLines'] = hunk['newLines']
          ..['oldStart'] = hunk['newStart']
          ..['newLines'] = hunk['oldLines']
          ..['newStart'] = hunk['oldStart']
          ..['lines'] = [
            for (final line in (hunk['lines'] as JSArray<JSString>).toDart) _reverseLine(line.toDart).toJS,
          ].toJS,
    ].toJS;
  if (truthy(patch['isCopy'])) {
    // Reversing a copy deletes the copy destination; the hunks cannot be reversed into a deletion.
    reversed
      ..['newFileName'] = '/dev/null'.toJS
      ..['newHeader'] = jsUndefined
      ..['isDelete'] = true.toJS;
    for (final key in ['isCreate', 'isCopy', 'isRename']) {
      _deleteProperty(reversed, key);
    }
    reversed['hunks'] = <JSAny>[].toJS;
  }
  return reversed;
}

/// A structured patch from JSON, for applyPatch.
FilePatch readFilePatch(JSAny? value) {
  double number(JSObject hunk, String key) => hunk[key].isA<JSNumber>() ? (hunk[key] as JSNumber).toDartDouble : double.nan;
  return FilePatch([
    for (final hunk in ((value as JSObject)['hunks'] as JSArray<JSObject>).toDart)
      Hunk(number(hunk, 'oldStart'), number(hunk, 'oldLines'), number(hunk, 'newStart'), number(hunk, 'newLines'),
          [for (final line in (hunk['lines'] as JSArray<JSString>).toDart) line.toDart]),
  ]);
}
