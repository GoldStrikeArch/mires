/// noble's hmac.ts.
library;

import 'dart:typed_data';

import 'utils.dart';

/// noble's _HMAC over the hash that [create] makes.
final class Hmac implements Hash {
  Hmac(Hash Function() create, Uint8List key) : _iHash = create(), _oHash = create() {
    final pad = Uint8List(blockLen);
    // A key longer than a block is replaced by its digest.
    pad.setAll(0, key.length > blockLen ? create().update(key).digest() : key);
    for (var i = 0; i < pad.length; i++) {
      pad[i] ^= 0x36;
    }
    _iHash.update(pad);
    // Undo the inner XOR and apply the outer one.
    for (var i = 0; i < pad.length; i++) {
      pad[i] ^= 0x36 ^ 0x5c;
    }
    _oHash.update(pad);
  }

  final Hash _iHash;
  final Hash _oHash;

  @override
  int get blockLen => _iHash.blockLen;
  @override
  int get outputLen => _iHash.outputLen;

  @override
  Hmac update(Uint8List data) {
    _iHash.update(data);
    return this;
  }

  @override
  Uint8List digest() => _oHash.update(_iHash.digest()).digest();
}
