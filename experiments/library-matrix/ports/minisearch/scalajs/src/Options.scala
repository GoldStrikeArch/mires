// MiniSearch's options (minisearch 7.2.0) and their merging.
//
// MiniSearch merges option objects with object spread (`{ ...a, ...b }`): a key that `b` has
// replaces `a`'s whole value, so the object-valued options (`boost`, `weights`, `bm25`) are
// replaced, never merged key by key. Every search option here is an Option, None meaning "not
// given", and `a ++ b` is that spread.

package minisearch

/** A document id. MiniSearch keys its id map by the id itself (a JS Map, so 1 and "1" differ); the
  * contract's ids are strings and 32-bit integers.
  */
type DocId = Int | String

/** A JSON scalar, as MiniSearch reads it from a document field. */
enum Value:
  case Text(text: String)
  case Number(number: Double)
  case Null

/** A document's stored fields (`{ name: value }`), names in `storeFields` order. */
type StoredFields = collection.Map[String, Value]

/** `prefix` given as a function: `(term, i, terms) => boolean`. */
type TermPredicate = (String, Int, collection.Seq[String]) => Boolean

final case class Weights(fuzzy: Option[Double] = None, prefix: Option[Double] = None)

final case class BM25Params(k: Double, b: Double, d: Double)

/** Search options: the defaults of every search (constructor), of one call, or of a query node. */
final case class SearchOptions(
    fields: Option[collection.Seq[String]] = None,
    boost: Option[collection.Map[String, Double]] = None,
    prefix: Option[Boolean | TermPredicate] = None,
    fuzzy: Option[Boolean | Double] = None,
    maxFuzzy: Option[Double] = None,
    combineWith: Option[String] = None,
    weights: Option[Weights] = None,
    bm25: Option[BM25Params] = None,
    filter: Option[SearchResult => Boolean] = None,
    boostDocument: Option[(DocId, String, Option[StoredFields]) => Double] = None,
    boostTerm: Option[(String, Int, collection.Seq[String]) => Double] = None,
):
  /** `{ ...this, ...overrides }`. */
  def ++(overrides: SearchOptions): SearchOptions = SearchOptions(
    fields = overrides.fields.orElse(fields),
    boost = overrides.boost.orElse(boost),
    prefix = overrides.prefix.orElse(prefix),
    fuzzy = overrides.fuzzy.orElse(fuzzy),
    maxFuzzy = overrides.maxFuzzy.orElse(maxFuzzy),
    combineWith = overrides.combineWith.orElse(combineWith),
    weights = overrides.weights.orElse(weights),
    bm25 = overrides.bm25.orElse(bm25),
    filter = overrides.filter.orElse(filter),
    boostDocument = overrides.boostDocument.orElse(boostDocument),
    boostTerm = overrides.boostTerm.orElse(boostTerm),
  )

object SearchOptions:
  val DefaultMaxFuzzy = 6.0
  val DefaultBM25: BM25Params = BM25Params(k = 1.2, b = 0.7, d = 0.5)

  /** `defaultSearchOptions`. */
  val defaults: SearchOptions = SearchOptions(
    combineWith = Some(MiniSearch.OR),
    prefix = Some(false),
    fuzzy = Some(false),
    maxFuzzy = Some(DefaultMaxFuzzy),
    boost = Some(Map.empty),
    weights = Some(Weights(fuzzy = Some(Weights.DefaultFuzzy), prefix = Some(Weights.DefaultPrefix))),
    bm25 = Some(DefaultBM25),
  )

  /** `defaultAutoSuggestOptions`: AND, and prefix matching on the last term only. */
  val autoSuggestDefaults: SearchOptions = SearchOptions(
    combineWith = Some(MiniSearch.AND),
    prefix = Some((_, i, terms) => i == terms.length - 1),
  )

/** `defaultSearchOptions.weights`, which every `weights` option is spread over. */
object Weights:
  val DefaultFuzzy = 0.45
  val DefaultPrefix = 0.375

/** `batchSize` of a vacuum. A missing or zero value falls back to 1000 where MiniSearch reads it
  * (`options.batchSize || 1000`); `batchWait` only paces the batches after the first, which run
  * after a synchronous call has returned.
  */
final case class VacuumOptions(batchSize: Option[Double] = None)

/** When an automatic vacuum runs; missing or zero values fall back to 20 and 0.1 (`||`). */
final case class VacuumConditions(minDirtCount: Option[Double] = None, minDirtFactor: Option[Double] = None)

/** `autoVacuum` as an object: its batch size and conditions. */
final case class AutoVacuumOptions(
    batchSize: Option[Double] = None,
    minDirtCount: Option[Double] = None,
    minDirtFactor: Option[Double] = None,
)

object AutoVacuumOptions:
  /** `defaultAutoVacuumOptions`, what `autoVacuum: true` (the default) means. */
  val defaults: AutoVacuumOptions = AutoVacuumOptions(batchSize = Some(1000), minDirtCount = Some(20), minDirtFactor = Some(0.1))

/** The constructor's options, for documents of type `D`.
  *
  * MiniSearch reads a field with `extractField(document, name)`; its default is `document[name]`.
  * The document type is the caller's, so here the reader is a parameter: it gives the field's value,
  * or None when the document has none (undefined).
  */
final case class Options[D](
    fields: collection.Seq[String],
    extractField: (D, String) => Option[Value],
    idField: String = "id",
    storeFields: collection.Seq[String] = Nil,
    searchOptions: SearchOptions = SearchOptions(),
    autoSuggestOptions: SearchOptions = SearchOptions(),
    autoVacuum: Boolean | AutoVacuumOptions = true,
    logger: (String, String, String) => Unit = (_, message, _) => System.err.println(message),
)
