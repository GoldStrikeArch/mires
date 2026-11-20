/// noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.
///
/// The state is noble's state32: 25 lanes of two 32-bit words, the low word first. noble derives
/// the pi, rho and iota tables at load time; they are written out here.
library;

import 'dart:math' show min;
import 'dart:typed_data';

import 'u64.dart' show mask32;
import 'utils.dart';

/// Pi: the word index each rho step moves its lane to (noble's SHA3_PI).
const _sha3Pi = <int>[20, 14, 22, 34, 36, 6, 10, 32, 16, 42, 48, 8, 30, 46, 38, 26, 24, 4, 40, 28, 44, 18, 12, 2];

/// Rho: the left rotation of each step (noble's SHA3_ROTL).
const _sha3Rotl = <int>[1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44];

/// Iota: the low and high words of each round constant (noble's SHA3_IOTA_H and SHA3_IOTA_L).
const _sha3IotaH = <int>[
  0x00000001, 0x00008082, 0x0000808a, 0x80008000, 0x0000808b, 0x80000001, 0x80008081, 0x00008009,
  0x0000008a, 0x00000088, 0x80008009, 0x8000000a, 0x8000808b, 0x0000008b, 0x00008089, 0x00008003,
  0x00008002, 0x00000080, 0x0000800a, 0x8000000a, 0x80008081, 0x00008080, 0x80000001, 0x80008008,
];
const _sha3IotaL = <int>[
  0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000,
  0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000,
  0x80000000, 0x80000000, 0x00000000, 0x80000000, 0x80000000, 0x80000000, 0x00000000, 0x80000000,
];

// Left rotations of a lane by s in 1..31 (S) or 33..63 (B). As in noble, h is the lane's first
// (low) word and l its second (high) word.
int _rotlSH(int h, int l, int s) => (h << s | l >> (32 - s)) & mask32;
int _rotlSL(int h, int l, int s) => (l << s | h >> (32 - s)) & mask32;
int _rotlBH(int h, int l, int s) => (l << (s - 32) | h >> (64 - s)) & mask32;
int _rotlBL(int h, int l, int s) => (h << (s - 32) | l >> (64 - s)) & mask32;
int _rotlH(int h, int l, int s) => s > 32 ? _rotlBH(h, l, s) : _rotlSH(h, l, s);
int _rotlL(int h, int l, int s) => s > 32 ? _rotlBL(h, l, s) : _rotlSL(h, l, s);

/// noble's keccakP: the 24 rounds of Keccak-f[1600] on [s], with [b] as the theta scratch.
void keccakP(Uint32List s, Uint32List b) {
  for (var round = 0; round < 24; round++) {
    // Theta
    for (var x = 0; x < 10; x++) {
      b[x] = s[x] ^ s[x + 10] ^ s[x + 20] ^ s[x + 30] ^ s[x + 40];
    }
    for (var x = 0; x < 10; x += 2) {
      final idx1 = (x + 8) % 10;
      final idx0 = (x + 2) % 10;
      final b0 = b[idx0];
      final b1 = b[idx0 + 1];
      final th = _rotlH(b0, b1, 1) ^ b[idx1];
      final tl = _rotlL(b0, b1, 1) ^ b[idx1 + 1];
      for (var y = 0; y < 50; y += 10) {
        s[x + y] ^= th;
        s[x + y + 1] ^= tl;
      }
    }
    // Rho and pi
    var curH = s[2];
    var curL = s[3];
    for (var t = 0; t < 24; t++) {
      final shift = _sha3Rotl[t];
      final th = _rotlH(curH, curL, shift);
      final tl = _rotlL(curH, curL, shift);
      final pi = _sha3Pi[t];
      curH = s[pi];
      curL = s[pi + 1];
      s[pi] = th;
      s[pi + 1] = tl;
    }
    // Chi
    for (var y = 0; y < 50; y += 10) {
      final b0 = s[y], b1 = s[y + 1], b2 = s[y + 2], b3 = s[y + 3];
      for (var x = 0; x < 6; x++) {
        s[y + x] ^= ~s[y + x + 2] & s[y + x + 4];
      }
      s[y + 6] ^= ~s[y + 8] & b0;
      s[y + 7] ^= ~s[y + 9] & b1;
      s[y + 8] ^= ~b0 & b2;
      s[y + 9] ^= ~b1 & b3;
    }
    // Iota
    s[0] ^= _sha3IotaH[round];
    s[1] ^= _sha3IotaL[round];
  }
}

/// noble's Keccak sponge with a byte-length rate [blockLen] and domain [suffix].
final class Keccak implements Hash {
  Keccak(this.blockLen, this.suffix, this.outputLen);

  @override
  final int blockLen;
  final int suffix;
  @override
  final int outputLen;
  final _state32 = Uint32List(50);
  final _theta = Uint32List(10);
  int _pos = 0;
  int _posOut = 0;

  /// XORs [byte] into state byte [index], as noble does through its byte view of state32.
  void _xorByte(int index, int byte) => _state32[index >> 2] ^= byte << 8 * (index & 3);

  /// The state byte at [index].
  int _stateByte(int index) => _state32[index >> 2] >> 8 * (index & 3) & 0xff;

  void _keccak() {
    keccakP(_state32, _theta);
    _posOut = 0;
    _pos = 0;
  }

  @override
  Keccak update(Uint8List data) {
    final len = data.length;
    final view = ByteData.sublistView(data);
    for (var pos = 0; pos < len;) {
      // Whole blocks with nothing buffered are absorbed a word at a time.
      if (_pos == 0 && len - pos >= blockLen) {
        for (var i = 0; i < blockLen ~/ 4; i++) {
          _state32[i] ^= view.getUint32(pos + 4 * i, Endian.little);
        }
        pos += blockLen;
        _pos = blockLen;
        _keccak();
        continue;
      }
      final take = min(blockLen - _pos, len - pos);
      for (var i = 0; i < take; i++) {
        _xorByte(_pos++, data[pos++]);
      }
      if (_pos == blockLen) _keccak();
    }
    return this;
  }

  void _finish() {
    // The suffix holds the domain bits and the first padding bit; 0x80 closes pad10*1.
    _xorByte(_pos, suffix);
    if ((suffix & 0x80) != 0 && _pos == blockLen - 1) _keccak();
    _xorByte(blockLen - 1, 0x80);
    _keccak();
  }

  @override
  Uint8List digest() {
    _finish();
    final out = Uint8List(outputLen);
    for (var pos = 0; pos < outputLen;) {
      if (_posOut >= blockLen) _keccak();
      final take = min(blockLen - _posOut, outputLen - pos);
      for (var i = 0; i < take; i++) {
        out[pos++] = _stateByte(_posOut++);
      }
    }
    return out;
  }
}
