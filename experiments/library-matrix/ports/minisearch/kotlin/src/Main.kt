// Entry of the Kotlin/JS port: the operation-script runner of libraries/minisearch/reference.mjs.
//
// The boundary: the corpus input is a JS object typed by external interfaces (read in place, no
// copy). Documents stay host objects that MiniSearch reads field by field (`document[name]`, each
// value read into a `Value`); options, queries and ids are read into Kotlin types, and the
// declarative filter, boostDocument and boostTerm become Kotlin lambdas, as in the adapter. The
// result is built as plain JS objects (`kotlin.js.json`) and arrays in the adapter's key order.
// Strings are JS strings throughout. `run` is a native ES module export through @JsExport.

import kotlin.js.Json
import kotlin.js.json
import minisearch.AsPlainObject
import minisearch.AutoVacuum
import minisearch.AutoVacuumOptions
import minisearch.BM25Params
import minisearch.BoostDocument
import minisearch.BoostTerm
import minisearch.Document
import minisearch.Fuzzy
import minisearch.Logger
import minisearch.MiniSearch
import minisearch.MiniSearchException
import minisearch.Options
import minisearch.Prefix
import minisearch.Query
import minisearch.SearchOptions
import minisearch.SearchResult
import minisearch.Suggestion
import minisearch.VacuumOptions
import minisearch.Value
import minisearch.Weights

external interface Input {
    val options: IndexOptionsJson
    val operations: Array<OperationJson>
}

external interface IndexOptionsJson {
    val fields: Array<String>
    val storeFields: Array<String>?
    val idField: String?
    val searchOptions: SearchOptionsJson?
    val autoSuggestOptions: SearchOptionsJson?
    /** `true`, `false` or an [AutoVacuumJson]. */
    val autoVacuum: Any?
}

external interface AutoVacuumJson {
    val batchSize: Double?
    val minDirtCount: Double?
    val minDirtFactor: Double?
}

external interface SearchOptionsJson {
    val fields: Array<String>?
    /** `{ fieldName: number }`. */
    val boost: Any?
    val prefix: Boolean?
    /** A boolean or a number. */
    val fuzzy: Any?
    val maxFuzzy: Double?
    val combineWith: String?
    val weights: WeightsJson?
    val bm25: BM25Json?
    val filter: FilterJson?
    /** `[[id, factor], ...]`. */
    val boostDocument: Array<Array<Any?>>?
    /** `[[term, factor], ...]`. */
    val boostTerm: Array<Array<Any?>>?
}

external interface WeightsJson {
    val fuzzy: Double?
    val prefix: Double?
}

external interface BM25Json {
    val k: Double
    val b: Double
    val d: Double
}

external interface FilterJson {
    val field: String
    val `in`: Array<Any?>
}

/** `{ queries: [...], ...SearchOptions }`, or `{ wildcard: true }`. */
external interface QueryJson : SearchOptionsJson {
    val wildcard: Boolean?
    /** Strings and [QueryJson] objects. */
    val queries: Array<Any>
}

/** The `options` of a search or an autoSuggest (search options), or of a vacuum (`batchSize`). */
external interface OperationOptionsJson : SearchOptionsJson {
    val batchSize: Double?
}

external interface OperationJson {
    val op: String
    val document: DocumentJson
    val documents: Array<DocumentJson>?
    val id: Any?
    val ids: Array<Any?>
    /** A string, or a [QueryJson] for `search`. */
    val query: Any
    val options: OperationOptionsJson?
}

/** A document: a JSON object, read by field name. */
external interface DocumentJson

private operator fun DocumentJson.get(name: String): Any? = asDynamic()[name]

/** The host's `Object.keys`, for the keys of a `boost` object. */
@JsName("Object")
private external object HostObject {
    fun keys(value: Any): Array<String>
}

@OptIn(ExperimentalJsExport::class)
@JsExport
fun run(input: Input): Json {
    // What MiniSearch logged during the current operation.
    val warnings = ArrayList<String>()
    val index = MiniSearch(indexOptions(input.options) { _, message, _ -> warnings.add(message) })
    val results = ArrayList<Json>()
    for (operation in input.operations) {
        val record = json("op" to operation.op)
        warnings.clear()
        try {
            perform(index, operation, record)
        } catch (error: MiniSearchException) {
            // The script goes on with the index as MiniSearch left it.
            if (warnings.isNotEmpty()) record["warnings"] = warnings.toTypedArray()
            record["error"] = error.message
            results.add(record)
            continue
        }
        if (warnings.isNotEmpty()) record["warnings"] = warnings.toTypedArray()
        results.add(record)
    }
    return json("results" to results.toTypedArray())
}

