// The MiniSearch class of minisearch 7.2.0 (dist/es/index.js), method by method, for the subset in
// libraries/minisearch/CONTRACT.md: documents, the inverted index, search with BM25+, prefix and
// fuzzy matching and query trees, autoSuggest, discarding with its vacuuming, toJSON.
//
// The data structures are MiniSearch's: every JavaScript Map is a Dart Map (a LinkedHashMap, which
// keeps insertion order as a JavaScript Map does and which MiniSearch's results and iteration
// orders depend on); arrays with holes are lists of nullable values, a hole being null.

import 'dart:collection';
import 'dart:math' as math;

import 'options.dart';
import 'searchable_map.dart';

export 'options.dart';

/// An error MiniSearch throws, with its message.
final class MiniSearchException implements Exception {
  const MiniSearchException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// A term's index data (`fieldsData`): field id to posting list, a posting list being short
/// document id to term frequency.
typedef FieldsData = Map<int, Map<int, int>>;

/// A result's `match`: document term to the names of the fields it matched in. MiniSearch keeps it
/// in a plain object, whose keys `Object.keys` lists array indices first ([objectKeys]).
typedef MatchInfo = Map<String, List<String>>;

/// A search result: MiniSearch's result object, on which `Object.assign` has put the stored fields.
final class SearchResult {
  const SearchResult({
    required this.id,
    required this.score,
    required this.terms,
    required this.queryTerms,
    required this.match,
    required this.storedFields,
  });

  final Object id;
  final double score;

  /// The document terms that matched: `Object.keys(match)`.
  final List<String> terms;

  /// The query terms that matched.
  final List<String> queryTerms;
  final MatchInfo match;
  final StoredFields? storedFields;

  /// `result[name]` for a scalar property: a stored field, else the result's own `id` or `score`;
  /// null when the result has no such property (`undefined`).
  FieldValue? operator [](String name) =>
      storedFields?[name] ??
      switch (name) {
        'id' => idValue(id),
        'score' => NumberValue(score),
        _ => null,
      };
}

final class Suggestion {
  const Suggestion(this.suggestion, this.terms, this.score);

  final String suggestion;
  final List<String> terms;
  final double score;
}

/// A query: a string, [MiniSearch.wildcard], or a combination of queries with its own options.
sealed class Query {
  const Query();
}

final class TermsQuery extends Query {
  const TermsQuery(this.text);

  final String text;
}

final class WildcardQuery extends Query {
  const WildcardQuery._();
}

final class QueryCombination extends Query {
  const QueryCombination(this.queries, [this.options = const SearchOptions()]);

  final List<Query> queries;
  final SearchOptions options;
}

/// One document's accumulated result in a result map (`{ score, terms, match }`).
final class _RawResult {
  _RawResult(this.score, this.terms, this.match);

  double score;
  final List<String> terms;
  final MatchInfo match;
}

/// A result map, keyed by short id: its insertion order is the order of the results.
typedef _RawResults = Map<int, _RawResult>;

/// One query term with its options (`termToQuerySpec`).
typedef _QuerySpec = ({String term, double fuzzy, bool prefix, double termBoost});

final class MiniSearch<D> {
  MiniSearch(Options<D> options)
    : _fields = options.fields,
      _idField = options.idField,
      _storeFields = options.storeFields,
      _extractField = options.extractField,
      _logger = options.logger,
      _searchOptions = defaultSearchOptions.merge(options.searchOptions),
      _autoSuggestOptions = defaultAutoSuggestOptions.merge(options.autoSuggestOptions),
      _autoVacuum = options.autoVacuum {
    _addFields(_fields);
  }

  /// `MiniSearch.wildcard`: matches every document.
  static const Query wildcard = WildcardQuery._();

  final List<String> _fields;
  final String _idField;
  final List<String> _storeFields;
  final FieldValue? Function(D document, String fieldName) _extractField;
  final Logger _logger;

  /// The library defaults under the constructor's `searchOptions`.
  final SearchOptions _searchOptions;

  /// The library defaults under the constructor's `autoSuggestOptions`.
  final SearchOptions _autoSuggestOptions;

  /// `autoVacuum`, null for `false`. `discardAll` turns it off while it runs.
  AutoVacuumOptions? _autoVacuum;

  SearchableMap<FieldsData> _index = SearchableMap();
  int _documentCount = 0;
  Map<int, Object> _documentIds = {};
  Map<Object, int> _idToShortId = {};
  final Map<String, int> _fieldIds = {};

