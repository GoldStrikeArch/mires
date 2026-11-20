// DEFLATE decompression: shft, subarray, slc, inflt, inflateSync.
// Follows fflate 0.8.3's inflate state machine bit-for-bit.

import 'dart:typed_data';

import 'errors.dart';
import 'tables.dart';

final class InflateOptions {
  const InflateOptions({this.out, this.dictionary});

  final Uint8List? out;
  final Uint8List? dictionary;
}

final class InflateState {
  InflateState({
    this.i = 0,
    this.f = 0,
    this.p = 0,
    this.b = 0,
    this.l,
    this.d,
    this.m = 0,
    this.n = 0,
  });

  /// Mode: 0 = streaming non-final, 1 = streaming final, 2 = one-shot sync.
  int i;
  int f;
  int p;
  int b;
  Uint16List? l;
  Uint16List? d;
  int m;
  int n;
}

int shft(int p) => (p + 7) >>> 3;

/// JS `TypedArray.prototype.subarray` semantics (negative indices + clamping).
Uint8List subarray(Uint8List v, int start, [int? end]) {
  final len = v.length;
  var s = start < 0 ? len + start : start;
  if (s < 0) s = 0;
  if (s > len) s = len;
  var e = end == null ? len : (end < 0 ? len + end : end);
  if (e < 0) e = 0;
  if (e > len) e = len;
  if (e <= s) return tables.emptyU8;
  return Uint8List.sublistView(v, s, e);
}

/// Copies `v[s..e]` into a freshly allocated `Uint8List`, clamping negative `s` to 0
/// and `e > v.length` to `v.length`, matching fflate's `slc`.
Uint8List slc(Uint8List v, int s, [int? e]) {
  final len = v.length;
  var start = s < 0 ? 0 : s;
  if (start > len) start = len;
  var end = e == null || e > len ? len : e;
  if (end <= start) return Uint8List(0);
  final out = Uint8List(end - start);
  out.setRange(0, end - start, v, start);
  return out;
}

int _bits(Uint8List d, int p, int m) {
  final o = p >> 3;
  final sl = d.length;
  final b0 = o < sl ? d[o] : 0;
  final b1 = o + 1 < sl ? d[o + 1] : 0;
  return ((b0 | (b1 << 8)) >> (p & 7)) & m;
}

int _maxU8(Uint8List a) {
  var m = a[0];
  final len = a.length;
  for (var i = 1; i < len; ++i) {
    final v = a[i];
    if (v > m) m = v;
  }
  return m;
}

Uint8List _growBuf(Uint8List buf, int needed) {
  final bl = buf.length;
  final dbl = bl * 2;
  final nbuf = Uint8List(dbl > needed ? dbl : needed);
  nbuf.setRange(0, bl, buf);
  return nbuf;
}

