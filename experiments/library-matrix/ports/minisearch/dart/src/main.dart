// Entry of the Dart port and its boundary (README "Port convention"), the Dart port of
// libraries/minisearch/reference.mjs.
//
// The corpus input is the JS object itself, seen through dart:js_interop extension types (no
// copy). Each operation's arguments are read into the port's Dart types as the script runs, the
// way the adapter maps JSON onto the MiniSearch API: options and queries into SearchOptions and
// Query, the declarative `filter`, `boostDocument` and `boostTerm` into the closures they describe,
// ids into Strings and doubles. Documents stay JS objects that MiniSearch reads through its
// `extractField` (`document[name]`, the value read into a FieldValue). The result is built as plain
// JS objects and arrays, keys in the adapter's order.
//
// dart2js cannot emit ES module exports, so main() publishes `run` on globalThis.__matrixExports,
// which the builder's facade reads once and deletes.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'mini_search.dart';

extension type Input._(JSObject _) implements JSObject {
  external IndexOptionsJson get options;
  external JSArray<OperationJson> get operations;
}

extension type IndexOptionsJson._(JSObject _) implements JSObject {
  external JSArray<JSString>? get fields;
  external JSArray<JSString>? get storeFields;
  external String? get idField;
  external SearchOptionsJson? get searchOptions;
  external SearchOptionsJson? get autoSuggestOptions;

  /// `true`, `false` or an [AutoVacuumJson].
  external JSAny? get autoVacuum;
}

extension type AutoVacuumJson._(JSObject _) implements JSObject {
  external double? get batchSize;
  external double? get minDirtCount;
  external double? get minDirtFactor;
}

extension type SearchOptionsJson._(JSObject _) implements JSObject {
  external JSArray<JSString>? get fields;

  /// `{ name: number }`.
  external JSObject? get boost;
  external bool? get prefix;

  /// A boolean or a number.
  external JSAny? get fuzzy;
  external JSNumber? get maxFuzzy;
  external String? get combineWith;
  external WeightsJson? get weights;
  external Bm25Json? get bm25;
  external FilterJson? get filter;

  /// `[[id, factor], ...]`.
  external JSArray<JSArray<JSAny?>>? get boostDocument;

  /// `[[term, factor], ...]`.
  external JSArray<JSArray<JSAny?>>? get boostTerm;
}

/// A query tree node: search options and subqueries, or `{ wildcard: true }`.
extension type QueryNodeJson._(JSObject _) implements SearchOptionsJson {
  external bool? get wildcard;
  external JSArray<JSAny> get queries;
}

extension type WeightsJson._(JSObject _) implements JSObject {
  external double? get fuzzy;
  external double? get prefix;
}

extension type Bm25Json._(JSObject _) implements JSObject {
  external double? get k;
  external double? get b;
  external double? get d;
}

/// `{ field, in: [values] }`: `(result) => values.includes(result[field])`.
extension type FilterJson._(JSObject _) implements JSObject {
  external String get field;
  @JS('in')
  external JSArray<JSAny?> get values;
}

extension type VacuumOptionsJson._(JSObject _) implements JSObject {
  external double? get batchSize;
}

extension type OperationJson._(JSObject _) implements JSObject {
  external String get op;

  /// A corpus document: a JSON object.
  external JSObject get document;

  /// An array; `null` for `removeAll(null)`, absent for `removeAll()`.
  external JSArray<JSObject>? get documents;
  external JSAny? get id;
  external JSArray<JSAny?> get ids;

  /// A string or a [QueryNodeJson] (`search`), a string (`autoSuggest`).
  external JSAny get query;
  @JS('options')
  external SearchOptionsJson? get searchOptions;
  @JS('options')
  external VacuumOptionsJson? get vacuumOptions;
}

/// `Object.keys`, to read the `boost` object.
@JS('Object.keys')
external JSArray<JSString> _objectKeys(JSObject object);

