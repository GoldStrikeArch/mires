// Entry of the Dart port and its JS interop boundary (port of libraries/fflate/reference.mjs).
//
// Reads corpus JSON objects directly via dart:js_interop extension types (no copy, realm-safe)
// and returns raw Dart Uint8Lists as JS Uint8Arrays via .toJS (zero-copy on dart2js).
// main() publishes `run` on globalThis.__matrixExports for the builder's ESM facade.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'codec.dart';
import 'deflate.dart';
import 'errors.dart';
import 'framing.dart';
import 'inflate.dart';
import 'stream.dart';

extension type InputJson._(JSObject _) implements JSObject {
  external String get op;
  external PayloadJson? get data;
  external String? get text;
  external bool? get latin1;
  external bool? get expectErrorCode;
  external bool? get noHandler;
  @JS('options')
  external CompressOptionsJson? get compressOptions;
  @JS('options')
  external DecompressOptionsJson? get decompressOptions;
  external JSArray<StreamStepJson>? get steps;
}

extension type PayloadJson._(JSObject _) implements JSObject {
  external JSArray<JSNumber>? get bytes;
  external String? get text;
  external bool? get latin1;
}

extension type CompressOptionsJson._(JSObject _) implements JSObject {
  external int? get level;
  external int? get mem;
  external PayloadJson? get dictionary;
  external double? get mtime;
  external String? get filename;
}

extension type DecompressOptionsJson._(JSObject _) implements JSObject {
  external PayloadJson? get dictionary;
  external int? get outSize;
}

extension type StreamStepJson._(JSObject _) implements JSObject {
  external String get action;
  external PayloadJson? get data;
  @JS('final')
  external bool? get isFinal;
  external bool? get sync;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}

JSUint8Array run(InputJson input) {
  if (input.expectErrorCode != true) {
    return _execute(input).toJS;
  }
  try {
    _execute(input);
  } on FlateException catch (error) {
    final out = Uint8List(2);
    out[0] = 0xFF;
    out[1] = error.code;
    return out.toJS;
  }
  throw StateError('expected FlateError for op ${input.op}, but call succeeded');
}

Uint8List _payload(PayloadJson spec) {
  final bytes = spec.bytes;
  if (bytes != null) {
    final list = bytes.toDart;
    final len = list.length;
    final out = Uint8List(len);
    for (var i = 0; i < len; ++i) {
      out[i] = list[i].toDartInt;
    }
    return out;
  }
  return strToU8(spec.text!, spec.latin1 == true);
}

DeflateOptions _compressOptions(CompressOptionsJson? json) {
  if (json == null) return const DeflateOptions();
  final dictJson = json.dictionary;
  return DeflateOptions(
    level: json.level,
    mem: json.mem,
    dictionary: dictJson != null ? _payload(dictJson) : null,
    mtime: json.mtime,
    filename: json.filename,
  );
}

InflateOptions _decompressOptions(DecompressOptionsJson? json) {
  if (json == null) return const InflateOptions();
  final dictJson = json.dictionary;
  final outSize = json.outSize;
  return InflateOptions(
    dictionary: dictJson != null ? _payload(dictJson) : null,
    out: outSize != null ? Uint8List(outSize) : null,
  );
}

Uint8List _frameRecords(List<Uint8List> chunks, List<bool> finals) {
  var total = 0;
  final count = chunks.length;
  for (var i = 0; i < count; ++i) {
    total += 5 + chunks[i].length;
  }
  final out = Uint8List(total);
  var offset = 0;
  for (var i = 0; i < count; ++i) {
    final chunk = chunks[i];
    final len = chunk.length;
    out[offset] = len & 0xFF;
    out[offset + 1] = (len >>> 8) & 0xFF;
    out[offset + 2] = (len >>> 16) & 0xFF;
    out[offset + 3] = (len >>> 24) & 0xFF;
    out[offset + 4] = finals[i] ? 1 : 0;
    out.setRange(offset + 5, offset + 5 + len, chunk);
    offset += 5 + len;
  }
  return out;
}

