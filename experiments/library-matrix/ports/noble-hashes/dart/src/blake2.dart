/// noble's blake2.ts and _blake.ts: BLAKE2b and BLAKE2s on noble's shared _BLAKE2 base.
///
/// BLAKE2b keeps noble's G1b/G2b on 64-bit words as low and high halves in a 32-word work vector
/// (noble's BBUF); BLAKE2s keeps G1s/G2s, which return the four mixed words as noble's do. The two
/// do not share a mix function.
library;

import 'dart:math' show min;
import 'dart:typed_data';

import 'sha2.dart' show sha256IV;
import 'u64.dart';
import 'utils.dart';

/// Message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and 11 repeat 0 and 1.
const _bsigma = <int>[
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, //
  14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
  11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4,
  7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
  9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13,
  2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
  12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11,
  13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
  6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5,
  10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
  14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
];

/// noble's Blake2Opts: the options reference.mjs passes to every hash.
final class Blake2Opts {
  const Blake2Opts({this.dkLen, this.key, this.salt, this.personalization});

  final double? dkLen;
  final Uint8List? key;
  final Uint8List? salt;
  final Uint8List? personalization;
}

/// noble's checkBlake2Opts: [keyLen] bounds both the digest and the key length.
void _checkBlake2Opts(int outputLen, Blake2Opts opts, int keyLen, int saltLen, int persLen) {
  if (outputLen <= 0 || outputLen > keyLen) {
    throw ArgumentError('"dkLen" must be 1..$keyLen, got $outputLen');
  }
  final key = opts.key;
  if (key != null && (key.isEmpty || key.length > keyLen)) {
    throw ArgumentError('"key" expected to be undefined or of length=1..$keyLen');
  }
  if (opts.salt case final salt? when salt.length != saltLen) {
    throw ArgumentError('"salt" expected Uint8Array of length $saltLen');
  }
  if (opts.personalization case final pers? when pers.length != persLen) {
    throw ArgumentError('"personalization" expected Uint8Array of length $persLen');
  }
}

/// noble's _BLAKE2: the final block is compressed with a flag, so a full buffer waits for more input.
abstract class Blake2 implements Hash {
  Blake2(this.blockLen, double dkLen)
      : outputLen = anumber(dkLen, 'outputLen'),
        _buffer = Uint8List(blockLen) {
    _view = ByteData.sublistView(_buffer);
  }

  @override
  final int blockLen;
  @override
  final int outputLen;
  final Uint8List _buffer;
  late final ByteData _view;

  /// Bytes compressed or buffered so far: the BLAKE2 offset counter.
  int length = 0;
  int _pos = 0;

  /// Compresses the block of [view] at [offset].
  void compress(ByteData view, int offset, bool isLast);

  /// The state as little-endian 32-bit words.
  List<int> words();

  @override
  Blake2 update(Uint8List data) {
    final len = data.length;
    for (var pos = 0; pos < len;) {
      if (_pos == blockLen) {
        compress(_view, 0, false);
        _pos = 0;
      }
      final take = min(blockLen - _pos, len - pos);
      // Whole blocks that are not the last of the input are compressed straight from it.
      if (take == blockLen && pos + take < len) {
        final dataView = ByteData.sublistView(data);
        for (; pos + blockLen < len; pos += blockLen) {
          length += blockLen;
          compress(dataView, pos, false);
        }
        continue;
      }
      _buffer.setRange(_pos, _pos + take, data, pos);
      _pos += take;
      length += take;
      pos += take;
    }
    return this;
  }

  @override
  Uint8List digest() {
    _buffer.fillRange(_pos, blockLen, 0);
    compress(_view, 0, true);
    // The state's little-endian bytes, cut to the digest length.
    final state = words();
    final bytes = ByteData(4 * state.length);
    for (var i = 0; i < state.length; i++) {
      bytes.setUint32(4 * i, state[i], Endian.little);
    }
    return Uint8List.sublistView(bytes, 0, outputLen);
  }

