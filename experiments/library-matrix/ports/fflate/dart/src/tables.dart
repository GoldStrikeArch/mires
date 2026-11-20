// Precomputed immutable lookup tables matching fflate 0.8.3's module-load tables:
// fleb, fdeb, clim, fl, revfl, fd, revfd, rev, flt, fdt, flm, flrm, fdm, fdrm, deo, crct.
// No mutable cross-call state; every table is initialized once and only read.

import 'dart:typed_data';

final tables = Tables._init();

final class Tables {
  Tables._(
    this.fleb,
    this.fdeb,
    this.clim,
    this.fl,
    this.revfl,
    this.fd,
    this.revfd,
    this.rev,
    this.flt,
    this.fdt,
    this.flm,
    this.flrm,
    this.fdm,
    this.fdrm,
    this.deo,
    this.crct,
    this.emptyU8,
  );

  final Uint8List fleb;
  final Uint8List fdeb;
  final Uint8List clim;
  final Uint16List fl;
  final Int32List revfl;
  final Uint16List fd;
  final Int32List revfd;
  final Uint16List rev;
  final Uint8List flt;
  final Uint8List fdt;
  final Uint16List flm;
  final Uint16List flrm;
  final Uint16List fdm;
  final Uint16List fdrm;
  final Int32List deo;
  final Int32List crct;
  final Uint8List emptyU8;

  factory Tables._init() {
    final fleb = Uint8List.fromList(const [
      0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2,
      3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 0, 0, 0,
    ]);
    final fdeb = Uint8List.fromList(const [
      0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6,
      7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 0, 0,
    ]);
    final clim = Uint8List.fromList(const [
      16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ]);

    final (fl, revfl) = _freb(fleb, 2);
    fl[28] = 258;
    revfl[258] = 28;
    final (fd, revfd) = _freb(fdeb, 0);

    final rev = Uint16List(32768);
    for (var i = 0; i < 32768; ++i) {
      var x = ((i & 0xAAAA) >> 1) | ((i & 0x5555) << 1);
      x = ((x & 0xCCCC) >> 2) | ((x & 0x3333) << 2);
      x = ((x & 0xF0F0) >> 4) | ((x & 0x0F0F) << 4);
      rev[i] = (((x & 0xFF00) >> 8) | ((x & 0x00FF) << 8)) >> 1;
    }

    final flt = Uint8List(288);
    for (var i = 0; i < 144; ++i) {
      flt[i] = 8;
    }
    for (var i = 144; i < 256; ++i) {
      flt[i] = 9;
    }
    for (var i = 256; i < 280; ++i) {
      flt[i] = 7;
    }
    for (var i = 280; i < 288; ++i) {
      flt[i] = 8;
    }

    final fdt = Uint8List(32);
    for (var i = 0; i < 32; ++i) {
      fdt[i] = 5;
    }

    final flm = buildHMap(flt, 9, false, rev);
    final flrm = buildHMap(flt, 9, true, rev);
    final fdm = buildHMap(fdt, 5, false, rev);
    final fdrm = buildHMap(fdt, 5, true, rev);

    final deo = Int32List.fromList(const [
      65540, 131080, 131088, 131104, 262176, 1048704, 1048832, 2114560, 2117632,
    ]);

    final crct = Int32List(256);
    for (var i = 0; i < 256; ++i) {
      var c = i;
      for (var k = 0; k < 8; ++k) {
        c = ((c & 1) != 0 ? 0xEDB88320 : 0) ^ (c >>> 1);
      }
      crct[i] = c;
    }

    return Tables._(
      fleb, fdeb, clim, fl, revfl, fd, revfd, rev,
      flt, fdt, flm, flrm, fdm, fdrm, deo, crct, Uint8List(0),
    );
  }

  static (Uint16List, Int32List) _freb(Uint8List eb, int start) {
    final b = Uint16List(31);
    var cur = start;
    for (var i = 0; i < 31; ++i) {
      cur += 1 << (i == 0 ? 0 : eb[i - 1]);
      b[i] = cur;
    }
    final r = Int32List(b[30]);
    for (var i = 1; i < 30; ++i) {
      final bi = b[i];
      final bNext = b[i + 1];
      for (var j = bi; j < bNext; ++j) {
        r[j] = ((j - bi) << 5) | i;
      }
    }
    return (b, r);
  }
}

/// Canonical Huffman encoding (`reverse == false`) or decoding (`reverse == true`) table.
Uint16List buildHMap(Uint8List cd, int mb, bool reverse, Uint16List rev) {
  final s = cd.length;
  final l = Uint16List(mb);
  for (var i = 0; i < s; ++i) {
    final len = cd[i];
    if (len != 0) ++l[len - 1];
  }
  final le = Uint16List(mb);
  for (var i = 1; i < mb; ++i) {
    le[i] = (le[i - 1] + l[i - 1]) << 1;
  }
  if (reverse) {
    final co = Uint16List(1 << mb);
    final rvb = 15 - mb;
    for (var i = 0; i < s; ++i) {
      final len = cd[i];
      if (len != 0) {
        final sv = (i << 4) | len;
        final r1 = mb - len;
        final code = le[len - 1];
        le[len - 1] = code + 1;
        var v = code << r1;
        final m = v | ((1 << r1) - 1);
        for (; v <= m; ++v) {
          co[(v < 32768 ? rev[v] : 0) >> rvb] = sv;
        }
      }
    }
    return co;
  } else {
    final co = Uint16List(s);
    for (var i = 0; i < s; ++i) {
      final len = cd[i];
      if (len != 0) {
        final code = le[len - 1];
        le[len - 1] = code + 1;
        co[i] = (code < 32768 ? rev[code] : 0) >> (15 - len);
      }
    }
    return co;
  }
}
