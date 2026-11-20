// The JavaScript values jmespath works on, and the host functions it calls.
//
// jmespath 0.16.0 evaluates over plain JS values (the JSON document, objects it builds, and its own
// AST nodes, which `&expr` hands to functions and to the caller) and leans on JS semantics for them:
// `Object.keys` order, `===` identity in `contains`, relational operators with type coercion,
// `JSON.parse`/`JSON.stringify`, `parseInt`, `Number(string)`, `localeCompare` and the default
// `Array.prototype.sort`. The port keeps those values as `JSAny?` and calls those host functions
// through dart:js_interop, so every one of those semantics is the engine's own.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

/// An AST node as jmespath builds it: a plain object whose properties depend on [type].
extension type Node._(JSObject _) implements JSObject {
  external JSAny? get type;
  external JSAny? get name;
  external JSAny? get value;
  external JSArray<Node?> get children;
  external set jmespathType(JSString value);
  external JSAny? get jmespathType;
}

@JS('Object.keys')
external JSArray<JSString> objectKeys(JSAny value);

@JS('Array.isArray')
external bool isArray(JSAny? value);

@JS('JSON.parse')
external JSAny? _jsonParse(JSString text);

@JS('JSON.stringify')
external JSAny? jsonStringify(JSAny? value);

@JS('parseInt')
external JSNumber _parseInt(JSString text);

/// `Number(value)`: the unary `+` that `to_number` applies to strings.
@JS('Number')
external JSNumber _number(JSAny? value);

/// `String(value)`: the conversion `String.prototype.indexOf` applies to its argument.
@JS('String')
external JSString _string(JSAny? value);

/// The global `undefined`, which jmespath returns from an empty `max_by` and keeps in arrays.
@JS('undefined')
external JSAny? get undefined;

extension JSStringLocale on JSString {
  external int localeCompare(JSString other);
}

extension JSArraySort on JSArray {
  /// `Array.prototype.sort`, in place: the default order compares elements as strings.
  external JSArray sort([JSFunction compareFn]);
}

JSAny? parseJson(String text) => _jsonParse(text.toJS);

double parseInt(String text) => _parseInt(text.toJS).toDartDouble;

double jsNumber(JSAny? value) => _number(value).toDartDouble;

String jsString(JSAny? value) => _string(value).toDart;

bool isString(JSAny? value) => value.typeofEquals('string');

/// A JS string as a Dart string, or null for any other value.
String? stringValue(JSAny? value) => isString(value) ? (value as JSString).toDart : null;

bool isNumber(JSAny? value) => value.typeofEquals('number');

/// jmespath's `isObject`: `Object.prototype.toString` is `[object Object]`, which for these values
/// means a non-null, non-array object (JSON objects, built objects and AST nodes alike).
bool isObject(JSAny? value) => value.typeofEquals('object') && !value.isNull && !isArray(value);

/// `object[key]`, through the prototype chain as in JS.
JSAny? property(JSObject object, JSAny key) => object.getProperty(key);

/// `array[index]` for a JS number index: an element, or undefined off the end, at NaN and at ±∞.
JSAny? elementAt(JSArray array, double index) =>
    index >= 0 && index < array.length && index == index.floorToDouble() ? array[index.toInt()] : undefined;

/// The relational operators of jmespath's comparators, `max_by` and `sort_by`, with JS coercion.
bool jsGreater(JSAny? a, JSAny? b) => a.greaterThan(b).toDart;
bool jsGreaterOrEqual(JSAny? a, JSAny? b) => a.greaterThanOrEqualTo(b).toDart;
bool jsLess(JSAny? a, JSAny? b) => a.lessThan(b).toDart;
bool jsLessOrEqual(JSAny? a, JSAny? b) => a.lessThanOrEqualTo(b).toDart;
bool strictEquals(JSAny? a, JSAny? b) => a.strictEquals(b).toDart;
