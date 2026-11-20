// DEFLATE compression: wbits, wbits16, wfblk, wblk, dflt, dopt, deflateSync.
// Follows fflate 0.8.3 algorithms, tie-breaks, and quirks bit-for-bit.

import 'dart:typed_data';

import 'huffman.dart';
import 'inflate.dart';
import 'tables.dart';

final class DeflateOptions {
  const DeflateOptions({
    this.level,
    this.mem,
    this.dictionary,
    this.mtime,
    this.filename,
  });

  final int? level;
  final int? mem;
  final Uint8List? dictionary;
  final double? mtime;
  final String? filename;
}

final class DeflateState {
  DeflateState({
    this.l = 0,
    this.i = 0,
    this.w = 0,
    this.z = 0,
    this.r = 0,
    this.p,
    this.h,
  });

  int l;
  int i;
  int w;
  int z;
  int r;
  Uint16List? p;
  Uint16List? h;
}

void wbits(Uint8List d, int p, int v) {
  final shifted = v << (p & 7);
  final o = p >>> 3;
  d[o] |= shifted & 0xFF;
  d[o + 1] |= (shifted >>> 8) & 0xFF;
}

void wbits16(Uint8List d, int p, int v) {
  final shifted = v << (p & 7);
  final o = p >>> 3;
  d[o] |= shifted & 0xFF;
  d[o + 1] |= (shifted >>> 8) & 0xFF;
  d[o + 2] |= (shifted >>> 16) & 0xFF;
}

/// Writes an uncompressed (stored) DEFLATE block; returns new bit position.
int wfblk(Uint8List out, int pos, Uint8List dat, int start, int length) {
  final o = shft(pos + 2);
  final lo = length & 255;
  final hi = (length >>> 8) & 255;
  out[o] = lo;
  out[o + 1] = hi;
  out[o + 2] = lo ^ 255;
  out[o + 3] = hi ^ 255;
  if (length > 0) {
    out.setRange(o + 4, o + 4 + length, dat, start);
  }
  return (o + 4 + length) * 8;
}

