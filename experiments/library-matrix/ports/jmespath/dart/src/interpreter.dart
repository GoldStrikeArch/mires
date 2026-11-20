// jmespath's TreeInterpreter and the value helpers it uses (strictDeepEqual, isFalse, objValues).

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'errors.dart';
import 'host.dart';
import 'runtime.dart';

/// The part of `Object.prototype.toString` that strictDeepEqual distinguishes.
String _typeTag(JSAny? value) => value.isNull
    ? 'null'
    : isArray(value)
        ? 'array'
        : value.isUndefined
            ? 'undefined'
            : isString(value)
                ? 'string'
                : isNumber(value)
                    ? 'number'
                    : value.typeofEquals('boolean')
                        ? 'boolean'
                        : 'object';

/// jmespath's `strictDeepEqual`: `===`, then the same `Object.prototype.toString` tag, then by
/// content. Objects compare every own key of [first] against `second[key]` (so an undefined value
/// equals a missing key), then require [second]'s own keys to be among them.
bool strictDeepEqual(JSAny? first, JSAny? second) {
  if (strictEquals(first, second)) return true;
  final firstType = _typeTag(first);
  if (firstType != _typeTag(second)) return false;
  if (firstType == 'array') {
    final a = first as JSArray, b = second as JSArray;
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (!strictDeepEqual(a[i], b[i])) return false;
    }
    return true;
  }
  if (firstType == 'object') {
    final a = first as JSObject, b = second as JSObject;
    final keysSeen = <String>{};
    for (final key in objectKeys(a).toDart) {
      if (!strictDeepEqual(property(a, key), property(b, key))) return false;
      keysSeen.add(key.toDart);
    }
    return objectKeys(b).toDart.every((key) => keysSeen.contains(key.toDart));
  }
  return false;
}

/// jmespath's `isFalse`: "", false, null, an empty array and an object with no own keys.
bool isFalse(JSAny? value) {
  if (stringValue(value) == '' || strictEquals(value, false.toJS) || value.isNull) return true;
  if (isArray(value)) return (value as JSArray).length == 0;
  if (isObject(value)) return objectKeys(value!).length == 0;
  return false;
}

JSArray<JSAny?> objValues(JSObject object) => [for (final key in objectKeys(object).toDart) property(object, key)].toJS;

class TreeInterpreter {
  late final Runtime runtime = Runtime(this);

  JSAny? search(Node? node, JSAny? value) => visit(node, value);

