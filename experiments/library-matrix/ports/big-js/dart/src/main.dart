import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'decimal.dart';

extension type Operation._(JSObject _) implements JSObject {
  external String get op;
  external JSAny? get x;
  external JSAny? get y;
  external double? get dp;
  external double? get rm;
  external double? get n;
  external double? get mode;
}

extension type DecimalInput._(JSObject _) implements JSObject {
  external JSArray<Operation> get cases;
  external int? get repeat;
}

@JS('String')
external String text(JSAny? x);

int integer(double? x, int fallback) => x == null
    ? fallback
    : (x == x.toInt() && x != -2000002 ? x.toInt() : -2000001);

JSString run(DecimalInput input) {
  final lines = <String>[];
  final rows = input.cases.toDart;
  for (var k = 0; k < (input.repeat ?? 1); k++) {
    for (final c in rows) {
      final rm = integer(c.rm, 1);
      try {
        lines.add(
          operate(
            c.op,
            text(c.x),
            text(c.y),
            integer(c.n, -2000002),
            integer(c.dp, 20),
            rm,
            integer(c.mode, rm),
          ),
        );
      } on FormatException catch (e) {
        lines.add(e.message);
      }
    }
  }
  return lines.join('\n').toJS;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
