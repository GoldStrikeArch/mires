// Entry of the Dart port and its JS interop boundary (port of libraries/marked/reference.mjs).
//
// Reads corpus JSON objects directly via dart:js_interop extension types (no copy, realm-safe)
// and returns the rendered HTML Dart String as a JSString via .toJS (cast in dart2js).
// main() publishes `run` on globalThis.__matrixExports for the builder's ESM facade.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'marked.dart';
import 'options.dart';

extension type InputJson._(JSObject _) implements JSObject {
  external String get op;
  external String get markdown;
  external OptionsJson? get options;
}

extension type OptionsJson._(JSObject _) implements JSObject {
  external bool? get gfm;
  external bool? get breaks;
  external bool? get pedantic;
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}

JSString run(InputJson input) {
  final optsJson = input.options;
  final options = optsJson == null
      ? const MarkedOptions()
      : MarkedOptions(
          gfm: optsJson.gfm ?? true,
          breaks: optsJson.breaks ?? false,
          pedantic: optsJson.pedantic ?? false,
        );
  final marked = Marked(options);
  final op = input.op;
  if (op == 'parse') return marked.parse(input.markdown).toJS;
  if (op == 'parseInline') return marked.parseInline(input.markdown).toJS;
  throw ArgumentError('unknown operation $op');
}