/// Selects stored/fixed/dynamic block encoding and writes it into `out` at bit `p`.
int wblk(
  Uint8List dat,
  Uint8List out,
  int isFinal,
  Int32List syms,
  Uint16List lf,
  Uint16List df,
  int eb,
  int li,
  int bs,
  int bl,
  int p,
) {
  final t = tables;
  final clim = t.clim;
  final fleb = t.fleb;
  final fdeb = t.fdeb;
  final flt = t.flt;
  final fdt = t.fdt;

  wbits(out, p++, isFinal);
  ++lf[256];

  final dltTree = hTree(lf, 15);
  final dlt = dltTree.t;
  final mlb = dltTree.l;
  final ddtTree = hTree(df, 15);
  final ddt = ddtTree.t;
  final mdb = ddtTree.l;

  final lcltRes = lc(dlt);
  final lclt = lcltRes.c;
  final nlc = lcltRes.n;
  final lcdtRes = lc(ddt);
  final lcdt = lcdtRes.c;
  final ndc = lcdtRes.n;

  final lcfreq = Uint16List(19);
  for (var i = 0; i < lclt.length; ++i) {
    ++lcfreq[lclt[i] & 31];
  }
  for (var i = 0; i < lcdt.length; ++i) {
    ++lcfreq[lcdt[i] & 31];
  }

  final lctTree = hTree(lcfreq, 7);
  final lct = lctTree.t;
  final mlcb = lctTree.l;
  final lctLen = lct.length;

  var nlcc = 19;
  while (nlcc > 4) {
    final cIdx = clim[nlcc - 1];
    if (cIdx < lctLen && lct[cIdx] != 0) break;
    --nlcc;
  }

  final flen = (bl + 5) << 3;
  final ftlen = clen(lf, flt) + clen(df, fdt) + eb;
  final dtlen = clen(lf, dlt) +
      clen(df, ddt) +
      eb +
      14 +
      3 * nlcc +
      clen(lcfreq, lct) +
      2 * lcfreq[16] +
      3 * lcfreq[17] +
      7 * lcfreq[18];

  if (bs >= 0 && flen <= ftlen && flen <= dtlen) {
    return wfblk(out, p, dat, bs, bl);
  }

  final Uint16List lm;
  final Uint8List ll;
  final Uint16List dm;
  final Uint8List dl;

  final useDynamic = dtlen < ftlen;
  wbits(out, p, useDynamic ? 2 : 1);
  p += 2;

  if (useDynamic) {
    final rev = t.rev;
    lm = buildHMap(dlt, mlb, false, rev);
    ll = dlt;
    dm = buildHMap(ddt, mdb, false, rev);
    dl = ddt;
    final llm = buildHMap(lct, mlcb, false, rev);
    wbits(out, p, nlc - 257);
    wbits(out, p + 5, ndc - 1);
    wbits(out, p + 10, nlcc - 4);
    p += 14;
    for (var i = 0; i < nlcc; ++i) {
      final cIdx = clim[i];
      wbits(out, p + 3 * i, cIdx < lctLen ? lct[cIdx] : 0);
    }
    p += 3 * nlcc;

    // Emit RLE-encoded literal/length then distance code-length sequences.
    for (var it = 0; it < 2; ++it) {
      final clct = it == 0 ? lclt : lcdt;
      final clctLen = clct.length;
      for (var i = 0; i < clctLen; ++i) {
        final entry = clct[i];
        final symLen = entry & 31;
        wbits(out, p, llm[symLen]);
        p += lct[symLen];
        if (symLen > 15) {
          wbits(out, p, (entry >>> 5) & 127);
          p += entry >>> 12;
        }
      }
    }
  } else {
    lm = t.flm;
    ll = flt;
    dm = t.fdm;
    dl = fdt;
  }

  for (var i = 0; i < li; ++i) {
    final sym = syms[i];
    if (sym > 255) {
      final len = (sym >>> 18) & 31;
      final lSym = len + 257;
      {
        final v = lm[lSym] << (p & 7);
        final o = p >>> 3;
        out[o] |= v & 0xFF;
        out[o + 1] |= (v >>> 8) & 0xFF;
        out[o + 2] |= (v >>> 16) & 0xFF;
        p += ll[lSym];
      }
      if (len > 7) {
        final v = ((sym >>> 23) & 31) << (p & 7);
        final o = p >>> 3;
        out[o] |= v & 0xFF;
        out[o + 1] |= (v >>> 8) & 0xFF;
        p += fleb[len];
      }
      final dst = sym & 31;
      {
        final v = dm[dst] << (p & 7);
        final o = p >>> 3;
        out[o] |= v & 0xFF;
        out[o + 1] |= (v >>> 8) & 0xFF;
        out[o + 2] |= (v >>> 16) & 0xFF;
        p += dl[dst];
      }
      if (dst > 3) {
        final v = ((sym >>> 5) & 8191) << (p & 7);
        final o = p >>> 3;
        out[o] |= v & 0xFF;
        out[o + 1] |= (v >>> 8) & 0xFF;
        out[o + 2] |= (v >>> 16) & 0xFF;
        p += fdeb[dst];
      }
    } else {
      final v = lm[sym] << (p & 7);
      final o = p >>> 3;
      out[o] |= v & 0xFF;
      out[o + 1] |= (v >>> 8) & 0xFF;
      out[o + 2] |= (v >>> 16) & 0xFF;
      p += ll[sym];
    }
  }
  wbits16(out, p, lm[256]);
  return p + ll[256];
}

int _defaultPlvl(int len) {
  if (len <= 2980) return 12;
  if (len <= 5806) return 13;
  if (len <= 11308) return 14;
  if (len <= 22026) return 15;
  if (len <= 42901) return 16;
  if (len <= 83561) return 17;
  if (len <= 162754) return 18;
  if (len <= 317003) return 19;
  return 20;
}

