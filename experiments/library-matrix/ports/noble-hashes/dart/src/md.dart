/// noble's _md.ts: the Merkle–Damgård base class of SHA-256 and SHA-512, and its Chi and Maj.
library;

import 'dart:math' show min;
import 'dart:typed_data';

import 'u64.dart';
import 'utils.dart';

/// Bits of [b] where [a] is set, else of [c].
int chi(int a, int b, int c) => (a & b) ^ (~a & c);

/// The majority of each bit of [a], [b] and [c].
int maj(int a, int b, int c) => (a & b) ^ (a & c) ^ (b & c);

/// noble's HashMD: buffers partial blocks, then pads with 0x80, zeros and the bit length.
abstract class HashMD implements Hash {
  HashMD(this.blockLen, this.outputLen, this.padOffset) : _buffer = Uint8List(blockLen) {
    _view = ByteData.sublistView(_buffer);
  }

  @override
  final int blockLen;
  @override
  final int outputLen;
  final int padOffset;
  final Uint8List _buffer;
  late final ByteData _view;
  int _length = 0;
  int _pos = 0;

  /// Compresses the [blockLen] bytes of [view] at [offset].
  void process(ByteData view, int offset);

  /// The state words, the high half before the low one for 64-bit words.
  List<int> words();

  @override
  HashMD update(Uint8List data) {
    final len = data.length;
    for (var pos = 0; pos < len;) {
      final take = min(blockLen - _pos, len - pos);
      // With nothing buffered, whole blocks are compressed straight from the input.
      if (take == blockLen) {
        final dataView = ByteData.sublistView(data);
        for (; blockLen <= len - pos; pos += blockLen) {
          process(dataView, pos);
        }
        continue;
      }
      _buffer.setRange(_pos, _pos + take, data, pos);
      _pos += take;
      pos += take;
      if (_pos == blockLen) {
        process(_view, 0);
        _pos = 0;
      }
    }
    _length += len;
    return this;
  }

  @override
  Uint8List digest() {
    var pos = _pos;
    _buffer[pos++] = 0x80;
    _buffer.fillRange(pos, blockLen, 0);
    // No room for the length field: pad this block and put the length in the next.
    if (padOffset > blockLen - pos) {
      process(_view, 0);
      _buffer.fillRange(0, blockLen, 0);
    }
    // noble's setU64FromNum: only the low 64 bits of the length field; the rest stays zero.
    final bits = _length * 8;
    _view
      ..setUint32(blockLen - 8, fromNumH(bits))
      ..setUint32(blockLen - 4, fromNumL(bits));
    process(_view, 0);
    final state = words();
    final out = Uint8List(outputLen);
    final outView = ByteData.sublistView(out);
    for (var i = 0; i < outputLen ~/ 4; i++) {
      outView.setUint32(4 * i, state[i]);
    }
    return out;
  }
}