private fun perform(index: MiniSearch, operation: OperationJson, record: Json) {
    when (operation.op) {
        "add" -> index.add(document(operation.document))
        "addAll" -> index.addAll(operation.documents!!.map(::document))
        "remove" -> index.remove(document(operation.document))
        // No argument, or an explicit (null) one.
        "removeAll" -> if (jsTypeOf(operation.documents) == "undefined") index.removeAll() else index.removeAll(operation.documents?.map(::document))
        "discard" -> index.discard(id(operation.id))
        "discardAll" -> index.discardAll(operation.ids.map(::id))
        "replace" -> index.replace(document(operation.document))
        "vacuum" -> index.vacuum(VacuumOptions(batchSize = operation.options?.batchSize))
        "search" -> record["hits"] = index.search(query(operation.query), callOptions(operation.options)).map(::hit).toTypedArray()
        "autoSuggest" -> record["suggestions"] = index.autoSuggest(operation.query.unsafeCast<String>(), callOptions(operation.options))
            .map(::suggestion).toTypedArray()
        "has" -> record["value"] = index.has(id(operation.id))
        "getStoredFields" -> record["value"] = index.getStoredFields(id(operation.id))?.let(::plainObject)
        "stats" -> {
            record["documentCount"] = index.documentCount
            record["termCount"] = index.termCount
            record["dirtCount"] = index.dirtCount
            record["dirtFactor"] = index.dirtFactor
            record["isVacuuming"] = index.isVacuuming
        }
        "toJSON" -> record["value"] = serialized(index.toJSON())
        // The adapter throws a TypeError, which its catch records like MiniSearch's errors.
        else -> throw MiniSearchException("unknown operation ${operation.op}")
    }
}

// ---------------------------------------------------------------------------------------------
// Input.

/** A JSON value MiniSearch reads: a string, a number or null; Kotlin null when absent (`undefined`). */
private fun value(json: Any?): Value? = when (jsTypeOf(json)) {
    "undefined" -> null
    "string" -> Value.Text(json.unsafeCast<String>())
    "number" -> Value.Number(json.unsafeCast<Double>())
    else -> if (json == null) Value.Null else throw IllegalArgumentException("outside the parity domain: a ${jsTypeOf(json)} value")
}

private fun id(json: Any?): Value = value(json) ?: throw IllegalArgumentException("outside the parity domain: a missing id")

/** MiniSearch's default `extractField`: `document[fieldName]`. */
private fun document(json: DocumentJson) = Document { fieldName -> value(json[fieldName]) }

private fun indexOptions(json: IndexOptionsJson, logger: Logger) = Options(
    fields = json.fields.toList(),
    idField = json.idField,
    storeFields = json.storeFields?.toList(),
    searchOptions = json.searchOptions?.let(::searchOptions),
    autoSuggestOptions = json.autoSuggestOptions?.let(::searchOptions),
    autoVacuum = json.autoVacuum?.let(::autoVacuum),
    logger = logger,
)

private fun autoVacuum(json: Any): AutoVacuum {
    if (jsTypeOf(json) == "boolean") return AutoVacuum.Flag(json.unsafeCast<Boolean>())
    val options = json.unsafeCast<AutoVacuumJson>()
    return AutoVacuumOptions(batchSize = options.batchSize, minDirtCount = options.minDirtCount, minDirtFactor = options.minDirtFactor)
}

/** A search's or an autoSuggest's own options; none is MiniSearch's default `{}`. */
private fun callOptions(json: SearchOptionsJson?) = json?.let(::searchOptions) ?: SearchOptions()

/** The adapter's `callbacks`: the options as given, the declarative callbacks made functions. */
private fun searchOptions(json: SearchOptionsJson) = SearchOptions(
    fields = json.fields?.toList(),
    filter = json.filter?.let(::filter),
    boost = json.boost?.let(::boost),
    weights = json.weights?.let { Weights(fuzzy = it.fuzzy, prefix = it.prefix) },
    boostDocument = json.boostDocument?.let(::boostDocument),
    boostTerm = json.boostTerm?.let(::boostTerm),
    prefix = json.prefix?.let { Prefix.Flag(it) },
    fuzzy = json.fuzzy?.let(::fuzzy),
    maxFuzzy = json.maxFuzzy,
    combineWith = json.combineWith,
    bm25 = json.bm25?.let { BM25Params(k = it.k, b = it.b, d = it.d) },
)

private fun fuzzy(json: Any): Fuzzy =
    if (jsTypeOf(json) == "boolean") Fuzzy.Flag(json.unsafeCast<Boolean>()) else Fuzzy.Amount(json.unsafeCast<Double>())