/// Compresses `dat` into raw DEFLATE bytes, reserving `pre` header and `post` trailer bytes.
Uint8List dflt(Uint8List dat, int lvl, int plvl, int pre, int post, DeflateState st) {
  final s = st.z != 0 ? st.z : dat.length;
  // +2 slack bytes inside `w` so wbits16 never writes out of bounds on final EOB.
  final o = Uint8List(pre + s + 5 * (1 + ((s + 6999) ~/ 7000)) + post + 2);
  final w = Uint8List.sublistView(o, pre, o.length - post);
  final lst = st.l;
  var pos = st.r & 7;

  if (lvl != 0) {
    final t = tables;
    final revfl = t.revfl;
    final revfd = t.revfd;
    final fleb = t.fleb;
    final fdeb = t.fdeb;

    if (pos != 0) w[0] = st.r >>> 3;
    final opt = t.deo[lvl - 1];
    final n = opt >>> 13;
    final c = opt & 8191;
    final msk = (1 << (plvl & 31)) - 1;
    final prev = st.p ?? Uint16List(32768);
    final head = st.h ?? Uint16List(msk + 1);
    final bs1 = ((plvl + 2) ~/ 3) & 31;
    final bs2 = (2 * bs1) & 31;

    final syms = Int32List(25000);
    final lf = Uint16List(288);
    final df = Uint16List(32);

    var lcCnt = 0;
    var eb = 0;
    var i = st.i;
    var li = 0;
    var wi = st.w;
    var bs = 0;

    for (; i + 2 < s; ++i) {
      final hv = (dat[i] ^ (dat[i + 1] << bs1) ^ (dat[i + 2] << bs2)) & msk;
      var imod = i & 32767;
      var pimod = head[hv];
      prev[imod] = pimod;
      head[hv] = imod;

      if (wi <= i) {
        final rem = s - i;
        if ((lcCnt > 7000 || li > 24576) && (rem > 423 || lst == 0)) {
          pos = wblk(dat, w, 0, syms, lf, df, eb, li, bs, i - bs, pos);
          li = 0;
          lcCnt = 0;
          eb = 0;
          bs = i;
          for (var j = 0; j < 286; ++j) {
            lf[j] = 0;
          }
          for (var j = 0; j < 30; ++j) {
            df[j] = 0;
          }
        }

        var l = 2;
        var d = 0;
        var ch = c;
        var dif = (imod - pimod) & 32767;
        if (rem > 2 &&
            i >= dif &&
            hv == ((dat[i - dif] ^ (dat[i - dif + 1] << bs1) ^ (dat[i - dif + 2] << bs2)) & msk)) {
          final maxn = (n < rem ? n : rem) - 1;
          final maxd = 32767 < i ? 32767 : i;
          final ml = 258 < rem ? 258 : rem;
          while (dif <= maxd && --ch != 0 && imod != pimod) {
            if (dat[i + l] == dat[i + l - dif]) {
              var nl = 0;
              final base = i - dif;
              while (nl < ml && dat[i + nl] == dat[base + nl]) {
                ++nl;
              }
              if (nl > l) {
                l = nl;
                d = dif;
                if (nl > maxn) break;
                final nl2 = nl - 2;
                final mmd = dif < nl2 ? dif : nl2;
                var md = 0;
                for (var j = 0; j < mmd; ++j) {
                  final ti = (base + j) & 32767;
                  final pti = prev[ti];
                  final cd = (ti - pti) & 32767;
                  if (cd > md) {
                    md = cd;
                    pimod = ti;
                  }
                }
              }
            }
            imod = pimod;
            pimod = prev[imod];
            dif += (imod - pimod) & 32767;
          }
        }

        if (d != 0) {
          final rfl = revfl[l];
          final rfd = revfd[d];
          syms[li++] = 268435456 | (rfl << 18) | rfd;
          final lin = rfl & 31;
          final din = rfd & 31;
          eb += fleb[lin] + fdeb[din];
          ++lf[257 + lin];
          ++df[din];
          wi = i + l;
          ++lcCnt;
        } else {
          final b = dat[i];
          syms[li++] = b;
          ++lf[b];
        }
      }
    }

    if (wi > i) i = wi;
    for (; i < s; ++i) {
      final b = dat[i];
      syms[li++] = b;
      ++lf[b];
    }

    pos = wblk(dat, w, lst, syms, lf, df, eb, li, bs, i - bs, pos);
    if (lst == 0) {
      st.r = (pos & 7) | (w[pos >>> 3] << 3);
      pos -= 7;
      st.h = head;
      st.p = prev;
      st.i = i;
      st.w = wi;
    }
  } else {
    final limit = s + lst;
    for (var i = st.w; i < limit; i += 65535) {
      var e = i + 65535;
      if (e >= s) {
        w[pos >>> 3] = lst;
        e = s;
      }
      pos = wfblk(w, pos + 1, dat, i, e > i ? e - i : 0);
    }
    st.i = s;
  }

  return slc(o, 0, pre + shft(pos) + post);
}

Uint8List dopt(Uint8List dat, DeflateOptions opt, int pre, int post, [DeflateState? st]) {
  var state = st;
  var input = dat;
  if (state == null) {
    state = DeflateState(l: 1);
    final rawDict = opt.dictionary;
    if (rawDict != null && rawDict.isNotEmpty) {
      final dict = subarray(rawDict, -32768);
      final newDat = Uint8List(dict.length + dat.length);
      newDat.setRange(0, dict.length, dict);
      newDat.setRange(dict.length, newDat.length, dat);
      input = newDat;
      state.w = dict.length;
    }
  }
  final lvl = opt.level ?? 6;
  final plvl = opt.mem != null
      ? 12 + opt.mem!
      : (state.l != 0 ? _defaultPlvl(input.length) : 20);
  return dflt(input, lvl, plvl, pre, post, state);
}

Uint8List deflateSync(Uint8List data, [DeflateOptions opts = const DeflateOptions()]) =>
    dopt(data, opts, 0, 0);
