// The boundary (README "Port convention"), the Scala port of libraries/minisearch/reference.mjs.
// The corpus input is the JS object itself, seen through @js.native facade traits (no copy). Each
// operation's arguments are read into the port's Scala types as the script runs, the way the
// adapter maps JSON onto the MiniSearch API: options and queries into SearchOptions and Query, the
// declarative `filter`, `boostDocument` and `boostTerm` into the callbacks they describe, ids into
// DocId. Documents stay host objects that MiniSearch reads through its `extractField`
// (`document[name]`, the value read into a Value). The result is built as plain JS objects and
// arrays (js.Dictionary, js.Array), keys in the adapter's order. `run` is a native ES module export.

package minisearch

import scala.collection.mutable.ArrayBuffer
import scala.scalajs.js
import scala.scalajs.js.JSConverters.*
import scala.scalajs.js.annotation.{JSBracketAccess, JSExportTopLevel, JSName}

@js.native
trait Input extends js.Object:
  val options: IndexOptionsJson = js.native
  val operations: js.Array[OperationJson] = js.native

@js.native
trait IndexOptionsJson extends js.Object:
  val fields: js.UndefOr[js.Array[String]] = js.native
  val storeFields: js.UndefOr[js.Array[String]] = js.native
  val idField: js.UndefOr[String] = js.native
  val searchOptions: js.UndefOr[SearchOptionsJson] = js.native
  val autoSuggestOptions: js.UndefOr[SearchOptionsJson] = js.native
  /** `true`, `false` or an AutoVacuumJson. */
  val autoVacuum: js.UndefOr[js.Any] = js.native

@js.native
trait AutoVacuumJson extends js.Object:
  val batchSize: js.UndefOr[Double] = js.native
  val minDirtCount: js.UndefOr[Double] = js.native
  val minDirtFactor: js.UndefOr[Double] = js.native

@js.native
trait SearchOptionsJson extends js.Object:
  val fields: js.UndefOr[js.Array[String]] = js.native
  val boost: js.UndefOr[js.Dictionary[Double]] = js.native
  val prefix: js.UndefOr[Boolean] = js.native
  /** A boolean or a number. */
  val fuzzy: js.UndefOr[js.Any] = js.native
  val maxFuzzy: js.UndefOr[Double] = js.native
  val combineWith: js.UndefOr[String] = js.native
  val weights: js.UndefOr[WeightsJson] = js.native
  val bm25: js.UndefOr[BM25Json] = js.native
  val filter: js.UndefOr[FilterJson] = js.native
  /** `[[id, factor], ...]`. */
  val boostDocument: js.UndefOr[js.Array[js.Tuple2[js.Any, Double]]] = js.native
  /** `[[term, factor], ...]`. */
  val boostTerm: js.UndefOr[js.Array[js.Tuple2[String, Double]]] = js.native

/** A query tree node: search options and subqueries, or `{ wildcard: true }`. */
@js.native
trait QueryNodeJson extends SearchOptionsJson:
  val wildcard: js.UndefOr[Boolean] = js.native
  val queries: js.Array[js.Any] = js.native

@js.native
trait WeightsJson extends js.Object:
  val fuzzy: js.UndefOr[Double] = js.native
  val prefix: js.UndefOr[Double] = js.native

@js.native
trait BM25Json extends js.Object:
  val k: js.UndefOr[Double] = js.native
  val b: js.UndefOr[Double] = js.native
  val d: js.UndefOr[Double] = js.native

/** `{ field, in: [values] }`: `(result) => values.includes(result[field])`. */
@js.native
trait FilterJson extends js.Object:
  val field: String = js.native
  @JSName("in")
  val values: js.Array[js.Any] = js.native

@js.native
trait VacuumOptionsJson extends js.Object:
  val batchSize: js.UndefOr[Double] = js.native

@js.native
trait OperationJson extends js.Object:
  val op: String = js.native
  val document: DocumentJson = js.native
  /** An array, or null for `removeAll(null)`. */
  val documents: js.UndefOr[js.Array[DocumentJson]] = js.native
  val id: js.Any = js.native
  val ids: js.Array[js.Any] = js.native
  /** A string or a QueryNodeJson. */
  val query: js.Any = js.native
  /** SearchOptionsJson for search and autoSuggest, VacuumOptionsJson for vacuum. */
  val options: js.UndefOr[js.Object] = js.native

/** A corpus document, a JSON object. */
@js.native
trait DocumentJson extends js.Object:
  /** `document[name]`: MiniSearch's default `extractField`. */
  @JSBracketAccess
  def apply(name: String): js.UndefOr[js.Any] = js.native

