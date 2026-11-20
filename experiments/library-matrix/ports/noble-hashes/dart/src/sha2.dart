// SHA-256 and SHA-512 compression and streaming HMAC, adapted from @noble/hashes 2.4.0.
import 'dart:typed_data';

final shaIV256 = Uint32List.fromList([
  0x6a09e667,
  0xbb67ae85,
  0x3c6ef372,
  0xa54ff53a,
  0x510e527f,
  0x9b05688c,
  0x1f83d9ab,
  0x5be0cd19,
]);

final shaIV512 = Uint32List.fromList([
  0x6a09e667,
  0xf3bcc908,
  0xbb67ae85,
  0x84caa73b,
  0x3c6ef372,
  0xfe94f82b,
  0xa54ff53a,
  0x5f1d36f1,
  0x510e527f,
  0xade682d1,
  0x9b05688c,
  0x2b3e6c1f,
  0x1f83d9ab,
  0xfb41bd6b,
  0x5be0cd19,
  0x137e2179,
]);

final shaK256 = Uint32List.fromList([
  0x428a2f98,
  0x71374491,
  0xb5c0fbcf,
  0xe9b5dba5,
  0x3956c25b,
  0x59f111f1,
  0x923f82a4,
  0xab1c5ed5,
  0xd807aa98,
  0x12835b01,
  0x243185be,
  0x550c7dc3,
  0x72be5d74,
  0x80deb1fe,
  0x9bdc06a7,
  0xc19bf174,
  0xe49b69c1,
  0xefbe4786,
  0x0fc19dc6,
  0x240ca1cc,
  0x2de92c6f,
  0x4a7484aa,
  0x5cb0a9dc,
  0x76f988da,
  0x983e5152,
  0xa831c66d,
  0xb00327c8,
  0xbf597fc7,
  0xc6e00bf3,
  0xd5a79147,
  0x06ca6351,
  0x14292967,
  0x27b70a85,
  0x2e1b2138,
  0x4d2c6dfc,
  0x53380d13,
  0x650a7354,
  0x766a0abb,
  0x81c2c92e,
  0x92722c85,
  0xa2bfe8a1,
  0xa81a664b,
  0xc24b8b70,
  0xc76c51a3,
  0xd192e819,
  0xd6990624,
  0xf40e3585,
  0x106aa070,
  0x19a4c116,
  0x1e376c08,
  0x2748774c,
  0x34b0bcb5,
  0x391c0cb3,
  0x4ed8aa4a,
  0x5b9cca4f,
  0x682e6ff3,
  0x748f82ee,
  0x78a5636f,
  0x84c87814,
  0x8cc70208,
  0x90befffa,
  0xa4506ceb,
  0xbef9a3f7,
  0xc67178f2,
]);

final shaK512h = Uint32List.fromList([
  0x428a2f98,
  0x71374491,
  0xb5c0fbcf,
  0xe9b5dba5,
  0x3956c25b,
  0x59f111f1,
  0x923f82a4,
  0xab1c5ed5,
  0xd807aa98,
  0x12835b01,
  0x243185be,
  0x550c7dc3,
  0x72be5d74,
  0x80deb1fe,
  0x9bdc06a7,
  0xc19bf174,
  0xe49b69c1,
  0xefbe4786,
  0x0fc19dc6,
  0x240ca1cc,
  0x2de92c6f,
  0x4a7484aa,
  0x5cb0a9dc,
  0x76f988da,
  0x983e5152,
  0xa831c66d,
  0xb00327c8,
  0xbf597fc7,
  0xc6e00bf3,
  0xd5a79147,
  0x06ca6351,
  0x14292967,
  0x27b70a85,
  0x2e1b2138,
  0x4d2c6dfc,
  0x53380d13,
  0x650a7354,
  0x766a0abb,
  0x81c2c92e,
  0x92722c85,
  0xa2bfe8a1,
  0xa81a664b,
  0xc24b8b70,
  0xc76c51a3,
  0xd192e819,
  0xd6990624,
  0xf40e3585,
  0x106aa070,
  0x19a4c116,
  0x1e376c08,
  0x2748774c,
  0x34b0bcb5,
  0x391c0cb3,
  0x4ed8aa4a,
  0x5b9cca4f,
  0x682e6ff3,
  0x748f82ee,
  0x78a5636f,
  0x84c87814,
  0x8cc70208,
  0x90befffa,
  0xa4506ceb,
  0xbef9a3f7,
  0xc67178f2,
  0xca273ece,
  0xd186b8c7,
  0xeada7dd6,
  0xf57d4f7f,
  0x06f067aa,
  0x0a637dc5,
  0x113f9804,
  0x1b710b35,
  0x28db77f5,
  0x32caab7b,
  0x3c9ebe0a,
  0x431d67c4,
  0x4cc5d4be,
  0x597f299c,
  0x5fcb6fab,
  0x6c44198c,
]);