void main() {
  final exports = JSObject();
  exports['run'] = run.toJS;
  globalContext['__matrixExports'] = exports;
}

JSObject run(Input input) {
  // The adapter's logger: the messages MiniSearch logs during the current operation.
  var warnings = <String>[];
  final index = MiniSearch(_indexOptions(input.options, (_, message, _) => warnings.add(message)));
  final results = <JSObject>[];
  for (final operation in input.operations.toDart) {
    final op = operation.op;
    final record = JSObject()..['op'] = op.toJS;
    warnings = [];
    // MiniSearch's own errors are recorded and the script goes on with the index as MiniSearch
    // left it, as the adapter does; anything else is a failure of the port, and propagates.
    String? error;
    try {
      _execute(index, op, operation, record);
    } on MiniSearchException catch (exception) {
      error = exception.message;
    } on _UnknownOperation catch (exception) {
      error = exception.message;
    }
    if (warnings.isNotEmpty) record['warnings'] = _strings(warnings);
    if (error != null) record['error'] = error.toJS;
    results.add(record);
  }
  return JSObject()..['results'] = results.toJS;
}

/// The adapter's `TypeError` for an operation it does not know, which it records as an error.
final class _UnknownOperation implements Exception {
  _UnknownOperation(String op) : message = 'unknown operation $op';

  final String message;
}

void _execute(MiniSearch<JSObject> index, String op, OperationJson operation, JSObject record) {
  switch (op) {
    case 'add':
      index.add(operation.document);
    case 'addAll':
      index.addAll(operation.documents!.toDart);
    case 'remove':
      index.remove(operation.document);
    case 'removeAll':
      final documents = operation.documents;
      if (documents.isUndefined) {
        index.removeAll();
      } else {
        index.removeAll(documents?.toDart);
      }
    case 'discard':
      index.discard(_id(operation.id));
    case 'discardAll':
      index.discardAll([for (final id in operation.ids.toDart) _id(id)]);
    case 'replace':
      index.replace(operation.document);
    // MiniSearch's promise settles after the synchronous run (CONTRACT.md, "Vacuuming").
    case 'vacuum':
      index.vacuum((batchSize: operation.vacuumOptions?.batchSize));
    case 'search':
      final hits = index.search(_query(operation.query), _callOptions(operation));
      record['hits'] = [for (final hit in hits) _hit(hit)].toJS;
    case 'autoSuggest':
      final suggestions = index.autoSuggest((operation.query as JSString).toDart, _callOptions(operation));
      record['suggestions'] = [for (final suggestion in suggestions) _suggestion(suggestion)].toJS;
    case 'has':
      record['value'] = index.has(_id(operation.id)).toJS;
    case 'getStoredFields':
      final stored = index.getStoredFields(_id(operation.id));
      record['value'] = stored == null ? null : _stored(stored);
    case 'stats':
      record['documentCount'] = index.documentCount.toJS;
      record['termCount'] = index.termCount.toJS;
      record['dirtCount'] = index.dirtCount.toJS;
      record['dirtFactor'] = index.dirtFactor.toJS;
      record['isVacuuming'] = index.isVacuuming.toJS;
    case 'toJSON':
      record['value'] = _json(index.toJson());
    default:
      throw _UnknownOperation(op);
  }
}

// -------------------------------------------------------------------------------------------------
// Reading.

Options<JSObject> _indexOptions(IndexOptionsJson json, Logger logger) {
  final fields = json.fields;
  if (fields == null) throw const MiniSearchException('MiniSearch: option "fields" must be provided');
  final storeFields = json.storeFields;
  final searchOptions = json.searchOptions;
  final autoSuggestOptions = json.autoSuggestOptions;
  return Options(
    fields: _names(fields),
    extractField: _extractField,
    idField: json.idField ?? 'id',
    storeFields: storeFields == null ? const [] : _names(storeFields),
    searchOptions: searchOptions == null ? const SearchOptions() : _searchOptions(searchOptions),
    autoSuggestOptions: autoSuggestOptions == null ? const SearchOptions() : _searchOptions(autoSuggestOptions),
    autoVacuum: _autoVacuum(json.autoVacuum),
    logger: logger,
  );
}

