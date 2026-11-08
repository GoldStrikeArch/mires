library;

import 'dart:js_interop';

import 'host.dart';

JSAny? errorToObject(JSAny? error) {
  final result = jsObject();
  setp(result, 'name', getp(error, 'name'));
  setp(result, 'message', getp(error, 'message'));
  final stack = getp(error, 'stack');
  if (stack.isUndefinedOrNull) {
    setp(result, 'stack', jsUndefined);
  } else {
    setp(result, 'stack', jsSplit(stack, '\n'));
  }
  final keys = objectKeys(error);
  for (var i = 0; i < keys.length; i++) {
    final key = jsIndex(keys, i);
    final name = jsString(key);
    setp(result, name, getp(error, name));
  }
  return result;
}

bool isFunctionNative(JSAny? func) {
  return jsTest(
    jsRegex(r'^function\s.*?\{\s*\[native code]\s*}$'),
    jsString(func),
  );
}