object Main:
  @JSExportTopLevel("run")
  def run(input: Input): js.Dictionary[js.Any] =
    // The adapter's logger: the messages MiniSearch logs during the current operation.
    val warnings = ArrayBuffer.empty[String]
    val index = MiniSearch(indexOptions(input.options, (_, message, _) => warnings += message))
    val results = js.Array[js.Any]()
    for operation <- input.operations do
      val record = js.Dictionary[js.Any]("op" -> operation.op)
      warnings.clear()
      // MiniSearch's own errors are recorded and the script goes on with the index as MiniSearch
      // left it; anything else is a failure of the port, and propagates.
      val error =
        try
          execute(index, operation, record)
          None
        catch case error: MiniSearchError => Some(error.getMessage)
      if warnings.nonEmpty then record("warnings") = warnings.toJSArray
      for message <- error do record("error") = message
      results.push(record)
    js.Dictionary("results" -> results)

  private def execute(index: MiniSearch[DocumentJson], operation: OperationJson, record: js.Dictionary[js.Any]): Unit =
    operation.op match
      case "add"    => index.add(operation.document)
      case "addAll" => index.addAll(operation.documents.get)
      case "remove" => index.remove(operation.document)
      case "removeAll" =>
        operation.documents.toOption match
          case None            => index.removeAll()
          case Some(documents) => index.removeAll(if documents == null then null else documents.toSeq)
      case "discard"    => index.discard(docId(operation.id))
      case "discardAll" => index.discardAll(operation.ids.toSeq.map(docId))
      case "replace"    => index.replace(operation.document)
      // MiniSearch's promise settles after the synchronous run (CONTRACT.md, "Vacuuming").
      case "vacuum" =>
        val batchSize = operation.options.toOption.flatMap(_.asInstanceOf[VacuumOptionsJson].batchSize.toOption)
        index.vacuum(VacuumOptions(batchSize))
      case "search" =>
        record("hits") = index.search(query(operation.query), callOptions(operation)).map(hit).toJSArray
      case "autoSuggest" =>
        record("suggestions") = index.autoSuggest(operation.query.asInstanceOf[String], callOptions(operation)).map(suggestion).toJSArray
      case "has"             => record("value") = index.has(docId(operation.id))
      case "getStoredFields" => record("value") = index.getStoredFields(docId(operation.id)).fold[js.Any](null)(stored)
      case "stats" =>
        record("documentCount") = index.documentCount
        record("termCount") = index.termCount
        record("dirtCount") = index.dirtCount
        record("dirtFactor") = index.dirtFactor
        record("isVacuuming") = index.isVacuuming
      case "toJSON" => record("value") = serialized(index.toJSON)
      case other    => throw IllegalArgumentException(s"unknown operation $other")

  // --------------------------------------------------------------------------------------------------
  // Reading.

  private def indexOptions(json: IndexOptionsJson, logger: (String, String, String) => Unit): Options[DocumentJson] =
    Options(
      fields = json.fields.toOption.fold(throw MiniSearchError("MiniSearch: option \"fields\" must be provided"))(_.toSeq),
      extractField = (document, name) => document(name).toOption.map(value),
      idField = json.idField.getOrElse("id"),
      storeFields = json.storeFields.fold(Nil)(_.toSeq),
      searchOptions = json.searchOptions.fold(SearchOptions())(searchOptions),
      autoSuggestOptions = json.autoSuggestOptions.fold(SearchOptions())(searchOptions),
      autoVacuum = json.autoVacuum.fold(true)(autoVacuum),
      logger = logger,
    )

  private def autoVacuum(json: js.Any): Boolean | AutoVacuumOptions = (json: Any) match
    case flag: Boolean => flag
    case _ =>
      val options = json.asInstanceOf[AutoVacuumJson]
      AutoVacuumOptions(options.batchSize.toOption, options.minDirtCount.toOption, options.minDirtFactor.toOption)

  private def callOptions(operation: OperationJson): SearchOptions =
    operation.options.fold(SearchOptions())(options => searchOptions(options.asInstanceOf[SearchOptionsJson]))

  private def searchOptions(json: SearchOptionsJson): SearchOptions = SearchOptions(
    fields = json.fields.toOption.map(_.toSeq),
    boost = json.boost.toOption.map(_.toMap),
    prefix = json.prefix.toOption,
    fuzzy = json.fuzzy.toOption.map(fuzzy => (fuzzy: Any) match
      case flag: Boolean  => flag
      case number: Double => number
      case _              => throw IllegalArgumentException("fuzzy is a boolean or a number")),
    maxFuzzy = json.maxFuzzy.toOption,
    combineWith = json.combineWith.toOption,
    weights = json.weights.toOption.map(weights => Weights(weights.fuzzy.toOption, weights.prefix.toOption)),
    // A missing parameter is undefined in MiniSearch's arithmetic: NaN.
    bm25 = json.bm25.toOption.map(bm25 => BM25Params(bm25.k.getOrElse(Double.NaN), bm25.b.getOrElse(Double.NaN), bm25.d.getOrElse(Double.NaN))),
    filter = json.filter.toOption.map { filter =>
      val field = filter.field
      val values = filter.values.toSeq.map(value)
      (result: SearchResult) => result(field).exists(values.contains)
    },
    boostDocument = json.boostDocument.toOption.map { pairs =>
      val factors = pairs.toSeq.map(pair => (docId(pair._1), pair._2))
      (id: DocId, _: String, _: Option[StoredFields]) => factors.collectFirst { case (key, factor) if key == id => factor }.getOrElse(1.0)
    },
    boostTerm = json.boostTerm.toOption.map { pairs =>
      val factors = pairs.toSeq.map(pair => (pair._1, pair._2))
      (term: String, _: Int, _: collection.Seq[String]) => factors.collectFirst { case (key, factor) if key == term => factor }.getOrElse(1.0)
    },
  )

  /** A string, `{ wildcard: true }` (MiniSearch.wildcard, at the top or as a subquery), or a tree. */
  private def query(json: js.Any): Query = (json: Any) match
    case text: String => Query.Terms(text)
    case _ =>
      val node = json.asInstanceOf[QueryNodeJson]
      if node.wildcard.contains(true) then MiniSearch.wildcard
      else Query.Combination(searchOptions(node), node.queries.toSeq.map(query))

  /** A document field's value: a string, a number or null. */
  private def value(json: js.Any): Value = (json: Any) match
    case null           => Value.Null
    case text: String   => Value.Text(text)
    case number: Double => Value.Number(number)
    case _              => throw IllegalArgumentException(s"document values are strings, numbers or null, not ${js.typeOf(json)}")

  private def docId(json: js.Any): DocId = (json: Any) match
    case number: Int  => number
    case text: String => text
    case _            => throw IllegalArgumentException(s"ids are strings or 32-bit integers, not ${js.typeOf(json)}")

  // --------------------------------------------------------------------------------------------------
  // Writing.

  private def scalar(value: Value): js.Any = value match
    case Value.Text(text)     => text
    case Value.Number(number) => number
    case Value.Null           => null

  private def id(id: DocId): js.Any = id match
    case number: Int  => number
    case text: String => text

  private def strings(values: collection.Seq[String]): js.Array[String] = values.toJSArray

  /** The adapter's copy of a result: its keys in order, `match` as `[term, fields]` pairs in
    * `Object.keys` order (which `terms` is), then the stored fields, as `Object.assign` added them (a
    * stored `id` replaces the id in place).
    */
  private def hit(result: SearchResult): js.Dictionary[js.Any] =
    val out = js.Dictionary[js.Any](
      "id" -> id(result.id),
      "score" -> result.score,
      "terms" -> strings(result.terms),
      "queryTerms" -> strings(result.queryTerms),
      "match" -> result.terms.map(term => js.Array[js.Any](term, strings(result.matchInfo(term)))).toJSArray,
    )
    for storedFields <- result.storedFields; (name, value) <- storedFields do out(name) = scalar(value)
    out

  private def suggestion(suggestion: Suggestion): js.Dictionary[js.Any] =
    js.Dictionary("suggestion" -> suggestion.suggestion, "terms" -> strings(suggestion.terms), "score" -> suggestion.score)

  private def stored(storedFields: StoredFields): js.Dictionary[js.Any] =
    val out = js.Dictionary.empty[js.Any]
    for (name, value) <- storedFields do out(name) = scalar(value)
    out

  /** A number after `JSON.parse(JSON.stringify(...))`: non-finite numbers become null, -0 becomes 0. */
  private def json(number: Double): js.Any = if !number.isFinite then null else if number == 0 then 0.0 else number

  private def jsonScalar(value: Value): js.Any = value match
    case Value.Number(number) => json(number)
    case other                => scalar(other)

  /** `JSON.parse(JSON.stringify(index))`. Integer keys are set in the index's order; a JS object
    * lists them in ascending order, as the reference's do.
    */
  private def serialized(index: AsPlainObject): js.Dictionary[js.Any] =
    val documentIds = js.Dictionary.empty[js.Any]
    for (shortId, docId) <- index.documentIds do documentIds(shortId.toString) = id(docId)
    val fieldIds = js.Dictionary.empty[js.Any]
    for (name, fieldId) <- index.fieldIds do fieldIds(name) = fieldId
    val fieldLength = js.Dictionary.empty[js.Any]
    for (shortId, lengths) <- index.fieldLength do
      fieldLength(shortId.toString) = lengths.map(_.fold[js.Any](null)(length => length)).toJSArray
    val storedFields = js.Dictionary.empty[js.Any]
    for (shortId, fields) <- index.storedFields do
      val out = js.Dictionary.empty[js.Any]
      for (name, value) <- fields do out(name) = jsonScalar(value)
      storedFields(shortId.toString) = out
    val terms = index.index.map { (term, fieldsData) =>
      val data = js.Dictionary.empty[js.Any]
      for (fieldId, postings) <- fieldsData do
        val frequencies = js.Dictionary.empty[js.Any]
        for (shortId, frequency) <- postings do frequencies(shortId.toString) = frequency
        data(fieldId.toString) = frequencies
      js.Array[js.Any](term, data)
    }
    js.Dictionary[js.Any](
      "documentCount" -> index.documentCount,
      "nextId" -> index.nextId,
      "documentIds" -> documentIds,
      "fieldIds" -> fieldIds,
      "fieldLength" -> fieldLength,
      "averageFieldLength" -> index.averageFieldLength.map(_.fold[js.Any](null)(json)).toJSArray,
      "storedFields" -> storedFields,
      "dirtCount" -> index.dirtCount,
      "index" -> terms.toJSArray,
      "serializationVersion" -> index.serializationVersion,
    )
