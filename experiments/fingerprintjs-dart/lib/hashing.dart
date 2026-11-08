/// MurmurHash3 x64 128, matching FingerprintJS 5.2.0 / murmurHash3.js.
library;

int _u32(int x) => x & 0xffffffff;

class _X64 {
  int high;
  int low;

  _X64(this.high, this.low);

  void add(_X64 n) {
    final m0 = high >>> 16;
    final m1 = high & 0xffff;
    final m2 = low >>> 16;
    final m3 = low & 0xffff;
    final n0 = n.high >>> 16;
    final n1 = n.high & 0xffff;
    final n2 = n.low >>> 16;
    final n3 = n.low & 0xffff;
    var o0 = 0;
    var o1 = 0;
    var o2 = 0;
    var o3 = 0;
    o3 += m3 + n3;
    o2 += o3 >>> 16;
    o3 &= 0xffff;
    o2 += m2 + n2;
    o1 += o2 >>> 16;
    o2 &= 0xffff;
    o1 += m1 + n1;
    o0 += o1 >>> 16;
    o1 &= 0xffff;
    o0 += m0 + n0;
    o0 &= 0xffff;
    high = _u32((o0 << 16) | o1);
    low = _u32((o2 << 16) | o3);
  }

  void multiply(_X64 n) {
    final m0 = high >>> 16;
    final m1 = high & 0xffff;
    final m2 = low >>> 16;
    final m3 = low & 0xffff;
    final n0 = n.high >>> 16;
    final n1 = n.high & 0xffff;
    final n2 = n.low >>> 16;
    final n3 = n.low & 0xffff;
    var o0 = 0;
    var o1 = 0;
    var o2 = 0;
    var o3 = 0;
    o3 += m3 * n3;
    o2 += o3 >>> 16;
    o3 &= 0xffff;
    o2 += m2 * n3;
    o1 += o2 >>> 16;
    o2 &= 0xffff;
    o2 += m3 * n2;
    o1 += o2 >>> 16;
    o2 &= 0xffff;
    o1 += m1 * n3;
    o0 += o1 >>> 16;
    o1 &= 0xffff;
    o1 += m2 * n2;
    o0 += o1 >>> 16;
    o1 &= 0xffff;
    o1 += m3 * n1;
    o0 += o1 >>> 16;
    o1 &= 0xffff;
    o0 += m0 * n3 + m1 * n2 + m2 * n1 + m3 * n0;
    o0 &= 0xffff;
    high = _u32((o0 << 16) | o1);
    low = _u32((o2 << 16) | o3);
  }

  void rotl(int bits) {
    bits %= 64;
    final m0 = high;
    if (bits == 0) {
      return;
    } else if (bits == 32) {
      high = low;
      low = m0;
    } else if (bits < 32) {
      high = _u32((m0 << bits) | (low >>> (32 - bits)));
      low = _u32((low << bits) | (m0 >>> (32 - bits)));
    } else {
      bits -= 32;
      high = _u32((low << bits) | (m0 >>> (32 - bits)));
      low = _u32((m0 << bits) | (low >>> (32 - bits)));
    }
  }

  void xor(_X64 n) {
    high = _u32(high ^ n.high);
    low = _u32(low ^ n.low);
  }

  void xorByte(int byte, int shift) {
    if (shift < 32) {
      low = _u32(low ^ (byte << shift));
    } else {
      high = _u32(high ^ (byte << (shift - 32)));
    }
  }
}

final _f1 = _X64(0xff51afd7, 0xed558ccd);
final _f2 = _X64(0xc4ceb9fe, 0x1a85ec53);
final _c1 = _X64(0x87c37b91, 0x114253d5);
final _c2 = _X64(0x4cf5ad43, 0x2745937f);
final _m5 = _X64(0, 5);
final _n1 = _X64(0, 0x52dce729);
final _n2 = _X64(0, 0x38495ab5);

void _fmix(_X64 h) {
  h.xor(_X64(0, h.high >>> 1));
  h.multiply(_f1);
  h.xor(_X64(0, h.high >>> 1));
  h.multiply(_f2);
  h.xor(_X64(0, h.high >>> 1));
}

int _u32le(List<int> key, int i) =>
    _u32(key[i] | (key[i + 1] << 8) | (key[i + 2] << 16) | (key[i + 3] << 24));

