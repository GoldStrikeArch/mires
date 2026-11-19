// The MiniSearch class of minisearch 7.2.0 (dist/es/index.js), method by method, for the subset in
// libraries/minisearch/CONTRACT.md: documents, the inverted index, search with BM25+, prefix and
// fuzzy matching and query trees, autoSuggest, discarding with its vacuuming, toJSON.
//
// The data structures are MiniSearch's: every JS Map is a mutable.LinkedHashMap (insertion order,
// which MiniSearch's results and iteration orders depend on), except the id-to-short-id map, which
// is only ever looked up; every array is an ArrayBuffer, holes being None.

package minisearch

import scala.collection.mutable
import scala.collection.mutable.ArrayBuffer

/** An error MiniSearch throws, with its message. */
final class MiniSearchError(message: String) extends RuntimeException(message)

/** A document's position in insertion order (MiniSearch's short document id). */
type ShortId = Int

/** A field's position in `fields`. */
type FieldId = Int

/** A posting list (`fieldIndex`): short id to term frequency. */
type Postings = mutable.LinkedHashMap[ShortId, Int]

/** A term's index data (`fieldsData`): field id to posting list. */
type FieldsData = mutable.LinkedHashMap[FieldId, Postings]

/** A result's `match`: document term to the names of the fields it matched in. MiniSearch keeps it
  * in a plain object, whose keys `Object.keys` lists array indices first (see `MiniSearch.ownKeys`).
  */
type MatchInfo = mutable.LinkedHashMap[String, ArrayBuffer[String]]

/** A search result: MiniSearch's result object, on which `Object.assign` has put the stored fields. */
final case class SearchResult(
    id: DocId,
    score: Double,
    /** The document terms that matched: `Object.keys(match)`. */
    terms: collection.Seq[String],
    /** The query terms that matched. */
    queryTerms: collection.Seq[String],
    matchInfo: MatchInfo,
    storedFields: Option[StoredFields],
):
  /** `result[name]` for a scalar property: a stored field, else the result's own `id` or `score`. */
  def apply(name: String): Option[Value] =
    storedFields.flatMap(_.get(name)).orElse(name match
      case "id"    => Some(MiniSearch.idValue(id))
      case "score" => Some(Value.Number(score))
      case _       => None)

final case class Suggestion(suggestion: String, terms: collection.Seq[String], score: Double)

/** A query: a string, `MiniSearch.wildcard`, or a combination of queries with its own options. */
enum Query:
  case Terms(text: String)
  case Wildcard
  case Combination(options: SearchOptions, queries: collection.Seq[Query])

/** `JSON.parse(JSON.stringify(index))`, as the index's own collections. */
final case class AsPlainObject(
    documentCount: Int,
    nextId: Int,
    documentIds: collection.Map[ShortId, DocId],
    fieldIds: collection.Map[String, FieldId],
    fieldLength: collection.Map[ShortId, collection.Seq[Option[Int]]],
    averageFieldLength: collection.Seq[Option[Double]],
    storedFields: collection.Map[ShortId, StoredFields],
    dirtCount: Int,
    /** Every term in iteration order, with its field ids to posting lists. */
    index: collection.Seq[(String, FieldsData)],
    serializationVersion: Int = 2,
)

/** One document's accumulated result in a result map (`{ score, terms, match }`). */
private final class RawResult(var score: Double, val terms: ArrayBuffer[String], val matchInfo: MatchInfo)

/** A result map, keyed by short id: its insertion order is the order of the results. */
private type RawResults = mutable.LinkedHashMap[ShortId, RawResult]

/** One query term with its options (`termToQuerySpec`). */
private final case class QuerySpec(term: String, fuzzy: Boolean | Double, prefix: Boolean, termBoost: Double)

