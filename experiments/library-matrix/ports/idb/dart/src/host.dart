// The host APIs idb wraps: IndexedDB (the factory, requests, databases, transactions, object
// stores, indexes, cursors and key ranges) and DOM events, as dart:js_interop extension types.
//
// Keys, values, versions, counts, directions, modes and options are JSAny? host values. On the
// web, JS null and undefined are both Dart null, but dart2js keeps the JS value itself, so an
// absent field read from the input stays undefined and reaches IndexedDB as an absent argument.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

extension type Event._(JSObject _) implements JSObject {
  external JSObject get target;
}

extension type EventTarget._(JSObject _) implements JSObject {
  external void addEventListener(String type, JSFunction listener);
  external void removeEventListener(String type, JSFunction listener);
}

extension type IDBVersionChangeEvent._(JSObject _) implements Event {
  external double get oldVersion;

  /// Null for a deletion.
  external double? get newVersion;
}

extension type DOMStringList._(JSObject _) implements JSObject {
  external int get length;
  external String? item(int index);

  List<String> toList() => [for (var i = 0; i < length; i++) item(i)!];
}

extension type IDBRequest._(JSObject _) implements EventTarget {
  external JSAny? get result;
  external JSObject? get error;
  external IDBTransaction? get transaction;
}

extension type IDBFactory._(JSObject _) implements JSObject {
  /// Without a version: the current version, or 1 for a new database.
  external IDBRequest open(String name, [JSAny? version]);
  external IDBRequest deleteDatabase(String name);
}

@JS('indexedDB')
external IDBFactory get indexedDB;

extension type IDBDatabase._(JSObject _) implements EventTarget {
  external double get version;
  external DOMStringList get objectStoreNames;
  external IDBObjectStore createObjectStore(String name, JSAny? options);
  external void deleteObjectStore(String name);
  external IDBTransaction transaction(JSAny? storeNames, JSAny? mode);
  external void close();
}

extension type IDBTransaction._(JSObject _) implements EventTarget {
  external DOMStringList get objectStoreNames;
  external JSObject? get error;
  external IDBObjectStore objectStore(String name);
  external void abort();
}

/// IDBObjectStore or IDBIndex. Their request methods are called by name with the arguments as
/// idb gets them ([request]); a write method is undefined on an index, and calling it throws a
/// TypeError, as it does through idb's proxy.
extension type IDBSource._(JSObject _) implements JSObject {
  external String get name;
  external JSAny? get keyPath;

  IDBRequest request(String method, List<JSAny?> args) => callMethodVarArgs<IDBRequest>(method.toJS, args);
}

extension type IDBObjectStore._(JSObject _) implements IDBSource {
  external bool get autoIncrement;
  external DOMStringList get indexNames;
  external IDBIndex index(String name);
  external IDBIndex createIndex(String name, JSAny? keyPath, JSAny? options);
  external void deleteIndex(String name);
}

extension type IDBIndex._(JSObject _) implements IDBSource {
  external bool get unique;
  external bool get multiEntry;
}

/// An IDBCursor; `value` is undefined on a key cursor. Its moves are called by name with the
/// arguments as idb gets them.
extension type IDBCursor._(JSObject _) implements JSObject {
  external JSAny? get key;
  external JSAny? get primaryKey;
  external JSAny? get value;
  external IDBRequest get request;
  external IDBRequest update(JSAny? value);
  external IDBRequest delete();

  void move(String method, List<JSAny?> args) => callMethodVarArgs<JSAny?>(method.toJS, args);
}

@JS('IDBKeyRange')
extension type IDBKeyRange._(JSObject _) implements JSObject {
  external static IDBKeyRange only(JSAny? value);
  external static IDBKeyRange lowerBound(JSAny? lower, JSAny? open);
  external static IDBKeyRange upperBound(JSAny? upper, JSAny? open);
  external static IDBKeyRange bound(JSAny? lower, JSAny? upper, JSAny? lowerOpen, JSAny? upperOpen);
}

@JS('DOMException')
extension type DOMException._(JSObject _) implements JSObject {
  external DOMException(String message, String name);
}

@JS('TypeError')
extension type HostTypeError._(JSObject _) implements JSObject {
  external HostTypeError(String message);
}

@JS('crypto.randomUUID')
external String randomUUID();

@JS('Reflect.has')
external bool reflectHas(JSObject target, String key);

@JS('Array.isArray')
external bool isArray(JSAny? value);

/// The JS `name` of an exception from IndexedDB. A DOMException reaches a Dart `catch` as the JS
/// object it is. dart2js rewrites a JS TypeError that `catch` receives into one of its own errors,
/// which drop the name (a NoSuchMethodError, a NullError or an unknown-type error), and a
/// RangeError into an ArgumentError; IndexedDB throws only DOMExceptions and the TypeErrors of
/// its argument conversions. A rejection, and a JS value Dart throws itself, keep their identity.
String jsErrorName(Object error) => switch (error) {
  JSObject() => (error['name'] as JSString?)?.toDart ?? 'Error',
  ArgumentError() => 'RangeError',
  Error() => 'TypeError',
  _ => 'Error',
};