  JSAny? visit(Node? node, JSAny? value) {
    // `node.type` of a missing node is a TypeError in jmespath.
    if (node == null) throw syntaxError('Cannot read properties of undefined');
    Node? child(int index) => node.children[index];
    switch (stringValue(node.type)) {
      case 'Field':
        if (!isObject(value)) return null;
        final field = property(value as JSObject, node.name!);
        return field.isUndefined ? null : field;
      case 'Subexpression' || 'IndexExpression' || 'Pipe':
        return visit(child(1), visit(child(0), value));
      case 'Index':
        if (!isArray(value)) return null;
        final array = value as JSArray<JSAny?>;
        var index = (node.value as JSNumber).toDartDouble;
        if (index < 0) index += array.length;
        final result = elementAt(array, index);
        return result.isUndefined ? null : result;
      case 'Slice':
        return isArray(value) ? _slice(value as JSArray<JSAny?>, node.children as JSArray<JSNumber?>) : null;
      case 'Projection':
        final base = visit(child(0), value);
        return isArray(base) ? _collect((base as JSArray<JSAny?>).toDart, child(1)) : null;
      case 'ValueProjection':
        final base = visit(child(0), value);
        return isObject(base) ? _collect(objValues(base as JSObject).toDart, child(1)) : null;
      case 'FilterProjection':
        final base = visit(child(0), value);
        if (!isArray(base)) return null;
        // Every condition first, then the projection: the first error is jmespath's.
        final filtered = [
          for (final element in (base as JSArray<JSAny?>).toDart)
            if (!isFalse(visit(child(2), element))) element,
        ];
        return _collect(filtered, child(1));
      case 'Comparator':
        final first = visit(child(0), value);
        final second = visit(child(1), value);
        final bool result = switch (stringValue(node.name)) {
          'EQ' => strictDeepEqual(first, second),
          'NE' => !strictDeepEqual(first, second),
          'GT' => jsGreater(first, second),
          'GTE' => jsGreaterOrEqual(first, second),
          'LT' => jsLess(first, second),
          'LTE' => jsLessOrEqual(first, second),
          _ => throw syntaxError('Unknown comparator: ${node.name}'),
        };
        return result.toJS;
      case 'Flatten':
        final original = visit(child(0), value);
        if (!isArray(original)) return null;
        return [
          for (final current in (original as JSArray<JSAny?>).toDart)
            if (isArray(current)) ...(current as JSArray<JSAny?>).toDart else current,
        ].toJS;
      case 'Identity' || 'Current':
        return value;
      case 'MultiSelectList':
        if (value.isNull) return null;
        return [for (final element in node.children.toDart) visit(element, value)].toJS;
      case 'MultiSelectHash':
        if (value.isNull) return null;
        final collected = JSObject();
        for (final pair in node.children.toDart) {
          collected.setProperty(pair!.name!, visit(pair.value as Node?, value));
        }
        return collected;
      case 'OrExpression':
        final matched = visit(child(0), value);
        return isFalse(matched) ? visit(child(1), value) : matched;
      case 'AndExpression':
        final first = visit(child(0), value);
        return isFalse(first) ? first : visit(child(1), value);
      case 'NotExpression':
        return isFalse(visit(child(0), value)).toJS;
      case 'Literal':
        return node.value;
      case 'Function':
        return runtime.callFunction(node.name, [for (final element in node.children.toDart) visit(element, value)]);
      case 'ExpressionReference':
        // jmespath tags the referenced node in place and returns the node itself.
        final refNode = child(0);
        if (refNode == null) throw syntaxError('Cannot set properties of undefined');
        refNode.jmespathType = 'Expref'.toJS;
        return refNode;
      default:
        throw syntaxError('Unknown node type: ${node.type}');
    }
  }

  /// Projections keep every result except null; undefined (an empty `max_by`) is kept.
  JSArray<JSAny?> _collect(List<JSAny?> base, Node? right) => [
        for (final element in base)
          if (visit(right, element) case final current when !current.isNull) current,
      ].toJS;

  /// `Slice` with jmespath's computeSliceParams, in JS numbers: the tokens come from parseInt, so a
  /// bound may be NaN (`[0:3:-]`) or past 2^53, and the loop reads `value[i]` as it goes.
  JSArray<JSAny?> _slice(JSArray<JSAny?> array, JSArray<JSNumber?> params) {
    final length = array.length.toDouble();
    final stepParam = params[2]?.toDartDouble;
    if (stepParam == 0) throw runtimeError('Invalid slice, step cannot be 0');
    final step = stepParam ?? 1.0;
    final stepValueNegative = step < 0;
    final start = switch (params[0]?.toDartDouble) {
      final value? => _capSliceRange(length, value, step),
      null => stepValueNegative ? length - 1 : 0.0,
    };
    final stop = switch (params[1]?.toDartDouble) {
      final value? => _capSliceRange(length, value, step),
      null => stepValueNegative ? -1.0 : length,
    };
    final result = <JSAny?>[];
    if (step > 0) {
      for (var i = start; i < stop; i += step) {
        result.add(elementAt(array, i));
      }
    } else {
      for (var i = start; i > stop; i += step) {
        result.add(elementAt(array, i));
      }
    }
    return result.toJS;
  }

  double _capSliceRange(double arrayLength, double actualValue, double step) {
    if (actualValue < 0) {
      final value = actualValue + arrayLength;
      return value < 0 ? (step < 0 ? -1.0 : 0.0) : value;
    } else if (actualValue >= arrayLength) {
      return step < 0 ? arrayLength - 1 : arrayLength;
    }
    return actualValue;
  }
}
