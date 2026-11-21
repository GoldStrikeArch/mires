// Entry of the Dart port: what libraries/idb/reference.mjs does, on the port's idb. A case is a
// script of idb calls; each call works on databases of its own, named by a fresh prefix, and
// closes and deletes them before it settles, also after a failure.
//
// The boundary: the corpus input is the JS object itself, seen through dart:js_interop extension
// types (no copy). Keys, values, versions, counts, directions, modes and options stay the JS
// values they are (JSAny?) and reach IndexedDB unchanged; an argument list holds only the
// arguments present. The result is a plain object { log, databases } of JS arrays.
//
// dart2js cannot emit ES module exports, so main() publishes `run` on globalThis.__matrixExports,
// which the builder's facade reads once and deletes.

import 'dart:async';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'host.dart';
import 'idb.dart';

extension type Input._(JSObject _) implements JSObject {
  external JSArray<Step> get steps;
}

/// A step of any kind; [op] says which of the types below it is.
extension type Step._(JSObject _) implements JSObject {
  external String get op;
  @JS('catch')
  external bool? get catches;
}

extension type OpenStep._(JSObject _) implements Step {
  external String get db;
  external String get conn;
  external JSAny? get version;
  external JSArray<UpgradeStep>? get upgrade;
  external String? get blocking;

  /// `"log"` or `{ close: conn }`.
  external JSAny? get blocked;
}

extension type CloseStep._(JSObject _) implements Step {
  external String get conn;
}

extension type DeleteStep._(JSObject _) implements Step {
  external String get db;
  external JSAny? get blocked;
}

extension type BlockedClose._(JSObject _) implements JSObject {
  external String get close;
}

extension type TxStep._(JSObject _) implements Step {
  external String get conn;
  external JSAny? get stores;
  external JSAny? get mode;
  external JSArray<Step> get steps;
}

extension type CallStep._(JSObject _) implements Step {
  external String get conn;
  external String get method;
  external String get store;
  external String? get index;
  external JSArray<JSAny?> get args;
}

/// A request on `store` (null for `tx.store`) or its `index`.
extension type RequestStep._(JSObject _) implements Step {
  external String? get store;
  external String? get index;
  external JSArray<JSAny?> get args;
}

extension type WrapStep._(JSObject _) implements RequestStep {
  external String get method;
}

extension type BatchStep._(JSObject _) implements Step {
  external JSArray<RequestStep> get requests;
  external bool get abort;
  external bool get done;
}

/// openCursor, openKeyCursor or iterate; each action is `[name, ...values]`.
extension type WalkStep._(JSObject _) implements RequestStep {
  external JSArray<JSArray<JSArray<JSAny?>>> get positions;
  external JSArray<JSArray<JSAny?>> get rest;
}

extension type UpgradeStep._(JSObject _) implements JSObject {
  external String get op;
  external String get name;
  external JSAny? get options;
  external String get store;
  external JSAny? get keyPath;
  external JSArray<JSAny?> get args;
}

extension type KeyRangeSpec._(JSObject _) implements JSObject {
  external JSAny? get only;
  external JSAny? get lower;
  external JSAny? get upper;
  external JSAny? get lowerOpen;
  external JSAny? get upperOpen;
}

/// A failure that the step where it happened has already logged.
final class _Failed implements Exception {}

/// An input outside the contract (CONTRACT.md, "Parity domain"): it rejects the call after
/// cleanup, with a TypeError, and is never logged.
final class _OutOfContract implements Exception {
  _OutOfContract(this.message);

  final String message;
}

const _upgrades = {'createStore', 'deleteStore', 'createIndex', 'deleteIndex', 'put', 'add', 'abort'};

/// The methods whose first argument is a query: a key, or a key range written as an object.
const _queryFirst = {'get', 'getKey', 'getAll', 'getAllKeys', 'count', 'delete', 'openCursor', 'openKeyCursor', 'iterate'};

typedef Entry = List<JSAny?>;

/// A host result as the log shows it: nothing when it is undefined (a miss), else the value.
List<JSAny?> _shown(JSAny? value) => value.isUndefined ? const [] : [value];

/// `[label, "ok", ...shown]`.
Entry _ok(String label, List<JSAny?> shown) => [label.toJS, 'ok'.toJS, ...shown];

