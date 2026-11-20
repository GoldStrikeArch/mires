import 'rules.dart';

/// Exact ECMA-262 whitespace check (25 code points; excludes U+0085 NEL which
/// Dart's `String.trim` otherwise trims).
bool isJsWhitespace(int c) {
  if (c <= 0x20) {
    return c == 0x20 || (c >= 0x09 && c <= 0x0D);
  }
  if (c < 0xA0) return false;
  return c == 0x00A0 ||
      c == 0x1680 ||
      (c >= 0x2000 && c <= 0x200A) ||
      c == 0x2028 ||
      c == 0x2029 ||
      c == 0x202F ||
      c == 0x205F ||
      c == 0x3000 ||
      c == 0xFEFF;
}

String jsTrim(String s) {
  final len = s.length;
  var start = 0;
  while (start < len && isJsWhitespace(s.codeUnitAt(start))) {
    start++;
  }
  if (start == len) return '';
  var end = len;
  while (end > start && isJsWhitespace(s.codeUnitAt(end - 1))) {
    end--;
  }
  return (start == 0 && end == len) ? s : s.substring(start, end);
}

String jsTrimStart(String s) {
  final len = s.length;
  var start = 0;
  while (start < len && isJsWhitespace(s.codeUnitAt(start))) {
    start++;
  }
  return start == 0 ? s : (start == len ? '' : s.substring(start));
}

String jsTrimEnd(String s) {
  var end = s.length;
  while (end > 0 && isJsWhitespace(s.codeUnitAt(end - 1))) {
    end--;
  }
  return end == s.length ? s : (end == 0 ? '' : s.substring(0, end));
}

/// JS `str.substring(start, end)` semantics (clamping negative / past-end indices).
String jsSubstring(String s, int start, [int? end]) {
  final len = s.length;
  var a = start < 0 ? 0 : (start > len ? len : start);
  var b = end == null ? len : (end < 0 ? 0 : (end > len ? len : end));
  if (a > b) {
    final tmp = a;
    a = b;
    b = tmp;
  }
  return s.substring(a, b);
}

/// JS `str.slice(start, end)` semantics (negative indices count from end, clamped).
String jsSlice(String s, int start, [int? end]) {
  final len = s.length;
  var a = start < 0 ? len + start : start;
  if (a < 0) a = 0;
  if (a > len) a = len;
  var b = end == null ? len : (end < 0 ? len + end : end);
  if (b < 0) b = 0;
  if (b > len) b = len;
  if (b <= a) return '';
  return s.substring(a, b);
}

/// Equivalent to `/^[ \t]*$/.test(line)`.
bool isBlankLine(String line) {
  final len = line.length;
  for (var i = 0; i < len; i++) {
    final c = line.codeUnitAt(i);
    if (c != 0x20 && c != 0x09) return false;
  }
  return true;
}

/// Equivalent to `s.search(/[^ ]/)`.
int indexOfNonSpace(String s) {
  final len = s.length;
  for (var i = 0; i < len; i++) {
    if (s.codeUnitAt(i) != 0x20) return i;
  }
  return -1;
}

/// Equivalent to `s.split('\n', 1)[0]`.
String firstLineOf(String s) {
  final nl = s.indexOf('\n');
  return nl == -1 ? s : s.substring(0, nl);
}

/// UTF-16 code-unit length (1 or 2) of the first Unicode scalar value in [s].
/// Equivalent to `[...s][0].length` in JS.
int firstCodePointUtf16Length(String s) {
  if (s.length >= 2) {
    final c0 = s.codeUnitAt(0);
    if (c0 >= 0xD800 && c0 <= 0xDBFF) {
      final c1 = s.codeUnitAt(1);
      if (c1 >= 0xDC00 && c1 <= 0xDFFF) return 2;
    }
  }
  return 1;
}

bool _isAsciiDigit(int c) => c >= 0x30 && c <= 0x39;

bool _isAsciiHex(int c) =>
    (c >= 0x30 && c <= 0x39) ||
    (c >= 0x41 && c <= 0x46) ||
    (c >= 0x61 && c <= 0x66);

bool _isAsciiWord(int c) =>
    (c >= 0x30 && c <= 0x39) ||
    (c >= 0x41 && c <= 0x5A) ||
    (c >= 0x61 && c <= 0x7A) ||
    c == 0x5F;