  /// Per document, the length of each field it has; null (a hole) for the others.
  Map<int, List<int?>> _fieldLength = {};

  /// The average length of each field; null (a hole) where none was ever set.
  List<double?> _avgFieldLength = [];
  int _nextId = 0;
  Map<int, StoredFields> _storedFields = {};
  int _dirtCount = 0;

  /// `_currentVacuum != null`. MiniSearch clears it in a microtask after a vacuum's last batch,
  /// which a synchronous run never reaches: once a vacuum has started it stays set (CONTRACT.md,
  /// "Vacuuming in a synchronous run").
  bool _vacuumCurrent = false;

  // -----------------------------------------------------------------------------------------------
  // Documents.

  void add(D document) {
    final id = _documentId(document);
    if (_idToShortId.containsKey(id)) throw MiniSearchException('MiniSearch: duplicate ID $id');
    final shortDocumentId = _addDocumentId(id);
    _saveStoredFields(shortDocumentId, document);
    for (final field in _fields) {
      final fieldValue = _extractField(document, field);
      if (fieldValue == null || fieldValue is NullValue) continue;
      final tokens = _tokenize(_stringifyField(fieldValue));
      final fieldId = _fieldIds[field]!;
      final uniqueTerms = _distinctCount(tokens);
      _addFieldLength(shortDocumentId, fieldId, _documentCount - 1, uniqueTerms);
      for (final term in tokens) {
        final processedTerm = _processTerm(term);
        if (processedTerm.isNotEmpty) _addTerm(fieldId, shortDocumentId, processedTerm);
      }
    }
  }

  void addAll(Iterable<D> documents) {
    for (final document in documents) {
      add(document);
    }
  }

  /// Removes a document, which must be given as it was added (nothing checks it).
  void remove(D document) {
    final id = _documentId(document);
    final shortId = _idToShortId[id];
    if (shortId == null) {
      throw MiniSearchException('MiniSearch: cannot remove document with ID $id: it is not in the index');
    }
    for (final field in _fields) {
      final fieldValue = _extractField(document, field);
      if (fieldValue == null || fieldValue is NullValue) continue;
      final tokens = _tokenize(_stringifyField(fieldValue));
      final fieldId = _fieldIds[field]!;
      final uniqueTerms = _distinctCount(tokens);
      _removeFieldLength(shortId, fieldId, _documentCount, uniqueTerms);
      for (final term in tokens) {
        final processedTerm = _processTerm(term);
        if (processedTerm.isNotEmpty) _removeTerm(fieldId, shortId, processedTerm);
      }
    }
    _storedFields.remove(shortId);
    _documentIds.remove(shortId);
    _idToShortId.remove(id);
    _fieldLength.remove(shortId);
    _documentCount -= 1;
  }

  /// `removeAll(documents)` removes each document; `removeAll()` with no argument makes a new,
  /// empty index (`dirtCount` and the vacuum state are kept). An explicit null argument is
  /// MiniSearch's error: it tells the two forms apart by `arguments.length`, which the default
  /// value stands for here.
  void removeAll([Iterable<D>? documents = const _NoArgument()]) {
    if (documents is _NoArgument) {
      _index = SearchableMap();
      _documentCount = 0;
      _documentIds = {};
      _idToShortId = {};
      _fieldLength = {};
      _avgFieldLength = [];
      _storedFields = {};
      _nextId = 0;
    } else if (documents == null) {
      throw const MiniSearchException(
        'Expected documents to be present. Omit the argument to remove all documents.',
      );
    } else {
      for (final document in documents) {
        remove(document);
      }
    }
  }

  /// Discards a document: it leaves the id maps and the averages now; its postings stay in the
  /// index until a search meets them or a vacuum runs.
  void discard(Object id) {
    final shortId = _idToShortId[id];
    if (shortId == null) {
      throw MiniSearchException('MiniSearch: cannot discard document with ID $id: it is not in the index');
    }
    _idToShortId.remove(id);
    _documentIds.remove(shortId);
    _storedFields.remove(shortId);
    // `forEach` over the array skips its holes: the fields the document did not have.
    for (final (fieldId, fieldLength) in (_fieldLength[shortId] ?? const <int?>[]).indexed) {
      if (fieldLength != null) _removeFieldLength(shortId, fieldId, _documentCount, fieldLength);
    }
    _fieldLength.remove(shortId);
    _documentCount -= 1;
    _dirtCount += 1;
    _maybeAutoVacuum();
  }