final shaK512l = Uint32List.fromList([
  0xd728ae22,
  0x23ef65cd,
  0xec4d3b2f,
  0x8189dbbc,
  0xf348b538,
  0xb605d019,
  0xaf194f9b,
  0xda6d8118,
  0xa3030242,
  0x45706fbe,
  0x4ee4b28c,
  0xd5ffb4e2,
  0xf27b896f,
  0x3b1696b1,
  0x25c71235,
  0xcf692694,
  0x9ef14ad2,
  0x384f25e3,
  0x8b8cd5b5,
  0x77ac9c65,
  0x592b0275,
  0x6ea6e483,
  0xbd41fbd4,
  0x831153b5,
  0xee66dfab,
  0x2db43210,
  0x98fb213f,
  0xbeef0ee4,
  0x3da88fc2,
  0x930aa725,
  0xe003826f,
  0x0a0e6e70,
  0x46d22ffc,
  0x5c26c926,
  0x5ac42aed,
  0x9d95b3df,
  0x8baf63de,
  0x3c77b2a8,
  0x47edaee6,
  0x1482353b,
  0x4cf10364,
  0xbc423001,
  0xd0f89791,
  0x0654be30,
  0xd6ef5218,
  0x5565a910,
  0x5771202a,
  0x32bbd1b8,
  0xb8d2d0c8,
  0x5141ab53,
  0xdf8eeb99,
  0xe19b48a8,
  0xc5c95a63,
  0xe3418acb,
  0x7763e373,
  0xd6b2b8a3,
  0x5defb2fc,
  0x43172f60,
  0xa1f0ab72,
  0x1a6439ec,
  0x23631e28,
  0xde82bde9,
  0xb2c67915,
  0xe372532b,
  0xea26619c,
  0x21c0c207,
  0xcde0eb1e,
  0xee6ed178,
  0x72176fba,
  0xa2c898a6,
  0xbef90dae,
  0x131c471b,
  0x23047d84,
  0x40c72493,
  0x15c9bebc,
  0x9c100d4c,
  0xcb3e42b6,
  0xfc657e2a,
  0x3ad6faec,
  0x4a475817,
]);

int shaRor(int x, int n) {
  return ((x >>> n) | (x << (32 - n)));
}

int shaChoice(int a, int b, int c) {
  return ((a & b) ^ (~a & c));
}

int shaMajority(int a, int b, int c) {
  return (((a & b) ^ (a & c)) ^ (b & c));
}

int shaCarry(int a, int b) {
  return (a + b).toUnsigned(32) < a.toUnsigned(32) ? 1 : 0;
}

int shaRead(Uint8List data, int at) {
  return ((((data[at] << 24) | (data[at + 1] << 16)) | (data[at + 2] << 8)) |
      data[at + 3]);
}

void shaCompress256(Uint32List state, Uint8List data, int at, Uint32List wh) {
  for (var i = 0; i < 16; i++) {
    wh[i] = shaRead(data, at + i * 4);
  }
  for (var i = 16; i < 64; i++) {
    final x = wh[i - 15];
    final y = wh[i - 2];
    final s0 = ((shaRor(x, 7) ^ shaRor(x, 18)) ^ (x >>> 3));
    final s1 = ((shaRor(y, 17) ^ shaRor(y, 19)) ^ (y >>> 10));
    wh[i] = (s1 + wh[i - 7] + s0 + wh[i - 16]).toUnsigned(32);
  }
  var a = state[0];
  var b = state[1];
  var c = state[2];
  var d = state[3];
  var e = state[4];
  var f = state[5];
  var g = state[6];
  var h = state[7];
  for (var i = 0; i < 64; i++) {
    final s1 = ((shaRor(e, 6) ^ shaRor(e, 11)) ^ shaRor(e, 25));
    final t1 = (h + s1 + shaChoice(e, f, g) + shaK256[i] + wh[i]).toUnsigned(
      32,
    );
    final s0 = ((shaRor(a, 2) ^ shaRor(a, 13)) ^ shaRor(a, 22));
    final newA = (t1 + s0 + shaMajority(a, b, c)).toUnsigned(32);
    final newE = (d + t1).toUnsigned(32);
    h = g;
    g = f;
    f = e;
    e = newE;
    d = c;
    c = b;
    b = a;
    a = newA;
  }
  state[0] = (state[0] + a).toUnsigned(32);
  state[1] = (state[1] + b).toUnsigned(32);
  state[2] = (state[2] + c).toUnsigned(32);
  state[3] = (state[3] + d).toUnsigned(32);
  state[4] = (state[4] + e).toUnsigned(32);
  state[5] = (state[5] + f).toUnsigned(32);
  state[6] = (state[6] + g).toUnsigned(32);
  state[7] = (state[7] + h).toUnsigned(32);
}

