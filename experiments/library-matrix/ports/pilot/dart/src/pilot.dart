// Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
// libraries/pilot/CONTRACT.md. Dart strings are UTF-16; `runes` iterates code points.

import 'dart:convert';

class Entry {
  Entry(this.word, this.length, this.first);

  final String word;
  final int length;
  final int first;
  int count = 1;
}

bool _isWordCodePoint(int cp) =>
    cp >= 0x80 || (cp >= 0x30 && cp <= 0x39) || (cp >= 0x41 && cp <= 0x5a) || (cp >= 0x61 && cp <= 0x7a);

/// FNV-1a 32 over the UTF-8 bytes of [text].
///
/// On the web a Dart `int` is a JavaScript number, so `hash * 0x01000193` would exceed 2^53 and
/// lose bits. The prime is 2^24 + 403, so the product is split into a shift and a small multiply,
/// which is exact on the web and on the VM alike.
int fnv1a(String text) {
  var hash = 0x811c9dc5;
  for (final byte in utf8.encode(text)) {
    final x = hash ^ byte;
    hash = ((x << 24) + x * 403) & 0xffffffff;
  }
  return hash;
}

String _hex8(int value) => value.toRadixString(16).padLeft(8, '0');

String analyse(String text, int top) {
  final entries = <String, Entry>{};
  var tokens = 0;
  var codePoints = 0;
  final word = StringBuffer();
  var length = 0;
  void flush() {
    if (length == 0) return;
    tokens++;
    codePoints += length;
    final key = word.toString();
    final entry = entries[key];
    if (entry != null) {
      entry.count++;
    } else {
      entries[key] = Entry(key, length, entries.length);
    }
    word.clear();
    length = 0;
  }

  for (final cp in text.runes) {
    if (_isWordCodePoint(cp)) {
      // ASCII-only case folding.
      word.writeCharCode(cp >= 0x41 && cp <= 0x5a ? cp + 0x20 : cp);
      length++;
    } else {
      flush();
    }
  }
  flush();

  // List.sort is not guaranteed to be stable, so first occurrence is the last explicit key.
  final ranked = entries.values.toList()
    ..sort((a, b) {
      if (a.count != b.count) return b.count - a.count;
      if (a.length != b.length) return b.length - a.length;
      return a.first - b.first;
    });
  final mean = tokens == 0 ? 0.0 : codePoints / tokens;
  final lines = <String>[
    'tokens $tokens',
    'distinct ${entries.length}',
    'mean ${mean.toStringAsFixed(3)}',
    'fnv ${_hex8(fnv1a(text))}',
    for (final entry in ranked.take(top))
      '${entry.word} ${entry.count} ${_hex8(fnv1a(entry.word))} ${(entry.count / tokens).toStringAsFixed(3)}',
  ];
  return lines.join('\n');
}