  /// Discards every id with automatic vacuuming off, then considers one automatic vacuum. A
  /// failing discard ends it there, before that vacuum.
  void discardAll(Iterable<Object> ids) {
    final autoVacuum = _autoVacuum;
    try {
      _autoVacuum = null;
      for (final id in ids) {
        discard(id);
      }
    } finally {
      _autoVacuum = autoVacuum;
    }
    _maybeAutoVacuum();
  }

  /// `discard(document[idField])` (which may vacuum), then `add(document)`.
  void replace(D updatedDocument) {
    final id = switch (_extractField(updatedDocument, _idField)) {
      TextValue(:final text) => text,
      NumberValue(:final number) => number,
      // `discard(undefined)` or `discard(null)`: no document has that id.
      final missing => throw MiniSearchException(
        'MiniSearch: cannot discard document with ID ${missing == null ? 'undefined' : 'null'}: it is not in the index',
      ),
    };
    discard(id);
    add(updatedDocument);
  }

  bool has(Object id) => _idToShortId.containsKey(id);

  StoredFields? getStoredFields(Object id) {
    final shortId = _idToShortId[id];
    if (shortId == null) return null;
    return _storedFields[shortId];
  }

  // -----------------------------------------------------------------------------------------------
  // Vacuuming.

  /// A vacuum without conditions, unless one is already current. MiniSearch returns a promise of
  /// its end, which settles after a synchronous run.
  void vacuum([VacuumOptions options = (batchSize: null)]) => _conditionalVacuum(options, null);

  void _maybeAutoVacuum() {
    final autoVacuum = _autoVacuum;
    if (autoVacuum == null) return;
    final (:batchSize, :minDirtCount, :minDirtFactor) = autoVacuum;
    _conditionalVacuum((batchSize: batchSize), (minDirtCount: minDirtCount, minDirtFactor: minDirtFactor));
  }

  void _conditionalVacuum(VacuumOptions options, VacuumConditions? conditions) {
    // A request made while a vacuum is current is chained behind it (`_enqueuedVacuum`), to run
    // once it settles: after a synchronous run, so never here.
    if (_vacuumCurrent) return;
    if (!_vacuumConditionsMet(conditions)) return;
    // `this._currentVacuum = this.performVacuuming(options)`: set once the vacuum's synchronous
    // part has run.
    _performVacuuming(options);
    _vacuumCurrent = true;
  }

  /// The synchronous part of `performVacuuming`: every term in iteration order loses its postings
  /// of documents no longer present, until the first batch boundary, where MiniSearch awaits a
  /// timer and the rest of the vacuum happens after the run. Only a walk that ends without
  /// reaching a boundary reduces `dirtCount`.
  void _performVacuuming(VacuumOptions options) {
    final initialDirtCount = _dirtCount;
    final batchSize = _orElse(options.batchSize, defaultBatchSize);
    var i = 1;
    for (final (term, fieldsData) in _index) {
      // Over snapshots of the maps: MiniSearch deletes the entry it is visiting from the live map,
      // which a JavaScript Map iterator survives; nothing else is deleted during these loops.
      for (final MapEntry(key: fieldId, value: fieldIndex) in fieldsData.entries.toList()) {
        for (final shortId in fieldIndex.keys.toList()) {
          if (_documentIds.containsKey(shortId)) continue;
          if (fieldIndex.length <= 1) {
            fieldsData.remove(fieldId);
          } else {
            fieldIndex.remove(shortId);
          }
        }
      }
      if (_index.get(term)!.isEmpty) _index.delete(term);
      if (i % batchSize == 0) return;
      i += 1;
    }
    _dirtCount -= initialDirtCount;
  }

  bool _vacuumConditionsMet(VacuumConditions? conditions) {
    if (conditions == null) return true;
    final minDirtCount = _orElse(conditions.minDirtCount, defaultMinDirtCount);
    final minDirtFactor = _orElse(conditions.minDirtFactor, defaultMinDirtFactor);
    return dirtCount >= minDirtCount && dirtFactor >= minDirtFactor;
  }

  // -----------------------------------------------------------------------------------------------
  // State.

  bool get isVacuuming => _vacuumCurrent;

  /// The number of documents discarded since the most recent completed vacuum.
  int get dirtCount => _dirtCount;

  double get dirtFactor => _dirtCount / (1 + _documentCount + _dirtCount);

  int get documentCount => _documentCount;

  int get termCount => _index.length;