/// MiniSearch's default `extractField`, `document[fieldName]`.
FieldValue? _extractField(JSObject document, String fieldName) => _fieldValue(document[fieldName]);

/// `autoVacuum`: absent or `true` is the default options, `false` none, an object as given.
AutoVacuumOptions? _autoVacuum(JSAny? json) {
  if (json == null) return defaultAutoVacuumOptions;
  if (json.isA<JSBoolean>()) return (json as JSBoolean).toDart ? defaultAutoVacuumOptions : null;
  final options = json as AutoVacuumJson;
  return (batchSize: options.batchSize, minDirtCount: options.minDirtCount, minDirtFactor: options.minDirtFactor);
}

SearchOptions _callOptions(OperationJson operation) {
  final options = operation.searchOptions;
  return options == null ? const SearchOptions() : _searchOptions(options);
}

SearchOptions _searchOptions(SearchOptionsJson json) {
  final fields = json.fields;
  final boost = json.boost;
  final weights = json.weights;
  final bm25 = json.bm25;
  final filter = json.filter;
  final boostDocument = json.boostDocument;
  final boostTerm = json.boostTerm;
  return SearchOptions(
    fields: fields == null ? null : _names(fields),
    boost: boost == null
        ? null
        : {for (final name in _names(_objectKeys(boost))) name: (boost[name] as JSNumber).toDartDouble},
    prefix: switch (json.prefix) {
      null => null,
      true => everyTerm,
      false => noTerm,
    },
    fuzzy: _fuzzy(json.fuzzy),
    maxFuzzy: json.maxFuzzy?.toDartInt,
    combineWith: json.combineWith,
    weights: weights == null ? null : (fuzzy: weights.fuzzy, prefix: weights.prefix),
    // A missing parameter is undefined in MiniSearch's arithmetic: NaN.
    bm25: bm25 == null ? null : (k: bm25.k ?? double.nan, b: bm25.b ?? double.nan, d: bm25.d ?? double.nan),
    filter: filter == null ? null : _filter(filter.field, [for (final value in filter.values.toDart) _fieldValue(value)]),
    boostDocument: boostDocument == null
        ? null
        : _boostDocument([for (final pair in boostDocument.toDart) (_id(pair[0]), (pair[1] as JSNumber).toDartDouble)]),
    boostTerm: boostTerm == null
        ? null
        : _boostTerm([
            for (final pair in boostTerm.toDart) ((pair[0] as JSString).toDart, (pair[1] as JSNumber).toDartDouble),
          ]),
  );
}

/// `fuzzy`: a boolean or a number; `true` is 0.2 and `false` is 0 (see [SearchOptions.fuzzy]).
double? _fuzzy(JSAny? json) {
  if (json == null) return null;
  if (json.isA<JSBoolean>()) return (json as JSBoolean).toDart ? 0.2 : 0;
  return (json as JSNumber).toDartDouble;
}

/// `(result) => values.includes(result[field])`. A property the result does not have is
/// `undefined`, which no JSON value equals.
bool Function(SearchResult result) _filter(String field, List<FieldValue?> values) =>
    (result) => values.contains(result[field]);

/// `(id) => factor of the first pair whose id === id, else 1`.
BoostDocument _boostDocument(List<(Object, double)> factors) => (id, _, _) {
  for (final (key, factor) in factors) {
    if (key == id) return factor;
  }
  return 1;
};

/// `(term) => factor of the first pair whose term === term, else 1`.
BoostTerm _boostTerm(List<(String, double)> factors) => (term, _, _) {
  for (final (key, factor) in factors) {
    if (key == term) return factor;
  }
  return 1;
};

