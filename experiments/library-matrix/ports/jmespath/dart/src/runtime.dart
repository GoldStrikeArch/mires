// jmespath's Runtime: the function table, argument validation and the 26 built-in functions.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' as math;

import 'errors.dart';
import 'host.dart';
import 'interpreter.dart';

/// jmespath's TYPE_* constants, with the names its error messages use.
enum JmesType {
  number('number'),
  any('any'),
  string('string'),
  array('array'),
  object('object'),
  boolean('boolean'),
  expref('expression'),
  null_('null'),
  arrayNumber('Array<number>'),
  arrayString('Array<string>');

  const JmesType(this.label);
  final String label;
}

/// jmespath's `_getTypeName`; null where its switch falls through (undefined).
JmesType? typeName(JSAny? value) {
  if (isString(value)) return JmesType.string;
  if (isNumber(value)) return JmesType.number;
  if (value.typeofEquals('boolean')) return JmesType.boolean;
  if (value.isNull) return JmesType.null_;
  if (isArray(value)) return JmesType.array;
  if (value.typeofEquals('object')) {
    return stringValue((value as Node).jmespathType) == 'Expref' ? JmesType.expref : JmesType.object;
  }
  return null;
}

class _Parameter {
  const _Parameter(this.types, {this.variadic = false});
  final List<JmesType> types;
  final bool variadic;
}

typedef _Body = JSAny? Function(Runtime runtime, List<JSAny?> args);

class _Function {
  const _Function(this.signature, this.body);
  final List<_Parameter> signature;
  final _Body body;
}

List<JSAny?> _array(JSAny? value) => (value as JSArray<JSAny?>).toDart;
double _number(JSAny? value) => (value as JSNumber).toDartDouble;
String _string(JSAny? value) => (value as JSString).toDart;

/// `for (key in value)` over a value jmespath's merge did not type-check: an object's or array's
/// own keys, a string's indices, and nothing for other primitives, null and undefined.
void _forIn(JSAny? value, void Function(JSAny key, JSAny? item) visit) {
  if (isString(value)) {
    final text = _string(value);
    for (var i = 0; i < text.length; i++) {
      visit('$i'.toJS, text[i].toJS);
    }
  } else if (value.typeofEquals('object') && !value.isNull) {
    final object = value as JSObject;
    for (final key in objectKeys(object).toDart) {
      visit(key, property(object, key));
    }
  }
}

JSAny? _maxBy(Runtime runtime, List<JSAny?> args, bool Function(JSAny? a, JSAny? b) better, double initial) {
  final keyFunction = runtime.createKeyFunction(args[1] as Node, const [JmesType.number, JmesType.string]);
  JSAny? best = initial.toJS;
  JSAny? record = undefined;
  for (final element in _array(args[0])) {
    final current = keyFunction(element);
    if (better(current, best)) {
      best = current;
      record = element;
    }
  }
  return record;
}

