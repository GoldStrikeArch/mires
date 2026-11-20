/// noble's sha2.ts: SHA-256 (SHA2_32B) and SHA-512 (SHA2_64B), with the IVs from _md.ts.
library;

import 'dart:typed_data';

import 'md.dart';
import 'u64.dart';
import 'utils.dart';

const sha256IV = <int>[
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19, //
];

const _sha256K = <int>[
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// noble's SHA2_32B with the SHA-256 IV.
final class Sha256 extends HashMD {
  Sha256() : super(64, 32, 8);

  int _a = sha256IV[0];
  int _b = sha256IV[1];
  int _c = sha256IV[2];
  int _d = sha256IV[3];
  int _e = sha256IV[4];
  int _f = sha256IV[5];
  int _g = sha256IV[6];
  int _h = sha256IV[7];

  /// The message schedule; noble shares one module-level SHA256_W.
  final _w = Uint32List(64);

  @override
  List<int> words() => [_a, _b, _c, _d, _e, _f, _g, _h];

  @override
  void process(ByteData view, int offset) {
    final w = _w;
    for (var i = 0; i < 16; i++, offset += 4) {
      w[i] = view.getUint32(offset);
    }
    for (var i = 16; i < 64; i++) {
      final w15 = w[i - 15];
      final w2 = w[i - 2];
      final s0 = rotr(w15, 7) ^ rotr(w15, 18) ^ (w15 >> 3);
      final s1 = rotr(w2, 17) ^ rotr(w2, 19) ^ (w2 >> 10);
      w[i] = s1 + w[i - 7] + s0 + w[i - 16];
    }
    var a = _a, b = _b, c = _c, d = _d, e = _e, f = _f, g = _g, h = _h;
    for (var i = 0; i < 64; i++) {
      final sigma1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      final t1 = (h + sigma1 + chi(e, f, g) + _sha256K[i] + w[i]) & mask32;
      final sigma0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      final t2 = (sigma0 + maj(a, b, c)) & mask32;
      h = g;
      g = f;
      f = e;
      e = (d + t1) & mask32;
      d = c;
      c = b;
      b = a;
      a = (t1 + t2) & mask32;
    }
    _a = (_a + a) & mask32;
    _b = (_b + b) & mask32;
    _c = (_c + c) & mask32;
    _d = (_d + d) & mask32;
    _e = (_e + e) & mask32;
    _f = (_f + f) & mask32;
    _g = (_g + g) & mask32;
    _h = (_h + h) & mask32;
  }
}

const _sha512IV = <int>[
  0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b, 0xa54ff53a, 0x5f1d36f1,
  0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f, 0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179,
];

// The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is.
const _sha512Kh = <int>[
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  0xca273ece, 0xd186b8c7, 0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35,
  0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c, 0x5fcb6fab, 0x6c44198c,
];

const _sha512Kl = <int>[
  0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019, 0xaf194f9b, 0xda6d8118,
  0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2, 0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694,
  0x9ef14ad2, 0x384f25e3, 0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5,
  0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725, 0xe003826f, 0x0a0e6e70,
  0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df, 0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b,
  0x4cf10364, 0xbc423001, 0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8,
  0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb, 0x7763e373, 0xd6b2b8a3,
  0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec, 0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b,
  0xea26619c, 0x21c0c207, 0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b,
  0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a, 0x3ad6faec, 0x4a475817,
];

/// noble's SHA2_64B with the SHA-512 IV: eight 64-bit words as high and low halves.
final class Sha512 extends HashMD {
  Sha512() : super(128, 64, 16);

  /// Ah, Al, Bh, Bl, ..., Hh, Hl.
  final _state = Uint32List.fromList(_sha512IV);

  /// The high and low halves of the message schedule (noble's SHA512_W_H and SHA512_W_L).
  final _wh = Uint32List(80);
  final _wl = Uint32List(80);

  @override
  List<int> words() => _state;

  @override
  void process(ByteData view, int offset) {
    final wh = _wh, wl = _wl;
    for (var i = 0; i < 16; i++, offset += 8) {
      wh[i] = view.getUint32(offset);
      wl[i] = view.getUint32(offset + 4);
    }
    for (var i = 16; i < 80; i++) {
      // s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7)
      final w15h = wh[i - 15], w15l = wl[i - 15];
      final s0h = rotrSH(w15h, w15l, 1) ^ rotrSH(w15h, w15l, 8) ^ shrSH(w15h, w15l, 7);
      final s0l = rotrSL(w15h, w15l, 1) ^ rotrSL(w15h, w15l, 8) ^ shrSL(w15h, w15l, 7);
      // s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6)
      final w2h = wh[i - 2], w2l = wl[i - 2];
      final s1h = rotrSH(w2h, w2l, 19) ^ rotrBH(w2h, w2l, 61) ^ shrSH(w2h, w2l, 6);
      final s1l = rotrSL(w2h, w2l, 19) ^ rotrBL(w2h, w2l, 61) ^ shrSL(w2h, w2l, 6);
      final sumL = add4L(s0l, s1l, wl[i - 7], wl[i - 16]);
      wh[i] = add4H(sumL, s0h, s1h, wh[i - 7], wh[i - 16]);
      wl[i] = sumL;
    }
    final state = _state;
    var ah = state[0], al = state[1], bh = state[2], bl = state[3];
    var ch = state[4], cl = state[5], dh = state[6], dl = state[7];
    var eh = state[8], el = state[9], fh = state[10], fl = state[11];
    var gh = state[12], gl = state[13], hh = state[14], hl = state[15];
    for (var i = 0; i < 80; i++) {
      // S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
      final sigma1h = rotrSH(eh, el, 14) ^ rotrSH(eh, el, 18) ^ rotrBH(eh, el, 41);
      final sigma1l = rotrSL(eh, el, 14) ^ rotrSL(eh, el, 18) ^ rotrBL(eh, el, 41);
      final chiH = chi(eh, fh, gh), chiL = chi(el, fl, gl);
      // T1 = H + S1 + Chi(E, F, G) + K[i] + W[i]
      final t1ll = add5L(hl, sigma1l, chiL, _sha512Kl[i], wl[i]);
      final t1h = add5H(t1ll, hh, sigma1h, chiH, _sha512Kh[i], wh[i]);
      final t1l = t1ll & mask32;
      // S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
      final sigma0h = rotrSH(ah, al, 28) ^ rotrBH(ah, al, 34) ^ rotrBH(ah, al, 39);
      final sigma0l = rotrSL(ah, al, 28) ^ rotrBL(ah, al, 34) ^ rotrBL(ah, al, 39);
      final majH = maj(ah, bh, ch), majL = maj(al, bl, cl);
      hh = gh;
      hl = gl;
      gh = fh;
      gl = fl;
      fh = eh;
      fl = el;
      (h: eh, l: el) = add(dh, dl, t1h, t1l);
      dh = ch;
      dl = cl;
      ch = bh;
      cl = bl;
      bh = ah;
      bl = al;
      final all = add3L(t1l, sigma0l, majL);
      ah = add3H(all, t1h, sigma0h, majH);
      al = all & mask32;
    }
    // Add the compressed block to the state.
    final working = [ah, al, bh, bl, ch, cl, dh, dl, eh, el, fh, fl, gh, gl, hh, hl];
    for (var i = 0; i < 16; i += 2) {
      final (:h, :l) = add(state[i], state[i + 1], working[i], working[i + 1]);
      state[i] = h;
      state[i + 1] = l;
    }
  }
}