  /// noble's constructor tail: the zero-padded key is the first block.
  void absorbKey(Uint8List? key) {
    if (key == null) return;
    update(Uint8List(blockLen)..setAll(0, key));
  }
}

/// The SHA-512 IV as little-endian low and high halves (noble's B2B_IV).
const _b2bIV = <int>[
  0xf3bcc908, 0x6a09e667, 0x84caa73b, 0xbb67ae85, 0xfe94f82b, 0x3c6ef372, 0x5f1d36f1, 0xa54ff53a,
  0xade682d1, 0x510e527f, 0x2b3e6c1f, 0x9b05688c, 0xfb41bd6b, 0x1f83d9ab, 0x137e2179, 0x5be0cd19,
];

/// noble's _BLAKE2b.
final class Blake2b extends Blake2 {
  Blake2b(Blake2Opts opts) : super(128, opts.dkLen ?? 64) {
    _checkBlake2Opts(outputLen, opts, 64, 16, 16);
    final keyLength = opts.key?.length ?? 0;
    // RFC 7693 §2.5: p[0] = 0x0101kknn goes into the low half of h[0].
    _v[0] ^= outputLen | keyLength << 8 | 0x01 << 16 | 0x01 << 24;
    if (opts.salt case final salt?) {
      final words = ByteData.sublistView(salt);
      for (var i = 0; i < 4; i++) {
        _v[8 + i] ^= words.getUint32(4 * i, Endian.little);
      }
    }
    if (opts.personalization case final pers?) {
      final words = ByteData.sublistView(pers);
      for (var i = 0; i < 4; i++) {
        _v[12 + i] ^= words.getUint32(4 * i, Endian.little);
      }
    }
    absorbKey(opts.key);
  }

  /// v0l, v0h, ..., v7l, v7h.
  final _v = Uint32List.fromList(_b2bIV);

  /// The work vector of 16 words as low and high halves (noble's BBUF).
  final _bbuf = Uint32List(32);

  /// The block being compressed as little-endian 32-bit words.
  final _msg = Uint32List(32);

  @override
  List<int> words() => _v;

  // G's first half: a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24).
  void _g1b(int a, int b, int c, int d, int x) {
    final v = _bbuf;
    final xl = _msg[x], xh = _msg[x + 1];
    var al = v[2 * a], ah = v[2 * a + 1];
    var bl = v[2 * b], bh = v[2 * b + 1];
    var cl = v[2 * c], ch = v[2 * c + 1];
    var dl = v[2 * d], dh = v[2 * d + 1];
    final ll = add3L(al, bl, xl);
    ah = add3H(ll, ah, bh, xh);
    al = ll & mask32;
    var th = dh ^ ah, tl = dl ^ al;
    dh = rotr32H(th, tl);
    dl = rotr32L(th, tl);
    (h: ch, l: cl) = add(ch, cl, dh, dl);
    th = bh ^ ch;
    tl = bl ^ cl;
    bh = rotrSH(th, tl, 24);
    bl = rotrSL(th, tl, 24);
    v[2 * a] = al;
    v[2 * a + 1] = ah;
    v[2 * b] = bl;
    v[2 * b + 1] = bh;
    v[2 * c] = cl;
    v[2 * c + 1] = ch;
    v[2 * d] = dl;
    v[2 * d + 1] = dh;
  }