  /// `toJSON()`: the index as JSON-compatible values (maps keyed by strings, lists, strings,
  /// numbers and null), short ids and field ids written as decimal keys. `averageFieldLength` can
  /// hold NaN, which JSON writes as null, as MiniSearch's can.
  Map<String, Object?> toJson() => {
    'documentCount': _documentCount,
    'nextId': _nextId,
    'documentIds': {for (final MapEntry(key: shortId, value: id) in _documentIds.entries) '$shortId': id},
    'fieldIds': {..._fieldIds},
    'fieldLength': {for (final MapEntry(key: shortId, value: lengths) in _fieldLength.entries) '$shortId': lengths},
    'averageFieldLength': _avgFieldLength,
    'storedFields': {
      for (final MapEntry(key: shortId, value: fields) in _storedFields.entries)
        '$shortId': {for (final MapEntry(key: name, value: value) in fields.entries) name: jsonValue(value)},
    },
    'dirtCount': _dirtCount,
    'index': [
      for (final (term, fieldIndex) in _index)
        [
          term,
          {
            for (final MapEntry(key: fieldId, value: freqs) in fieldIndex.entries)
              '$fieldId': {for (final MapEntry(key: shortId, value: frequency) in freqs.entries) '$shortId': frequency},
          },
        ],
    ],
    'serializationVersion': 2,
  };

  // -----------------------------------------------------------------------------------------------
  // Search.

  List<SearchResult> search(Query query, [SearchOptions searchOptions = const SearchOptions()]) {
    final searchOptionsWithDefaults = _searchOptions.merge(searchOptions);
    final rawResults = _executeQuery(query, searchOptions);
    final results = <SearchResult>[];
    final filter = searchOptionsWithDefaults.filter;
    for (final MapEntry(key: docId, value: raw) in rawResults.entries) {
      // The quality is the number of distinct query terms that matched (not the document terms,
      // which prefix and fuzzy matching make differ).
      final quality = raw.terms.isEmpty ? 1 : raw.terms.length;
      final result = SearchResult(
        id: _documentIds[docId]!,
        score: raw.score * quality,
        terms: objectKeys(raw.match),
        queryTerms: raw.terms,
        match: raw.match,
        storedFields: _storedFields[docId],
      );
      if (filter == null || filter(result)) results.add(result);
    }
    // A wildcard query without boostDocument is not sorted: every score is 1.
    if (query is WildcardQuery && searchOptionsWithDefaults.boostDocument == null) return results;
    return _sortedByScore(results, (result) => result.score);
  }

  List<Suggestion> autoSuggest(String queryString, [SearchOptions options = const SearchOptions()]) {
    final opts = _autoSuggestOptions.merge(options);
    final suggestions = <String, _SuggestionGroup>{};
    for (final SearchResult(:score, :terms) in search(TermsQuery(queryString), opts)) {
      final phrase = terms.join(' ');
      final suggestion = suggestions[phrase];
      if (suggestion != null) {
        suggestion.score += score;
        suggestion.count += 1;
      } else {
        suggestions[phrase] = _SuggestionGroup(score, terms);
      }
    }
    final results = [
      for (final MapEntry(key: suggestion, value: group) in suggestions.entries)
        Suggestion(suggestion, group.terms, group.score / group.count),
    ];
    return _sortedByScore(results, (suggestion) => suggestion.score);
  }

  _RawResults _executeQuery(Query query, SearchOptions searchOptions) {
    switch (query) {
      case WildcardQuery():
        return _executeWildcardQuery(searchOptions);
      case QueryCombination(:final queries, options: final nodeOptions):
        // `{ ...searchOptions, ...query }`: the constructor's search options are not merged in, so
        // a node with no combineWith in it or above it combines with OR.
        final options = searchOptions.merge(nodeOptions);
        final results = [for (final subquery in queries) _executeQuery(subquery, options)];
        return _combineResults(results, options.combineWith);
      case TermsQuery(:final text):
        final options = _searchOptions.merge(searchOptions);
        final terms = [
          for (final token in _tokenize(text))
            if (_processTerm(token) case final term when term.isNotEmpty) term,
        ];
        final queries = [for (final (i, term) in terms.indexed) _termToQuerySpec(options, term, i, terms)];
        final results = [for (final query in queries) _executeQuerySpec(query, options)];
        return _combineResults(results, options.combineWith);
    }
  }

