// idb 8.0.4 (build/index.js) for Dart: IndexedDB requests become Futures, a transaction gets a
// `done` Future when it is wrapped, cursors move by returning a Future of the next cursor, the
// database has the database-extras shortcuts, and `iterate` walks a store or an index.
//
// idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back.
// Here each wrapped object is a class over the host object; its members call the host member and
// wrap the result the same way. A request's Future is completed by idb's own listener logic, so a
// request is issued when its method is called, and a host exception is thrown at once, before any
// Future exists.

import 'dart:async';
import 'dart:js_interop';

import 'host.dart';

/// Listeners added together and removed together, as idb's `unlisten` removes its own.
class _Listeners {
  _Listeners(this._target);

  final EventTarget _target;
  final _added = <(String, JSFunction)>[];

  void on(String type, void Function(Event event) listener) {
    final function = listener.toJS;
    _added.add((type, function));
    _target.addEventListener(type, function);
  }

  void remove() {
    for (final (type, function) in _added) {
      _target.removeEventListener(type, function);
    }
  }
}

/// A wrapped IndexedDB object; [unwrap] gives the host object behind it.
abstract class Wrapped<T extends JSObject> {
  Wrapped(this._raw);

  final T _raw;
}

/// idb's `unwrap`.
T unwrap<T extends JSObject>(Wrapped<T> value) => value._raw;

/// idb's `promisifyRequest`: completes once, with the request's result on `success` (wrapped by
/// [wrapResult], as idb's `wrap(request.result)`) or with its error on `error`, then removes both
/// listeners. A cursor request fires once per move, so each move promisifies it again.
Future<T> _promisify<T>(IDBRequest request, T Function(JSAny? result) wrapResult) {
  final completer = Completer<T>();
  final listeners = _Listeners(request);
  listeners.on('success', (_) {
    completer.complete(wrapResult(request.result));
    listeners.remove();
  });
  listeners.on('error', (_) {
    completer.completeError(request.error!);
    listeners.remove();
  });
  return completer.future;
}

/// idb's `wrap` of a request: a Future of its result, which is undefined for a miss.
Future<JSAny?> wrap(IDBRequest request) => _promisify(request, (result) => result);

Future<Cursor?> _wrapCursor(IDBRequest request) =>
    _promisify(request, (result) => result == null ? null : Cursor._(result as IDBCursor));

/// idb's `cacheDonePromiseForTransaction`: completed by the first `complete`, `error` or `abort`.
/// A request error bubbles to the transaction before the transaction records it, so `done` then
/// fails with a new AbortError.
///
/// It is marked with [Future.ignore]: idb leaves a `done` nobody awaits unhandled, which JS only
/// reports as an unhandled rejection, while Dart would report it as an uncaught error. A caller
/// that awaits it still gets its error.
Future<void> _cacheDone(IDBTransaction tx) {
  final completer = Completer<void>();
  final listeners = _Listeners(tx);
  listeners.on('complete', (_) {
    completer.complete();
    listeners.remove();
  });
  void failed(Event _) {
    completer.completeError(tx.error ?? DOMException('AbortError', 'AbortError'));
    listeners.remove();
  }

  listeners.on('error', failed);
  listeners.on('abort', failed);
  return completer.future..ignore();
}

/// The request methods idb promisifies on a store or an index, with database-extras'
/// `readMethods` and `writeMethods` split.
enum Method {
  get(false),
  getKey(false),
  getAll(false),
  getAllKeys(false),
  count(false),
  put(true),
  add(true),
  delete(true),
  clear(true);

  const Method(this.isWrite);

  final bool isWrite;

  static Method? of(String name) {
    for (final method in values) {
      if (method.name == name) return method;
    }
    return null;
  }
}

/// An object store or an index: the methods both have.
sealed class Source extends Wrapped<IDBSource> {
  Source(super.raw);

  String get name => _raw.name;
  JSAny? get keyPath => _raw.keyPath;

  Future<JSAny?> request(Method method, [List<JSAny?> args = const []]) => wrap(_raw.request(method.name, args));

  Future<Cursor?> openCursor([List<JSAny?> args = const []]) => _wrapCursor(_raw.request('openCursor', args));

  Future<Cursor?> openKeyCursor([List<JSAny?> args = const []]) => _wrapCursor(_raw.request('openKeyCursor', args));

