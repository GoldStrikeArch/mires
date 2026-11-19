// Floor port (Dart): a JS string to a Dart String and back through dart:js_interop, published on
// globalThis.__matrixExports for the generated facade.
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

extension type Input._(JSObject _) implements JSObject {
  external String get text;
}

void main() {
  final exports = JSObject();
  exports['run'] = ((Input input) => input.text.toJS).toJS;
  globalContext['__matrixExports'] = exports;
}
