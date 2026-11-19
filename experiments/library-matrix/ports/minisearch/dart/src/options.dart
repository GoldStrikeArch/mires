// MiniSearch's options (minisearch 7.2.0, dist/es/index.js), their defaults and their merging, and
// the values MiniSearch reads from documents.
//
// MiniSearch merges option objects with object spread (`{ ...a, ...b }`): a key that `b` has
// replaces `a`'s whole value, so the object-valued options (`boost`, `weights`, `bm25`) are
// replaced, never merged key by key. Every [SearchOptions] field is nullable, null meaning "not
// given" (option values are never null in the contract's domain), and `a.merge(b)` is that spread.

import 'mini_search.dart';

/// A value MiniSearch reads from a document field (`extractField`): a string, a number or `null`.
/// A field the document does not have (`undefined` in JavaScript) is a Dart `null` instead of a
/// [FieldValue], so `FieldValue?` keeps the two apart, as `saveStoredFields` (`!== undefined`) and
/// the `filter` of the contract (`in: [null]`) need.
sealed class FieldValue {
  const FieldValue();
}

final class TextValue extends FieldValue {
  const TextValue(this.text);

  final String text;

  @override
  bool operator ==(Object other) => other is TextValue && other.text == text;

  @override
  int get hashCode => text.hashCode;
}

final class NumberValue extends FieldValue {
  const NumberValue(this.number);

  final double number;

  /// SameValueZero, as `Array.prototype.includes` and `Map` keys compare numbers: `-0` equals `0`
  /// (Dart's `==` on doubles already says so) and NaN equals NaN.
  @override
  bool operator ==(Object other) =>
      other is NumberValue && (other.number == number || (other.number.isNaN && number.isNaN));

  @override
  int get hashCode => number.isNaN ? 0 : number.hashCode;
}

final class NullValue extends FieldValue {
  const NullValue();

  @override
  bool operator ==(Object other) => other is NullValue;

  @override
  int get hashCode => 0;
}

/// A document's stored fields (`{ name: value }`), names in `storeFields` order.
typedef StoredFields = Map<String, FieldValue>;

/// `prefix` (and autoSuggest's default) as a function of the query term, its position and all the
/// query's terms. MiniSearch also takes a boolean: `true` and `false` are the constant predicates
/// [everyTerm] and [noTerm].
typedef TermPredicate = bool Function(String term, int index, List<String> terms);

/// `boostDocument`: a factor for a document, given its id, the matched term and its stored fields.
typedef BoostDocument = double Function(Object id, String term, StoredFields? storedFields);

/// `boostTerm`: a factor for a query term.
typedef BoostTerm = double Function(String term, int index, List<String> terms);

/// `weights`: the weight of fuzzy and of prefix matches; a missing one comes from
/// [defaultWeights] (`{ ...defaultSearchOptions.weights, ...weights }`).
typedef Weights = ({double? fuzzy, double? prefix});

/// `bm25`: the BM25+ parameters. Within the contract's domain all three are given.
typedef Bm25Params = ({double k, double b, double d});

bool everyTerm(String _, int _, List<String> _) => true;

bool noTerm(String _, int _, List<String> _) => false;

/// Search options: the defaults of every search (constructor), of one call, or of a query node.
final class SearchOptions {
  const SearchOptions({
    this.fields,
    this.boost,
    this.prefix,
    this.fuzzy,
    this.maxFuzzy,
    this.combineWith,
    this.weights,
    this.bm25,
    this.filter,
    this.boostDocument,
    this.boostTerm,
  });

  final List<String>? fields;
  final Map<String, double>? boost;
  final TermPredicate? prefix;

  /// `fuzzy`: MiniSearch takes `false`, `true` or a number (a fraction of the term length below 1,
  /// an edit distance from 1). Here it is the number: `false` is 0 and `true` is 0.2, which is
  /// what MiniSearch turns them into (`options.fuzzy || false`, then `fuzzy === true ? 0.2 :
  /// fuzzy`), so every search computes the same edit distance.
  final double? fuzzy;
  final int? maxFuzzy;
  final String? combineWith;
  final Weights? weights;
  final Bm25Params? bm25;
  final bool Function(SearchResult result)? filter;
  final BoostDocument? boostDocument;
  final BoostTerm? boostTerm;

