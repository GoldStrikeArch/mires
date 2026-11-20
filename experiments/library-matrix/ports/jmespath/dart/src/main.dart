// Entry of the Dart port: what libraries/jmespath/reference.mjs does, on the port's jmespath.
//
// The boundary: the corpus input is the JS object itself, seen through dart:js_interop extension
// types (no copy). The document stays the JS value it is (jmespath returns parts of it), each
// expression becomes a Dart string (on the web, the same JS string), and each result is a plain JS
// object `{ value }` or `{ error }` with the reference's error category.
//
// dart2js cannot emit ES module exports, so main() publishes `run` on globalThis.__matrixExports,
// which the builder's facade reads once and deletes.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'errors.dart';
import 'host.dart';
import 'interpreter.dart';
import 'parser.dart';

extension type Input._(JSObject _) implements JSObject {
  external JSAny? get data;
  external JSArray<Query> get queries;
}

extension type Query._(JSObject _) implements JSObject {
  external String get op;
  external String get expression;
}

/// jmespath's `compile`.
Node? compile(String expression) => Parser().parse(expression);

/// jmespath's `search`: a fresh parser and interpreter per call.
JSAny? search(JSAny? data, String expression) => TreeInterpreter().search(Parser().parse(expression), data);

JSObject _result(String key, JSAny? value) => JSObject()..[key] = value;

JSArray<JSObject> run(Input input) => [
      for (final query in input.queries.toDart) _query(query, input.data),
    ].toJS;

JSObject _query(Query query, JSAny? data) {
  try {
    return _result('value', query.op == 'compile' ? compile(query.expression) : search(data, query.expression));
  } on JmesPathException catch (e) {
    return _result('error', e.category.label.toJS);
  } catch (_) {
    // The reference's category() default: JSON.parse's SyntaxError and engine errors.
    return _result('error', ErrorCategory.syntax.label.toJS);
  }
}

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