Entry _error(String label, Object error) => [label.toJS, 'error'.toJS, jsErrorName(error).toJS];

Method _requestMethod(String op) => Method.of(op) ?? (throw _OutOfContract('not a request method: $op'));

/// The query methods take a key, or a key range written as an object, first.
List<JSAny?> _args(String method, JSArray<JSAny?> list) {
  final values = list.toDart;
  return _queryFirst.contains(method) && values.isNotEmpty ? [_query(values.first), ...values.skip(1)] : values;
}

/// A key is passed as it is, null included. Presence is tested with `in`, so `{ only: null }` is
/// only(null); an absent open flag is undefined, which IndexedDB reads as false.
JSAny? _query(JSAny? value) {
  if (value == null || !value.isA<JSObject>() || isArray(value)) return value;
  final range = value as KeyRangeSpec;
  final lower = reflectHas(range, 'lower');
  final upper = reflectHas(range, 'upper');
  if (reflectHas(range, 'only')) return IDBKeyRange.only(range.only);
  if (lower && upper) return IDBKeyRange.bound(range.lower, range.upper, range.lowerOpen, range.upperOpen);
  if (lower) return IDBKeyRange.lowerBound(range.lower, range.lowerOpen);
  return IDBKeyRange.upperBound(range.upper, range.upperOpen);
}

List<List<JSAny?>> _actions(WalkStep step, int position) {
  final positions = step.positions.toDart;
  return (position < positions.length ? positions[position] : step.rest).toDart.map((action) => action.toDart).toList();
}

String _name(List<JSAny?> action) => (action[0] as JSString).toDart;

bool _isWrite(List<JSAny?> action) => const {'update', 'delete'}.contains(_name(action));

void _remember(List<String> names, String name) {
  if (!names.contains(name)) names.add(name);
}

/// One move made at a walk's position: the Future it returned, and the move's name.
typedef _Move = ({String label, Future<Cursor?> next});

class _Session {
  _Session(this._input);

  final Input _input;
  final _prefix = 'mires-matrix-idb-${randomUUID()}-';
  final _log = <Entry>[];
  final _connections = <String, Database>{};
  final _opened = <Database>[];
  final _attempted = <String>[];
  final _live = <String>[];

  Future<JSObject> run() async {
    try {
      try {
        for (final step in _input.steps.toDart) {
          await _perform(step, null);
        }
      } on _Failed {
        // A logged failure ends the script there; anything else rejects the call.
      }
      final databases = <JSAny?>[];
      for (final name in _live) {
        databases.add(await _dump(name));
      }
      return JSObject()
        ..['log'] = [for (final entry in _log) entry.toJS].toJS
        ..['databases'] = databases.toJS;
    } finally {
      for (final db in _opened) {
        db.close();
      }
      for (final name in _attempted) {
        await deleteDB(_prefix + name);
      }
    }
  }

  /// Logs a failure where it happens; the [_Failed] that propagates says it is logged.
  Object _failed(String label, Object error) {
    if (error is _OutOfContract) return error;
    _log.add(_error(label, error));
    return _Failed();
  }

  /// One call: logs `[label, "ok", ...show(value)]` or `[label, "error", name]`.
  Future<T> _attempt<T>(String label, FutureOr<T> Function() action, List<JSAny?> Function(T value) show) async {
    final T value;
    try {
      value = await action();
    } catch (error) {
      throw _failed(label, error);
    }
    _log.add(_ok(label, show(value)));
    return value;
  }

  /// A call whose result is a host value, undefined for a miss.
  Future<void> _attemptValue(String label, FutureOr<JSAny?> Function() action) => _attempt<JSAny?>(label, action, _shown);

  /// A call with no result to show.
  Future<void> _attemptVoid(String label, FutureOr<void> Function() action) => _attempt<void>(label, action, (_) => const []);

  Database _connection(String conn) => _connections[conn] ?? (throw _OutOfContract('no connection named $conn'));

  Transaction _inside(Transaction? tx) => tx ?? (throw _OutOfContract('a transaction step outside a transaction'));

  Source _source(Transaction? tx, RequestStep step) {
    final transaction = _inside(tx);
    final storeName = step.store;
    final store = storeName == null
        ? transaction.store ?? (throw _OutOfContract('store: null in a transaction with several stores'))
        : transaction.objectStore(storeName);
    final indexName = step.index;
    return indexName == null ? store : store.index(indexName);
  }