  // G's second half: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63).
  void _g2b(int a, int b, int c, int d, int x) {
    final v = _bbuf;
    final xl = _msg[x], xh = _msg[x + 1];
    var al = v[2 * a], ah = v[2 * a + 1];
    var bl = v[2 * b], bh = v[2 * b + 1];
    var cl = v[2 * c], ch = v[2 * c + 1];
    var dl = v[2 * d], dh = v[2 * d + 1];
    final ll = add3L(al, bl, xl);
    ah = add3H(ll, ah, bh, xh);
    al = ll & mask32;
    var th = dh ^ ah, tl = dl ^ al;
    dh = rotrSH(th, tl, 16);
    dl = rotrSL(th, tl, 16);
    (h: ch, l: cl) = add(ch, cl, dh, dl);
    th = bh ^ ch;
    tl = bl ^ cl;
    bh = rotrBH(th, tl, 63);
    bl = rotrBL(th, tl, 63);
    v[2 * a] = al;
    v[2 * a + 1] = ah;
    v[2 * b] = bl;
    v[2 * b + 1] = bh;
    v[2 * c] = cl;
    v[2 * c + 1] = ch;
    v[2 * d] = dl;
    v[2 * d + 1] = dh;
  }

  @override
  void compress(ByteData view, int offset, bool isLast) {
    for (var i = 0; i < 32; i++) {
      _msg[i] = view.getUint32(offset + 4 * i, Endian.little);
    }
    final v = _bbuf
      ..setAll(0, _v)
      ..setAll(16, _b2bIV);
    // The byte counter goes into v12; v14 is inverted on the last block.
    v[24] = _b2bIV[8] ^ fromNumL(length);
    v[25] = _b2bIV[9] ^ fromNumH(length);
    if (isLast) {
      v[28] = ~v[28];
      v[29] = ~v[29];
    }
    // BSIGMA picks 64-bit message words; each spans two entries of _msg.
    var j = 0;
    for (var i = 0; i < 12; i++) {
      _g1b(0, 4, 8, 12, 2 * _bsigma[j++]);
      _g2b(0, 4, 8, 12, 2 * _bsigma[j++]);
      _g1b(1, 5, 9, 13, 2 * _bsigma[j++]);
      _g2b(1, 5, 9, 13, 2 * _bsigma[j++]);
      _g1b(2, 6, 10, 14, 2 * _bsigma[j++]);
      _g2b(2, 6, 10, 14, 2 * _bsigma[j++]);
      _g1b(3, 7, 11, 15, 2 * _bsigma[j++]);
      _g2b(3, 7, 11, 15, 2 * _bsigma[j++]);
      _g1b(0, 5, 10, 15, 2 * _bsigma[j++]);
      _g2b(0, 5, 10, 15, 2 * _bsigma[j++]);
      _g1b(1, 6, 11, 12, 2 * _bsigma[j++]);
      _g2b(1, 6, 11, 12, 2 * _bsigma[j++]);
      _g1b(2, 7, 8, 13, 2 * _bsigma[j++]);
      _g2b(2, 7, 8, 13, 2 * _bsigma[j++]);
      _g1b(3, 4, 9, 14, 2 * _bsigma[j++]);
      _g2b(3, 4, 9, 14, 2 * _bsigma[j++]);
    }
    for (var i = 0; i < 16; i++) {
      _v[i] ^= v[i] ^ v[i + 16];
    }
  }
}

/// BLAKE2s's G first half: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12).
(int, int, int, int) _g1s(int a, int b, int c, int d, int x) {
  a = (a + b + x) & mask32;
  d = rotr(d ^ a, 16);
  c = (c + d) & mask32;
  b = rotr(b ^ c, 12);
  return (a, b, c, d);
}

/// BLAKE2s's G second half: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7).
(int, int, int, int) _g2s(int a, int b, int c, int d, int x) {
  a = (a + b + x) & mask32;
  d = rotr(d ^ a, 8);
  c = (c + d) & mask32;
  b = rotr(b ^ c, 7);
  return (a, b, c, d);
}

