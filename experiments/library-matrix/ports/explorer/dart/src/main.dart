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
// One idb script (idb CONTRACT.md, "Input"): create the store, put every row in
// one readwrite transaction, then walk the store with a cursor in primary-key
// order.
JSObject persist(Iterable<JSObject> rows) {
  JSObject transaction(String mode, JSObject step) => record({
    'op': 'tx'.toJS,
    'conn': 'explorer'.toJS,
    'stores': 'rows'.toJS,
    'mode': mode.toJS,
    'steps': [step].asArray,
    'catch': false.toJS,
  });
  final schema = [
    record({
      'op': 'createStore'.toJS,
      'name': 'rows'.toJS,
      'options': record({'keyPath': 'id'.toJS}),
    }),
    record({
      'op': 'createIndex'.toJS,
      'store': 'rows'.toJS,
      'name': 'byGroup'.toJS,
      'keyPath': 'group'.toJS,
    }),
  ].asArray;
  final openDb = record({
    'op': 'openDB'.toJS,
    'db': 'explorer'.toJS,
    'conn': 'explorer'.toJS,
    'version': 1.toJS,
    'upgrade': schema,
    'catch': false.toJS,
  });
  final puts = rows
      .map(
        (row) => record({
          'op': 'put'.toJS,
          'store': null,
          'index': null,
          'args': [row].asArray,
          'catch': false.toJS,
        }),
      )
      .asArray;
  final write = record({
    'op': 'batch'.toJS,
    'requests': puts,
    'abort': false.toJS,
    'done': true.toJS,
  });
  final walk = record({
    'op': 'openCursor'.toJS,
    'store': null,
    'index': null,
    'args': <Value>[].asArray,
    'positions': <Value>[].asArray,
    'rest': [
      ['continue'.toJS].asArray,
    ].asArray,
    'catch': false.toJS,
  });
  return record({
    'steps': [
      openDb,
      transaction('readwrite', write),
      transaction('readonly', walk),
    ].asArray,
  });
}

// The stored rows' `value` texts, from the cursor entries of the idb log.
List<String> stored(List<Value> history) =>
    items(field(history[0], 'log'))
        .where((entry) => (items(entry)[0] as JSString).toDart == 'cursor')
        .map((entry) => (field(items(entry)[3], 'value') as JSString).toDart)
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
      final rows = items(field(input, 'records')).map(
        (doc) => record({
          'id': field(doc, 'id'),
          'group': 'documents'.toJS,
          'value': stringify(doc as JSObject).toJS,
        }),
      );
      return command('idb', persist(rows));
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