void shaCompress512(
  Uint32List state,
  Uint8List data,
  int at,
  Uint32List wh,
  Uint32List wl,
) {
  for (var i = 0; i < 16; i++) {
    wh[i] = shaRead(data, at + i * 8);
    wl[i] = shaRead(data, at + i * 8 + 4);
  }
  for (var i = 16; i < 80; i++) {
    final xh = wh[i - 15];
    final xl = wl[i - 15];
    final yh = wh[i - 2];
    final yl = wl[i - 2];
    final s0h =
        ((((xh >>> 1) | (xl << 31)) ^ ((xh >>> 8) | (xl << 24))) ^ (xh >>> 7));
    final s0l =
        ((((xl >>> 1) | (xh << 31)) ^ ((xl >>> 8) | (xh << 24))) ^
        ((xl >>> 7) | (xh << 25)));
    final s1h =
        ((((yh >>> 19) | (yl << 13)) ^ ((yl >>> 29) | (yh << 3))) ^ (yh >>> 6));
    final s1l =
        ((((yl >>> 19) | (yh << 13)) ^ ((yh >>> 29) | (yl << 3))) ^
        ((yl >>> 6) | (yh << 26)));
    final nextl2 = (s0l + s1l).toUnsigned(32);
    final nextl3 = (nextl2 + wl[i - 7]).toUnsigned(32);
    final nextl = (nextl3 + wl[i - 16]).toUnsigned(32);
    final nexth =
        (s0h +
                s1h +
                wh[i - 7] +
                wh[i - 16] +
                shaCarry(s0l, s1l) +
                shaCarry(nextl2, wl[i - 7]) +
                shaCarry(nextl3, wl[i - 16]))
            .toUnsigned(32);
    wh[i] = nexth;
    wl[i] = nextl;
  }
  var ah = state[0];
  var al = state[1];
  var bh = state[2];
  var bl = state[3];
  var ch = state[4];
  var cl = state[5];
  var dh = state[6];
  var dl = state[7];
  var eh = state[8];
  var el = state[9];
  var fh = state[10];
  var fl = state[11];
  var gh = state[12];
  var gl = state[13];
  var hh = state[14];
  var hl = state[15];
  for (var i = 0; i < 80; i++) {
    final s1h =
        ((((eh >>> 14) | (el << 18)) ^ ((eh >>> 18) | (el << 14))) ^
        ((el >>> 9) | (eh << 23)));
    final s1l =
        ((((el >>> 14) | (eh << 18)) ^ ((el >>> 18) | (eh << 14))) ^
        ((eh >>> 9) | (el << 23)));
    final chh = shaChoice(eh, fh, gh);
    final chl = shaChoice(el, fl, gl);
    final t1l2 = (hl + s1l).toUnsigned(32);
    final t1l3 = (t1l2 + chl).toUnsigned(32);
    final t1l4 = (t1l3 + shaK512l[i]).toUnsigned(32);
    final t1l = (t1l4 + wl[i]).toUnsigned(32);
    final t1h =
        (hh +
                s1h +
                chh +
                shaK512h[i] +
                wh[i] +
                shaCarry(hl, s1l) +
                shaCarry(t1l2, chl) +
                shaCarry(t1l3, shaK512l[i]) +
                shaCarry(t1l4, wl[i]))
            .toUnsigned(32);
    final s0h =
        ((((ah >>> 28) | (al << 4)) ^ ((al >>> 2) | (ah << 30))) ^
        ((al >>> 7) | (ah << 25)));
    final s0l =
        ((((al >>> 28) | (ah << 4)) ^ ((ah >>> 2) | (al << 30))) ^
        ((ah >>> 7) | (al << 25)));
    final majh = shaMajority(ah, bh, ch);
    final majl = shaMajority(al, bl, cl);
    final newAl2 = (t1l + s0l).toUnsigned(32);
    final newAl = (newAl2 + majl).toUnsigned(32);
    final newAh =
        (t1h + s0h + majh + shaCarry(t1l, s0l) + shaCarry(newAl2, majl))
            .toUnsigned(32);
    final newEl = (dl + t1l).toUnsigned(32);
    final newEh = (dh + t1h + shaCarry(dl, t1l)).toUnsigned(32);
    hh = gh;
    hl = gl;
    gh = fh;
    gl = fl;
    fh = eh;
    fl = el;
    eh = newEh;
    el = newEl;
    dh = ch;
    dl = cl;
    ch = bh;
    cl = bl;
    bh = ah;
    bl = al;
    ah = newAh;
    al = newAl;
  }
  state[0] = (state[0] + ah + shaCarry(state[1], al)).toUnsigned(32);
  state[1] = (state[1] + al).toUnsigned(32);
  state[2] = (state[2] + bh + shaCarry(state[3], bl)).toUnsigned(32);
  state[3] = (state[3] + bl).toUnsigned(32);
  state[4] = (state[4] + ch + shaCarry(state[5], cl)).toUnsigned(32);
  state[5] = (state[5] + cl).toUnsigned(32);
  state[6] = (state[6] + dh + shaCarry(state[7], dl)).toUnsigned(32);
  state[7] = (state[7] + dl).toUnsigned(32);
  state[8] = (state[8] + eh + shaCarry(state[9], el)).toUnsigned(32);
  state[9] = (state[9] + el).toUnsigned(32);
  state[10] = (state[10] + fh + shaCarry(state[11], fl)).toUnsigned(32);
  state[11] = (state[11] + fl).toUnsigned(32);
  state[12] = (state[12] + gh + shaCarry(state[13], gl)).toUnsigned(32);
  state[13] = (state[13] + gl).toUnsigned(32);
  state[14] = (state[14] + hh + shaCarry(state[15], hl)).toUnsigned(32);
  state[15] = (state[15] + hl).toUnsigned(32);
}