  /// idb's async iterator, `openCursor(...args)` walked by [CursorIterator.moveNext].
  CursorIterator iterate([List<JSAny?> args = const []]) => CursorIterator._(this, args);
}

final class ObjectStore extends Source {
  ObjectStore._(IDBObjectStore super.raw) : _store = raw;

  final IDBObjectStore _store;

  bool get autoIncrement => _store.autoIncrement;
  List<String> get indexNames => _store.indexNames.toList();

  Index index(String name) => Index._(_store.index(name));

  Index createIndex(String name, JSAny? keyPath, [JSAny? options]) => Index._(_store.createIndex(name, keyPath, options));

  void deleteIndex(String name) => _store.deleteIndex(name);
}

final class Index extends Source {
  Index._(IDBIndex super.raw) : _index = raw;

  final IDBIndex _index;

  bool get unique => _index.unique;
  bool get multiEntry => _index.multiEntry;
}

/// A wrapped IDBCursor. Its moves are idb's cursor advance methods: they call the host method,
/// then promisify the cursor's request again, which gives this cursor or null past the end.
final class Cursor extends Wrapped<IDBCursor> {
  Cursor._(super.raw);

  JSAny? get key => _raw.key;
  JSAny? get primaryKey => _raw.primaryKey;
  JSAny? get value => _raw.value;

  Future<Cursor?> _move(String method, List<JSAny?> args) {
    _raw.move(method, args);
    return _wrapCursor(_raw.request);
  }

  Future<Cursor?> advance(List<JSAny?> args) => _move('advance', args);

  /// idb's `continue`.
  Future<Cursor?> continueTo([List<JSAny?> args = const []]) => _move('continue', args);

  Future<Cursor?> continuePrimaryKey(List<JSAny?> args) => _move('continuePrimaryKey', args);

  Future<JSAny?> update(JSAny? value) => wrap(_raw.update(value));

  Future<JSAny?> delete() => wrap(_raw.delete());
}

final class Transaction extends Wrapped<IDBTransaction> {
  /// `done` is created with the wrapper, as idb caches it when it first wraps the transaction, so
  /// it also completes for a caller that only asks after the transaction finished.
  Transaction._(super.raw) : done = _cacheDone(raw);

  final Future<void> done;

  /// idb's `tx.store`: the only store in the scope, or null when there are several. idb tests the
  /// second name before it calls objectStore(), so on a finished transaction this throws
  /// InvalidStateError for one store and gives null for several.
  ObjectStore? get store {
    final names = _raw.objectStoreNames;
    final second = names.item(1);
    return second != null && second.isNotEmpty ? null : objectStore(names.item(0)!);
  }

  ObjectStore objectStore(String name) => ObjectStore._(_raw.objectStore(name));

  void abort() => _raw.abort();
}

/// A database shortcut (database-extras' `getMethod`): a request method on the store, or with
/// [useIndex] (`getFromIndex`, …) on one of its indexes. Only read methods have index forms.
final class Shortcut {
  const Shortcut._(this.method, this.useIndex);

  final Method method;
  final bool useIndex;

  static Shortcut? of(String name) {
    const suffix = 'FromIndex';
    final useIndex = name.endsWith(suffix);
    final method = Method.of(useIndex ? name.substring(0, name.length - suffix.length) : name);
    if (method == null || (useIndex && method.isWrite)) return null;
    return Shortcut._(method, useIndex);
  }
}

final class Database extends Wrapped<IDBDatabase> {
  Database._(super.raw);

  double get version => _raw.version;
  List<String> get objectStoreNames => _raw.objectStoreNames.toList();

  ObjectStore createObjectStore(String name, [JSAny? options]) => ObjectStore._(_raw.createObjectStore(name, options));

  void deleteObjectStore(String name) => _raw.deleteObjectStore(name);

  /// [storeNames] is a name or an array of names. [mode] is passed as it is, so a JS undefined is
  /// readonly and a null throws, as in JS.
  Transaction transaction(JSAny? storeNames, JSAny? mode) => Transaction._(_raw.transaction(storeNames, mode));

  void close() => _raw.close();