/// A string, `{ wildcard: true }` (MiniSearch.wildcard, at the top or as a subquery), or a tree.
Query _query(JSAny json) {
  if (json.isA<JSString>()) return TermsQuery((json as JSString).toDart);
  final node = json as QueryNodeJson;
  if (node.wildcard == true) return MiniSearch.wildcard;
  return QueryCombination([for (final subquery in node.queries.toDart) _query(subquery)], _searchOptions(node));
}

/// A document value: a string, a number or null; null for a property the document does not
/// have (`undefined`).
FieldValue? _fieldValue(JSAny? json) {
  if (json.isUndefined) return null;
  if (json.isNull) return const NullValue();
  if (json.isA<JSString>()) return TextValue((json as JSString).toDart);
  if (json.isA<JSNumber>()) return NumberValue((json as JSNumber).toDartDouble);
  throw ArgumentError('document values are strings, numbers or null');
}

/// An id: a string or a number (the contract's are strings and 32-bit integers).
Object _id(JSAny? json) {
  if (json.isA<JSString>()) return (json as JSString).toDart;
  if (json.isA<JSNumber>()) return (json as JSNumber).toDartDouble;
  throw ArgumentError('ids are strings or numbers');
}

List<String> _names(JSArray<JSString> names) => [for (final name in names.toDart) name.toDart];

// -------------------------------------------------------------------------------------------------
// Writing.

JSArray<JSString> _strings(List<String> values) => [for (final value in values) value.toJS].toJS;

JSAny _jsId(Object id) => switch (id) {
  final String text => text.toJS,
  final num number => number.toJS,
  _ => throw ArgumentError.value(id, 'id', 'ids are strings or numbers'),
};

JSAny? _jsValue(FieldValue value) => switch (value) {
  TextValue(:final text) => text.toJS,
  NumberValue(:final number) => number.toJS,
  NullValue() => null,
};

/// The adapter's copy of a result: its keys in order, `match` as `[term, fields]` pairs in
/// `Object.keys` order (which `terms` is), then the stored fields, as `Object.assign` added them
/// (a stored `id` replaces the id in place).
JSObject _hit(SearchResult result) {
  final hit = JSObject()
    ..['id'] = _jsId(result.id)
    ..['score'] = result.score.toJS
    ..['terms'] = _strings(result.terms)
    ..['queryTerms'] = _strings(result.queryTerms)
    ..['match'] = [
      for (final term in result.terms) [term.toJS, _strings(result.match[term]!)].toJS,
    ].toJS;
  final storedFields = result.storedFields;
  if (storedFields != null) {
    for (final MapEntry(key: name, :value) in storedFields.entries) {
      hit[name] = _jsValue(value);
    }
  }
  return hit;
}

JSObject _suggestion(Suggestion suggestion) => JSObject()
  ..['suggestion'] = suggestion.suggestion.toJS
  ..['terms'] = _strings(suggestion.terms)
  ..['score'] = suggestion.score.toJS;

/// `{ ...stored }`.
JSObject _stored(StoredFields storedFields) {
  final object = JSObject();
  for (final MapEntry(key: name, :value) in storedFields.entries) {
    object[name] = _jsValue(value);
  }
  return object;
}

/// `JSON.parse(JSON.stringify(value))` of a JSON-compatible Dart value: non-finite numbers become
/// null and -0 becomes 0. A JS object lists integer keys in ascending order, as the reference's
/// do.
JSAny? _json(Object? value) => switch (value) {
  null => null,
  final String text => text.toJS,
  final bool flag => flag.toJS,
  final num number => !number.isFinite ? null : (number == 0 ? 0.toJS : number.toJS),
  final List<Object?> items => [for (final item in items) _json(item)].toJS,
  final Map<String, Object?> entries => _jsonObject(entries),
  _ => throw ArgumentError.value(value, 'value', 'not a JSON value'),
};

JSObject _jsonObject(Map<String, Object?> entries) {
  final object = JSObject();
  for (final MapEntry(:key, :value) in entries.entries) {
    object[key] = _json(value);
  }
  return object;
}