/// Checks whether [s] starting at [ampIndex] (which holds `'&'`) matches
/// `&(#\d{1,7}|#[Xx][a-fA-F0-9]{1,6}|\w+);`.
bool _isHtmlEntityAt(String s, int ampIndex) {
  final len = s.length;
  var i = ampIndex + 1;
  if (i >= len) return false;
  final first = s.codeUnitAt(i);
  if (first == 0x23 /* # */) {
    i++;
    if (i >= len) return false;
    final next = s.codeUnitAt(i);
    if (next == 0x78 /* x */ || next == 0x58 /* X */) {
      i++;
      final start = i;
      while (i < len && i - start < 6 && _isAsciiHex(s.codeUnitAt(i))) {
        i++;
      }
      return i > start && i < len && s.codeUnitAt(i) == 0x3B /* ; */;
    } else {
      final start = i;
      while (i < len && i - start < 7 && _isAsciiDigit(s.codeUnitAt(i))) {
        i++;
      }
      return i > start && i < len && s.codeUnitAt(i) == 0x3B /* ; */;
    }
  } else if (_isAsciiWord(first)) {
    i++;
    while (i < len && _isAsciiWord(s.codeUnitAt(i))) {
      i++;
    }
    return i < len && s.codeUnitAt(i) == 0x3B /* ; */;
  }
  return false;
}

String escapeHtmlEntities(String html, [bool encode = false]) {
  final len = html.length;
  StringBuffer? buf;
  var last = 0;

  for (var i = 0; i < len; i++) {
    final c = html.codeUnitAt(i);
    if (c > 0x3E || c < 0x22) continue;
    String? rep;
    if (c == 0x26 /* & */) {
      if (encode || !_isHtmlEntityAt(html, i)) {
        rep = '&amp;';
      }
    } else if (c == 0x3C /* < */) {
      rep = '&lt;';
    } else if (c == 0x3E /* > */) {
      rep = '&gt;';
    } else if (c == 0x22 /* " */) {
      rep = '&quot;';
    } else if (c == 0x27 /* ' */) {
      rep = '&#39;';
    }
    if (rep != null) {
      buf ??= StringBuffer();
      if (i > last) buf.write(html.substring(last, i));
      buf.write(rep);
      last = i + 1;
    }
  }

  if (buf == null) return html;
  if (last < len) buf.write(html.substring(last));
  return buf.toString();
}

String decodeNumericCharacterReferences(String text) {
  if (!text.contains('&#')) return text;
  final other = markedRules.other;
  if (!other.numericCharacterReference.hasMatch(text)) {
    return text;
  }
  return text.replaceAllMapped(other.numericCharacterReferenceOrAmpersand, (m) {
    final dec = m[1];
    final hex = m[2];
    if (dec == null && hex == null) {
      return '&amp;';
    }
    final code = dec == null ? int.parse(hex!, radix: 16) : int.parse(dec);
    if (code == 0 || code > 0x10FFFF || (code >= 0xD800 && code <= 0xDFFF)) {
      return '\uFFFD';
    }
    return code == 38 ? '&amp;' : String.fromCharCode(code);
  });
}

bool _isCleanUrlUnescaped(int c) {
  // ECMA-262 encodeURI unescaped set plus '%' (since .replace(/%25/g, '%')
  // restores every encoded '%25' to '%' and no other byte encodes to '%25').
  if (c >= 0x61 && c <= 0x7A) return true; // a-z
  if (c >= 0x41 && c <= 0x5A) return true; // A-Z
  if (c >= 0x23 && c <= 0x3B) return true; // #$%&'()*+,-./0-9:;
  return c == 0x21 ||
      c == 0x3D ||
      c == 0x3F ||
      c == 0x40 ||
      c == 0x5F ||
      c == 0x7E;
}

const _hexChars = '0123456789ABCDEF';

void _writePctByte(StringBuffer buf, int byte) {
  buf.writeCharCode(0x25);
  buf.writeCharCode(_hexChars.codeUnitAt((byte >>> 4) & 0xF));
  buf.writeCharCode(_hexChars.codeUnitAt(byte & 0xF));
}

/// Exact port of `encodeURI(href).replace(/%25/g, '%')`, returning `null` on
/// lone surrogates (matching `encodeURI` throwing `URIError`).
String? cleanUrl(String href) {
  final len = href.length;
  StringBuffer? buf;
  var last = 0;

  for (var i = 0; i < len; i++) {
    final c = href.codeUnitAt(i);
    if (c < 0x80) {
      if (_isCleanUrlUnescaped(c)) continue;
      buf ??= StringBuffer();
      if (i > last) buf.write(href.substring(last, i));
      _writePctByte(buf, c);
      last = i + 1;
    } else if (c < 0x800) {
      buf ??= StringBuffer();
      if (i > last) buf.write(href.substring(last, i));
      _writePctByte(buf, 0xC0 | (c >>> 6));
      _writePctByte(buf, 0x80 | (c & 0x3F));
      last = i + 1;
    } else if (c >= 0xD800 && c <= 0xDBFF) {
      if (i + 1 >= len) return null;
      final c2 = href.codeUnitAt(i + 1);
      if (c2 < 0xDC00 || c2 > 0xDFFF) return null;
      final cp = 0x10000 + ((c - 0xD800) << 10) + (c2 - 0xDC00);
      buf ??= StringBuffer();
      if (i > last) buf.write(href.substring(last, i));
      _writePctByte(buf, 0xF0 | (cp >>> 18));
      _writePctByte(buf, 0x80 | ((cp >>> 12) & 0x3F));
      _writePctByte(buf, 0x80 | ((cp >>> 6) & 0x3F));
      _writePctByte(buf, 0x80 | (cp & 0x3F));
      i++;
      last = i + 1;
    } else if (c >= 0xDC00 && c <= 0xDFFF) {
      return null;
    } else {
      buf ??= StringBuffer();
      if (i > last) buf.write(href.substring(last, i));
      _writePctByte(buf, 0xE0 | (c >>> 12));
      _writePctByte(buf, 0x80 | ((c >>> 6) & 0x3F));
      _writePctByte(buf, 0x80 | (c & 0x3F));
      last = i + 1;
    }
  }

  if (buf == null) return href;
  if (last < len) buf.write(href.substring(last));
  return buf.toString();
}