  /// The exact term, then (reading the index as the exact term's cleanup left it) the prefix
  /// matches, then the fuzzy matches that are not prefix matches.
  _RawResults _executeQuerySpec(_QuerySpec query, SearchOptions searchOptions) {
    final options = _searchOptions.merge(searchOptions);
    // `fieldBoosts`, an object built field by field: a repeated field keeps its first position.
    final boosts = <String, double>{
      for (final field in options.fields ?? _fields) field: _orElse(options.boost?[field], 1),
    };
    final boostDocument = options.boostDocument;
    // Both are always set: the index's search options hold the library defaults.
    final maxFuzzy = options.maxFuzzy!;
    final bm25params = options.bm25!;
    // `{ ...defaultSearchOptions.weights, ...weights }`
    final fuzzyWeight = options.weights?.fuzzy ?? defaultWeights.fuzzy;
    final prefixWeight = options.weights?.prefix ?? defaultWeights.prefix;
    final data = _index.get(query.term);
    final results = _termResults(query.term, query.term, 1, query.termBoost, data, boosts, boostDocument, bm25params);
    SearchableMap<FieldsData>? prefixMatches;
    Map<String, (FieldsData, int)>? fuzzyMatches;
    if (query.prefix) prefixMatches = _index.atPrefix(query.term);
    if (query.fuzzy != 0) {
      final fuzzy = query.fuzzy;
      final maxDistance = fuzzy < 1 ? math.min(maxFuzzy, (query.term.length * fuzzy).round()) : fuzzy.toInt();
      if (maxDistance != 0) fuzzyMatches = _index.fuzzyGet(query.term, maxDistance);
    }
    if (prefixMatches != null) {
      for (final (term, data) in prefixMatches) {
        final distance = term.length - query.term.length;
        if (distance == 0) continue; // Skip exact match.
        // A term that is also a prefix match is scored as a prefix match only.
        fuzzyMatches?.remove(term);
        // The weight approaches 0 as the distance grows, prefixWeight at the (hypothetical)
        // distance 0, more slowly than a fuzzy match's.
        final weight = prefixWeight * term.length / (term.length + 0.3 * distance);
        _termResults(query.term, term, weight, query.termBoost, data, boosts, boostDocument, bm25params, results);
      }
    }
    if (fuzzyMatches != null) {
      for (final MapEntry(key: term, value: (data, distance)) in fuzzyMatches.entries) {
        if (distance == 0) continue; // Skip exact match.
        // The weight approaches 0 as the distance grows, fuzzyWeight at the (hypothetical)
        // distance 0.
        final weight = fuzzyWeight * term.length / (term.length + distance);
        _termResults(query.term, term, weight, query.termBoost, data, boosts, boostDocument, bm25params, results);
      }
    }
    return results;
  }

  _RawResults _executeWildcardQuery(SearchOptions searchOptions) {
    final results = <int, _RawResult>{};
    final options = _searchOptions.merge(searchOptions);
    final boostDocument = options.boostDocument;
    for (final MapEntry(key: shortId, value: id) in _documentIds.entries) {
      final score = boostDocument != null ? boostDocument(id, '', _storedFields[shortId]) : 1.0;
      results[shortId] = _RawResult(score, [], {});
    }
    return results;
  }

  /// No results is an empty map, whatever the operator; otherwise the operator is looked up in
  /// lower case and the results folded from the left.
  _RawResults _combineResults(List<_RawResults> results, [String? combineWith]) {
    if (results.isEmpty) return {};
    final operator = combineWith ?? or;
    final combinator = _combinators[operator.toLowerCase()];
    if (combinator == null) throw MiniSearchException('Invalid combination operator: $operator');
    return results.reduce(combinator);
  }

  _QuerySpec _termToQuerySpec(SearchOptions options, String term, int i, List<String> terms) {
    final boostTerm = options.boostTerm;
    return (
      term: term,
      // `options.fuzzy || false`
      fuzzy: _orElse(options.fuzzy, 0),
      prefix: (options.prefix ?? noTerm)(term, i, terms),
      termBoost: boostTerm != null ? boostTerm(term, i, terms) : 1,
    );
  }