  /// One shortcut call, as getMethod's async method: a transaction per call, `readwrite` for a
  /// write, on `tx.store` or its index [indexName], and `Promise.all([request, isWrite &&
  /// tx.done])`, so a write completes only after `done`, the request's error comes first, and both
  /// Futures are handled. A synchronous failure fails the call.
  Future<JSAny?> shortcut(Shortcut shortcut, String storeName, String? indexName, List<JSAny?> args) async {
    final method = shortcut.method;
    final tx = transaction(storeName.toJS, (method.isWrite ? 'readwrite' : 'readonly').toJS);
    final store = tx.store!;
    final Source target = shortcut.useIndex ? store.index(indexName!) : store;
    final request = target.request(method, args);
    return (await Future.wait([request, if (method.isWrite) tx.done], eagerError: true))[0] as JSAny?;
  }
}

/// idb's `iterate` generator, in Dart's async iteration protocol (StreamIterator's [moveNext] and
/// [current]). The first [moveNext] opens `openCursor(...args)`; each later one awaits the move
/// the loop body made on [current], or calls `continue()` when it made none. A failed step ends
/// the iteration, as a throwing generator is done.
final class CursorIterator {
  CursorIterator._(this._source, this._args);

  final Source _source;
  final List<JSAny?> _args;
  bool _opened = false;
  Cursor? _cursor;

  /// The move the loop body made: the iterator's `advanceResults` entry.
  Future<Cursor?>? _advanced;

  IteratedCursor? _current;

  IteratedCursor get current => _current!;

  Future<bool> moveNext() async {
    _current = null;
    final Future<Cursor?> step;
    if (!_opened) {
      _opened = true;
      step = _source.openCursor(_args);
    } else {
      final cursor = _cursor;
      if (cursor == null) return false;
      _cursor = null;
      step = _advanced ?? cursor.continueTo();
    }
    final next = await step;
    _advanced = null;
    _cursor = next;
    if (next == null) return false;
    _current = IteratedCursor._(this, next);
    return true;
  }
}

/// The cursor an iteration yields (idb's proxied cursor): its moves call the host method and
/// record the result for the iterator instead of returning it.
final class IteratedCursor {
  IteratedCursor._(this._iterator, this.cursor);

  final CursorIterator _iterator;
  final Cursor cursor;

  void advance(List<JSAny?> args) => _iterator._advanced = cursor.advance(args);
  void continueTo([List<JSAny?> args = const []]) => _iterator._advanced = cursor.continueTo(args);
  void continuePrimaryKey(List<JSAny?> args) => _iterator._advanced = cursor.continuePrimaryKey(args);
}

/// openDB's `upgrade(db, oldVersion, newVersion, transaction)`.
typedef Upgrade = void Function(Database db, double oldVersion, double? newVersion, Transaction transaction);

/// openDB's `blocked` and `blocking`: `(currentVersion, blockedVersion, event)`.
typedef VersionChange = void Function(double currentVersion, double? blockedVersion, IDBVersionChangeEvent event);

/// idb's `openDB(name, version, { upgrade, blocked, blocking })`. Without a [version] (null) the
/// open is made without one: the current version, or 1 for a new database; an invalid one throws
/// here. `blocking` is attached to the connection once the open succeeds, before the returned
/// Future completes.
Future<Database> openDB(String name, {JSAny? version, Upgrade? upgrade, VersionChange? blocked, VersionChange? blocking}) {
  final request = version == null ? indexedDB.open(name) : indexedDB.open(name, version);
  final openPromise = _promisify(request, (result) => Database._(result as IDBDatabase));
  if (upgrade != null) {
    request.addEventListener('upgradeneeded', ((IDBVersionChangeEvent event) {
      upgrade(Database._(request.result as IDBDatabase), event.oldVersion, event.newVersion, Transaction._(request.transaction!));
    }).toJS);
  }
  if (blocked != null) {
    request.addEventListener('blocked', ((IDBVersionChangeEvent event) {
      blocked(event.oldVersion, event.newVersion, event);
    }).toJS);
  }
  return openPromise.then((db) {
    if (blocking != null) {
      db._raw.addEventListener('versionchange', ((IDBVersionChangeEvent event) {
        blocking(event.oldVersion, event.newVersion, event);
      }).toJS);
    }
    return db;
  });
}

/// idb's `deleteDB(name, { blocked })`; `blocked(currentVersion, event)`.
Future<void> deleteDB(String name, {void Function(double currentVersion, IDBVersionChangeEvent event)? blocked}) {
  final request = indexedDB.deleteDatabase(name);
  if (blocked != null) {
    request.addEventListener('blocked', ((IDBVersionChangeEvent event) {
      blocked(event.oldVersion, event);
    }).toJS);
  }
  return wrap(request).then((_) {});
}