Uint8List _runCompressStream(String op, InputJson input) {
  final opts = _compressOptions(input.compressOptions);
  final chunks = <Uint8List>[];
  final finals = <bool>[];
  final StreamHandler? cb = input.noHandler == true
      ? null
      : (Uint8List chunk, bool isFinal) {
          chunks.add(slc(chunk, 0));
          finals.add(isFinal);
        };
  final Deflate stream;
  if (op == 'Deflate') {
    stream = Deflate(opts, cb);
  } else if (op == 'Gzip') {
    stream = Gzip(opts, cb);
  } else {
    stream = Zlib(opts, cb);
  }
  final steps = input.steps!.toDart;
  for (var i = 0; i < steps.length; ++i) {
    final step = steps[i];
    final action = step.action;
    if (action == 'push') {
      stream.push(_payload(step.data!), step.isFinal == true);
    } else if (action == 'flush') {
      stream.flush(step.sync == true);
    } else {
      throw ArgumentError('unknown stream action $action');
    }
  }
  return _frameRecords(chunks, finals);
}

Uint8List _runDecompressStream(String op, InputJson input) {
  final opts = _decompressOptions(input.decompressOptions);
  final chunks = <Uint8List>[];
  final finals = <bool>[];
  final StreamHandler? cb = input.noHandler == true
      ? null
      : (Uint8List chunk, bool isFinal) {
          chunks.add(slc(chunk, 0));
          finals.add(isFinal);
        };
  final steps = input.steps!.toDart;
  if (op == 'Decompress') {
    final stream = Decompress(opts, cb);
    for (var i = 0; i < steps.length; ++i) {
      final step = steps[i];
      if (step.action == 'push') {
        stream.push(_payload(step.data!), step.isFinal == true);
      } else {
        throw ArgumentError('unknown stream action ${step.action}');
      }
    }
  } else {
    final Inflate stream;
    if (op == 'Inflate') {
      stream = Inflate(opts, cb);
    } else if (op == 'Gunzip') {
      stream = Gunzip(opts, cb);
    } else {
      stream = Unzlib(opts, cb);
    }
    for (var i = 0; i < steps.length; ++i) {
      final step = steps[i];
      if (step.action == 'push') {
        stream.push(_payload(step.data!), step.isFinal == true);
      } else {
        throw ArgumentError('unknown stream action ${step.action}');
      }
    }
  }
  return _frameRecords(chunks, finals);
}

Uint8List _execute(InputJson input) {
  final op = input.op;
  switch (op) {
    case 'deflateSync':
      return deflateSync(_payload(input.data!), _compressOptions(input.compressOptions));
    case 'inflateSync':
      return inflateSync(_payload(input.data!), _decompressOptions(input.decompressOptions));
    case 'gzipSync':
      return gzipSync(_payload(input.data!), _compressOptions(input.compressOptions));
    case 'gunzipSync':
      return gunzipSync(_payload(input.data!), _decompressOptions(input.decompressOptions));
    case 'zlibSync':
      return zlibSync(_payload(input.data!), _compressOptions(input.compressOptions));
    case 'unzlibSync':
      return unzlibSync(_payload(input.data!), _decompressOptions(input.decompressOptions));
    case 'decompressSync':
      return decompressSync(_payload(input.data!), _decompressOptions(input.decompressOptions));
    case 'strToU8':
      return strToU8(input.text!, input.latin1 == true);
    case 'strFromU8':
      return stringToUtf16Le(strFromU8(_payload(input.data!), input.latin1 == true));
    case 'Deflate':
    case 'Gzip':
    case 'Zlib':
      return _runCompressStream(op, input);
    case 'Inflate':
    case 'Gunzip':
    case 'Unzlib':
    case 'Decompress':
      return _runDecompressStream(op, input);
    default:
      throw ArgumentError('unknown operation $op');
  }
}
