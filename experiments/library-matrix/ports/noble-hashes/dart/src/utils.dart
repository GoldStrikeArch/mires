/// The parts of noble's utils.ts the selected hashes use.
library;

import 'dart:typed_data';

/// noble's Hash: an incremental hash with its block and digest lengths.
abstract interface class Hash {
  int get blockLen;
  int get outputLen;
  Hash update(Uint8List data);
  Uint8List digest();
}

/// noble's anumber: [n] must be a safe integer >= 0.
int anumber(double n, String title) {
  if (!(n >= 0 && n <= 9007199254740991 && n == n.truncateToDouble())) {
    throw ArgumentError('"$title" expected integer >= 0, got $n');
  }
  return n.toInt();
}

/// noble's rotr for 32-bit words.
int rotr(int word, int shift) => (word >> shift | word << (32 - shift)) & 0xffffffff;

/// noble's bytesToHex.
String bytesToHex(Uint8List bytes) {
  const digits = '0123456789abcdef';
  final out = StringBuffer();
  for (final b in bytes) {
    out
      ..write(digits[b >> 4])
      ..write(digits[b & 15]);
  }
  return out.toString();
}