final class MiniSearch[D](options: Options[D]):
  import MiniSearch.*

  private val extractField = options.extractField
  private val idField = options.idField
  private val fields = options.fields
  private val storeFields = options.storeFields
  private val logger = options.logger
  /** The library defaults under the constructor's `searchOptions`. */
  private val searchOptions = SearchOptions.defaults ++ options.searchOptions
  /** The library defaults under the constructor's `autoSuggestOptions`. */
  private val autoSuggestOptions = SearchOptions.autoSuggestDefaults ++ options.autoSuggestOptions
  /** `autoVacuum`: None for `false`; `true` is the default options. `discardAll` turns it off while it runs. */
  private var autoVacuum: Option[AutoVacuumOptions] = options.autoVacuum match
    case true                     => Some(AutoVacuumOptions.defaults)
    case false                    => None
    case custom: AutoVacuumOptions => Some(custom)

  private var index = SearchableMap[FieldsData]()
  private var _documentCount = 0
  private var documentIds = mutable.LinkedHashMap.empty[ShortId, DocId]
  private var idToShortId = mutable.HashMap.empty[DocId, ShortId]
  private val fieldIds = mutable.LinkedHashMap.empty[String, FieldId]
  /** Per document, the length of each field it has, holes (None) for the others. */
  private var fieldLength = mutable.LinkedHashMap.empty[ShortId, ArrayBuffer[Option[Int]]]
  /** The average length of each field, holes (None) where none was ever set. */
  private var avgFieldLength = ArrayBuffer.empty[Option[Double]]
  private var nextId = 0
  private var storedFields = mutable.LinkedHashMap.empty[ShortId, mutable.LinkedHashMap[String, Value]]
  private var _dirtCount = 0
  /** `_currentVacuum != null`. MiniSearch clears it in a microtask after a vacuum's last batch, which a
    * synchronous run never reaches: once a vacuum has started it stays set (CONTRACT.md, "Vacuuming").
    */
  private var currentVacuum = false

  addFields(fields)

  // ------------------------------------------------------------------------------------------------
  // Documents.

  def add(document: D): Unit =
    val id = documentId(document)
    if idToShortId.contains(id) then throw MiniSearchError(s"MiniSearch: duplicate ID $id")
    val shortDocumentId = addDocumentId(id)
    saveStoredFields(shortDocumentId, document)
    for field <- fields; fieldValue <- extractField(document, field) if fieldValue != Value.Null do
      val tokens = tokenize(stringifyField(fieldValue))
      val fieldId = fieldIds(field)
      val uniqueTerms = tokens.distinct.size
      addFieldLength(shortDocumentId, fieldId, _documentCount - 1, uniqueTerms)
      for term <- tokens do
        val processedTerm = processTerm(term)
        if processedTerm.nonEmpty then addTerm(fieldId, shortDocumentId, processedTerm)

  def addAll(documents: Iterable[D]): Unit = documents.foreach(add)

  /** Removes a document, which must be given as it was added (nothing checks it). */
  def remove(document: D): Unit =
    val id = documentId(document)
    val shortId = idToShortId.getOrElse(id, throw MiniSearchError(s"MiniSearch: cannot remove document with ID $id: it is not in the index"))
    for field <- fields; fieldValue <- extractField(document, field) if fieldValue != Value.Null do
      val tokens = tokenize(stringifyField(fieldValue))
      val fieldId = fieldIds(field)
      val uniqueTerms = tokens.distinct.size
      removeFieldLength(shortId, fieldId, _documentCount, uniqueTerms)
      for term <- tokens do
        val processedTerm = processTerm(term)
        if processedTerm.nonEmpty then removeTerm(fieldId, shortId, processedTerm)
    storedFields -= shortId
    documentIds -= shortId
    idToShortId -= id
    fieldLength -= shortId
    _documentCount -= 1

  /** `removeAll(documents)`; an explicit null argument is MiniSearch's error, not `removeAll()`. */
  def removeAll(documents: Iterable[D] | Null): Unit =
    if documents == null then throw MiniSearchError("Expected documents to be present. Omit the argument to remove all documents.")
    documents.foreach(remove)

  /** `removeAll()`: a new, empty index. `dirtCount` and the vacuum state are kept. */
  def removeAll(): Unit =
    index = SearchableMap()
    _documentCount = 0
    documentIds = mutable.LinkedHashMap.empty
    idToShortId = mutable.HashMap.empty
    fieldLength = mutable.LinkedHashMap.empty
    avgFieldLength = ArrayBuffer.empty
    storedFields = mutable.LinkedHashMap.empty
    nextId = 0

  /** Discards a document: it leaves the id maps and the averages now, its postings stay in the index
    * until a search meets them or a vacuum runs.
    */
  def discard(id: DocId): Unit =
    val shortId = idToShortId.getOrElse(id, throw MiniSearchError(s"MiniSearch: cannot discard document with ID $id: it is not in the index"))
    idToShortId -= id
    documentIds -= shortId
    storedFields -= shortId
    // `forEach` over the array skips its holes: the fields the document did not have.
    for lengths <- fieldLength.get(shortId); (slot, fieldId) <- lengths.zipWithIndex; length <- slot do
      removeFieldLength(shortId, fieldId, _documentCount, length)
    fieldLength -= shortId
    _documentCount -= 1
    _dirtCount += 1
    maybeAutoVacuum()

  /** Discards every id with automatic vacuuming off, then considers one automatic vacuum. A failing
    * discard ends it there, before that vacuum.
    */
  def discardAll(ids: Iterable[DocId]): Unit =
    val autoVacuum = this.autoVacuum
    try
      this.autoVacuum = None
      ids.foreach(discard)
    finally this.autoVacuum = autoVacuum
    maybeAutoVacuum()

  /** `discard(document[idField])` (which may vacuum), then `add(document)`. */
  def replace(updatedDocument: D): Unit =
    val id = extractField(updatedDocument, idField) match
      case Some(Value.Text(text))     => text
      case Some(Value.Number(number)) => numericId(number)
      case other => throw MiniSearchError(s"MiniSearch: cannot discard document with ID ${other.fold("undefined")(_ => "null")}: it is not in the index")
    discard(id)
    add(updatedDocument)

  def has(id: DocId): Boolean = idToShortId.contains(id)

  def getStoredFields(id: DocId): Option[StoredFields] = idToShortId.get(id).flatMap(storedFields.get)

  // ------------------------------------------------------------------------------------------------
  // Vacuuming.

  /** A vacuum without conditions, unless one is already current. */
  def vacuum(options: VacuumOptions = VacuumOptions()): Unit = conditionalVacuum(options, None)

  private def maybeAutoVacuum(): Unit =
    for AutoVacuumOptions(batchSize, minDirtCount, minDirtFactor) <- autoVacuum do
      conditionalVacuum(VacuumOptions(batchSize), Some(VacuumConditions(minDirtCount, minDirtFactor)))

  private def conditionalVacuum(options: VacuumOptions, conditions: Option[VacuumConditions]): Unit =
    // A request made while a vacuum is current is chained after it, and runs once it settles: after a
    // synchronous run, so never here.
    if !currentVacuum && vacuumConditionsMet(conditions) then
      // `this._currentVacuum = this.performVacuuming(options)`: the promise is stored once the
      // vacuum's synchronous part has run.
      performVacuuming(options)
      currentVacuum = true

  /** The synchronous part of `performVacuuming`: every term in iteration order, its postings of
    * documents no longer present removed, until the first batch boundary, where MiniSearch awaits a
    * timer and the rest of the vacuum happens after the run. Only a walk that ends without reaching a
    * boundary resets `dirtCount`.
    */
  private def performVacuuming(options: VacuumOptions): Unit =
    val initialDirtCount = _dirtCount
    val batchSize = orElse(options.batchSize, DefaultBatchSize)
    var i = 1
    val terms = index.iterator
    while terms.hasNext do
      val (term, fieldsData) = terms.next()
      // Over snapshots of the maps: MiniSearch deletes the entry it is visiting from the live map,
      // which a JS Map iterator survives; nothing else is deleted during the loops.
      for (fieldId, fieldIndex) <- fieldsData.toArray; shortId <- fieldIndex.keys.toArray do
        if !documentIds.contains(shortId) then
          if fieldIndex.size <= 1 then fieldsData -= fieldId
          else fieldIndex -= shortId
      if index.get(term).exists(_.isEmpty) then index.delete(term)
      if i % batchSize == 0 then return
      i += 1
    _dirtCount -= initialDirtCount

  private def vacuumConditionsMet(conditions: Option[VacuumConditions]): Boolean = conditions match
    case None => true
    case Some(VacuumConditions(minDirtCount, minDirtFactor)) =>
      _dirtCount >= orElse(minDirtCount, DefaultMinDirtCount) && dirtFactor >= orElse(minDirtFactor, DefaultMinDirtFactor)

  // ------------------------------------------------------------------------------------------------
  // State.

  def isVacuuming: Boolean = currentVacuum

  /** The number of documents discarded since the most recent completed vacuum. */
  def dirtCount: Int = _dirtCount

  def dirtFactor: Double = _dirtCount.toDouble / (1 + _documentCount + _dirtCount)

  def documentCount: Int = _documentCount

  def termCount: Int = index.size

  def toJSON: AsPlainObject = AsPlainObject(
    documentCount = _documentCount,
    nextId = nextId,
    documentIds = documentIds,
    fieldIds = fieldIds,
    fieldLength = fieldLength,
    averageFieldLength = avgFieldLength,
    storedFields = storedFields,
    dirtCount = _dirtCount,
    index = index.iterator.to(ArrayBuffer),
  )

  // ------------------------------------------------------------------------------------------------
  // Search.

  def search(query: Query, searchOptions: SearchOptions = SearchOptions()): collection.Seq[SearchResult] =
    val searchOptionsWithDefaults = this.searchOptions ++ searchOptions
    val rawResults = executeQuery(query, searchOptions)
    val results = ArrayBuffer.empty[SearchResult]
    rawResults.foreachEntry { (docId, raw) =>
      // The quality is the number of distinct query terms that matched (not the document terms,
      // which prefix and fuzzy matching make differ).
      val quality = if raw.terms.isEmpty then 1 else raw.terms.length
      val result = SearchResult(
        id = documentIds(docId),
        score = raw.score * quality,
        terms = ownKeys(raw.matchInfo.keys),
        queryTerms = raw.terms,
        matchInfo = raw.matchInfo,
        storedFields = storedFields.get(docId),
      )
      if searchOptionsWithDefaults.filter.forall(filter => filter(result)) then results += result
    }
    // A wildcard query without boostDocument is not sorted: every score is 1.
    if query == Query.Wildcard && searchOptionsWithDefaults.boostDocument.isEmpty then results
    else results.sortWith(byScore(_.score))

  def autoSuggest(queryString: String, options: SearchOptions = SearchOptions()): collection.Seq[Suggestion] =
    val opts = autoSuggestOptions ++ options
    final class Group(var score: Double, val terms: collection.Seq[String], var count: Int)
    val suggestions = mutable.LinkedHashMap.empty[String, Group]
    for SearchResult(_, score, terms, _, _, _) <- search(Query.Terms(queryString), opts) do
      val phrase = terms.mkString(" ")
      suggestions.get(phrase) match
        case Some(suggestion) =>
          suggestion.score += score
          suggestion.count += 1
        case None => suggestions(phrase) = Group(score, terms, 1)
    val results = ArrayBuffer.empty[Suggestion]
    suggestions.foreachEntry((suggestion, group) => results += Suggestion(suggestion, group.terms, group.score / group.count))
    results.sortWith(byScore(_.score))

  private def executeQuery(query: Query, searchOptions: SearchOptions): RawResults = query match
    case Query.Wildcard => executeWildcardQuery(searchOptions)
    case Query.Combination(nodeOptions, queries) =>
      // `{ ...searchOptions, ...query }`: the constructor's search options are not merged in, so a
      // node with no combineWith in it or above it combines with OR.
      val options = searchOptions ++ nodeOptions
      val results = queries.map(subquery => executeQuery(subquery, options))
      combineResults(results, options.combineWith)
    case Query.Terms(text) =>
      val options = this.searchOptions ++ searchOptions
      val terms = tokenize(text).map(processTerm).filter(_.nonEmpty)
      val queries = terms.indices.map(i => termToQuerySpec(options, terms(i), i, terms))
      val results = queries.map(query => executeQuerySpec(query, options))
      combineResults(results, options.combineWith)

  /** The exact term, then (reading the index as the exact term's cleanup left it) the prefix
    * matches, then the fuzzy matches that are not prefix matches.
    */
  private def executeQuerySpec(query: QuerySpec, searchOptions: SearchOptions): RawResults =
    val options = this.searchOptions ++ searchOptions
    // `fieldBoosts`, an object built field by field: a repeated field keeps its first position.
    val boosts = mutable.LinkedHashMap.empty[String, Double]
    for field <- options.fields.getOrElse(fields) do boosts(field) = orElse(options.boost.flatMap(_.get(field)), 1)
    val boostDocument = options.boostDocument
    // Both are always set: the index's search options hold the library defaults.
    val maxFuzzy = options.maxFuzzy.getOrElse(SearchOptions.DefaultMaxFuzzy)
    val bm25params = options.bm25.getOrElse(SearchOptions.DefaultBM25)
    // `{ ...defaultSearchOptions.weights, ...weights }`
    val weights = options.weights.getOrElse(Weights())
    val fuzzyWeight = weights.fuzzy.getOrElse(Weights.DefaultFuzzy)
    val prefixWeight = weights.prefix.getOrElse(Weights.DefaultPrefix)
    val data = index.get(query.term)
    val results = termResults(query.term, query.term, 1, query.termBoost, data, boosts, boostDocument, bm25params)
    val prefixMatches = if query.prefix then Some(index.atPrefix(query.term)) else None
    val fuzzyMatches = query.fuzzy match
      case false => None
      case setting =>
        val fuzzy = setting match
          case number: Double => number
          case _              => 0.2
        val maxDistance = if fuzzy < 1 then math.min(maxFuzzy, round(query.term.length * fuzzy)) else fuzzy
        if maxDistance != 0 then Some(index.fuzzyGet(query.term, maxDistance.toInt)) else None
    for matches <- prefixMatches; (term, termData) <- matches.iterator do
      val distance = term.length - query.term.length
      if distance != 0 then // Skip exact match.
        // A term that is also a prefix match is scored as a prefix match only.
        fuzzyMatches.foreach(_ -= term)
        // The weight approaches 0 as the distance grows, prefixWeight at the (hypothetical) distance 0.
        val weight = prefixWeight * term.length / (term.length + 0.3 * distance)
        termResults(query.term, term, weight, query.termBoost, Some(termData), boosts, boostDocument, bm25params, results)
    for matches <- fuzzyMatches; (term, (termData, distance)) <- matches do
      if distance != 0 then // Skip exact match.
        val weight = fuzzyWeight * term.length / (term.length + distance)
        termResults(query.term, term, weight, query.termBoost, Some(termData), boosts, boostDocument, bm25params, results)
    results

  private def executeWildcardQuery(searchOptions: SearchOptions): RawResults =
    val results: RawResults = mutable.LinkedHashMap.empty
    val options = this.searchOptions ++ searchOptions
    documentIds.foreachEntry { (shortId, id) =>
      val score = options.boostDocument.fold(1.0)(boostDocument => boostDocument(id, "", storedFields.get(shortId)))
      results(shortId) = RawResult(score, ArrayBuffer.empty, mutable.LinkedHashMap.empty)
    }
    results

  private def termToQuerySpec(options: SearchOptions, term: String, i: Int, terms: collection.Seq[String]): QuerySpec =
    // `options.fuzzy || false`
    val fuzzy: Boolean | Double = options.fuzzy match
      case Some(true)                                        => true
      case Some(number: Double) if number != 0 && !number.isNaN => number
      case _                                                 => false
    val prefix = options.prefix match
      case Some(predicate: TermPredicate) => predicate(term, i, terms)
      case Some(flag: Boolean)            => flag
      case None                           => false
    val termBoost = options.boostTerm.fold(1.0)(boostTerm => boostTerm(term, i, terms))
    QuerySpec(term, fuzzy, prefix, termBoost)

  /** BM25+ for every posting of the derived term in the boosted fields, accumulated into `results`.
    *
    * A posting of a document that is no longer present is skipped, lowers `matchingFields` for the
    * postings after it, and is removed from the index (`removeTerm`, which may delete the term and
    * merge radix nodes). MiniSearch removes it as it meets it; here the removals of a posting list
    * follow its loop, in the same order: nothing the loop reads is changed by them (the posting list
    * is only read for the documents still to come, which a removal does not touch).
    */
  private def termResults(
      sourceTerm: String,
      derivedTerm: String,
      termWeight: Double,
      termBoost: Double,
      fieldTermData: Option[FieldsData],
      fieldBoosts: collection.Map[String, Double],
      boostDocumentFn: Option[(DocId, String, Option[StoredFields]) => Double],
      bm25params: BM25Params,
      results: RawResults = mutable.LinkedHashMap.empty,
  ): RawResults =
    for fieldTermData <- fieldTermData do
      fieldBoosts.foreachEntry { (field, fieldBoost) =>
        for fieldId <- fieldIds.get(field); fieldTermFreqs <- fieldTermData.get(fieldId) do
          var matchingFields = fieldTermFreqs.size
          val avgFieldLength = this.avgFieldLength.lift(fieldId).flatten.getOrElse(Double.NaN)
          var removed = List.empty[ShortId]
          fieldTermFreqs.foreachEntry { (docId, termFreq) =>
            documentIds.get(docId) match
              case None =>
                removed = docId :: removed
                matchingFields -= 1
              case Some(id) =>
                val docBoost = boostDocumentFn.fold(1.0)(boostDocument => boostDocument(id, derivedTerm, storedFields.get(docId)))
                if docBoost != 0 && !docBoost.isNaN then
                  // A hole reads as undefined, which makes the score NaN.
                  val fieldLength = this.fieldLength(docId).lift(fieldId).flatten.fold(Double.NaN)(_.toDouble)
                  val rawScore = calcBM25Score(termFreq, matchingFields, _documentCount, fieldLength, avgFieldLength, bm25params)
                  val weightedScore = termWeight * termBoost * fieldBoost * docBoost * rawScore
                  results.get(docId) match
                    case Some(result) =>
                      result.score += weightedScore
                      assignUniqueTerm(result.terms, sourceTerm)
                      result.matchInfo.get(derivedTerm) match
                        case Some(matched) => matched += field
                        case None          => result.matchInfo(derivedTerm) = ArrayBuffer(field)
                    case None =>
                      results(docId) = RawResult(weightedScore, ArrayBuffer(sourceTerm), mutable.LinkedHashMap(derivedTerm -> ArrayBuffer(field)))
          }
          removed.reverse.foreach(removeTerm(fieldId, _, derivedTerm))
      }
    results

  // ------------------------------------------------------------------------------------------------
  // The index.

  private def addTerm(fieldId: FieldId, documentId: ShortId, term: String): Unit =
    val indexData = index.fetch(term, mutable.LinkedHashMap.empty)
    indexData.get(fieldId) match
      case None              => indexData(fieldId) = mutable.LinkedHashMap(documentId -> 1)
      case Some(fieldIndex) => fieldIndex(documentId) = fieldIndex.getOrElse(documentId, 0) + 1

  private def removeTerm(fieldId: FieldId, documentId: ShortId, term: String): Unit =
    if !index.has(term) then warnDocumentChanged(documentId, fieldId, term)
    else
      val indexData = index.fetch(term, mutable.LinkedHashMap.empty)
      indexData.get(fieldId).filter(_.contains(documentId)) match
        case None => warnDocumentChanged(documentId, fieldId, term)
        case Some(fieldIndex) =>
          val frequency = fieldIndex(documentId)
          if frequency <= 1 then
            if fieldIndex.size <= 1 then indexData -= fieldId
            else fieldIndex -= documentId
          else fieldIndex(documentId) = frequency - 1
      if index.get(term).exists(_.isEmpty) then index.delete(term)

  private def warnDocumentChanged(shortDocumentId: ShortId, fieldId: FieldId, term: String): Unit =
    for (fieldName, _) <- fieldIds.find((_, id) => id == fieldId) do
      val id = documentIds.get(shortDocumentId).fold("undefined")(_.toString)
      logger(
        "warn",
        s"MiniSearch: document with ID $id has changed before removal: term \"$term\" was not present in field \"$fieldName\". Removing a document after it has changed can corrupt the index!",
        "version_conflict",
      )

  private def documentId(document: D): DocId = extractField(document, idField) match
    case Some(Value.Text(text))     => text
    case Some(Value.Number(number)) => numericId(number)
    case _                          => throw MiniSearchError(s"MiniSearch: document does not have ID field \"$idField\"")

  private def addDocumentId(documentId: DocId): ShortId =
    val shortDocumentId = nextId
    idToShortId(documentId) = shortDocumentId
    documentIds(shortDocumentId) = documentId
    _documentCount += 1
    nextId += 1
    shortDocumentId

  private def addFields(fields: collection.Seq[String]): Unit =
    for (field, i) <- fields.zipWithIndex do fieldIds(field) = i

  private def addFieldLength(documentId: ShortId, fieldId: FieldId, count: Int, length: Int): Unit =
    val fieldLengths = fieldLength.getOrElseUpdate(documentId, ArrayBuffer.empty)
    setSlot(fieldLengths, fieldId, length)
    val averageFieldLength = orElse(avgFieldLength.lift(fieldId).flatten, 0)
    val totalFieldLength = (averageFieldLength * count) + length
    setSlot(avgFieldLength, fieldId, totalFieldLength / (count + 1))

  private def removeFieldLength(documentId: ShortId, fieldId: FieldId, count: Int, length: Int): Unit =
    if count == 1 then setSlot(avgFieldLength, fieldId, 0.0)
    else
      // A hole reads as undefined, which makes the average NaN.
      val totalFieldLength = (avgFieldLength.lift(fieldId).flatten.getOrElse(Double.NaN) * count) - length
      setSlot(avgFieldLength, fieldId, totalFieldLength / (count - 1))

  private def saveStoredFields(documentId: ShortId, doc: D): Unit =
    if storeFields.nonEmpty then
      val documentFields = storedFields.getOrElseUpdate(documentId, mutable.LinkedHashMap.empty)
      // An undefined value is not stored; null is.
      for fieldName <- storeFields; fieldValue <- extractField(doc, fieldName) do documentFields(fieldName) = fieldValue

