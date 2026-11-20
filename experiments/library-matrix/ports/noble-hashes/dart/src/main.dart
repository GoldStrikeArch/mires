import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'sha2.dart';
import 'sha3.dart';
import 'blake2.dart';

extension type HashInput._(JSObject _) implements JSObject {
  external String get algorithm;
  external JSArray<JSNumber>? get bytes;
  external JSArray<JSArray<JSNumber>>? get chunks;
  external int? get repeat;
  external double? get dkLen;
  external JSArray<JSNumber>? get key;
  external JSArray<JSNumber>? get salt;
  external JSArray<JSNumber>? get personalization;
}

Uint8List read(JSArray<JSNumber>? value) => Uint8List.fromList([
  if (value != null)
    for (final b in value.toDart) b.toDartInt,
]);

JSString run(HashInput input) {
  final raw = input.bytes;
  final blocks = raw == null
      ? input.chunks!.toDart.map(read).toList()
      : [read(raw)];
  final repeat = raw == null ? input.repeat ?? 1 : 1;
  final chunks = [for (var i = 0; i < repeat; i++) ...blocks];
  final key = read(input.key);
  final Uint8List result;
  switch (input.algorithm) {
    case 'sha256':
      result = sha2Digest(0, chunks, key);
    case 'sha512':
      result = sha2Digest(1, chunks, key);
    case 'hmac_sha256':
      if (input.key == null) throw ArgumentError('missing HMAC key');
      result = sha2Digest(2, chunks, key);
    case 'sha3_256':
    case 'keccak_256':
      result = sha3Digest(input.algorithm == 'keccak_256', chunks);
    case 'blake2b':
    case 'blake2s':
      final wide = input.algorithm == 'blake2b';
      final limit = wide ? 64 : 32;
      final size = input.dkLen ?? limit.toDouble();
      final salt = read(input.salt);
      final pers = read(input.personalization);
      if (size < 1 ||
          size > limit ||
          size != size.truncate() ||
          (input.key != null && (key.isEmpty || key.length > limit)) ||
          (input.salt != null && salt.length != limit ~/ 4) ||
          (input.personalization != null && pers.length != limit ~/ 4)) {
        throw ArgumentError('invalid BLAKE2 parameters');
      }
      result = blake2Digest(wide, chunks, size.toInt(), key, salt, pers);
    default:
      throw ArgumentError('unknown hash');
  }
  return result.map((b) => b.toRadixString(16).padLeft(2, '0')).join().toJS;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