  Future<void> _perform(Step step, Transaction? tx) async {
    try {
      await _execute(step, tx);
    } on _Failed {
      if (step.catches != true) rethrow;
    }
  }

  Future<void> _execute(Step step, Transaction? tx) async {
    switch (step.op) {
      case 'openDB':
        await _open(step as OpenStep);
      case 'close':
        await _attemptVoid('close', () => _connection((step as CloseStep).conn).close());
      case 'deleteDB':
        await _remove(step as DeleteStep);
      case 'tx':
        await _transaction(step as TxStep);
      case 'call':
        await _shortcut(step as CallStep);
      case 'batch':
        await _batch(step as BatchStep, _inside(tx));
      case 'openCursor' || 'openKeyCursor':
        await _walk(step as WalkStep, tx);
      case 'iterate':
        await _iterate(step as WalkStep, tx);
      case 'abort':
        await _attemptVoid('abort', () => _inside(tx).abort());
      case 'done':
        await _attemptVoid('done', () => _inside(tx).done);
      case 'store':
        // Without a store there is no name to show, as `tx.store?.name` is undefined.
        await _attempt<String?>('store', () => _inside(tx).store?.name, (name) => [if (name != null) name.toJS]);
      case 'wrap':
        final request = step as WrapStep;
        final method = _requestMethod(request.method);
        await _attemptValue('wrap', () => wrap(unwrap(_source(tx, request)).request(method.name, _args(method.name, request.args))));
      case final op:
        final request = step as RequestStep;
        final method = _requestMethod(op);
        await _attemptValue(op, () => _source(tx, request).request(method, _args(op, request.args)));
    }
  }

  Database? _closingConnection(JSAny? blocked) =>
      blocked != null && blocked.isA<JSObject>() ? _connection((blocked as BlockedClose).close) : null;

  Future<void> _open(OpenStep step) async {
    final upgrades = step.upgrade?.toDart;
    for (final op in upgrades ?? const <UpgradeStep>[]) {
      if (!_upgrades.contains(op.op)) throw _OutOfContract('not an upgrade step: ${op.op}');
    }
    _remember(_attempted, step.db);
    final closing = _closingConnection(step.blocked);
    final blocking = step.blocking;
    final db = await _attempt(
      'openDB',
      () => openDB(
        _prefix + step.db,
        version: step.version,
        upgrade: upgrades == null
            ? null
            : (db, oldVersion, newVersion, transaction) {
                _log.add(['upgrade'.toJS, oldVersion.toJS, newVersion?.toJS]);
                for (final op in upgrades) {
                  _upgrade(op, db, transaction);
                }
              },
        blocked: step.blocked == null
            ? null
            : (currentVersion, blockedVersion, _) {
                _log.add(['blocked'.toJS, currentVersion.toJS, blockedVersion?.toJS]);
                closing?.close();
              },
        blocking: blocking == null
            ? null
            : (currentVersion, blockedVersion, event) {
                _log.add(['blocking'.toJS, step.conn.toJS, currentVersion.toJS, blockedVersion?.toJS]);
                if (blocking == 'close') (event.target as IDBDatabase).close();
              },
      ),
      (Database db) => [
        [db.version.toJS, [for (final name in db.objectStoreNames) name.toJS].toJS].toJS,
      ],
    );
    _connections[step.conn] = db;
    _opened.add(db);
    _remember(_live, step.db);
  }

  Future<void> _remove(DeleteStep step) async {
    _remember(_attempted, step.db);
    final closing = _closingConnection(step.blocked);
    await _attemptVoid(
      'deleteDB',
      () => deleteDB(
        _prefix + step.db,
        blocked: step.blocked == null
            ? null
            : (currentVersion, _) {
                _log.add(['blocked'.toJS, currentVersion.toJS]);
                closing?.close();
              },
      ),
    );
    _live.remove(step.db);
  }