/// noble's _BLAKE2s, whose IV is SHA-256's.
final class Blake2s extends Blake2 {
  Blake2s(Blake2Opts opts) : super(64, opts.dkLen ?? 32) {
    _checkBlake2Opts(outputLen, opts, 32, 8, 8);
    final keyLength = opts.key?.length ?? 0;
    _v[0] ^= outputLen | keyLength << 8 | 0x01 << 16 | 0x01 << 24;
    if (opts.salt case final salt?) {
      final words = ByteData.sublistView(salt);
      _v[4] ^= words.getUint32(0, Endian.little);
      _v[5] ^= words.getUint32(4, Endian.little);
    }
    if (opts.personalization case final pers?) {
      final words = ByteData.sublistView(pers);
      _v[6] ^= words.getUint32(0, Endian.little);
      _v[7] ^= words.getUint32(4, Endian.little);
    }
    absorbKey(opts.key);
  }

  /// v0 ... v7.
  final _v = Uint32List.fromList(sha256IV);

  /// The block being compressed as little-endian 32-bit words.
  final _msg = Uint32List(16);

  @override
  List<int> words() => _v;

  @override
  void compress(ByteData view, int offset, bool isLast) {
    final msg = _msg;
    for (var i = 0; i < 16; i++) {
      msg[i] = view.getUint32(offset + 4 * i, Endian.little);
    }
    const iv = sha256IV;
    // noble's _compress: v8..v15 come from the IV, with the byte counter in v12 and v13 and v14
    // inverted on the last block.
    var v0 = _v[0], v1 = _v[1], v2 = _v[2], v3 = _v[3], v4 = _v[4], v5 = _v[5], v6 = _v[6], v7 = _v[7];
    var v8 = iv[0], v9 = iv[1], v10 = iv[2], v11 = iv[3];
    var v12 = fromNumL(length) ^ iv[4], v13 = fromNumH(length) ^ iv[5];
    var v14 = isLast ? ~iv[6] & mask32 : iv[6], v15 = iv[7];
    var j = 0;
    for (var i = 0; i < 10; i++) {
      (v0, v4, v8, v12) = _g1s(v0, v4, v8, v12, msg[_bsigma[j++]]);
      (v0, v4, v8, v12) = _g2s(v0, v4, v8, v12, msg[_bsigma[j++]]);
      (v1, v5, v9, v13) = _g1s(v1, v5, v9, v13, msg[_bsigma[j++]]);
      (v1, v5, v9, v13) = _g2s(v1, v5, v9, v13, msg[_bsigma[j++]]);
      (v2, v6, v10, v14) = _g1s(v2, v6, v10, v14, msg[_bsigma[j++]]);
      (v2, v6, v10, v14) = _g2s(v2, v6, v10, v14, msg[_bsigma[j++]]);
      (v3, v7, v11, v15) = _g1s(v3, v7, v11, v15, msg[_bsigma[j++]]);
      (v3, v7, v11, v15) = _g2s(v3, v7, v11, v15, msg[_bsigma[j++]]);
      (v0, v5, v10, v15) = _g1s(v0, v5, v10, v15, msg[_bsigma[j++]]);
      (v0, v5, v10, v15) = _g2s(v0, v5, v10, v15, msg[_bsigma[j++]]);
      (v1, v6, v11, v12) = _g1s(v1, v6, v11, v12, msg[_bsigma[j++]]);
      (v1, v6, v11, v12) = _g2s(v1, v6, v11, v12, msg[_bsigma[j++]]);
      (v2, v7, v8, v13) = _g1s(v2, v7, v8, v13, msg[_bsigma[j++]]);
      (v2, v7, v8, v13) = _g2s(v2, v7, v8, v13, msg[_bsigma[j++]]);
      (v3, v4, v9, v14) = _g1s(v3, v4, v9, v14, msg[_bsigma[j++]]);
      (v3, v4, v9, v14) = _g2s(v3, v4, v9, v14, msg[_bsigma[j++]]);
    }
    _v[0] ^= v0 ^ v8;
    _v[1] ^= v1 ^ v9;
    _v[2] ^= v2 ^ v10;
    _v[3] ^= v3 ^ v11;
    _v[4] ^= v4 ^ v12;
    _v[5] ^= v5 ^ v13;
    _v[6] ^= v6 ^ v14;
    _v[7] ^= v7 ^ v15;
  }
}
