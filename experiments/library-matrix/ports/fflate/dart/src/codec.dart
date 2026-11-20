// UTF-8 and Latin-1 string <-> Uint8List codecs (strToU8, strFromU8) matching
// fflate 0.8.3 / WHATWG Encoding Standard, plus UTF-16LE encoding for the boundary.

import 'dart:typed_data';

/// Encodes `str` into UTF-8 (default) or Latin-1 bytes (`codeUnitAt(i) & 0xFF`).
Uint8List strToU8(String str, [bool latin1 = false]) {
  final len = str.length;
  if (latin1) {
    final out = Uint8List(len);
    for (var i = 0; i < len; ++i) {
      out[i] = str.codeUnitAt(i);
    }
    return out;
  }
  if (len == 0) return Uint8List(0);
  // Worst-case UTF-8 length is 3 bytes per UTF-16 code unit (surrogate pairs take 4 <= 6).
  // Keeping `out` final avoids reassignment so dart2js omits unmodifiable-view checks per byte.
  final out = Uint8List(len * 3);
  var ai = 0;
  for (var i = 0; i < len; ++i) {
    final c = str.codeUnitAt(i);
    if (c < 0x80) {
      out[ai++] = c;
    } else if (c < 0x800) {
      out[ai++] = 0xC0 | (c >>> 6);
      out[ai++] = 0x80 | (c & 0x3F);
    } else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < len) {
      final c2 = str.codeUnitAt(i + 1);
      if (c2 >= 0xDC00 && c2 <= 0xDFFF) {
        ++i;
        final cp = 0x10000 + (((c & 0x3FF) << 10) | (c2 & 0x3FF));
        out[ai++] = 0xF0 | (cp >>> 18);
        out[ai++] = 0x80 | ((cp >>> 12) & 0x3F);
        out[ai++] = 0x80 | ((cp >>> 6) & 0x3F);
        out[ai++] = 0x80 | (cp & 0x3F);
      } else {
        out[ai++] = 0xEF;
        out[ai++] = 0xBF;
        out[ai++] = 0xBD;
      }
    } else if (c >= 0xD800 && c <= 0xDFFF) {
      out[ai++] = 0xEF;
      out[ai++] = 0xBF;
      out[ai++] = 0xBD;
    } else {
      out[ai++] = 0xE0 | (c >>> 12);
      out[ai++] = 0x80 | ((c >>> 6) & 0x3F);
      out[ai++] = 0x80 | (c & 0x3F);
    }
  }
  return Uint8List.sublistView(out, 0, ai);
}

/// Decodes `dat` to a Dart (UTF-16) string as Latin-1 or WHATWG UTF-8
/// (strips single leading UTF-8 BOM, replaces maximal ill-formed subparts with U+FFFD).
String strFromU8(Uint8List dat, [bool latin1 = false]) {
  final len = dat.length;
  if (len == 0) return '';
  if (latin1) {
    return String.fromCharCodes(dat);
  }

  var i = 0;
  if (len >= 3 && dat[0] == 0xEF && dat[1] == 0xBB && dat[2] == 0xBF) {
    i = 3;
  }

  final units = Uint16List(len - i);
  var ui = 0;
  var codePoint = 0;
  var bytesSeen = 0;
  var bytesNeeded = 0;
  var lowerBoundary = 0x80;
  var upperBoundary = 0xBF;

  for (; i < len; ++i) {
    final b = dat[i];
    if (bytesNeeded == 0) {
      if (b < 0x80) {
        units[ui++] = b;
      } else if (b >= 0xC2 && b <= 0xDF) {
        bytesNeeded = 1;
        codePoint = b & 0x1F;
      } else if (b >= 0xE0 && b <= 0xEF) {
        if (b == 0xE0) lowerBoundary = 0xA0;
        if (b == 0xED) upperBoundary = 0x9F;
        bytesNeeded = 2;
        codePoint = b & 0x0F;
      } else if (b >= 0xF0 && b <= 0xF4) {
        if (b == 0xF0) lowerBoundary = 0x90;
        if (b == 0xF4) upperBoundary = 0x8F;
        bytesNeeded = 3;
        codePoint = b & 0x07;
      } else {
        units[ui++] = 0xFFFD;
      }
    } else if (b < lowerBoundary || b > upperBoundary) {
      codePoint = 0;
      bytesNeeded = 0;
      bytesSeen = 0;
      lowerBoundary = 0x80;
      upperBoundary = 0xBF;
      --i;
      units[ui++] = 0xFFFD;
    } else {
      lowerBoundary = 0x80;
      upperBoundary = 0xBF;
      codePoint = (codePoint << 6) | (b & 0x3F);
      ++bytesSeen;
      if (bytesSeen == bytesNeeded) {
        final cp = codePoint;
        codePoint = 0;
        bytesNeeded = 0;
        bytesSeen = 0;
        if (cp <= 0xFFFF) {
          units[ui++] = cp;
        } else {
          final adj = cp - 0x10000;
          units[ui++] = 0xD800 | (adj >>> 10);
          units[ui++] = 0xDC00 | (adj & 0x3FF);
        }
      }
    }
  }
  if (bytesNeeded != 0) {
    units[ui++] = 0xFFFD;
  }

  return String.fromCharCodes(units, 0, ui);
}

/// Encodes a Dart (UTF-16) string as exact UTF-16LE bytes (2 bytes per code unit).
Uint8List stringToUtf16Le(String text) {
  final len = text.length;
  final out = Uint8List(len * 2);
  for (var i = 0; i < len; ++i) {
    final unit = text.codeUnitAt(i);
    final off = i * 2;
    out[off] = unit & 0xFF;
    out[off + 1] = unit >>> 8;
  }
  return out;
}