Uint8List _hash(bool wide, List<Uint8List> chunks) {
  final block = wide ? 128 : 64;
  final state = Uint32List.fromList(wide ? shaIV512 : shaIV256);
  final wh = Uint32List(80);
  final wl = Uint32List(80);
  final tail = Uint8List(block);
  void process(Uint8List data, int at) {
    if (wide) {
      shaCompress512(state, data, at, wh, wl);
    } else {
      shaCompress256(state, data, at, wh);
    }
  }

  var pos = 0;
  var length = 0;
  for (final chunk in chunks) {
    var at = 0;
    length += chunk.length;
    while (at < chunk.length) {
      if (pos == 0 && chunk.length - at >= block) {
        process(chunk, at);
        at += block;
      } else {
        final take = (chunk.length - at).clamp(0, block - pos);
        tail.setRange(pos, pos + take, chunk, at);
        pos += take;
        at += take;
        if (pos == block) {
          process(tail, 0);
          pos = 0;
        }
      }
    }
  }
  tail[pos] = 128;
  tail.fillRange(pos + 1, block, 0);
  if (pos + 1 > block - block ~/ 8) {
    process(tail, 0);
    tail.fillRange(0, block, 0);
  }
  var bits = length * 8;
  for (var i = 0; i < 8; i++) {
    tail[block - 1 - i] = bits % 256;
    bits = bits ~/ 256;
  }
  process(tail, 0);
  final result = Uint8List(state.length * 4);
  for (var i = 0; i < result.length; i++) {
    result[i] = state[i ~/ 4] >>> (24 - i % 4 * 8);
  }
  return result;
}

Uint8List sha2Digest(int mode, List<Uint8List> chunks, Uint8List key) {
  if (mode != 2) return _hash(mode == 1, chunks);
  final k = key.length > 64 ? _hash(false, [key]) : key;
  final inner = Uint8List(64), outer = Uint8List(64);
  for (var i = 0; i < 64; i++) {
    final b = i < k.length ? k[i] : 0;
    inner[i] = b ^ 0x36;
    outer[i] = b ^ 0x5c;
  }
  return _hash(false, [
    outer,
    _hash(false, [inner, ...chunks]),
  ]);
}
