// SHA3-256 and Keccak-256, adapted from @noble/hashes 2.4.0 (MIT).
import 'dart:typed_data';

const _pi = [
  20,
  14,
  22,
  34,
  36,
  6,
  10,
  32,
  16,
  42,
  48,
  8,
  30,
  46,
  38,
  26,
  24,
  4,
  40,
  28,
  44,
  18,
  12,
  2,
];
const _rotations = [
  1,
  3,
  6,
  10,
  15,
  21,
  28,
  36,
  45,
  55,
  2,
  14,
  27,
  41,
  56,
  8,
  25,
  43,
  62,
  18,
  39,
  61,
  20,
  44,
];
const _rcLo = [
  1,
  32898,
  32906,
  2147516416,
  32907,
  2147483649,
  2147516545,
  32777,
  138,
  136,
  2147516425,
  2147483658,
  2147516555,
  139,
  32905,
  32771,
  32770,
  128,
  32778,
  2147483658,
  2147516545,
  32896,
  2147483649,
  2147516424,
];
const _rcHi = [
  0,
  0,
  2147483648,
  2147483648,
  0,
  0,
  2147483648,
  2147483648,
  0,
  0,
  0,
  0,
  0,
  2147483648,
  2147483648,
  2147483648,
  2147483648,
  2147483648,
  0,
  2147483648,
  2147483648,
  2147483648,
  0,
  2147483648,
];
// The 25 little-endian lanes occupy 50 words.
void _permute(Uint32List s) {
  final columns = Uint32List(10);
  for (var round = 0; round < 24; round++) {
    // Theta mixes column parities into every lane.
    for (var x = 0; x < 10; x++) {
      columns[x] = s[x] ^ s[x + 10] ^ s[x + 20] ^ s[x + 30] ^ s[x + 40];
    }
    for (var col = 0; col < 5; col++) {
      final x = 2 * col;
      final left = (x + 8) % 10;
      final right = (x + 2) % 10;
      final low =
          columns[left] ^ ((columns[right] << 1) | (columns[right + 1] >>> 31));
      final high =
          columns[left + 1] ^
          ((columns[right + 1] << 1) | (columns[right] >>> 31));
      for (var row = 0; row < 5; row++) {
        final i = row * 10 + x;
        s[i] = s[i] ^ low;
        s[i + 1] = s[i + 1] ^ high;
      }
    }
    // Rho rotates and Pi moves the 24 non-origin lanes in a single cycle.
    var low = s[2];
    var high = s[3];
    for (var t = 0; t < 24; t++) {
      final shift = _rotations[t];
      final k = shift & 31;
      final a = shift > 32 ? high : low;
      final b = shift > 32 ? low : high;
      final p = _pi[t];
      low = s[p];
      high = s[p + 1];
      s[p] = ((a << k) | (b >>> (32 - k)));
      s[p + 1] = ((b << k) | (a >>> (32 - k)));
    }
    // Chi reads the original five lanes in each row.
    for (var row = 0; row < 5; row++) {
      final y = 10 * row;
      final saved0 = s[y];
      final saved1 = s[y + 1];
      final saved2 = s[y + 2];
      final saved3 = s[y + 3];
      for (var x = 0; x < 6; x++) {
        s[y + x] = s[y + x] ^ (~s[y + x + 2] & s[y + x + 4]);
      }
      s[y + 6] = s[y + 6] ^ (~s[y + 8] & saved0);
      s[y + 7] = s[y + 7] ^ (~s[y + 9] & saved1);
      s[y + 8] = s[y + 8] ^ (~saved0 & saved2);
      s[y + 9] = s[y + 9] ^ (~saved1 & saved3);
    }
    s[0] = s[0] ^ _rcLo[round];
    s[1] = s[1] ^ _rcHi[round];
  }
}

Uint8List sha3Digest(bool legacy, List<Uint8List> chunks) {
  final s = Uint32List(50);
  var pos = 0;
  for (final chunk in chunks) {
    var index = 0;
    while (index < chunk.length) {
      if (pos == 0 && chunk.length - index >= 136) {
        for (var word = 0; word < 34; word++) {
          final i = index + 4 * word;
          s[word] =
              s[word] ^
              (chunk[i] |
                  (chunk[i + 1] << 8) |
                  (chunk[i + 2] << 16) |
                  (chunk[i + 3] << 24));
        }
        index += 136;
        _permute(s);
      } else {
        s[pos ~/ 4] = s[pos ~/ 4] ^ (chunk[index] << (8 * (pos % 4)));
        index++;
        pos++;
        if (pos == 136) {
          _permute(s);
          pos = 0;
        }
      }
    }
  }
  final suffix = legacy ? 1 : 6;
  s[pos ~/ 4] = s[pos ~/ 4] ^ (suffix << (8 * (pos % 4)));
  s[33] = s[33] ^ 0x80000000;
  _permute(s);
  final result = Uint8List(32);
  for (var i = 0; i < 32; i++) {
    result[i] = (s[i ~/ 4] >>> (8 * (i % 4))) & 255;
  }
  return result;
}
