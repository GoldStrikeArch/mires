// Synchronous streaming classes matching fflate 0.8.3:
// Deflate, Gzip, Zlib, Inflate, Gunzip, Unzlib, Decompress.

import 'dart:typed_data';

import 'deflate.dart';
import 'errors.dart';
import 'framing.dart';
import 'inflate.dart';
import 'tables.dart';

typedef StreamHandler = void Function(Uint8List chunk, bool isFinal);

class Deflate {
  Deflate([DeflateOptions opts = const DeflateOptions(), this.ondata])
      : o = opts,
        s = DeflateState(l: 0, i: 32768, w: 32768, z: 32768),
        b = Uint8List(98304) {
    final rawDict = o.dictionary;
    if (rawDict != null) {
      final dict = subarray(rawDict, -32768);
      b.setRange(32768 - dict.length, 32768, dict);
      s.i = 32768 - dict.length;
    }
  }

  StreamHandler? ondata;
  DeflateOptions o;
  DeflateState s;
  Uint8List b;

  void emitChunk(Uint8List c, bool isFinal) {
    ondata!(dopt(c, o, 0, 0, s), isFinal);
  }

  void push(Uint8List chunk, [bool isFinal = false]) {
    if (ondata == null) throw FlateException(FlateErrorCode.noStreamHandler);
    if (s.l != 0) throw FlateException(FlateErrorCode.streamFinished);
    final endLen = chunk.length + s.z;
    if (endLen > b.length) {
      if (endLen > 2 * b.length - 32768) {
        final newBuf = Uint8List(endLen - (endLen & 32767));
        newBuf.setRange(0, s.z, b);
        b = newBuf;
      }
      final split = b.length - s.z;
      b.setRange(s.z, b.length, chunk, 0);
      s.z = b.length;
      emitChunk(b, false);
      b.setRange(0, 32768, b, b.length - 32768);
      final remLen = chunk.length - split;
      b.setRange(32768, 32768 + remLen, chunk, split);
      s.z = remLen + 32768;
      s.i = 32766;
      s.w = 32768;
    } else {
      b.setRange(s.z, s.z + chunk.length, chunk);
      s.z += chunk.length;
    }
    s.l = isFinal ? 1 : 0;
    if (s.z > s.w + 8191 || isFinal) {
      emitChunk(b, isFinal);
      s.w = s.i;
      s.i -= 2;
    }
    if (isFinal) {
      s = DeflateState();
      o = const DeflateOptions();
      b = tables.emptyU8;
    }
  }

  void flush([bool sync = false]) {
    if (ondata == null) throw FlateException(FlateErrorCode.noStreamHandler);
    if (s.l != 0) throw FlateException(FlateErrorCode.streamFinished);
    emitChunk(b, false);
    s.w = s.i;
    s.i -= 2;
    if (sync) {
      final c = Uint8List(6);
      c[0] = s.r >>> 3;
      final ep = wfblk(c, s.r, tables.emptyU8, 0, 0);
      s.r = 0;
      ondata!(subarray(c, 0, ep >>> 3), false);
    }
  }
}

final class Gzip extends Deflate {
  Gzip([super.opts, super.ondata]);

  final Crc32 _c = Crc32();
  int _l = 0;
  bool _v = true;

  @override
  void push(Uint8List chunk, [bool isFinal = false]) {
    _c.push(chunk);
    _l += chunk.length;
    super.push(chunk, isFinal);
  }

  @override
  void emitChunk(Uint8List c, bool isFinal) {
    final raw = dopt(c, o, _v ? gzhl(o) : 0, isFinal ? 8 : 0, s);
    if (_v) {
      gzh(raw, o);
      _v = false;
    }
    if (isFinal) {
      wbytes(raw, raw.length - 8, _c.digest());
      wbytes(raw, raw.length - 4, _l);
    }
    ondata!(raw, isFinal);
  }
}

final class Zlib extends Deflate {
  Zlib([super.opts, super.ondata]);

  final Adler32 _c = Adler32();
  bool _v = true;

  @override
  void push(Uint8List chunk, [bool isFinal = false]) {
    _c.push(chunk);
    super.push(chunk, isFinal);
  }

  @override
  void emitChunk(Uint8List c, bool isFinal) {
    final raw = dopt(c, o, _v ? (o.dictionary != null ? 6 : 2) : 0, isFinal ? 4 : 0, s);
    if (_v) {
      zlh(raw, o);
      _v = false;
    }
    if (isFinal) {
      wbytes(raw, raw.length - 4, _c.digest());
    }
    ondata!(raw, isFinal);
  }
}