  /// Runs synchronously inside the upgrade callback; a failure is logged and the callback goes on.
  void _upgrade(UpgradeStep op, Database db, Transaction transaction) {
    try {
      switch (op.op) {
        case 'createStore':
          _log.add(['createStore'.toJS, 'ok'.toJS, db.createObjectStore(op.name, op.options).name.toJS]);
        case 'deleteStore':
          db.deleteObjectStore(op.name);
          _log.add(['deleteStore'.toJS, 'ok'.toJS]);
        case 'createIndex':
          _log.add(['createIndex'.toJS, 'ok'.toJS, transaction.objectStore(op.store).createIndex(op.name, op.keyPath, op.options).name.toJS]);
        case 'deleteIndex':
          transaction.objectStore(op.store).deleteIndex(op.name);
          _log.add(['deleteIndex'.toJS, 'ok'.toJS]);
        case 'abort':
          transaction.abort();
          _log.add(['abort'.toJS, 'ok'.toJS]);
        case final write:
          // put or add: issued, not awaited. A failed write aborts the upgrade, which rejects the open.
          transaction.objectStore(op.store).request(_requestMethod(write), op.args.toDart).ignore();
      }
    } catch (error) {
      _log.add(_error(op.op, error));
    }
  }

  Future<void> _transaction(TxStep step) async {
    final Transaction tx;
    try {
      tx = _connection(step.conn).transaction(step.stores, step.mode);
    } catch (error) {
      throw _failed('tx', error);
    }
    for (final inner in step.steps.toDart) {
      await _perform(inner, tx);
    }
  }

  Future<void> _shortcut(CallStep step) async {
    final shortcut = Shortcut.of(step.method) ?? (throw _OutOfContract('not a database shortcut: ${step.method}'));
    if (shortcut.useIndex != (step.index != null)) throw _OutOfContract('an index goes with the FromIndex forms only');
    await _attemptValue(step.method, () {
      final db = _connection(step.conn);
      return db.shortcut(shortcut, step.store, step.index, _args(shortcut.method.name, step.args));
    });
  }

  /// Issues every request before awaiting any, then logs each outcome in issue order.
  Future<void> _batch(BatchStep step, Transaction tx) async {
    final requests = step.requests.toDart;
    final methods = [for (final request in requests) _requestMethod(request.op)];
    // Each issue is the reference's async callback: a synchronous failure is a failed Future.
    // Every outcome is settled at once, as Promise.allSettled does, so none is left unhandled.
    final pending = [
      for (final (i, request) in requests.indexed)
        _settle(() => _source(tx, request).request(methods[i], _args(request.op, request.args)).then(_shown)),
    ];
    if (step.abort) {
      try {
        await _attemptVoid('abort', tx.abort);
      } on _Failed {
        // The batch goes on.
      }
    }
    final labels = [for (final request in requests) request.op];
    if (step.done) {
      pending.add(_settle(() => tx.done.then((_) => const <JSAny?>[])));
      labels.add('done');
    }
    final outcomes = await Future.wait(pending);
    for (final outcome in outcomes) {
      if (outcome case (_, _OutOfContract outside)) throw outside;
    }
    for (final (i, (shown, error)) in outcomes.indexed) {
      _log.add(error == null ? _ok(labels[i], shown) : _error(labels[i], error));
    }
  }

  /// An outcome: what the log shows of its value, or what it threw or failed with.
  Future<(List<JSAny?>, Object?)> _settle(Future<List<JSAny?>> Function() issue) {
    try {
      return issue().then((shown) => (shown, null), onError: (Object error) => (const <JSAny?>[], error));
    } catch (error) {
      return Future.value((const <JSAny?>[], error));
    }
  }

  void _position(WalkStep step, Cursor cursor) {
    _log.add(step.op == 'openKeyCursor'
        ? ['cursor'.toJS, cursor.key, cursor.primaryKey]
        : ['cursor'.toJS, cursor.key, cursor.primaryKey, cursor.value]);
  }

  /// `cursor.update(value)` and `cursor.delete()`, awaited and logged.
  Future<void> _write(Cursor cursor, List<JSAny?> action) => _name(action) == 'update'
      ? _attemptValue('update', () => cursor.update(action.length > 1 ? action[1] : null))
      : _attemptValue('delete', cursor.delete);