object MiniSearch:
  /** `MiniSearch.wildcard`: matches every document. */
  val wildcard: Query = Query.Wildcard

  val OR = "or"
  val AND = "and"
  val AND_NOT = "and_not"

  // `defaultAutoVacuumOptions`, the fallbacks of every vacuum's `||`.
  private val DefaultBatchSize = 1000.0
  private val DefaultMinDirtCount = 20.0
  private val DefaultMinDirtFactor = 0.1

  private val combinators: Map[String, (RawResults, RawResults) => RawResults] = Map(
    OR -> { (a, b) =>
      b.foreachEntry { (docId, incoming) =>
        a.get(docId) match
          case None => a(docId) = incoming
          case Some(existing) =>
            existing.score = existing.score + incoming.score
            assignMatch(existing.matchInfo, incoming.matchInfo)
            assignUniqueTerms(existing.terms, incoming.terms)
      }
      a
    },
    // A new map, in b's order.
    AND -> { (a, b) =>
      val combined: RawResults = mutable.LinkedHashMap.empty
      b.foreachEntry { (docId, incoming) =>
        for existing <- a.get(docId) do
          assignUniqueTerms(existing.terms, incoming.terms)
          combined(docId) = RawResult(existing.score + incoming.score, existing.terms, assignMatch(existing.matchInfo, incoming.matchInfo))
      }
      combined
    },
    AND_NOT -> { (a, b) =>
      b.keys.foreach(a -= _)
      a
    },
  )

  /** No results is an empty map, whatever the operator; otherwise the operator is looked up in lower
    * case and the results folded from the left.
    */
  private def combineResults(results: collection.Seq[RawResults], combineWith: Option[String]): RawResults =
    if results.isEmpty then mutable.LinkedHashMap.empty
    else
      val operator = combineWith.getOrElse(OR)
      val combinator = combinators.getOrElse(operator.toLowerCase, throw MiniSearchError(s"Invalid combination operator: $operator"))
      results.reduceLeft(combinator)

  /** `calcBM25Score`, operation for operation, in doubles. */
  private def calcBM25Score(
      termFreq: Double,
      matchingCount: Double,
      totalCount: Double,
      fieldLength: Double,
      avgFieldLength: Double,
      bm25params: BM25Params,
  ): Double =
    val BM25Params(k, b, d) = bm25params
    val invDocFreq = math.log(1 + (totalCount - matchingCount + 0.5) / (matchingCount + 0.5))
    invDocFreq * (d + termFreq * (k + 1) / (termFreq + k * (1 - b + b * fieldLength / avgFieldLength)))

  /** The default `tokenize`, `stringifyField` and `processTerm`. */
  private def tokenize(text: String): collection.IndexedSeq[String] = Host.splitOnSpaceOrPunctuation(text)

  /** `fieldValue.toString()`: Scala.js formats a Double as JavaScript does (a safe integer as its
    * plain decimal digits).
    */
  private def stringifyField(fieldValue: Value): String = fieldValue match
    case Value.Text(text)     => text
    case Value.Number(number) => number.toString
    case Value.Null           => "null"

  private def processTerm(term: String): String = term.toLowerCase

  /** A numeric id: the contract's are 32-bit integers. */
  private def numericId(number: Double): DocId =
    if number.isValidInt then number.toInt else throw IllegalArgumentException(s"numeric ids are 32-bit integers, not $number")

  private[minisearch] def idValue(id: DocId): Value = id match
    case number: Int  => Value.Number(number)
    case text: String => Value.Text(text)

  /** `x || fallback` for a number: 0 and NaN are falsy. */
  private def orElse(value: Option[Double], fallback: Double): Double = value match
    case Some(number) if number != 0 && !number.isNaN => number
    case _                                             => fallback

  /** `Math.round` of a non-negative number: halves round up. */
  private def round(value: Double): Double =
    val floor = math.floor(value)
    if value - floor >= 0.5 then floor + 1 else floor

  /** `byScore`, `(a, b) => b.score - a.score`, as the "comes first" test of a stable sort: equal
    * scores, -0 and 0 included, keep their order (Ordering[Double] would put -0 after 0).
    */
  private def byScore[A](score: A => Double)(a: A, b: A): Boolean = score(a) > score(b)

  private def assignUniqueTerm(target: ArrayBuffer[String], term: String): Unit =
    if !target.contains(term) then target += term

  private def assignUniqueTerms(target: ArrayBuffer[String], source: collection.Seq[String]): Unit =
    source.foreach(assignUniqueTerm(target, _))

  /** `Object.assign(target, source)` on two `match` objects: source's keys in `Object.keys` order; an
    * existing key keeps its place and takes the new field list, a new key is added last.
    */
  private def assignMatch(target: MatchInfo, source: MatchInfo): MatchInfo =
    for key <- ownKeys(source.keys) do target(key) = source(key)
    target

  /** `Object.keys` of an object whose keys were set in this order: the keys that are array indices
    * (the canonical decimal form of an integer from 0 to 2^32 - 2) first, in ascending numeric order,
    * then the others in insertion order.
    */
  private[minisearch] def ownKeys(keys: Iterable[String]): collection.Seq[String] =
    val (indices, names) = keys.to(ArrayBuffer).partition(isArrayIndex)
    if indices.isEmpty then names
    else indices.sortWith((a, b) => a.length < b.length || (a.length == b.length && a < b)) ++ names

  private def isArrayIndex(key: String): Boolean =
    key.nonEmpty && key.length <= 10 && key.forall(unit => unit >= '0' && unit <= '9') &&
      (key == "0" || key.charAt(0) != '0') && (key.length < 10 || key <= "4294967294")

  /** `array[index] = value`, growing the array with holes. */
  private def setSlot[A](slots: ArrayBuffer[Option[A]], index: Int, value: A): Unit =
    while slots.length <= index do slots += None
    slots(index) = Some(value)
