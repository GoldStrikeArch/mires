import 'dart:js_interop';
import 'dart:js_interop_unsafe';

@JS('Promise')
extension type EventPromise._(JSObject _) implements JSObject {
  external factory EventPromise(JSFunction executor);
}

JSPromise<JSAny?> makePromise(JSFunction executor) =>
    EventPromise(executor) as JSPromise<JSAny?>;
@JS('Promise.all')
external JSPromise<JSArray<JSAny?>> all(JSArray<JSAny?> promises);
@JS('Promise.allSettled')
external JSPromise<JSArray<JSObject>> allSettled(JSArray<JSAny?> promises);

@JS('DOMException')
extension type DatabaseError._(JSObject _) implements JSObject {
  external factory DatabaseError(String message, String name);
}

@JS('crypto.randomUUID')
external String randomId();
@JS('indexedDB')
external JSObject get factory;
@JS('IDBKeyRange.only')
external JSObject equality(JSAny key);

JSAny? get(JSObject o, String k) => o.getProperty(k.toJS);
JSObject prop(JSObject o, String k) => get(o, k) as JSObject;
JSAny? call(JSObject o, String key, [List<JSAny?> args = const []]) =>
    o.callMethodVarArgs(key.toJS, args);
JSObject objCall(JSObject o, String key, [List<JSAny?> args = const []]) =>
    call(o, key, args) as JSObject;
bool flag(JSObject o, String k) => (get(o, k) as JSBoolean?)?.toDart ?? false;
List<JSAny?> items(JSObject o, String k) =>
    (get(o, k) as JSArray<JSAny?>).toDart;

JSPromise<JSAny?> request(JSObject req) => makePromise(
  ((JSFunction resolve, JSFunction reject) {
    void clear() {
      req['onerror'] = null;
      req['onsuccess'] = null;
    }

    req['onsuccess'] = ((JSAny? event) {
      clear();
      resolve.callAsFunction(null, get(req, 'result'));
    }).toJS;
    req['onerror'] = ((JSAny? event) {
      clear();
      reject.callAsFunction(null, get(req, 'error'));
    }).toJS;
  }).toJS,
);

JSPromise<JSAny?> transactionDone(JSObject tx) => makePromise(
  ((JSFunction resolve, JSFunction reject) {
    void clear() {
      tx['oncomplete'] = null;
      tx['onabort'] = null;
      tx['onerror'] = null;
    }

    tx['oncomplete'] = ((JSAny? event) {
      clear();
      resolve.callAsFunction(null);
    }).toJS;
    final failure = ((JSAny? event) {
      final reason =
          get(tx, 'error') ?? DatabaseError('AbortError', 'AbortError');
      clear();
      reject.callAsFunction(null, reason);
    }).toJS;
    tx['onabort'] = failure;
    tx['onerror'] = failure;
  }).toJS,
);

Future<JSObject> openDatabase(JSString name) async {
  final req = objCall(factory, 'open', [name, 1.toJS]);
  req['onupgradeneeded'] = ((JSAny? event) {
    final options = JSObject()..['keyPath'] = 'id'.toJS;
    final store = objCall(prop(req, 'result'), 'createObjectStore', [
      'rows'.toJS,
      options,
    ]);
    call(store, 'createIndex', ['byGroup'.toJS, 'group'.toJS]);
  }).toJS;
  return (await request(req).toDart) as JSObject;
}

Future<JSArray<JSAny?>> write(JSObject db, JSObject batch) async {
  final tx = objCall(db, 'transaction', ['rows'.toJS, 'readwrite'.toJS]);
  final done = transactionDone(tx),
      store = objCall(tx, 'objectStore', ['rows'.toJS]);
  final method = flag(batch, 'add') ? 'add' : 'put';
  final writes = <JSAny?>[
    for (final r in items(batch, 'rows')) request(objCall(store, method, [r])),
    done,
  ];
  final settled = allSettled(writes.toJS);
  if (flag(batch, 'abort')) call(tx, 'abort');
  final out = <JSAny?>[];
  for (final item in (await settled.toDart).toDart) {
    if ((get(item, 'status') as JSString).toDart == 'rejected') {
      final name = (get(prop(item, 'reason'), 'name') as JSString).toDart;
      out.add('!$name'.toJS);
    } else {
      out.add(get(item, 'value') ?? 'done'.toJS);
    }
  }
  return out.toJS;
}

JSPromise<JSArray<JSAny?>> lookup(JSObject source, List<JSAny?> keys) => all(
  <JSAny?>[
    for (final key in keys) request(objCall(source, 'get', [key])),
  ].toJS,
);

JSArray<JSAny?> values(JSArray<JSAny?> rows) => <JSAny?>[
  for (final row in rows.toDart)
    if (row == null) null else get(row as JSObject, 'value'),
].toJS;

Future<JSArray<JSAny?>> exercise(JSObject db, JSObject input) async {
  final batches = <JSAny?>[];
  for (final value in items(input, 'batches')) {
    batches.add(await write(db, value as JSObject));
  }
  final tx = objCall(db, 'transaction', ['rows'.toJS]);
  final done = transactionDone(tx);
  final store = objCall(tx, 'objectStore', ['rows'.toJS]),
      index = objCall(store, 'index', ['byGroup'.toJS]);
  final reads = lookup(store, items(input, 'keys')),
      indexed = lookup(index, items(input, 'groups'));
  final source = flag(input, 'index') ? index : store;
  final only = get(input, 'only');
  final req = objCall(source, 'openCursor', [
    only == null ? null : equality(only),
    (flag(input, 'reverse') ? 'prev' : 'next').toJS,
  ]);
  final step = get(input, 'step') as JSNumber;
  final cursors = <JSAny?>[];
  var cursor = (await request(req).toDart) as JSObject?;
  while (cursor != null) {
    cursors.add(
      <JSAny?>[
        get(cursor, 'key'),
        get(cursor, 'primaryKey'),
        get(prop(cursor, 'value'), 'value'),
      ].toJS,
    );
    if (step.toDartInt == 1) {
      call(cursor, 'continue');
    } else {
      call(cursor, 'advance', [step]);
    }
    cursor = (await request(req).toDart) as JSObject?;
  }
  final out = <JSAny?>[
    batches.toJS,
    values(await reads.toDart),
    values(await indexed.toDart),
    cursors.toJS,
  ].toJS;
  await done.toDart;
  return out;
}

Future<JSArray<JSAny?>> run(JSObject input) async {
  final name = 'mires-matrix-idb-${randomId()}'.toJS;
  JSObject? db;
  try {
    db = await openDatabase(name);
    return await exercise(db, input);
  } finally {
    if (db != null) call(db, 'close');
    await request(objCall(factory, 'deleteDatabase', [name])).toDart;
  }
}

void main() {
  final exports = JSObject();
  exports['run'] = ((JSObject input) => run(input).toJS).toJS;
  globalContext['__matrixExports'] = exports;
}