  /// `{ ...this, ...overrides }`.
  SearchOptions merge(SearchOptions overrides) => SearchOptions(
    fields: overrides.fields ?? fields,
    boost: overrides.boost ?? boost,
    prefix: overrides.prefix ?? prefix,
    fuzzy: overrides.fuzzy ?? fuzzy,
    maxFuzzy: overrides.maxFuzzy ?? maxFuzzy,
    combineWith: overrides.combineWith ?? combineWith,
    weights: overrides.weights ?? weights,
    bm25: overrides.bm25 ?? bm25,
    filter: overrides.filter ?? filter,
    boostDocument: overrides.boostDocument ?? boostDocument,
    boostTerm: overrides.boostTerm ?? boostTerm,
  );
}

/// The operators of `combineWith` (MiniSearch's `OR`, `AND`, `AND_NOT`).
const or = 'or';
const and = 'and';
const andNot = 'and_not';

const ({double fuzzy, double prefix}) defaultWeights = (fuzzy: 0.45, prefix: 0.375);

const Bm25Params defaultBm25Params = (k: 1.2, b: 0.7, d: 0.5);

/// `defaultSearchOptions`.
const defaultSearchOptions = SearchOptions(
  combineWith: or,
  prefix: noTerm,
  fuzzy: 0,
  maxFuzzy: 6,
  boost: {},
  weights: defaultWeights,
  bm25: defaultBm25Params,
);

/// `defaultAutoSuggestOptions`: AND, and prefix matching on the last query term only.
const defaultAutoSuggestOptions = SearchOptions(combineWith: and, prefix: lastTerm);

bool lastTerm(String _, int index, List<String> terms) => index == terms.length - 1;

/// The options of one vacuum: `batchSize` (a missing or zero value falls back to 1000 where
/// MiniSearch reads it, `options.batchSize || 1000`). `batchWait` only paces the batches after the
/// first, which run after a synchronous call has returned (CONTRACT.md, "Vacuuming"), so it is
/// left out.
typedef VacuumOptions = ({double? batchSize});

/// When an automatic vacuum runs; missing or zero values fall back to 20 and 0.1 (`||`).
typedef VacuumConditions = ({double? minDirtCount, double? minDirtFactor});

/// `autoVacuum` given as an object: its batch size and conditions, used as given.
typedef AutoVacuumOptions = ({double? batchSize, double? minDirtCount, double? minDirtFactor});

const defaultBatchSize = 1000.0;
const defaultMinDirtCount = 20.0;
const defaultMinDirtFactor = 0.1;

/// `defaultAutoVacuumOptions`, what `autoVacuum: true` (the default) means.
const AutoVacuumOptions defaultAutoVacuumOptions = (
  batchSize: defaultBatchSize,
  minDirtCount: defaultMinDirtCount,
  minDirtFactor: defaultMinDirtFactor,
);

/// The logger MiniSearch calls with a level, a message and a code (`version_conflict`).
typedef Logger = void Function(String level, String message, String? code);

void _printMessage(String _, String message, String? _) => print(message);

/// The constructor's options, for documents of type [D].
///
/// MiniSearch reads a field with `extractField(document, name)`, by default `document[name]`.
/// Documents are the caller's type, so here the reader is a parameter: it returns the field's
/// value, or null when the document has no such field (`undefined`).
final class Options<D> {
  const Options({
    required this.fields,
    required this.extractField,
    this.idField = 'id',
    this.storeFields = const [],
    this.searchOptions = const SearchOptions(),
    this.autoSuggestOptions = const SearchOptions(),
    this.autoVacuum = defaultAutoVacuumOptions,
    this.logger = _printMessage,
  });

  final List<String> fields;
  final FieldValue? Function(D document, String fieldName) extractField;
  final String idField;
  final List<String> storeFields;
  final SearchOptions searchOptions;
  final SearchOptions autoSuggestOptions;

  /// `autoVacuum`: `true` (the default) is [defaultAutoVacuumOptions], and null is `false`
  /// (no automatic vacuuming).
  final AutoVacuumOptions? autoVacuum;
  final Logger logger;
}