final Map<String, _Function> _functionTable = {
  'abs': _Function(const [_Parameter([JmesType.number])], (_, args) => _number(args[0]).abs().toJS),
  'avg': _Function(const [_Parameter([JmesType.arrayNumber])], (_, args) {
    final inputArray = _array(args[0]);
    var sum = 0.0;
    for (final x in inputArray) {
      sum += _number(x);
    }
    return (sum / inputArray.length).toJS;
  }),
  'ceil': _Function(const [_Parameter([JmesType.number])], (_, args) => _number(args[0]).ceilToDouble().toJS),
  'contains': _Function(
    const [_Parameter([JmesType.string, JmesType.array]), _Parameter([JmesType.any])],
    // `indexOf`: String.prototype.indexOf converts its argument with String(); the array version
    // compares with `===`, so only the same object is found.
    (_, args) => (isString(args[0])
            ? _string(args[0]).contains(jsString(args[1]))
            : _array(args[0]).any((element) => strictEquals(element, args[1])))
        .toJS,
  ),
  'ends_with': _Function(const [_Parameter([JmesType.string]), _Parameter([JmesType.string])], (_, args) {
    final searchStr = _string(args[0]);
    final suffix = _string(args[1]);
    return (searchStr.indexOf(suffix, math.max(0, searchStr.length - suffix.length)) != -1).toJS;
  }),
  'floor': _Function(const [_Parameter([JmesType.number])], (_, args) => _number(args[0]).floorToDouble().toJS),
  'length': _Function(const [_Parameter([JmesType.string, JmesType.array, JmesType.object])], (_, args) {
    final subject = args[0];
    if (isString(subject)) return _string(subject).length.toJS;
    if (isArray(subject)) return (subject as JSArray).length.toJS;
    return objectKeys(subject!).length.toJS;
  }),
  'map': _Function(
    const [_Parameter([JmesType.expref]), _Parameter([JmesType.array])],
    (runtime, args) =>
        [for (final element in _array(args[1])) runtime.interpreter.visit(args[0] as Node, element)].toJS,
  ),
  'max': _Function(const [_Parameter([JmesType.arrayNumber, JmesType.arrayString])], (_, args) {
    final elements = _array(args[0]);
    if (elements.isEmpty) return null;
    // Math.max.apply: NaN wins, and +0 is larger than -0 (dart2js's math.max is Math.max).
    if (typeName(elements[0]) == JmesType.number) {
      return elements.fold(double.negativeInfinity, (acc, x) => math.max(acc, _number(x))).toJS;
    }
    return elements.reduce((maxElement, x) => (maxElement as JSString).localeCompare(x as JSString) < 0 ? x : maxElement);
  }),
  'merge': _Function(const [_Parameter([JmesType.object], variadic: true)], (_, args) {
    // `for (key in current)`: only the first argument is type-checked, so later ones may be
    // strings or arrays (their index keys) or primitives and null (no keys).
    final merged = JSObject();
    for (final current in args) {
      _forIn(current, merged.setProperty);
    }
    return merged;
  }),
  'max_by': _Function(
    const [_Parameter([JmesType.array]), _Parameter([JmesType.expref])],
    (runtime, args) => _maxBy(runtime, args, jsGreater, double.negativeInfinity),
  ),
  'sum': _Function(
    const [_Parameter([JmesType.arrayNumber])],
    (_, args) => _array(args[0]).fold(0.0, (double sum, x) => sum + _number(x)).toJS,
  ),
  'starts_with': _Function(const [_Parameter([JmesType.string]), _Parameter([JmesType.string])], (_, args) {
    // `lastIndexOf(prefix) === 0`, searching from the end: `starts_with('abab', 'ab')` is false.
    final subject = _string(args[0]);
    return (subject.lastIndexOf(_string(args[1]), subject.length) == 0).toJS;
  }),
  'min': _Function(const [_Parameter([JmesType.arrayNumber, JmesType.arrayString])], (_, args) {
    final elements = _array(args[0]);
    if (elements.isEmpty) return null;
    if (typeName(elements[0]) == JmesType.number) {
      return elements.fold(double.infinity, (acc, x) => math.min(acc, _number(x))).toJS;
    }
    return elements.reduce((minElement, x) => (x as JSString).localeCompare(minElement as JSString) < 0 ? x : minElement);
  }),
  'min_by': _Function(
    const [_Parameter([JmesType.array]), _Parameter([JmesType.expref])],
    (runtime, args) => _maxBy(runtime, args, jsLess, double.infinity),
  ),
  'type': _Function(const [_Parameter([JmesType.any])], (_, args) {
    final label = switch (typeName(args[0])) {
      JmesType.number => 'number',
      JmesType.string => 'string',
      JmesType.array => 'array',
      JmesType.object => 'object',
      JmesType.boolean => 'boolean',
      JmesType.expref => 'expref',
      JmesType.null_ => 'null',
      _ => null,
    };
    return label == null ? undefined : label.toJS;
  }),
  'keys': _Function(const [_Parameter([JmesType.object])], (_, args) => objectKeys(args[0]!)),
  'values': _Function(const [_Parameter([JmesType.object])], (_, args) => objValues(args[0] as JSObject)),
  'sort': _Function(
    const [_Parameter([JmesType.arrayString, JmesType.arrayNumber])],
    // The engine's default sort: elements compared as strings, numbers too.
    (_, args) => _array(args[0]).toList().toJS..sort(),
  ),
  'sort_by': _Function(
    const [_Parameter([JmesType.array]), _Parameter([JmesType.expref])],
    (runtime, args) => runtime.sortBy(_array(args[0]), args[1] as Node),
  ),
  'join': _Function(
    const [_Parameter([JmesType.string]), _Parameter([JmesType.arrayString])],
    (_, args) => _array(args[1]).map(_string).join(_string(args[0])).toJS,
  ),
  'reverse': _Function(const [_Parameter([JmesType.string, JmesType.array])], (_, args) {
    final subject = args[0];
    // Code unit by code unit, as jmespath does: surrogate pairs come out reversed.
    if (isString(subject)) return String.fromCharCodes(_string(subject).codeUnits.reversed).toJS;
    return _array(subject).reversed.toList().toJS;
  }),
  'to_array': _Function(
    const [_Parameter([JmesType.any])],
    (_, args) => typeName(args[0]) == JmesType.array ? args[0] : [args[0]].toJS,
  ),
  'to_string': _Function(
    const [_Parameter([JmesType.any])],
    (_, args) => typeName(args[0]) == JmesType.string ? args[0] : jsonStringify(args[0]),
  ),
  'to_number': _Function(const [_Parameter([JmesType.any])], (_, args) {
    switch (typeName(args[0])) {
      case JmesType.number:
        return args[0];
      case JmesType.string:
        final convertedValue = jsNumber(args[0]);
        return convertedValue.isNaN ? null : convertedValue.toJS;
      default:
        return null;
    }
  }),
  'not_null': _Function(const [_Parameter([JmesType.any], variadic: true)], (_, args) {
    // Undefined is not null here: `not_null(max_by(`[]`, &a))` is undefined.
    for (final arg in args) {
      if (typeName(arg) != JmesType.null_) return arg;
    }
    return null;
  }),
};

