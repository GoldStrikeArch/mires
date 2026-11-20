/// The boundary: libraries/noble-hashes/reference.mjs in Dart. The input is the corpus object
/// itself, read through a dart:js_interop extension type; byte arrays are copied into Uint8Lists.
library;

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'blake2.dart';
import 'hmac.dart';
import 'sha2.dart';
import 'sha3.dart';
import 'utils.dart';

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

/// Uint8Array.from(values).
Uint8List _toBytes(JSArray<JSNumber> values) {
  final list = values.toDart;
  final out = Uint8List(list.length);
  for (var i = 0; i < out.length; i++) {
    out[i] = list[i].toDartInt;
  }
  return out;
}

Uint8List? _optionalBytes(JSArray<JSNumber>? values) => values == null ? null : _toBytes(values);

/// reference.mjs's `hashes` table plus HMAC-SHA-256: the constructor for [algorithm].
Hash Function(Blake2Opts) _hasher(String algorithm) => switch (algorithm) {
      'sha256' => (_) => Sha256(),
      'sha512' => (_) => Sha512(),
      'sha3_256' => (_) => Keccak(136, 0x06, 32),
      'keccak_256' => (_) => Keccak(136, 0x01, 32),
      'blake2b' => Blake2b.new,
      'blake2s' => Blake2s.new,
      'hmac_sha256' => (opts) => Hmac(Sha256.new, opts.key ?? (throw ArgumentError('"key" expected Uint8Array'))),
      _ => throw ArgumentError('unknown hash'),
    };

String run(HashInput input) {
  final create = _hasher(input.algorithm);
  final opts = Blake2Opts(
    dkLen: input.dkLen,
    key: _optionalBytes(input.key),
    salt: _optionalBytes(input.salt),
    personalization: _optionalBytes(input.personalization),
  );
  final bytes = input.bytes;
  if (bytes != null) {
    final message = _toBytes(bytes);
    return bytesToHex(create(opts).update(message).digest());
  }
  final state = create(opts);
  final blocks = [for (final chunk in input.chunks!.toDart) _toBytes(chunk)];
  for (var i = 0; i < (input.repeat ?? 1); i++) {
    for (final block in blocks) {
      state.update(block);
    }
  }
  return bytesToHex(state.digest());
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