private fun boost(json: Any): Map<String, Double> {
    val boost = LinkedHashMap<String, Double>()
    for (field in HostObject.keys(json)) boost[field] = json.asDynamic()[field].unsafeCast<Double>()
    return boost
}

/** `{ field, in: [values] }` is `(result) => values.includes(result[field])`. */
private fun filter(json: FilterJson): (SearchResult) -> Boolean {
    val field = json.field
    val values = json.`in`.map(::value)
    return { result -> result[field] in values }
}

/** `[[id, factor], ...]` is `(id) => the factor of the first pair whose id === id, else 1`. */
private fun boostDocument(json: Array<Array<Any?>>): BoostDocument {
    val pairs = json.map { (key, factor) -> Pair(value(key), factor.unsafeCast<Double>()) }
    return { id, _, _ -> pairs.firstOrNull { (key, _) -> key == id }?.second ?: 1.0 }
}

/** `[[term, factor], ...]` is `(term) => the factor of the first pair whose term === term, else 1`. */
private fun boostTerm(json: Array<Array<Any?>>): BoostTerm {
    val pairs = json.map { (key, factor) -> Pair(key, factor.unsafeCast<Double>()) }
    return { term, _, _ -> pairs.firstOrNull { (key, _) -> key == term }?.second ?: 1.0 }
}

/** A string, `{ wildcard: true }` (`MiniSearch.wildcard`), or a combination of queries with its options. */
private fun query(json: Any): Query {
    if (jsTypeOf(json) == "string") return Query.Text(json.unsafeCast<String>())
    val node = json.unsafeCast<QueryJson>()
    if (node.wildcard == true) return Query.Wildcard
    return Query.Combination(queries = node.queries.map(::query), options = searchOptions(node))
}

// ---------------------------------------------------------------------------------------------
// Output.

private fun Value.toJs(): Any? = when (this) {
    is Value.Text -> text
    is Value.Number -> number
    Value.Null -> null
}

/** A search result as the adapter copies it: `match` as `[term, fields]` pairs, then the stored fields. */
private fun hit(result: SearchResult): Json {
    val out = json()
    out["id"] = result.id.toJs()
    out["score"] = result.score
    out["terms"] = result.terms.toTypedArray()
    out["queryTerms"] = result.queryTerms.toTypedArray()
    // `terms` is `Object.keys(match)`, the order the adapter lists `match` in.
    out["match"] = result.terms.map { term -> arrayOf(term, result.match.getValue(term).toTypedArray()) }.toTypedArray()
    // `Object.assign(result, storedFields)`: a stored `id` replaces the id in place.
    result.storedFields?.forEach { (name, value) -> out[name] = value.toJs() }
    return out
}

private fun suggestion(result: Suggestion) = json(
    "suggestion" to result.suggestion,
    "terms" to result.terms.toTypedArray(),
    "score" to result.score,
)

private fun plainObject(fields: Map<String, Value>): Json {
    val out = json()
    for ((name, value) in fields) out[name] = value.toJs()
    return out
}

/**
 * An object keyed by integers, as `Object.fromEntries` builds one: the keys are array indices, so
 * the object lists them in ascending order whatever order they are set in.
 */
private fun <V> integerKeyed(map: Map<Int, V>, transform: (V) -> Any?): Json {
    val out = json()
    for ((key, value) in map) out[key.toString()] = transform(value)
    return out
}

/** A number as `JSON.stringify` writes it: NaN and the infinities become `null`, as do holes. */
private fun jsonNumber(number: Double?): Double? = number?.takeIf { it.isFinite() }

/** `JSON.parse(JSON.stringify(index))`. */
private fun serialized(index: AsPlainObject): Json {
    val fieldIds = json()
    for ((name, id) in index.fieldIds) fieldIds[name] = id
    val out = json()
    out["documentCount"] = index.documentCount
    out["nextId"] = index.nextId
    out["documentIds"] = integerKeyed(index.documentIds) { it.toJs() }
    out["fieldIds"] = fieldIds
    out["fieldLength"] = integerKeyed(index.fieldLength) { lengths -> lengths.toTypedArray() }
    out["averageFieldLength"] = index.averageFieldLength.map(::jsonNumber).toTypedArray()
    out["storedFields"] = integerKeyed(index.storedFields, ::plainObject)
    out["dirtCount"] = index.dirtCount
    out["index"] = index.index.map { (term, data) -> arrayOf(term, integerKeyed(data) { freqs -> integerKeyed(freqs) { it } }) }
        .toTypedArray()
    out["serializationVersion"] = index.serializationVersion
    return out
}