class Inflate {
  Inflate([InflateOptions opts = const InflateOptions(), this.ondata])
      : s = InflateState(i: 0),
        o = Uint8List(32768),
        p = tables.emptyU8 {
    final rawDict = opts.dictionary;
    if (rawDict != null) {
      final dict = subarray(rawDict, -32768);
      s.b = dict.length;
      o.setRange(0, dict.length, dict);
    }
  }

  StreamHandler? ondata;
  InflateState s;
  Uint8List o;
  Uint8List p;
  bool d = false;

  void appendChunk(Uint8List c) {
    if (ondata == null) throw FlateException(FlateErrorCode.noStreamHandler);
    if (d) throw FlateException(FlateErrorCode.streamFinished);
    if (p.isEmpty) {
      p = c;
    } else if (c.isNotEmpty) {
      final n = Uint8List(p.length + c.length);
      n.setRange(0, p.length, p);
      n.setRange(p.length, n.length, c);
      p = n;
    }
  }

  void process(bool isFinal) {
    d = isFinal;
    s.i = isFinal ? 1 : 0;
    final bts = s.b;
    final dt = inflt(p, s, o);
    ondata!(slc(dt, bts, s.b), d);
    o = slc(dt, s.b - 32768);
    s.b = o.length;
    p = slc(p, s.p >>> 3);
    s.p &= 7;
  }

  void push(Uint8List chunk, [bool isFinal = false]) {
    appendChunk(chunk);
    process(isFinal);
  }
}

final class Gunzip extends Inflate {
  Gunzip([super.opts, super.ondata]);

  int _v = 1;

  @override
  void push(Uint8List chunk, [bool isFinal = false]) {
    appendChunk(chunk);
    if (_v != 0) {
      final sub = subarray(p, _v - 1);
      final st = sub.length > 3 ? gzs(sub) : 4;
      if (st > sub.length) {
        if (!isFinal) return;
      }
      p = subarray(sub, st);
      _v = 0;
    }
    process(false);
    if (s.f != 0 && s.l == null) {
      _v = shft(s.p) + 9;
      s = InflateState(i: 0);
      o = tables.emptyU8;
      push(tables.emptyU8, isFinal);
    } else if (isFinal) {
      process(true);
    }
  }
}

final class Unzlib extends Inflate {
  Unzlib([super.opts, super.ondata]) : _v = opts.dictionary != null ? 2 : 1;

  int _v;

  @override
  void push(Uint8List chunk, [bool isFinal = false]) {
    appendChunk(chunk);
    if (_v != 0) {
      if (p.length < 6 && !isFinal) return;
      p = subarray(p, zls(p, _v - 1 != 0));
      _v = 0;
    }
    if (isFinal) {
      if (p.length < 4) {
        throw FlateException(FlateErrorCode.invalidHeader, 'invalid zlib data');
      }
      p = subarray(p, 0, -4);
    }
    process(isFinal);
  }
}

final class Decompress {
  Decompress([InflateOptions opts = const InflateOptions(), this.ondata]) : _o = opts;

  StreamHandler? ondata;
  final InflateOptions _o;
  Inflate? _s;
  Uint8List? _p;

  void push(Uint8List chunk, [bool isFinal = false]) {
    if (ondata == null) throw FlateException(FlateErrorCode.noStreamHandler);
    final inner = _s;
    if (inner == null) {
      final prev = _p;
      if (prev != null && prev.isNotEmpty) {
        final n = Uint8List(prev.length + chunk.length);
        n.setRange(0, prev.length, prev);
        n.setRange(prev.length, n.length, chunk);
        _p = n;
      } else {
        _p = chunk;
      }
      final buf = _p!;
      if (buf.length > 2) {
        final Inflate next;
        if (buf[0] == 31 && buf[1] == 139 && buf[2] == 8) {
          next = Gunzip(_o, ondata);
        } else if ((buf[0] & 15) != 8 || (buf[0] >> 4) > 7 || (((buf[0] << 8) | buf[1]) % 31) != 0) {
          next = Inflate(_o, ondata);
        } else {
          next = Unzlib(_o, ondata);
        }
        _s = next;
        _p = null;
        next.push(buf, isFinal);
      }
    } else {
      inner.push(chunk, isFinal);
    }
  }
}
