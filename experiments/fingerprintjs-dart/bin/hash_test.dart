import 'package:fingerprintjs_dart/hashing.dart';

void main() {
  final vectors = <(String, int, String)>[
    ('', 0, '00000000000000000000000000000000'),
    ('hello', 0, 'cbd8a7b341bd9b025b1e906a48ae1d19'),
    ('Hello, world, hi', 0, '9a66b4567d520770dc8eaf9a508ecf1b'),
    ('Hello, world, h', 0, '4552c6409e0a7bd3b0f9eb318bb35f05'),
    ('Hello, world, hi!', 0, 'dce3e02d43da4d2374e84e484c566492'),
    (
      'Hello, world, hi, Hello, world, hi',
      0,
      'd49c261c833b671870b471c42df4dbf0',
    ),
  ];

  final input = 'Hello, world, hi';
  final nonAscii = String.fromCharCodes(
    input.codeUnits.map((c) => c + 256),
  );
  vectors.add((nonAscii, 0, '460892d2cab76edff07f62a97e106f6b'));
  vectors.add(('\ud800 lone surrogate', 17, ''));

  var failed = false;
  for (final (text, seed, expected) in vectors) {
    final got = x64hash128(text, seed);
    if (expected.isEmpty) {
      print('SEED17 ${got}');
      continue;
    }
    if (got != expected) {
      print('FAIL ${text.length} chars seed=$seed expected $expected got $got');
      failed = true;
    } else {
      print('ok ${expected}');
    }
  }
  if (failed) {
    throw StateError('hash vectors failed');
  }
}