class Runtime {
  Runtime(this.interpreter);
  final TreeInterpreter interpreter;

  JSAny? callFunction(JSAny? name, List<JSAny?> resolvedArgs) {
    final functionName = stringValue(name);
    final functionEntry = _functionTable[functionName];
    if (functionEntry == null) throw unknownFunction(functionName);
    _validateArgs(functionName!, resolvedArgs, functionEntry.signature);
    return functionEntry.body(this, resolvedArgs);
  }

  void _validateArgs(String name, List<JSAny?> args, List<_Parameter> signature) {
    final pluralized = signature.length == 1 ? ' argument' : ' arguments';
    if (signature.last.variadic) {
      if (args.length < signature.length) {
        throw argumentError('$name() takes at least ${signature.length}$pluralized but received ${args.length}');
      }
    } else if (args.length != signature.length) {
      throw argumentError('$name() takes ${signature.length}$pluralized but received ${args.length}');
    }
    for (var i = 0; i < signature.length; i++) {
      final actualType = typeName(args[i]);
      if (!signature[i].types.any((expected) => _typeMatches(actualType, expected, args[i]))) {
        final expected = signature[i].types.map((type) => type.label).join(',');
        throw typeError(
          '$name() expected argument ${i + 1} to be type $expected but received type ${actualType?.label} instead.',
        );
      }
    }
  }

  bool _typeMatches(JmesType? actual, JmesType expected, JSAny? argValue) {
    switch (expected) {
      case JmesType.any:
        return true;
      case JmesType.array:
        return actual == JmesType.array;
      case JmesType.arrayNumber || JmesType.arrayString:
        final subtype = expected == JmesType.arrayNumber ? JmesType.number : JmesType.string;
        return actual == JmesType.array &&
            _array(argValue).every((element) => _typeMatches(typeName(element), subtype, element));
      default:
        return actual == expected;
    }
  }

  /// `sort_by`: decorate with the index, sort with the engine's `Array.prototype.sort`, and check
  /// each key's type inside the comparator, as jmespath does. With NaN keys the comparator is
  /// inconsistent, and the engine's algorithm decides the order.
  JSArray<JSAny?> sortBy(List<JSAny?> array, Node exprefNode) {
    final sortedArray = array.toList();
    if (sortedArray.isEmpty) return sortedArray.toJS;
    final requiredType = typeName(interpreter.visit(exprefNode, sortedArray[0]));
    if (requiredType != JmesType.number && requiredType != JmesType.string) throw typeError('sort_by');
    final decorated = [for (var i = 0; i < sortedArray.length; i++) i.toJS].toJS;
    decorated.sort(((JSNumber a, JSNumber b) {
      final indexA = a.toDartInt, indexB = b.toDartInt;
      final exprA = interpreter.visit(exprefNode, sortedArray[indexA]);
      final exprB = interpreter.visit(exprefNode, sortedArray[indexB]);
      if (typeName(exprA) != requiredType) {
        throw typeError('expected ${requiredType!.label}, received ${typeName(exprA)?.label}');
      } else if (typeName(exprB) != requiredType) {
        throw typeError('expected ${requiredType!.label}, received ${typeName(exprB)?.label}');
      }
      if (jsGreater(exprA, exprB)) return 1.toJS;
      if (jsLess(exprA, exprB)) return (-1).toJS;
      return (indexA - indexB).toJS;
    }).toJS);
    return [for (final index in decorated.toDart) sortedArray[(index as JSNumber).toDartInt]].toJS;
  }

  JSAny? Function(JSAny?) createKeyFunction(Node exprefNode, List<JmesType> allowedTypes) => (x) {
        final current = interpreter.visit(exprefNode, x);
        if (!allowedTypes.contains(typeName(current))) {
          throw typeError('expected one of $allowedTypes, received ${typeName(current)?.label}');
        }
        return current;
      };
}