  /// BM25+ for every posting of [derivedTerm] in the boosted fields, accumulated into [results].
  ///
  /// A posting of a document that is no longer present is skipped, lowers `matchingFields` for
  /// the postings after it, and is removed from the index (`removeTerm`, which may delete the term
  /// and merge radix nodes). MiniSearch removes it as it meets it, deleting the posting list's
  /// current entry; a Dart map cannot change while it is iterated, so here a posting list's
  /// removals follow its loop, in the same order. Nothing the loop reads is changed by them: the
  /// later postings are read for their own documents only, and `matchingFields` is lowered at the
  /// same point. The term itself can only lose its last field to them, so no later field of the
  /// loop over [fieldBoosts] sees a difference either.
  _RawResults _termResults(
    String sourceTerm,
    String derivedTerm,
    double termWeight,
    double termBoost,
    FieldsData? fieldTermData,
    Map<String, double> fieldBoosts,
    BoostDocument? boostDocumentFn,
    Bm25Params bm25params, [
    _RawResults? results,
  ]) {
    results ??= {};
    if (fieldTermData == null) return results;
    for (final MapEntry(key: field, value: fieldBoost) in fieldBoosts.entries) {
      final fieldId = _fieldIds[field];
      if (fieldId == null) continue; // A field the index does not have.
      final fieldTermFreqs = fieldTermData[fieldId];
      if (fieldTermFreqs == null) continue;
      var matchingFields = fieldTermFreqs.length;
      // A hole reads as undefined, which makes the score NaN.
      final avgFieldLength = _slot(_avgFieldLength, fieldId) ?? double.nan;
      List<int>? removed;
      for (final MapEntry(key: docId, value: termFreq) in fieldTermFreqs.entries) {
        final id = _documentIds[docId];
        if (id == null) {
          (removed ??= []).add(docId);
          matchingFields -= 1;
          continue;
        }
        final docBoost = boostDocumentFn != null ? boostDocumentFn(id, derivedTerm, _storedFields[docId]) : 1.0;
        if (docBoost == 0 || docBoost.isNaN) continue;
        final fieldLength = _slot(_fieldLength[docId]!, fieldId)?.toDouble() ?? double.nan;
        final rawScore = _calcBM25Score(
          termFreq.toDouble(),
          matchingFields.toDouble(),
          _documentCount.toDouble(),
          fieldLength,
          avgFieldLength,
          bm25params,
        );
        final weightedScore = termWeight * termBoost * fieldBoost * docBoost * rawScore;
        final result = results[docId];
        if (result != null) {
          result.score += weightedScore;
          _assignUniqueTerm(result.terms, sourceTerm);
          final match = result.match[derivedTerm];
          if (match != null) {
            match.add(field);
          } else {
            result.match[derivedTerm] = [field];
          }
        } else {
          results[docId] = _RawResult(weightedScore, [sourceTerm], {
            derivedTerm: [field],
          });
        }
      }
      if (removed != null) {
        for (final docId in removed) {
          _removeTerm(fieldId, docId, derivedTerm);
        }
      }
    }
    return results;
  }

  // -----------------------------------------------------------------------------------------------
  // The index.

  void _addTerm(int fieldId, int documentId, String term) {
    final indexData = _index.fetch(term, _createMap);
    final fieldIndex = indexData[fieldId];
    if (fieldIndex == null) {
      indexData[fieldId] = {documentId: 1};
    } else {
      final docs = fieldIndex[documentId];
      fieldIndex[documentId] = (docs ?? 0) + 1;
    }
  }

  void _removeTerm(int fieldId, int documentId, String term) {
    if (!_index.has(term)) {
      _warnDocumentChanged(documentId, fieldId, term);
      return;
    }
    final indexData = _index.fetch(term, _createMap);
    final fieldIndex = indexData[fieldId];
    final frequency = fieldIndex?[documentId];
    if (fieldIndex == null || frequency == null) {
      _warnDocumentChanged(documentId, fieldId, term);
    } else if (frequency <= 1) {
      if (fieldIndex.length <= 1) {
        indexData.remove(fieldId);
      } else {
        fieldIndex.remove(documentId);
      }
    } else {
      fieldIndex[documentId] = frequency - 1;
    }
    if (_index.get(term)!.isEmpty) _index.delete(term);
  }

  void _warnDocumentChanged(int shortDocumentId, int fieldId, String term) {
    for (final MapEntry(key: fieldName, value: id) in _fieldIds.entries) {
      if (id == fieldId) {
        _logger(
          'warn',
          'MiniSearch: document with ID ${_documentIds[shortDocumentId] ?? 'undefined'} has changed before removal: '
              'term "$term" was not present in field "$fieldName". Removing a document after it has changed can '
              'corrupt the index!',
          'version_conflict',
        );
        return;
      }
    }
  }

  /// `document[idField]`, which must be present and not null.
  Object _documentId(D document) => switch (_extractField(document, _idField)) {
    TextValue(:final text) => text,
    NumberValue(:final number) => number,
    _ => throw MiniSearchException('MiniSearch: document does not have ID field "$_idField"'),
  };