/// Expands raw DEFLATE data using `st` as resume state.
Uint8List inflt(Uint8List dat, InflateState st, Uint8List? outBuf, [Uint8List? dict]) {
  final sl = dat.length;
  final dl = dict != null ? dict.length : 0;
  if (sl == 0 || (st.f != 0 && st.l == null)) {
    return outBuf ?? Uint8List(0);
  }
  final noBuf = outBuf == null;
  final noSt = st.i;
  final resize = noBuf || noSt != 2;
  var buf = outBuf ?? Uint8List(sl * 3);
  var bufLen = buf.length;

  var isFinal = st.f;
  var pos = st.p;
  var bt = st.b;
  var lm = st.l;
  var dm = st.d;
  var lbt = st.m;
  var dbt = st.n;
  final tbts = sl * 8;

  final t = tables;
  final fleb = t.fleb;
  final fdeb = t.fdeb;
  final clim = t.clim;
  final fl = t.fl;
  final fd = t.fd;
  final flrm = t.flrm;
  final fdrm = t.fdrm;
  final rev = t.rev;

  do {
    if (lm == null) {
      isFinal = _bits(dat, pos, 1);
      final type = _bits(dat, pos + 1, 3);
      pos += 3;
      if (type == 0) {
        final s = shft(pos) + 4;
        final b0 = s - 4 < sl ? dat[s - 4] : 0;
        final b1 = s - 3 < sl ? dat[s - 3] : 0;
        final l = b0 | (b1 << 8);
        final endPos = s + l;
        if (endPos > sl) {
          if (noSt != 0) throw FlateException(FlateErrorCode.unexpectedEOF);
          break;
        }
        if (resize && bt + l > bufLen) {
          buf = _growBuf(buf, bt + l);
          bufLen = buf.length;
        }
        // Throws RangeError if !resize and bt + l > bufLen (matching JS TypedArray.set).
        buf.setRange(bt, bt + l, dat, s);
        bt += l;
        st.b = bt;
        pos = endPos * 8;
        st.p = pos;
        st.f = isFinal;
        continue;
      } else if (type == 1) {
        lm = flrm;
        dm = fdrm;
        lbt = 9;
        dbt = 5;
      } else if (type == 2) {
        final hLit = _bits(dat, pos, 31) + 257;
        final hcLen = _bits(dat, pos + 10, 15) + 4;
        final tl = hLit + _bits(dat, pos + 5, 31) + 1;
        pos += 14;
        final ldt = Uint8List(tl);
        final clt = Uint8List(19);
        for (var i = 0; i < hcLen; ++i) {
          clt[clim[i]] = _bits(dat, pos + i * 3, 7);
        }
        pos += hcLen * 3;
        final clb = _maxU8(clt);
        final clbmsk = (1 << clb) - 1;
        final clm = buildHMap(clt, clb, true, rev);
        for (var i = 0; i < tl;) {
          final r = clm[_bits(dat, pos, clbmsk)];
          pos += r & 15;
          final sym = r >> 4;
          if (sym < 16) {
            ldt[i++] = sym;
          } else {
            var c = 0;
            var n = 0;
            if (sym == 16) {
              n = 3 + _bits(dat, pos, 3);
              pos += 2;
              c = i > 0 ? ldt[i - 1] : 0;
            } else if (sym == 17) {
              n = 3 + _bits(dat, pos, 7);
              pos += 3;
            } else if (sym == 18) {
              n = 11 + _bits(dat, pos, 127);
              pos += 7;
            }
            while (n-- > 0) {
              if (i < tl) ldt[i] = c;
              ++i;
            }
          }
        }
        final lt = Uint8List.sublistView(ldt, 0, hLit);
        final dtArr = Uint8List.sublistView(ldt, hLit, tl);
        lbt = _maxU8(lt);
        dbt = _maxU8(dtArr);
        lm = buildHMap(lt, lbt, true, rev);
        dm = buildHMap(dtArr, dbt, true, rev);
      } else {
        throw FlateException(FlateErrorCode.invalidBlockType);
      }
      if (pos > tbts) {
        if (noSt != 0) throw FlateException(FlateErrorCode.unexpectedEOF);
        break;
      }
    }

    final curLm = lm;
    final curDm = dm!;
    if (resize && bt + 131072 > bufLen) {
      buf = _growBuf(buf, bt + 131072);
      bufLen = buf.length;
    }
    final lms = (1 << lbt) - 1;
    final dms = (1 << dbt) - 1;
    var lpos = pos;

    for (;; lpos = pos) {
      final o = pos >> 3;
      final raw = (o + 2 < sl
              ? (dat[o] | (dat[o + 1] << 8) | (dat[o + 2] << 16))
              : (o < sl ? dat[o] : 0) | (o + 1 < sl ? (dat[o + 1] << 8) : 0)) >>
          (pos & 7);
      final c = curLm[raw & lms];
      final sym = c >> 4;
      pos += c & 15;
      if (pos > tbts) {
        if (noSt != 0) throw FlateException(FlateErrorCode.unexpectedEOF);
        break;
      }
      if (c == 0) throw FlateException(FlateErrorCode.invalidLengthLiteral);
      if (sym < 256) {
        if (bt < bufLen) buf[bt] = sym;
        ++bt;
      } else if (sym == 256) {
        lpos = pos;
        lm = null;
        break;
      } else {
        var add = sym - 254;
        if (sym > 264) {
          final idx = sym - 257;
          final b = fleb[idx];
          add = _bits(dat, pos, (1 << b) - 1) + fl[idx];
          pos += b;
        }
        final o2 = pos >> 3;
        final raw2 = (o2 + 2 < sl
                ? (dat[o2] | (dat[o2 + 1] << 8) | (dat[o2 + 2] << 16))
                : (o2 < sl ? dat[o2] : 0) | (o2 + 1 < sl ? (dat[o2 + 1] << 8) : 0)) >>
            (pos & 7);
        final d = curDm[raw2 & dms];
        final dsym = d >> 4;
        if (d == 0) throw FlateException(FlateErrorCode.invalidDistance);
        pos += d & 15;
        var dist = dsym < 31 ? fd[dsym] : -1;
        if (dsym > 3 && dsym < 31) {
          final b = fdeb[dsym];
          final o3 = pos >> 3;
          final raw3 = (o3 + 2 < sl
                  ? (dat[o3] | (dat[o3 + 1] << 8) | (dat[o3 + 2] << 16))
                  : (o3 < sl ? dat[o3] : 0) | (o3 + 1 < sl ? (dat[o3 + 1] << 8) : 0)) >>
              (pos & 7);
          dist += raw3 & ((1 << b) - 1);
          pos += b;
        }
        if (pos > tbts) {
          if (noSt != 0) throw FlateException(FlateErrorCode.unexpectedEOF);
          break;
        }
        if (resize && bt + 131072 > bufLen) {
          buf = _growBuf(buf, bt + 131072);
          bufLen = buf.length;
        }
        final end = bt + add;
        if (dist < 0) {
          for (; bt < end; ++bt) {
            if (bt < bufLen) buf[bt] = 0;
          }
        } else {
          if (bt < dist) {
            final shift = dl - dist;
            final dend = dist < end ? dist : end;
            if (shift + bt < 0) throw FlateException(FlateErrorCode.invalidDistance);
            final dictRef = dict!;
            for (; bt < dend; ++bt) {
              if (bt < bufLen) buf[bt] = dictRef[shift + bt];
            }
          }
          if (end <= bufLen) {
            for (; bt < end; ++bt) {
              buf[bt] = buf[bt - dist];
            }
          } else {
            for (; bt < end; ++bt) {
              if (bt < bufLen) {
                final src = bt - dist;
                buf[bt] = src < bufLen ? buf[src] : 0;
              }
            }
          }
        }
      }
    }

    st.l = lm;
    st.p = lpos;
    st.b = bt;
    st.f = isFinal;
    if (lm != null) {
      isFinal = 1;
      st.m = lbt;
      st.d = dm;
      st.n = dbt;
    }
  } while (isFinal == 0);

  final outLen = bt < bufLen ? bt : bufLen;
  return bt != bufLen && noBuf ? slc(buf, 0, outLen) : Uint8List.sublistView(buf, 0, outLen);
}

Uint8List inflateSync(Uint8List data, [InflateOptions opts = const InflateOptions()]) =>
    inflt(data, InflateState(i: 2), opts.out, opts.dictionary);
