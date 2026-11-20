import 'dart:typed_data';

// Words stay split into 32-bit limbs on dart2js, including the carry in BLAKE2b.
const _blakeHigh = <int>[
  0x6a09e667,
  0xbb67ae85,
  0x3c6ef372,
  0xa54ff53a,
  0x510e527f,
  0x9b05688c,
  0x1f83d9ab,
  0x5be0cd19,
];
const _blakeLow = <int>[
  0xf3bcc908,
  0x84caa73b,
  0xfe94f82b,
  0x5f1d36f1,
  0xade682d1,
  0x2b3e6c1f,
  0xfb41bd6b,
  0x137e2179,
];
const _blakeSigma = <List<int>>[
  [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
  [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
  [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
  [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
  [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
  [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
  [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
  [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
  [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
  [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

final class _Word {
  const _Word(this.lo, this.hi);
  final int lo;
  final int hi;

  _Word add(_Word other) {
    final sum = lo + other.lo;
    return _Word(
      sum & 0xffffffff,
      (hi + other.hi + (sum > 0xffffffff ? 1 : 0)) & 0xffffffff,
    );
  }

  _Word xor(_Word other) => _Word(lo ^ other.lo, hi ^ other.hi);

  _Word rotate(int n, bool wide) {
    if (!wide) return _Word(((lo >>> n) | (lo << (32 - n))) & 0xffffffff, 0);
    if (n == 32) return _Word(hi, lo);
    final l = n > 32 ? hi : lo, h = n > 32 ? lo : hi, bits = n & 31;
    return _Word(
      ((l >>> bits) | (h << (32 - bits))) & 0xffffffff,
      ((h >>> bits) | (l << (32 - bits))) & 0xffffffff,
    );
  }
}

_Word _iv(int i, bool wide) =>
    wide ? _Word(_blakeLow[i], _blakeHigh[i]) : _Word(_blakeHigh[i], 0);

int _load32(Uint8List bytes, int at) =>
    bytes[at] |
    (bytes[at + 1] << 8) |
    (bytes[at + 2] << 16) |
    (bytes[at + 3] << 24);

_Word _load(Uint8List bytes, int i, bool wide) => wide
    ? _Word(_load32(bytes, 8 * i), _load32(bytes, 8 * i + 4))
    : _Word(_load32(bytes, 4 * i), 0);

void _mix(
  List<_Word> v,
  int a,
  int b,
  int c,
  int d,
  _Word x,
  _Word y,
  bool wide,
) {
  v[a] = v[a].add(v[b]).add(x);
  v[d] = v[d].xor(v[a]).rotate(wide ? 32 : 16, wide);
  v[c] = v[c].add(v[d]);
  v[b] = v[b].xor(v[c]).rotate(wide ? 24 : 12, wide);
  v[a] = v[a].add(v[b]).add(y);
  v[d] = v[d].xor(v[a]).rotate(wide ? 16 : 8, wide);
  v[c] = v[c].add(v[d]);
  v[b] = v[b].xor(v[c]).rotate(wide ? 63 : 7, wide);
}

void _compress(
  List<_Word> h,
  Uint8List block,
  _Word count,
  bool last,
  bool wide,
) {
  final v = List.generate(16, (i) => i < 8 ? h[i] : _iv(i - 8, wide));
  final msg = List.generate(16, (i) => _load(block, i, wide));
  v[12] = v[12].xor(_Word(count.lo, wide ? count.hi : 0));
  if (!wide) v[13] = v[13].xor(_Word(count.hi, 0));
  if (last) v[14] = v[14].xor(const _Word(0xffffffff, 0xffffffff));
  for (var r = 0; r < (wide ? 12 : 10); r++) {
    final s = _blakeSigma[r % 10];
    for (var i = 0; i < 4; i++) {
      _mix(v, i, i + 4, i + 8, i + 12, msg[s[2 * i]], msg[s[2 * i + 1]], wide);
    }
    for (var i = 0; i < 4; i++) {
      _mix(
        v,
        i,
        4 + (i + 1) % 4,
        8 + (i + 2) % 4,
        12 + (i + 3) % 4,
        msg[s[8 + 2 * i]],
        msg[s[9 + 2 * i]],
        wide,
      );
    }
  }
  for (var i = 0; i < 8; i++) h[i] = h[i].xor(v[i]).xor(v[i + 8]);
}

Uint8List blake2Digest(
  bool wide,
  List<Uint8List> chunks,
  int dkLen,
  Uint8List key,
  Uint8List salt,
  Uint8List personalization,
) {
  final stride = wide ? 8 : 4, blockLen = 16 * stride;
  final h = List.generate(8, (i) => _iv(i, wide));
  h[0] = h[0].xor(_Word(0x01010000 | (key.length << 8) | dkLen, 0));
  for (var i = 0; i < 2; i++) {
    if (salt.isNotEmpty) h[4 + i] = h[4 + i].xor(_load(salt, i, wide));
    if (personalization.isNotEmpty)
      h[6 + i] = h[6 + i].xor(_load(personalization, i, wide));
  }
  final block = Uint8List(blockLen)..setAll(0, key);
  var pos = key.isEmpty ? 0 : blockLen, count = _Word(pos, 0);
  for (final chunk in chunks) {
    var at = 0;
    while (at < chunk.length) {
      if (pos == blockLen) {
        _compress(h, block, count, false, wide);
        pos = 0;
      }
      final remaining = chunk.length - at;
      final take = blockLen - pos < remaining ? blockLen - pos : remaining;
      block.setRange(pos, pos + take, chunk, at);
      count = count.add(_Word(take, 0));
      at += take;
      pos += take;
    }
  }
  block.fillRange(pos, blockLen, 0);
  _compress(h, block, count, true, wide);
  final out = Uint8List(dkLen);
  for (var i = 0; i < dkLen; i++) {
    final w = h[i ~/ stride], limb = i % stride >= 4 ? w.hi : w.lo;
    out[i] = (limb >>> ((i % 4) * 8)) & 255;
  }
  return out;
}
