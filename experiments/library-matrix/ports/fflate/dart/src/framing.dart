// Checksums (CRC-32, Adler-32), GZIP/Zlib framing, and one-shot sync APIs
// (gzipSync, gunzipSync, zlibSync, unzlibSync, decompressSync) matching fflate 0.8.3.

import 'dart:typed_data';

import 'deflate.dart';
import 'errors.dart';
import 'inflate.dart';
import 'tables.dart';

final class Crc32 {
  int _c = 0xFFFFFFFF;

  void push(Uint8List d) {
    var cr = _c;
    final crct = tables.crct;
    final len = d.length;
    for (var i = 0; i < len; ++i) {
      cr = crct[(cr ^ d[i]) & 255] ^ (cr >>> 8);
    }
    _c = cr;
  }

  int digest() => (_c ^ 0xFFFFFFFF) >>> 0;
}

final class Adler32 {
  int _a = 1;
  int _b = 0;

  void push(Uint8List d) {
    var n = _a;
    var m = _b;
    final l = d.length;
    var i = 0;
    while (i != l) {
      final end = i + 2655 < l ? i + 2655 : l;
      for (; i < end; ++i) {
        n += d[i];
        m += n;
      }
      n = (n & 65535) + 15 * (n >> 16);
      m = (m & 65535) + 15 * (m >> 16);
    }
    _a = n;
    _b = m;
  }

  int digest() {
    final a = _a % 65521;
    final b = _b % 65521;
    _a = a;
    _b = b;
    return (((a & 255) << 24) | ((a & 0xFF00) << 8) | ((b & 255) << 8) | (b >> 8)) >>> 0;
  }
}

/// Writes unsigned 32-bit `v` little-endian into zero-initialized buffer `d` at `b`.
void wbytes(Uint8List d, int b, int v) {
  var u = v >>> 0;
  while (u != 0) {
    d[b++] = u & 0xFF;
    u = u >>> 8;
  }
}

int gzhl(DeflateOptions o) {
  final fn = o.filename;
  return 10 + (fn != null && fn.isNotEmpty ? fn.length + 1 : 0);
}

void gzh(Uint8List c, DeflateOptions o) {
  final fn = o.filename;
  final lvl = o.level;
  c[0] = 31;
  c[1] = 139;
  c[2] = 8;
  c[8] = (lvl != null && lvl < 2) ? 4 : (lvl == 9 ? 2 : 0);
  c[9] = 3;
  final mtime = o.mtime;
  if (mtime != 0) {
    final ms = mtime ?? DateTime.now().millisecondsSinceEpoch.toDouble();
    wbytes(c, 4, (ms / 1000.0).floor());
  }
  if (fn != null && fn.isNotEmpty) {
    c[3] = 8;
    final fnLen = fn.length;
    for (var i = 0; i < fnLen; ++i) {
      c[i + 10] = fn.codeUnitAt(i);
    }
    c[fnLen + 10] = 0;
  }
}

int gzs(Uint8List d) {
  final len = d.length;
  if (len < 3 || d[0] != 31 || d[1] != 139 || d[2] != 8) {
    throw FlateException(FlateErrorCode.invalidHeader, 'invalid gzip data');
  }
  final flg = len > 3 ? d[3] : 0;
  var st = 10;
  if ((flg & 4) != 0) {
    final b0 = 10 < len ? d[10] : 0;
    final b1 = 11 < len ? d[11] : 0;
    st += (b0 | (b1 << 8)) + 2;
  }
  var zs = ((flg >> 3) & 1) + ((flg >> 4) & 1);
  while (zs > 0) {
    final b = st < len ? d[st] : 0;
    ++st;
    if (b == 0) --zs;
  }
  return st + (flg & 2);
}

int gzl(Uint8List d) {
  final l = d.length;
  return (d[l - 4] | (d[l - 3] << 8) | (d[l - 2] << 16) | (d[l - 1] << 24)) >>> 0;
}

void zlh(Uint8List c, DeflateOptions o) {
  final lv = o.level;
  final fl = lv == 0 ? 0 : (lv != null && lv < 6) ? 1 : (lv == 9 ? 3 : 2);
  c[0] = 120;
  var c1 = (fl << 6) | (o.dictionary != null ? 32 : 0);
  c1 |= 31 - (((c[0] << 8) | c1) % 31);
  c[1] = c1;
  final dict = o.dictionary;
  if (dict != null) {
    final h = Adler32();
    h.push(dict);
    wbytes(c, 2, h.digest());
  }
}

int zls(Uint8List d, bool hasDict) {
  final len = d.length;
  final b0 = len > 0 ? d[0] : 0;
  final b1 = len > 1 ? d[1] : 0;
  if ((b0 & 15) != 8 || (b0 >> 4) > 7 || (((b0 << 8) | b1) % 31) != 0) {
    throw FlateException(FlateErrorCode.invalidHeader, 'invalid zlib data');
  }
  if (((b1 >> 5) & 1) == (hasDict ? 0 : 1)) {
    throw FlateException(
      FlateErrorCode.invalidHeader,
      'invalid zlib data: ${(b1 & 32) != 0 ? 'need' : 'unexpected'} dictionary',
    );
  }
  return ((b1 >> 3) & 4) + 2;
}

Uint8List gzipSync(Uint8List data, [DeflateOptions opts = const DeflateOptions()]) {
  final c = Crc32();
  final l = data.length;
  c.push(data);
  final d = dopt(data, opts, gzhl(opts), 8);
  final s = d.length;
  gzh(d, opts);
  wbytes(d, s - 8, c.digest());
  wbytes(d, s - 4, l);
  return d;
}

Uint8List gunzipSync(Uint8List data, [InflateOptions opts = const InflateOptions()]) {
  final st = gzs(data);
  if (st + 8 > data.length) {
    throw FlateException(FlateErrorCode.invalidHeader, 'invalid gzip data');
  }
  return inflt(
    subarray(data, st, -8),
    InflateState(i: 2),
    opts.out ?? Uint8List(gzl(data)),
    opts.dictionary,
  );
}

Uint8List zlibSync(Uint8List data, [DeflateOptions opts = const DeflateOptions()]) {
  final a = Adler32();
  a.push(data);
  final d = dopt(data, opts, opts.dictionary != null ? 6 : 2, 4);
  zlh(d, opts);
  wbytes(d, d.length - 4, a.digest());
  return d;
}

Uint8List unzlibSync(Uint8List data, [InflateOptions opts = const InflateOptions()]) {
  final start = zls(data, opts.dictionary != null);
  return inflt(
    subarray(data, start, -4),
    InflateState(i: 2),
    opts.out,
    opts.dictionary,
  );
}

Uint8List decompressSync(Uint8List data, [InflateOptions opts = const InflateOptions()]) {
  final len = data.length;
  final b0 = len > 0 ? data[0] : 0;
  final b1 = len > 1 ? data[1] : 0;
  final b2 = len > 2 ? data[2] : 0;
  if (b0 == 31 && b1 == 139 && b2 == 8) {
    return gunzipSync(data, opts);
  }
  if ((b0 & 15) != 8 || (b0 >> 4) > 7 || (((b0 << 8) | b1) % 31) != 0) {
    return inflateSync(data, opts);
  }
  return unzlibSync(data, opts);
}
