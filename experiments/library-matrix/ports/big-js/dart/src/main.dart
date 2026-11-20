// The boundary: what libraries/big-js/reference.mjs does. `dp` and `rm` default with ??, as the
// adapter does (JS null and undefined are both Dart null); `n` and `mode` stay nullable and reach
// the methods as upstream's optional arguments.
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'big.dart';

extension type Operation._(JSObject _) implements JSObject {
  external String get op;
  external JSAny? get x;
  external JSAny? get y;
  external double? get n;
  external double? get mode;
  external double? get dp;
  external double? get rm;
}

extension type Input._(JSObject _) implements JSObject {
  external JSArray<Operation> get cases;
  external int? get repeat;
}

@JS('String')
external String _jsString(JSAny? value);

// The Big constructor's conversion: a number goes through String(), except that -0 becomes '-0'.
String _text(JSAny? value) {
  if (value.isA<JSNumber>()) {
    final n = (value as JSNumber).toDartDouble;
    if (n == 0 && n.isNegative) return '-0';
  }
  return _jsString(value);
}

String _operate(Operation c) {
  final dp = c.dp ?? 20.0, rm = c.rm ?? 1.0;
  final x = Big.parse(_text(c.x));
  final mode = c.mode ?? rm;
  Big y() => Big.parse(_text(c.y));
  return switch (c.op) {
    'toString' => x.toString(),
    'sqrt' => x.sqrt(dp, rm).toString(),
    'plus' => x.plus(y()).toString(),
    'minus' => x.minus(y()).toString(),
    'times' => x.times(y()).toString(),
    'div' => x.div(y(), dp, rm).toString(),
    'cmp' => x.cmp(y()).toString(),
    'pow' => x.pow(c.n, dp, rm).toString(),
    'round' => x.round(c.n, mode).toString(),
    'toFixed' => x.toFixed(c.n, mode),
    'toExponential' => x.toExponential(c.n, mode),
    'toPrecision' => x.toPrecision(c.n, mode),
    _ => throw const BigError('unknown decimal operation'),
  };
}

JSString run(Input input) {
  final outputs = <String>[];
  final cases = input.cases.toDart;
  for (var k = 0; k < (input.repeat ?? 1); k++) {
    for (final c in cases) {
      try {
        outputs.add(_operate(c));
      } on BigError catch (error) {
        outputs.add(error.message);
      }
    }
  }
  return outputs.join('\n').toJS;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