String _hex32(int n) => n.toRadixString(16).padLeft(8, '0');

/// UTF-8 bytes of a JS string: ASCII `charCodeAt` fast path, otherwise
/// TextEncoder semantics (unpaired surrogates become U+FFFD).
List<int> getUtf8Bytes(String input) {
  final length = input.length;
  final ascii = List<int>.filled(length, 0);
  for (var i = 0; i < length; i++) {
    final code = input.codeUnitAt(i);
    if (code > 127) {
      return _utf8EncodeJs(input);
    }
    ascii[i] = code;
  }
  return ascii;
}

List<int> _utf8EncodeJs(String input) {
  final out = <int>[];
  for (var i = 0; i < input.length; i++) {
    final cu = input.codeUnitAt(i);
    int code;
    if (cu >= 0xd800 && cu <= 0xdbff) {
      if (i + 1 < input.length) {
        final cu2 = input.codeUnitAt(i + 1);
        if (cu2 >= 0xdc00 && cu2 <= 0xdfff) {
          code = 0x10000 + ((cu - 0xd800) << 10) + (cu2 - 0xdc00);
          i++;
        } else {
          code = 0xfffd;
        }
      } else {
        code = 0xfffd;
      }
    } else if (cu >= 0xdc00 && cu <= 0xdfff) {
      code = 0xfffd;
    } else {
      code = cu;
    }
    if (code <= 0x7f) {
      out.add(code);
    } else if (code <= 0x7ff) {
      out.add(0xc0 | (code >> 6));
      out.add(0x80 | (code & 0x3f));
    } else if (code <= 0xffff) {
      out.add(0xe0 | (code >> 12));
      out.add(0x80 | ((code >> 6) & 0x3f));
      out.add(0x80 | (code & 0x3f));
    } else {
      out.add(0xf0 | (code >> 18));
      out.add(0x80 | ((code >> 12) & 0x3f));
      out.add(0x80 | ((code >> 6) & 0x3f));
      out.add(0x80 | (code & 0x3f));
    }
  }
  return out;
}

String x64hash128(String input, [int seed = 0]) {
  return x64hash128Bytes(getUtf8Bytes(input), seed);
}

String x64hash128Bytes(List<int> key, int seed) {
  seed = _u32(seed);
  final lengthVal = _X64(0, key.length);
  final remainder = key.length % 16;
  final bytes = key.length - remainder;
  final h1 = _X64(0, seed);
  final h2 = _X64(0, seed);
  var i = 0;
  while (i < bytes) {
    final k1 = _X64(_u32le(key, i + 4), _u32le(key, i));
    final k2 = _X64(_u32le(key, i + 12), _u32le(key, i + 8));
    k1.multiply(_c1);
    k1.rotl(31);
    k1.multiply(_c2);
    h1.xor(k1);
    h1.rotl(27);
    h1.add(h2);
    h1.multiply(_m5);
    h1.add(_n1);
    k2.multiply(_c2);
    k2.rotl(33);
    k2.multiply(_c1);
    h2.xor(k2);
    h2.rotl(31);
    h2.add(h1);
    h2.multiply(_m5);
    h2.add(_n2);
    i += 16;
  }
  final k1 = _X64(0, 0);
  final k2 = _X64(0, 0);
  final tail = bytes;
  for (var offset = 0; offset < remainder && offset < 8; offset++) {
    k1.xorByte(key[tail + offset], offset * 8);
  }
  for (var offset = 0; offset < remainder - 8; offset++) {
    k2.xorByte(key[tail + 8 + offset], offset * 8);
  }
  if (remainder > 8) {
    k2.multiply(_c2);
    k2.rotl(33);
    k2.multiply(_c1);
    h2.xor(k2);
  }
  if (remainder > 0) {
    k1.multiply(_c1);
    k1.rotl(31);
    k1.multiply(_c2);
    h1.xor(k1);
  }
  h1.xor(lengthVal);
  h2.xor(lengthVal);
  h1.add(h2);
  h2.add(h1);
  _fmix(h1);
  _fmix(h2);
  h1.add(h2);
  h2.add(h1);
  return _hex32(h1.high) + _hex32(h1.low) + _hex32(h2.high) + _hex32(h2.low);
}