  int _addDocumentId(Object documentId) {
    final shortDocumentId = _nextId;
    _idToShortId[documentId] = shortDocumentId;
    _documentIds[shortDocumentId] = documentId;
    _documentCount += 1;
    _nextId += 1;
    return shortDocumentId;
  }

  void _addFields(List<String> fields) {
    for (var i = 0; i < fields.length; i++) {
      _fieldIds[fields[i]] = i;
    }
  }

  void _addFieldLength(int documentId, int fieldId, int count, int length) {
    final fieldLengths = _fieldLength.putIfAbsent(documentId, () => []);
    _setSlot(fieldLengths, fieldId, length);
    final averageFieldLength = _orElse(_slot(_avgFieldLength, fieldId), 0);
    final totalFieldLength = (averageFieldLength * count) + length;
    _setSlot(_avgFieldLength, fieldId, totalFieldLength / (count + 1));
  }

  void _removeFieldLength(int documentId, int fieldId, int count, int length) {
    if (count == 1) {
      _setSlot(_avgFieldLength, fieldId, 0);
      return;
    }
    // A hole reads as undefined, which makes the average NaN.
    final totalFieldLength = ((_slot(_avgFieldLength, fieldId) ?? double.nan) * count) - length;
    _setSlot(_avgFieldLength, fieldId, totalFieldLength / (count - 1));
  }

  void _saveStoredFields(int documentId, D doc) {
    if (_storeFields.isEmpty) return;
    final documentFields = _storedFields.putIfAbsent(documentId, () => {});
    for (final fieldName in _storeFields) {
      // An undefined value is not stored; null is.
      final fieldValue = _extractField(doc, fieldName);
      if (fieldValue != null) documentFields[fieldName] = fieldValue;
    }
  }
}

/// autoSuggest's accumulator for one phrase (`{ score, terms, count }`).
final class _SuggestionGroup {
  _SuggestionGroup(this.score, this.terms);

  double score;
  final List<String> terms;
  int count = 1;
}

/// The default argument of [MiniSearch.removeAll], standing for "no argument".
final class _NoArgument extends Iterable<Never> {
  const _NoArgument();