  /// A walk with the promise-returning moves; a position without a move ends it.
  Future<void> _walk(WalkStep step, Transaction? tx) async {
    Cursor? cursor;
    try {
      final source = _source(tx, step);
      final args = _args(step.op, step.args);
      cursor = await (step.op == 'openKeyCursor' ? source.openKeyCursor(args) : source.openCursor(args));
    } catch (error) {
      throw _failed(step.op, error);
    }
    for (var position = 0; cursor != null; position++) {
      _position(step, cursor);
      _Move? moved;
      for (final action in _actions(step, position)) {
        if (_name(action) == 'break') return;
        if (_isWrite(action)) {
          await _write(cursor, action);
        } else {
          moved = _move(cursor, action);
        }
      }
      if (moved == null) return;
      try {
        cursor = await moved.next;
      } catch (error) {
        throw _failed(moved.label, error);
      }
    }
    _log.add(['end'.toJS]);
  }

  _Move _move(Cursor cursor, List<JSAny?> action) {
    final name = _name(action);
    final values = action.sublist(1);
    try {
      return (
        label: name,
        next: switch (name) {
          'continue' => cursor.continueTo(values),
          'advance' => cursor.advance(values),
          'continuePrimaryKey' => cursor.continuePrimaryKey(values),
          _ => throw _OutOfContract('not a cursor action: $name'),
        },
      );
    } catch (error) {
      throw _failed(name, error);
    }
  }

  /// The async iterator: a move only records the next step, and a position without one continues.
  Future<void> _iterate(WalkStep step, Transaction? tx) async {
    final CursorIterator iterator;
    try {
      iterator = _source(tx, step).iterate(_args('iterate', step.args));
    } catch (error) {
      throw _failed('iterate', error);
    }
    for (var position = 0;; position++) {
      final bool more;
      try {
        more = await iterator.moveNext();
      } catch (error) {
        throw _failed('iterate', error);
      }
      if (!more) break;
      final cursor = iterator.current;
      _log.add(['cursor'.toJS, cursor.cursor.key, cursor.cursor.primaryKey, cursor.cursor.value]);
      for (final action in _actions(step, position)) {
        if (_name(action) == 'break') return;
        if (_isWrite(action)) {
          await _write(cursor.cursor, action);
        } else {
          _intercepted(cursor, action);
        }
      }
    }
    _log.add(['end'.toJS]);
  }

  void _intercepted(IteratedCursor cursor, List<JSAny?> action) {
    final name = _name(action);
    final values = action.sublist(1);
    try {
      switch (name) {
        case 'continue':
          cursor.continueTo(values);
        case 'advance':
          cursor.advance(values);
        case 'continuePrimaryKey':
          cursor.continuePrimaryKey(values);
        default:
          throw _OutOfContract('not a cursor action: $name');
      }
    } catch (error) {
      throw _failed(name, error);
    }
  }

  /// `[name, version, stores]`, each store `[name, keyPath, autoIncrement, indexes, records]`, read
  /// in one readonly transaction; records are getAllKeys() zipped with getAll().
  Future<JSArray<JSAny?>> _dump(String name) async {
    final db = await openDB(_prefix + name);
    try {
      final names = db.objectStoreNames;
      final stores = <JSAny?>[];
      if (names.isNotEmpty) {
        // Named here: the reference's transaction(names) defaults to readonly.
        final tx = db.transaction([for (final name in names) name.toJS].toJS, 'readonly'.toJS);
        for (final storeName in names) {
          final store = tx.objectStore(storeName);
          final indexes = <JSAny?>[];
          for (final indexName in store.indexNames) {
            final index = store.index(indexName);
            indexes.add([indexName.toJS, index.keyPath, index.unique.toJS, index.multiEntry.toJS].toJS);
          }
          final keys = (await store.request(Method.getAllKeys) as JSArray<JSAny?>).toDart;
          final values = (await store.request(Method.getAll) as JSArray<JSAny?>).toDart;
          final records = [for (final (i, key) in keys.indexed) [key, values[i]].toJS];
          stores.add([storeName.toJS, store.keyPath, store.autoIncrement.toJS, indexes.toJS, records.toJS].toJS);
        }
        await tx.done;
      }
      return [name.toJS, db.version.toJS, stores.toJS].toJS;
    } finally {
      db.close();
    }
  }
}

JSPromise<JSObject> run(Input input) => () async {
      try {
        return await _Session(input).run();
      } on _OutOfContract catch (outside) {
        throw HostTypeError(outside.message);
      }
    }().toJS;

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}
