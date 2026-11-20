import 'dart:js_interop';
import 'dart:js_interop_unsafe';

typedef Value = JSAny?;
@JS('JSON.stringify')
external String stringify(JSObject document);
@JS('JSON.parse')
external JSObject parse(String text);

List<Value> items(Value value) {
  final array = value as JSObject;
  final length = (array['length'] as JSNumber).toDartInt;
  return List.generate(length, (index) => array.getProperty(index.toJS));
}

extension on Iterable<Value> {
  JSArray<Value> get asArray {
    final result = JSArray<Value>();
    var index = 0;
    for (final item in this) {
      result.setProperty((index++).toJS, item);
    }
    return result;
  }
}

JSObject record(Map<String, Value> fields) {
  final result = JSObject();
  fields.forEach((key, value) => result[key] = value);
  return result;
}

Value field(Value value, String key) => (value as JSObject)[key];
List<String> stored(List<Value> history) =>
    items(items(history[0])[3])
        .map((row) => (items(row)[2] as JSString).toDart)
        .toList();
List<Value> hits(List<Value> history) =>
    items(field(items(field(history[1], 'results'))[1], 'hits'));
JSArray<Value> command(String? name, Value value) =>
    <Value>[name?.toJS, value].asArray;

JSArray<Value> run(JSArray<Value> packet) {
  final input = items(packet)[0];
  final history = items(items(packet)[1]);
  switch (history.length) {
    case 0:
      final rows = items(field(input, 'records'))
          .map(
            (doc) => record({
              'id': field(doc, 'id'),
              'group': 'documents'.toJS,
              'value': stringify(doc as JSObject).toJS,
            }),
          )
          .asArray;
      return command(
        'idb',
        record({
          'batches': [
            record({'rows': rows}),
          ].asArray,
          'keys': <Value>[].asArray,
          'groups': <Value>[].asArray,
          'step': 1.toJS,
        }),
      );
    case 1:
      final options = record({
        'fields': ['title'.toJS, 'body'.toJS].asArray,
        'storeFields': ['body'.toJS].asArray,
      });
      final add = record({
        'op': 'addAll'.toJS,
        'documents': stored(history).map((text) => parse(text)).asArray,
      });
      final query = record({
        'op': 'search'.toJS,
        'query': field(input, 'query'),
      });
      return command(
        'minisearch',
        record({
          'options': options,
          'operations': [add, query].asArray,
        }),
      );
    case 2:
      final result = hits(history);
      final markdown = result.isEmpty ? ''.toJS : field(result.first, 'body');
      return command(
        'marked',
        record({'op': 'parse'.toJS, 'markdown': markdown}),
      );
    case 3:
      return command(
        'fflate',
        record({'op': 'strToU8'.toJS, 'text': stored(history).join('\n').toJS}),
      );
    case 4:
      return command(
        'fflate',
        record({
          'op': 'gzipSync'.toJS,
          'data': record({'bytes': history[3]}),
          'options': record({'level': 6.toJS, 'mtime': 0.toJS}),
        }),
      );
    case 5:
      return command('uplot', field(input, 'chart'));
    default:
      final ids = hits(history).map((hit) => field(hit, 'id')).asArray;
      return command(
        null,
        <Value>[
          ids,
          history[2],
          history[4],
          items(history[3]).length.toJS,
          history[5],
        ].asArray,
      );
  }
}

void main() {
  globalContext['__matrixExports'] = record({'run': run.toJS});
}
