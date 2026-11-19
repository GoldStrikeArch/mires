// MiniSearch's option, query and result types (minisearch 7.2.0, `MiniSearch.d.ts`) as Kotlin
// classes.
//
// MiniSearch merges option objects with spread (`{ ...a, ...b }`): a key present in `b` replaces
// `a`'s value whole, so object options (`boost`, `weights`, `bm25`) are replaced, never merged key
// by key. Every option here is nullable, null meaning "not given", and `SearchOptions.plus` is
// that spread. Union-typed options (`prefix`, `fuzzy`, `autoVacuum`) are sealed interfaces.

package minisearch

/** A JSON scalar MiniSearch reads from a document field, stores, and uses as a document id. */
sealed interface Value {
    /** A string. `toString` is the string, as a template literal writes it. */
    data class Text(val text: String) : Value {
        override fun toString() = text
    }

    /**
     * A number. Numbers are equal by SameValueZero, as a JavaScript `Map` compares its keys and
     * `includes` its elements (`-0` is `0`, NaN is NaN); a data class would compare with
     * `Double.equals`, which tells `-0` from `0`. Kotlin/JS's `Double.toString` is JavaScript's
     * `Number.prototype.toString`.
     */
    class Number(val number: Double) : Value {
        override fun equals(other: Any?) = other is Number && (number == other.number || (number.isNaN() && other.number.isNaN()))
        override fun hashCode() = (number + 0.0).hashCode() // -0 + 0 is 0
        override fun toString() = number.toString()
    }

    data object Null : Value {
        override fun toString() = "null"
    }
}

/** JavaScript's `value == null`: true for `null` and for an absent (`undefined`) field. */
internal fun Value?.isNullish() = this == null || this == Value.Null

/** A document as MiniSearch's default `extractField` reads it: `document[fieldName]`, null when absent. */
fun interface Document {
    operator fun get(fieldName: String): Value?
}

/** `prefix`: a boolean, or a function of the query term, its position and all the query terms. */
sealed interface Prefix {
    data class Flag(val value: Boolean) : Prefix
    class Predicate(val test: (term: String, index: Int, terms: List<String>) -> Boolean) : Prefix
}

/** `fuzzy`: a boolean, or a number (a fraction of the term length below 1, an edit distance from 1). */
sealed interface Fuzzy {
    data class Flag(val value: Boolean) : Fuzzy
    data class Amount(val value: Double) : Fuzzy
}

/** Whether a `fuzzy` value is truthy (`options.fuzzy || false`). */
internal val Fuzzy.isTruthy: Boolean
    get() = when (this) {
        is Fuzzy.Flag -> value
        is Fuzzy.Amount -> value != 0.0 && !value.isNaN()
    }

class Weights(val fuzzy: Double? = null, val prefix: Double? = null)

class BM25Params(val k: Double, val b: Double, val d: Double)

typealias BoostDocument = (id: Value, term: String, storedFields: Map<String, Value>?) -> Double

typealias BoostTerm = (term: String, index: Int, terms: List<String>) -> Double

class SearchOptions(
    val fields: List<String>? = null,
    val filter: ((SearchResult) -> Boolean)? = null,
    val boost: Map<String, Double>? = null,
    val weights: Weights? = null,
    val boostDocument: BoostDocument? = null,
    val boostTerm: BoostTerm? = null,
    val prefix: Prefix? = null,
    val fuzzy: Fuzzy? = null,
    val maxFuzzy: Double? = null,
    val combineWith: String? = null,
    val bm25: BM25Params? = null,
) {
    /** `{ ...this, ...other }`: every option [other] gives replaces this one. */
    operator fun plus(other: SearchOptions) = SearchOptions(
        fields = other.fields ?: fields,
        filter = other.filter ?: filter,
        boost = other.boost ?: boost,
        weights = other.weights ?: weights,
        boostDocument = other.boostDocument ?: boostDocument,
        boostTerm = other.boostTerm ?: boostTerm,
        prefix = other.prefix ?: prefix,
        fuzzy = other.fuzzy ?: fuzzy,
        maxFuzzy = other.maxFuzzy ?: maxFuzzy,
        combineWith = other.combineWith ?: combineWith,
        bm25 = other.bm25 ?: bm25,
    )
}

/** `autoVacuum`: a boolean, or the options of the automatic vacuum. */
sealed interface AutoVacuum {
    data class Flag(val value: Boolean) : AutoVacuum
}

class AutoVacuumOptions(
    val batchSize: Double? = null,
    val batchWait: Double? = null,
    val minDirtCount: Double? = null,
    val minDirtFactor: Double? = null,
) : AutoVacuum

class VacuumOptions(val batchSize: Double? = null, val batchWait: Double? = null)

class VacuumConditions(val minDirtCount: Double? = null, val minDirtFactor: Double? = null)

typealias Logger = (level: String, message: String, code: String?) -> Unit

/** The constructor's options; MiniSearch puts its defaults under them (`{ ...defaultOptions, ...options }`). */
class Options(
    val fields: List<String>,
    val idField: String? = null,
    val storeFields: List<String>? = null,
    val searchOptions: SearchOptions? = null,
    val autoSuggestOptions: SearchOptions? = null,
    val autoVacuum: AutoVacuum? = null,
    val logger: Logger? = null,
)

/** A query: a string, `MiniSearch.wildcard`, or a combination of queries with its own options. */
sealed interface Query {
    data class Text(val text: String) : Query
    data object Wildcard : Query
    class Combination(val queries: List<Query>, val options: SearchOptions) : Query
}

/** One search result. [terms] is `Object.keys(match)`. */
class SearchResult(
    val id: Value,
    val score: Double,
    val terms: List<String>,
    val queryTerms: List<String>,
    val match: Map<String, List<String>>,
    /** The document's stored fields, which MiniSearch copies onto the result (`Object.assign`). */
    val storedFields: Map<String, Value>?,
) {
    /**
     * `result[name]` on MiniSearch's result object: a stored field replaces a property of the same
     * name. `terms`, `queryTerms` and `match` are arrays and objects, which no scalar equals, so
     * they read as null here; an unknown name is `undefined`, null too.
     */
    operator fun get(name: String): Value? = storedFields?.get(name) ?: when (name) {
        "id" -> id
        "score" -> Value.Number(score)
        else -> null
    }
}

class Suggestion(val suggestion: String, val terms: List<String>, val score: Double)

/** What `toJSON` returns (`AsPlainObject`); `JSON.stringify` writes holes and NaN as `null`. */
class AsPlainObject(
    val documentCount: Int,
    val nextId: Int,
    val documentIds: Map<Int, Value>,
    val fieldIds: Map<String, Int>,
    val fieldLength: Map<Int, List<Int?>>,
    val averageFieldLength: List<Double?>,
    val storedFields: Map<Int, Map<String, Value>>,
    val dirtCount: Int,
    val index: List<Pair<String, Map<Int, Map<Int, Int>>>>,
    val serializationVersion: Int,
)

/** An error MiniSearch throws (`new Error(message)`). */
class MiniSearchException(message: String) : Exception(message)