  @override
  Iterator<Never> get iterator => const <Never>[].iterator;
}

/// `combinators`, by operator name.
const _combinators = <String, _RawResults Function(_RawResults, _RawResults)>{or: _or, and: _and, andNot: _andNot};

_RawResults _or(_RawResults a, _RawResults b) {
  for (final MapEntry(key: docId, value: incoming) in b.entries) {
    final existing = a[docId];
    if (existing == null) {
      a[docId] = incoming;
    } else {
      existing.score = existing.score + incoming.score;
      _assignMatch(existing.match, incoming.match);
      _assignUniqueTerms(existing.terms, incoming.terms);
    }
  }
  return a;
}

/// A new map, in b's order.
_RawResults _and(_RawResults a, _RawResults b) {
  final combined = <int, _RawResult>{};
  for (final MapEntry(key: docId, value: incoming) in b.entries) {
    final existing = a[docId];
    if (existing == null) continue;
    _assignUniqueTerms(existing.terms, incoming.terms);
    combined[docId] = _RawResult(
      existing.score + incoming.score,
      existing.terms,
      _assignMatch(existing.match, incoming.match),
    );
  }
  return combined;
}

_RawResults _andNot(_RawResults a, _RawResults b) {
  for (final docId in b.keys) {
    a.remove(docId);
  }
  return a;
}

/// MiniSearch's `SPACE_OR_PUNCTUATION`, a constant of its module as here: a run of newlines, Unicode
/// spaces or punctuation. Dart compiles it to a JavaScript RegExp with the `u` flag, and `split` on
/// a RegExp without captures to the host's `String.prototype.split`.
final _spaceOrPunctuation = RegExp(r'[\n\r\p{Z}\p{P}]+', unicode: true);

/// The default `tokenize`: separators at the start or end leave empty tokens, and the empty text is
/// one empty token.
List<String> _tokenize(String text) => text.split(_spaceOrPunctuation);

/// The default `processTerm`. Dart's `toLowerCase` is the host's on the web: full Unicode mappings
/// and Final_Sigma.
String _processTerm(String term) => term.toLowerCase();

/// The default `stringifyField`, `fieldValue.toString()`: on the web a Dart number prints as a
/// JavaScript number does (a safe integer as its plain decimal digits).
String _stringifyField(FieldValue fieldValue) => switch (fieldValue) {
  TextValue(:final text) => text,
  NumberValue(:final number) => number.toString(),
  NullValue() => 'null',
};

FieldsData _createMap() => {};

/// `new Set(tokens).size`, the field length: the number of distinct raw tokens. Only the size is
/// read, so the set need not keep insertion order (a HashSet).
int _distinctCount(List<String> tokens) => HashSet<String>.of(tokens).length;

/// `calcBM25Score`, operation for operation, in doubles.
double _calcBM25Score(
  double termFreq,
  double matchingCount,
  double totalCount,
  double fieldLength,
  double avgFieldLength,
  Bm25Params bm25params,
) {
  final (:k, :b, :d) = bm25params;
  final invDocFreq = math.log(1 + (totalCount - matchingCount + 0.5) / (matchingCount + 0.5));
  return invDocFreq * (d + termFreq * (k + 1) / (termFreq + k * (1 - b + b * fieldLength / avgFieldLength)));
}

/// `array.sort(byScore)`, `(a, b) => b.score - a.score`, as JavaScript sorts: stably, so equal
/// scores (-0 and 0 included) keep their order. Dart's `List.sort` is not stable, so the items are
/// ordered by score and then by their position. Each score is read once, not per comparison.
List<E> _sortedByScore<E>(List<E> items, double Function(E item) score) {
  final scores = [for (final item in items) score(item)];
  final positions = List<int>.generate(items.length, (position) => position)
    ..sort((a, b) {
      final difference = scores[b] - scores[a];
      if (difference < 0) return -1;
      if (difference > 0) return 1;
      return a - b;
    });
  return [for (final position in positions) items[position]];
}

void _assignUniqueTerm(List<String> target, String term) {
  // Avoid adding duplicate terms.
  if (!target.contains(term)) target.add(term);
}

void _assignUniqueTerms(List<String> target, List<String> source) {
  for (final term in source) {
    _assignUniqueTerm(target, term);
  }
}

/// `Object.assign(target, source)` on two `match` objects: source's keys in `Object.keys` order;
/// an existing key keeps its place and takes the new field list, a new key is added last.
MatchInfo _assignMatch(MatchInfo target, MatchInfo source) {
  for (final key in objectKeys(source)) {
    target[key] = source[key]!;
  }
  return target;
}

/// `Object.keys` of an object whose keys were set in [object]'s order: the keys that are array
/// indices (the canonical decimal form of an integer from 0 to 2^32 - 2) first, in ascending
/// numeric order, then the others in insertion order.
List<String> objectKeys(Map<String, Object?> object) {
  final names = object.keys.toList();
  if (!names.any(_isArrayIndex)) return names;
  final indices = names.where(_isArrayIndex).toList()
    ..sort((a, b) => a.length != b.length ? a.length - b.length : a.compareTo(b));
  return [...indices, ...names.where((name) => !_isArrayIndex(name))];
}

bool _isArrayIndex(String key) {
  if (key.isEmpty || key.length > 10) return false;
  for (var i = 0; i < key.length; i++) {
    final unit = key.codeUnitAt(i);
    if (unit < 0x30 || unit > 0x39) return false;
  }
  if (key.length > 1 && key.codeUnitAt(0) == 0x30) return false;
  return key.length < 10 || key.compareTo('4294967294') <= 0;
}

/// `value || fallback` for a number: null (undefined), 0 and NaN are falsy.
double _orElse(double? value, double fallback) => value == null || value == 0 || value.isNaN ? fallback : value;

/// `array[index]`, null for a hole or past the end.
T? _slot<T extends Object>(List<T?> slots, int index) => index < slots.length ? slots[index] : null;

/// `array[index] = value`, growing the array with holes.
void _setSlot<T extends Object>(List<T?> slots, int index, T value) {
  while (slots.length <= index) {
    slots.add(null);
  }
  slots[index] = value;
}

/// A document id as a field value (`result.id` read by a filter).
FieldValue idValue(Object id) => switch (id) {
  final String text => TextValue(text),
  final num number => NumberValue(number.toDouble()),
  _ => throw ArgumentError.value(id, 'id', 'ids are strings or numbers'),
};

/// A stored value as JSON: a string, a number or null.
Object? jsonValue(FieldValue value) => switch (value) {
  TextValue(:final text) => text,
  NumberValue(:final number) => number,
  NullValue() => null,
};
