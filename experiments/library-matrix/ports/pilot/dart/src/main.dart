// Entry of the Dart port. The boundary: the corpus input is a JS object seen through an
// interop extension type; `text` is converted to a Dart String and `top` to an int by
// dart:js_interop, and the String result goes back with `.toJS`. dart2js cannot emit ES module
// exports, so main() publishes `run` on globalThis.__matrixExports, which the builder's facade
// reads once and deletes.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'pilot.dart';

extension type Input._(JSObject _) implements JSObject {
  external String get text;
  external int get top;
}

void main() {
  final exports = JSObject();
  exports['run'] = ((Input input) => analyse(input.text, input.top).toJS).toJS;
  globalContext['__matrixExports'] = exports;
}