List<String> splitCells(String tableRow, [int? count]) {
  final len = tableRow.length;
  StringBuffer? buf;
  var last = 0;
  for (var i = 0; i < len; i++) {
    if (tableRow.codeUnitAt(i) == 0x7C /* | */) {
      var escaped = false;
      var curr = i;
      while (--curr >= 0 && tableRow.codeUnitAt(curr) == 0x5C /* \ */) {
        escaped = !escaped;
      }
      if (!escaped) {
        buf ??= StringBuffer();
        if (i > last) buf.write(tableRow.substring(last, i));
        buf.write(' |');
        last = i + 1;
      }
    }
  }
  final String row;
  if (buf == null) {
    row = tableRow;
  } else {
    if (last < len) buf.write(tableRow.substring(last));
    row = buf.toString();
  }

  final rawCells = row.split(' |');
  var startIdx = 0;
  var endIdx = rawCells.length;
  if (jsTrim(rawCells[0]).isEmpty) {
    startIdx = 1;
  }
  if (endIdx > startIdx && jsTrim(rawCells[endIdx - 1]).isEmpty) {
    endIdx--;
  }

  final sliceLen = endIdx - startIdx;
  final outLen = (count != null && count > 0) ? count : sliceLen;
  final copyLen = sliceLen < outLen ? sliceLen : outLen;
  final cells = List<String>.filled(outLen, '');
  for (var i = 0; i < copyLen; i++) {
    final trimmed = jsTrim(rawCells[startIdx + i]);
    cells[i] = trimmed.contains(r'\|') ? trimmed.replaceAll(r'\|', '|') : trimmed;
  }
  return cells;
}

String rtrim(String str, int charCode, [bool invert = false]) {
  final l = str.length;
  if (l == 0) return '';

  var suffLen = 0;
  while (suffLen < l) {
    final currChar = str.codeUnitAt(l - suffLen - 1);
    if (currChar == charCode && !invert) {
      suffLen++;
    } else if (currChar != charCode && invert) {
      suffLen++;
    } else {
      break;
    }
  }

  return suffLen == 0 ? str : str.substring(0, l - suffLen);
}

String trimTrailingBlankLines(String str) {
  final lines = str.split('\n');
  var end = lines.length - 1;
  while (end >= 0 && isBlankLine(lines[end])) {
    end--;
  }
  if (lines.length - end <= 2) {
    return str;
  }
  return lines.sublist(0, end + 1).join('\n');
}

String normalizeLabel(String label) =>
    jsTrim(label).toLowerCase().toUpperCase().toLowerCase();

int findClosingBracket(String str, int openCode, int closeCode) {
  final len = str.length;
  var foundAny = false;
  for (var i = 0; i < len; i++) {
    final c = str.codeUnitAt(i);
    if (c == openCode || c == closeCode) {
      foundAny = true;
      break;
    }
  }
  if (!foundAny) return -1;

  var level = 0;
  for (var i = 0; i < len; i++) {
    final c = str.codeUnitAt(i);
    if (c == 0x5C /* \ */) {
      i++;
    } else if (c == openCode) {
      level++;
    } else if (c == closeCode) {
      level--;
      if (level < 0) {
        return i;
      }
    }
  }
  if (level > 0) {
    return -2;
  }
  return -1;
}

String expandTabs(String line, [int indent = 0]) {
  if (!line.contains('\t')) return line;
  var col = indent;
  final buf = StringBuffer();
  final len = line.length;
  for (var i = 0; i < len; i++) {
    final c = line.codeUnitAt(i);
    if (c == 0x09 /* \t */) {
      final added = 4 - (col % 4);
      for (var s = 0; s < added; s++) {
        buf.writeCharCode(0x20);
      }
      col += added;
    } else {
      buf.writeCharCode(c);
      if (c >= 0xD800 && c <= 0xDBFF && i + 1 < len) {
        final c2 = line.codeUnitAt(i + 1);
        if (c2 >= 0xDC00 && c2 <= 0xDFFF) {
          buf.writeCharCode(c2);
          i++;
        }
      }
      col++;
    }
  }
  return buf.toString();
}
