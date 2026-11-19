// The `MiniSearch` class (minisearch 7.2.0, `dist/es/index.js`), method by method: documents and
// the inverted index, field lengths, vacuuming, search (option merging, query combinations,
// `executeQuerySpec`, `termResults`, BM25+, the combinators, result assembly), autoSuggest and
// `toJSON`.
//
// The default tokenizer and term processor are the host's, as the contract allows: Kotlin/JS's
// `Regex` is a JavaScript `RegExp` (with the `u` flag) and `lowercase()` is `toLowerCase()`.
// `kotlin.math.ln` is `Math.log`.

package minisearch

import kotlin.math.ln
import kotlin.math.min
import kotlin.math.roundToInt

private const val OR = "or"
private const val AND = "and"
private const val AND_NOT = "and_not"

private val defaultBM25params = BM25Params(k = 1.2, b = 0.7, d = 0.5)

private val defaultSearchOptions = SearchOptions(
    combineWith = OR,
    prefix = Prefix.Flag(false),
    fuzzy = Fuzzy.Flag(false),
    maxFuzzy = 6.0,
    boost = emptyMap(),
    weights = Weights(fuzzy = 0.45, prefix = 0.375),
    bm25 = defaultBM25params,
)

private val defaultAutoSuggestOptions = SearchOptions(
    combineWith = AND,
    prefix = Prefix.Predicate { _, i, terms -> i == terms.size - 1 },
)

private val defaultVacuumOptions = VacuumOptions(batchSize = 1000.0, batchWait = 10.0)

private val defaultVacuumConditions = VacuumConditions(minDirtCount = 20.0, minDirtFactor = 0.1)

private val defaultAutoVacuumOptions = AutoVacuumOptions(
    batchSize = defaultVacuumOptions.batchSize,
    batchWait = defaultVacuumOptions.batchWait,
    minDirtCount = defaultVacuumConditions.minDirtCount,
    minDirtFactor = defaultVacuumConditions.minDirtFactor,
)

/** This regular expression matches any Unicode space, newline, or punctuation character. */
private val SPACE_OR_PUNCTUATION = Regex("[\\n\\r\\p{Z}\\p{P}]+")

/** The default `tokenize`: `text.split(SPACE_OR_PUNCTUATION)`. */
private fun tokenize(text: String): List<String> = text.split(SPACE_OR_PUNCTUATION)

/** The default `processTerm`: `term.toLowerCase()`. */
private fun processTerm(term: String): String = term.lowercase()

/** JavaScript's `value || fallback` for a number: undefined, 0 and NaN are falsy. */
private fun Double?.orIfFalsy(fallback: Double): Double = if (this == null || this == 0.0 || isNaN()) fallback else this

/** A term's index data (`fieldsData`): field id to posting list, a posting list short id to term frequency. */
internal typealias FieldTermData = MutableMap<Int, MutableMap<Int, Int>>

/** `const createMap = () => new Map()`. A lambda value: a callable reference (`::f`) is an object per use. */
private val createMap: () -> FieldTermData = { LinkedHashMap() }

/** One document's accumulated result (`RawResultValue`). */
private class RawResultValue(
    var score: Double,
    /** The query terms that matched. */
    val terms: MutableList<String>,
    /** Document term to the fields it matched in (`MatchInfo`), in insertion order. */
    val match: MutableMap<String, MutableList<String>>,
)

/** Short id to result, in insertion order (`RawResult`). */
private typealias RawResult = LinkedHashMap<Int, RawResultValue>

/** One query term with its options (`QuerySpec`). [fuzzy] is null for `false`. */
private class QuerySpec(val term: String, val fuzzy: Fuzzy?, val prefix: Boolean, val termBoost: Double)

private class SuggestionAccumulator(var score: Double, val terms: List<String>, var count: Int)

class MiniSearch(options: Options) {
    private val fields = options.fields
    private val idField = options.idField ?: "id"
    private val storeFields = options.storeFields ?: emptyList()
    private val logger: Logger = options.logger ?: { level, message, _ -> println("$level: $message") }

    /** `autoVacuum`, null for `false`: absent or `true` is [defaultAutoVacuumOptions]. */
    private var autoVacuum: AutoVacuumOptions? = when (val autoVacuum = options.autoVacuum) {
        null -> defaultAutoVacuumOptions
        is AutoVacuum.Flag -> if (autoVacuum.value) defaultAutoVacuumOptions else null
        is AutoVacuumOptions -> autoVacuum
    }
    private val searchOptions = defaultSearchOptions + (options.searchOptions ?: SearchOptions())
    private val autoSuggestOptions = defaultAutoSuggestOptions + (options.autoSuggestOptions ?: SearchOptions())

    private var index = SearchableMap<FieldTermData>()

    var documentCount = 0
        private set
    private var documentIds = LinkedHashMap<Int, Value>()
    private var idToShortId = HashMap<Value, Int>()
    private val fieldIds = LinkedHashMap<String, Int>()
    /** Per short id, the field lengths, with holes (null) for the fields the document lacks. */
    private var fieldLength = LinkedHashMap<Int, MutableList<Int?>>()
    /** The average field lengths, with holes (null) for fields never set. */
    private var avgFieldLength: MutableList<Double?> = ArrayList()
    private var nextId = 0
    private var storedFields = LinkedHashMap<Int, MutableMap<String, Value>>()

    var dirtCount = 0
        private set

    /**
     * Whether `_currentVacuum` is set. MiniSearch clears it only in a microtask after the vacuum's
     * last batch, so once a vacuum has started it stays set for the rest of a synchronous run.
     */
    private var currentVacuum = false

    init {
        addFields(fields)
    }

    fun add(document: Document) {
        val id = document[idField]
        if (id == null || id == Value.Null) throw MiniSearchException("MiniSearch: document does not have ID field \"$idField\"")
        if (idToShortId.containsKey(id)) throw MiniSearchException("MiniSearch: duplicate ID $id")
        val shortDocumentId = addDocumentId(id)
        saveStoredFields(shortDocumentId, document)
        for (field in fields) {
            val fieldValue = document[field]
            if (fieldValue.isNullish()) continue
            val tokens = tokenize(fieldValue.toString())
            val fieldId = fieldIds.getValue(field)
            val uniqueTerms = tokens.toSet().size
            addFieldLength(shortDocumentId, fieldId, documentCount - 1, uniqueTerms)
            for (term in tokens) {
                val processedTerm = processTerm(term)
                if (processedTerm.isNotEmpty()) addTerm(fieldId, shortDocumentId, processedTerm)
            }
        }
    }

    fun addAll(documents: Iterable<Document>) {
        for (document in documents) add(document)
    }

    fun remove(document: Document) {
        val id = document[idField]
        if (id == null || id == Value.Null) throw MiniSearchException("MiniSearch: document does not have ID field \"$idField\"")
        val shortId = idToShortId[id]
            ?: throw MiniSearchException("MiniSearch: cannot remove document with ID $id: it is not in the index")
        for (field in fields) {
            val fieldValue = document[field]
            if (fieldValue.isNullish()) continue
            val tokens = tokenize(fieldValue.toString())
            val fieldId = fieldIds.getValue(field)
            val uniqueTerms = tokens.toSet().size
            removeFieldLength(shortId, fieldId, documentCount, uniqueTerms)
            for (term in tokens) {
                val processedTerm = processTerm(term)
                if (processedTerm.isNotEmpty()) removeTerm(fieldId, shortId, processedTerm)
            }
        }
        storedFields.remove(shortId)
        documentIds.remove(shortId)
        idToShortId.remove(id)
        fieldLength.remove(shortId)
        documentCount -= 1
    }

    /** `removeAll(documents)`; an explicit null argument is an error, unlike [removeAll] with none. */
    fun removeAll(documents: Iterable<Document>?) {
        if (documents == null) {
            throw MiniSearchException("Expected documents to be present. Omit the argument to remove all documents.")
        }
        for (document in documents) remove(document)
    }

    /** `removeAll()`: a fresh index. The dirt count and the vacuum state are kept. */
    fun removeAll() {
        index = SearchableMap()
        documentCount = 0
        documentIds = LinkedHashMap()
        idToShortId = HashMap()
        fieldLength = LinkedHashMap()
        avgFieldLength = ArrayList()
        storedFields = LinkedHashMap()
        nextId = 0
    }

    fun discard(id: Value) {
        val shortId = idToShortId[id]
            ?: throw MiniSearchException("MiniSearch: cannot discard document with ID $id: it is not in the index")
        idToShortId.remove(id)
        documentIds.remove(shortId)
        storedFields.remove(shortId)
        // `forEach` over the field lengths skips their holes.
        fieldLength[shortId]?.forEachIndexed { fieldId, length ->
            if (length != null) removeFieldLength(shortId, fieldId, documentCount, length)
        }
        fieldLength.remove(shortId)
        documentCount -= 1
        dirtCount += 1
        maybeAutoVacuum()
    }

    private fun maybeAutoVacuum() {
        val autoVacuum = autoVacuum ?: return
        conditionalVacuum(
            VacuumOptions(autoVacuum.batchSize, autoVacuum.batchWait),
            VacuumConditions(autoVacuum.minDirtCount, autoVacuum.minDirtFactor),
        )
    }

    fun discardAll(ids: Iterable<Value>) {
        val autoVacuum = autoVacuum
        try {
            this.autoVacuum = null
            for (id in ids) discard(id)
        } finally {
            this.autoVacuum = autoVacuum
        }
        maybeAutoVacuum()
    }

    fun replace(updatedDocument: Document) {
        // `discard(undefined)` for a document without an id.
        val id = updatedDocument[idField]
            ?: throw MiniSearchException("MiniSearch: cannot discard document with ID undefined: it is not in the index")
        discard(id)
        add(updatedDocument)
    }

    fun vacuum(options: VacuumOptions = VacuumOptions()) = conditionalVacuum(options)

    private fun conditionalVacuum(options: VacuumOptions, conditions: VacuumConditions? = null) {
        // MiniSearch chains a request made while a vacuum is current behind that vacuum's promise,
        // which settles after the synchronous run: within the run, never.
        if (currentVacuum) return
        if (!vacuumConditionsMet(conditions)) return
        performVacuuming(options)
        currentVacuum = true
    }

    /**
     * The synchronous part of `performVacuuming`, an async function: the terms in iteration order up
     * to the first batch boundary, where MiniSearch awaits a timer and the rest of the vacuum runs
     * after the synchronous run. Only a walk that ends before a boundary gives the dirt back.
     */
    private fun performVacuuming(options: VacuumOptions, conditions: VacuumConditions? = null) {
        val initialDirtCount = dirtCount
        if (!vacuumConditionsMet(conditions)) return
        val batchSize = options.batchSize.orIfFalsy(defaultVacuumOptions.batchSize!!)
        var i = 1
        for ((term, fieldsData) in index) {
            val fields = fieldsData.values.iterator()
            while (fields.hasNext()) {
                val fieldIndex = fields.next()
                val shortIds = fieldIndex.keys.iterator()
                while (shortIds.hasNext()) {
                    val shortId = shortIds.next()
                    if (documentIds.containsKey(shortId)) continue
                    if (fieldIndex.size <= 1) fields.remove() else shortIds.remove()
                }
            }
            if (index[term]!!.isEmpty()) index.delete(term)
            if (i % batchSize == 0.0) return
            i += 1
        }
        dirtCount -= initialDirtCount
    }

    private fun vacuumConditionsMet(conditions: VacuumConditions?): Boolean {
        if (conditions == null) return true
        val minDirtCount = conditions.minDirtCount.orIfFalsy(defaultAutoVacuumOptions.minDirtCount!!)
        val minDirtFactor = conditions.minDirtFactor.orIfFalsy(defaultAutoVacuumOptions.minDirtFactor!!)
        return dirtCount >= minDirtCount && dirtFactor >= minDirtFactor
    }

    val isVacuuming: Boolean get() = currentVacuum

    val dirtFactor: Double get() = dirtCount.toDouble() / (1 + documentCount + dirtCount)

    fun has(id: Value): Boolean = idToShortId.containsKey(id)

    fun getStoredFields(id: Value): Map<String, Value>? {
        val shortId = idToShortId[id] ?: return null
        return storedFields[shortId]
    }

    fun search(query: Query, searchOptions: SearchOptions = SearchOptions()): List<SearchResult> {
        val searchOptionsWithDefaults = this.searchOptions + searchOptions
        val rawResults = executeQuery(query, searchOptions)
        val results = ArrayList<SearchResult>()
        for ((docId, raw) in rawResults) {
            // The quality counts the distinct query terms that matched, not the document terms.
            val quality = raw.terms.size.takeIf { it != 0 } ?: 1
            val result = SearchResult(
                id = documentIds.getValue(docId),
                score = raw.score * quality,
                terms = objectKeys(raw.match),
                queryTerms = raw.terms,
                match = raw.match,
                storedFields = storedFields[docId],
            )
            val filter = searchOptionsWithDefaults.filter
            if (filter == null || filter(result)) results.add(result)
        }
        // A wildcard query without document boosts is not sorted: every score is 1.
        if (query == Query.Wildcard && searchOptionsWithDefaults.boostDocument == null) return results
        results.sortWith(byScore<SearchResult> { it.score })
        return results
    }

    fun autoSuggest(queryString: String, options: SearchOptions = SearchOptions()): List<Suggestion> {
        val merged = autoSuggestOptions + options
        val suggestions = LinkedHashMap<String, SuggestionAccumulator>()
        for (result in search(Query.Text(queryString), merged)) {
            val phrase = result.terms.joinToString(" ")
            val suggestion = suggestions[phrase]
            if (suggestion != null) {
                suggestion.score += result.score
                suggestion.count += 1
            } else {
                suggestions[phrase] = SuggestionAccumulator(result.score, result.terms, 1)
            }
        }
        val results = ArrayList<Suggestion>()
        for ((suggestion, accumulated) in suggestions) {
            results.add(Suggestion(suggestion, accumulated.terms, accumulated.score / accumulated.count))
        }
        results.sortWith(byScore<Suggestion> { it.score })
        return results
    }

    val termCount: Int get() = index.size

    private fun executeQuery(query: Query, searchOptions: SearchOptions = SearchOptions()): RawResult = when (query) {
        Query.Wildcard -> executeWildcardQuery(searchOptions)
        is Query.Combination -> {
            // `{ ...searchOptions, ...query }`: the index's search options are not merged in here.
            val options = searchOptions + query.options
            val results = query.queries.map { subquery -> executeQuery(subquery, options) }
            combineResults(results, options.combineWith)
        }
        is Query.Text -> {
            val options = this.searchOptions + searchOptions
            val terms = tokenize(query.text).map(::processTerm).filter { it.isNotEmpty() }
            val queries = terms.mapIndexed { i, term -> termToQuerySpec(options, term, i, terms) }
            val results = queries.map { spec -> executeQuerySpec(spec, options) }
            combineResults(results, options.combineWith)
        }
    }

    private fun executeQuerySpec(query: QuerySpec, searchOptions: SearchOptions): RawResult {
        val options = this.searchOptions + searchOptions
        // An object built field by field: a repeated field keeps its first position.
        val boosts = LinkedHashMap<String, Double>()
        for (field in options.fields ?: fields) boosts[field] = options.boost?.get(field).orIfFalsy(1.0)
        val boostDocument = options.boostDocument
        // `{ ...defaultSearchOptions.weights, ...weights }`.
        val fuzzyWeight = options.weights?.fuzzy ?: defaultSearchOptions.weights!!.fuzzy!!
        val prefixWeight = options.weights?.prefix ?: defaultSearchOptions.weights!!.prefix!!
        // Present: the index's search options start from the library defaults.
        val maxFuzzy = options.maxFuzzy!!
        val bm25params = options.bm25!!

        val data = index[query.term]
        val results = termResults(query.term, query.term, 1.0, query.termBoost, data, boosts, boostDocument, bm25params)

        var prefixMatches: SearchableMap<FieldTermData>? = null
        var fuzzyMatches: LinkedHashMap<String, FuzzyMatch<FieldTermData>>? = null
        if (query.prefix) prefixMatches = index.atPrefix(query.term)
        if (query.fuzzy != null) {
            val fuzzy = when (val value = query.fuzzy) {
                is Fuzzy.Flag -> 0.2
                is Fuzzy.Amount -> value.value
            }
            val maxDistance = if (fuzzy < 1) min(maxFuzzy, (query.term.length * fuzzy).roundToInt().toDouble()) else fuzzy
            if (maxDistance != 0.0 && !maxDistance.isNaN()) fuzzyMatches = index.fuzzyGet(query.term, maxDistance.toInt())
        }

        if (prefixMatches != null) {
            for ((term, termData) in prefixMatches) {
                val distance = term.length - query.term.length
                if (distance == 0) continue // Skip exact match.
                // A term that is also a prefix match is scored as a prefix match only.
                fuzzyMatches?.remove(term)
                val weight = prefixWeight * term.length / (term.length + 0.3 * distance)
                termResults(query.term, term, weight, query.termBoost, termData, boosts, boostDocument, bm25params, results)
            }
        }

        if (fuzzyMatches != null) {
            for (term in fuzzyMatches.keys) {
                val match = fuzzyMatches.getValue(term)
                if (match.distance == 0) continue // Skip exact match.
                val weight = fuzzyWeight * term.length / (term.length + match.distance)
                termResults(query.term, term, weight, query.termBoost, match.value, boosts, boostDocument, bm25params, results)
            }
        }

        return results
    }

    private fun executeWildcardQuery(searchOptions: SearchOptions): RawResult {
        val results = RawResult()
        val options = this.searchOptions + searchOptions
        for ((shortId, id) in documentIds) {
            val score = options.boostDocument?.invoke(id, "", storedFields[shortId]) ?: 1.0
            results[shortId] = RawResultValue(score, ArrayList(), LinkedHashMap())
        }
        return results
    }

    private fun combineResults(results: List<RawResult>, combineWith: String? = null): RawResult {
        if (results.isEmpty()) return RawResult()
        val operator = (combineWith ?: OR).lowercase()
        val combinator: (RawResult, RawResult) -> RawResult = when (operator) {
            OR -> ::combineOr
            AND -> ::combineAnd
            AND_NOT -> ::combineAndNot
            else -> throw MiniSearchException("Invalid combination operator: ${combineWith ?: OR}")
        }
        return results.reduce(combinator)
    }

    fun toJSON(): AsPlainObject {
        val index = ArrayList<Pair<String, Map<Int, Map<Int, Int>>>>()
        for ((term, fieldIndex) in this.index) {
            val data = LinkedHashMap<Int, Map<Int, Int>>()
            for ((fieldId, freqs) in fieldIndex) data[fieldId] = LinkedHashMap(freqs)
            index.add(Pair(term, data))
        }
        return AsPlainObject(
            documentCount = documentCount,
            nextId = nextId,
            documentIds = LinkedHashMap(documentIds),
            fieldIds = fieldIds,
            fieldLength = LinkedHashMap(fieldLength),
            averageFieldLength = avgFieldLength,
            storedFields = LinkedHashMap(storedFields),
            dirtCount = dirtCount,
            index = index,
            serializationVersion = 2,
        )
    }

    private fun termResults(
        sourceTerm: String,
        derivedTerm: String,
        termWeight: Double,
        termBoost: Double,
        fieldTermData: FieldTermData?,
        fieldBoosts: Map<String, Double>,
        boostDocumentFn: BoostDocument?,
        bm25params: BM25Params,
        results: RawResult = RawResult(),
    ): RawResult {
        if (fieldTermData == null) return results
        for ((field, fieldBoost) in fieldBoosts) {
            // A field the index does not have has no id, and no posting list.
            val fieldId = fieldIds[field] ?: continue
            val fieldTermFreqs = fieldTermData[fieldId] ?: continue
            var matchingFields = fieldTermFreqs.size
            val avgFieldLength = avgFieldLength.getOrNull(fieldId) ?: Double.NaN
            // A snapshot of the short ids: removing a discarded document's posting deletes the
            // current entry, which MiniSearch's live `Map` iteration survives.
            for (docId in fieldTermFreqs.keys.toList()) {
                if (!documentIds.containsKey(docId)) {
                    removeTerm(fieldId, docId, derivedTerm)
                    matchingFields -= 1
                    continue
                }
                val docBoost = boostDocumentFn?.invoke(documentIds.getValue(docId), derivedTerm, storedFields[docId]) ?: 1.0
                if (docBoost == 0.0 || docBoost.isNaN()) continue
                val termFreq = fieldTermFreqs.getValue(docId)
                val fieldLength = fieldLength[docId]?.getOrNull(fieldId)?.toDouble() ?: Double.NaN
                // The total number of fields is the number of documents (`documentCount`).
                val rawScore = calcBM25Score(termFreq, matchingFields, documentCount, fieldLength, avgFieldLength, bm25params)
                val weightedScore = termWeight * termBoost * fieldBoost * docBoost * rawScore
                val result = results[docId]
                if (result != null) {
                    result.score += weightedScore
                    assignUniqueTerm(result.terms, sourceTerm)
                    val match = result.match[derivedTerm]
                    if (match != null) match.add(field) else result.match[derivedTerm] = mutableListOf(field)
                } else {
                    results[docId] = RawResultValue(
                        score = weightedScore,
                        terms = mutableListOf(sourceTerm),
                        match = linkedMapOf(derivedTerm to mutableListOf(field)),
                    )
                }
            }
        }
        return results
    }

    private fun addTerm(fieldId: Int, documentId: Int, term: String) {
        val indexData = index.fetch(term, createMap)
        val fieldIndex = indexData[fieldId]
        if (fieldIndex == null) {
            val newFieldIndex = LinkedHashMap<Int, Int>()
            newFieldIndex[documentId] = 1
            indexData[fieldId] = newFieldIndex
        } else {
            val docs = fieldIndex[documentId]
            fieldIndex[documentId] = (docs ?: 0) + 1
        }
    }

    private fun removeTerm(fieldId: Int, documentId: Int, term: String) {
        if (!index.has(term)) {
            warnDocumentChanged(documentId, fieldId, term)
            return
        }
        val indexData = index.fetch(term, createMap)
        val fieldIndex = indexData[fieldId]
        val frequency = fieldIndex?.get(documentId)
        if (fieldIndex == null || frequency == null) {
            warnDocumentChanged(documentId, fieldId, term)
        } else if (frequency <= 1) {
            if (fieldIndex.size <= 1) indexData.remove(fieldId) else fieldIndex.remove(documentId)
        } else {
            fieldIndex[documentId] = frequency - 1
        }
        if (index[term]!!.isEmpty()) index.delete(term)
    }

    private fun warnDocumentChanged(shortDocumentId: Int, fieldId: Int, term: String) {
        for ((fieldName, id) in fieldIds) {
            if (id == fieldId) {
                val documentId = documentIds[shortDocumentId]?.toString() ?: "undefined"
                logger(
                    "warn",
                    "MiniSearch: document with ID $documentId has changed before removal: term \"$term\" was not present in field \"$fieldName\". Removing a document after it has changed can corrupt the index!",
                    "version_conflict",
                )
                return
            }
        }
    }

    private fun addDocumentId(documentId: Value): Int {
        val shortDocumentId = nextId
        idToShortId[documentId] = shortDocumentId
        documentIds[shortDocumentId] = documentId
        documentCount += 1
        nextId += 1
        return shortDocumentId
    }

    private fun addFields(fields: List<String>) {
        for (i in fields.indices) fieldIds[fields[i]] = i
    }

    private fun addFieldLength(documentId: Int, fieldId: Int, count: Int, length: Int) {
        val fieldLengths = fieldLength.getOrPut(documentId) { ArrayList() }
        fieldLengths.setGrowing(fieldId, length)
        val averageFieldLength = avgFieldLength.getOrNull(fieldId).orIfFalsy(0.0)
        val totalFieldLength = (averageFieldLength * count) + length
        avgFieldLength.setGrowing(fieldId, totalFieldLength / (count + 1))
    }

    private fun removeFieldLength(documentId: Int, fieldId: Int, count: Int, length: Int) {
        if (count == 1) {
            avgFieldLength.setGrowing(fieldId, 0.0)
            return
        }
        // A hole reads as undefined, which makes the average NaN.
        val totalFieldLength = ((avgFieldLength.getOrNull(fieldId) ?: Double.NaN) * count) - length
        avgFieldLength.setGrowing(fieldId, totalFieldLength / (count - 1))
    }

    private fun saveStoredFields(documentId: Int, doc: Document) {
        if (storeFields.isEmpty()) return
        val documentFields = storedFields.getOrPut(documentId) { LinkedHashMap() }
        for (fieldName in storeFields) {
            // `undefined` is not stored; `null` is.
            val fieldValue = doc[fieldName]
            if (fieldValue != null) documentFields[fieldName] = fieldValue
        }
    }
}

private fun combineOr(a: RawResult, b: RawResult): RawResult {
    for (docId in b.keys) {
        val existing = a[docId]
        if (existing == null) {
            a[docId] = b.getValue(docId)
        } else {
            val incoming = b.getValue(docId)
            existing.score = existing.score + incoming.score
            existing.match.putAll(incoming.match) // `Object.assign(existing.match, match)`
            assignUniqueTerms(existing.terms, incoming.terms)
        }
    }
    return a
}

/** AND builds a new map in `b`'s order. */
private fun combineAnd(a: RawResult, b: RawResult): RawResult {
    val combined = RawResult()
    for (docId in b.keys) {
        val existing = a[docId] ?: continue
        val incoming = b.getValue(docId)
        assignUniqueTerms(existing.terms, incoming.terms)
        existing.match.putAll(incoming.match) // `Object.assign(existing.match, match)`
        combined[docId] = RawResultValue(existing.score + incoming.score, existing.terms, existing.match)
    }
    return combined
}

private fun combineAndNot(a: RawResult, b: RawResult): RawResult {
    for (docId in b.keys) a.remove(docId)
    return a
}

/** `calcBM25Score`, operation for operation (no fused multiply-add in JavaScript). */
private fun calcBM25Score(
    termFreq: Int,
    matchingCount: Int,
    totalCount: Int,
    fieldLength: Double,
    avgFieldLength: Double,
    bm25params: BM25Params,
): Double {
    val k = bm25params.k
    val b = bm25params.b
    val d = bm25params.d
    val invDocFreq = ln(1 + (totalCount - matchingCount + 0.5) / (matchingCount + 0.5))
    return invDocFreq * (d + termFreq * (k + 1) / (termFreq + k * (1 - b + b * fieldLength / avgFieldLength)))
}

private fun termToQuerySpec(options: SearchOptions, term: String, i: Int, terms: List<String>): QuerySpec {
    val fuzzy = options.fuzzy?.takeIf { it.isTruthy }
    val prefix = when (val prefix = options.prefix) {
        is Prefix.Predicate -> prefix.test(term, i, terms)
        is Prefix.Flag -> prefix.value
        null -> false
    }
    val termBoost = options.boostTerm?.invoke(term, i, terms) ?: 1.0
    return QuerySpec(term, fuzzy, prefix, termBoost)
}

private fun assignUniqueTerm(target: MutableList<String>, term: String) {
    // Avoid adding duplicate terms.
    if (term !in target) target.add(term)
}

private fun assignUniqueTerms(target: MutableList<String>, source: List<String>) {
    for (term in source) {
        if (term !in target) target.add(term)
    }
}

/**
 * `byScore`, `(a, b) => b.score - a.score`, as `Array.prototype.sort` reads a comparator: by the
 * sign of the difference, so `-0` and `0` tie (`Double.compareTo` orders them). Kotlin's sorts are
 * stable, as JavaScript's is.
 */
private inline fun <T> byScore(crossinline score: (T) -> Double) = Comparator<T> { a, b ->
    val difference = score(b) - score(a)
    if (difference < 0) -1 else if (difference > 0) 1 else 0
}

/**
 * The keys of an object in `Object.keys` order: array indices (the canonical decimal form of an
 * integer from 0 to 2^32 - 2) first, ascending, then every other key in insertion order.
 */
private fun objectKeys(map: Map<String, *>): List<String> {
    val (indices, others) = map.keys.partition(::isArrayIndex)
    return if (indices.isEmpty()) others else indices.sortedBy(::arrayIndexValue) + others
}

private fun isArrayIndex(key: String): Boolean {
    if (key.isEmpty() || key.length > 10) return false
    if (key[0] == '0') return key.length == 1
    for (c in key) if (c !in '0'..'9') return false
    return arrayIndexValue(key) <= 4294967294.0
}

/** The value of a decimal digit string (exact: at most 10 digits). */
private fun arrayIndexValue(key: String): Double {
    var value = 0.0
    for (c in key) value = value * 10 + (c - '0')
    return value
}

/** `array[index] = value` on a JavaScript array: assigning past the end leaves holes (null). */
private fun <T : Any> MutableList<T?>.setGrowing(index: Int, value: T) {
    while (size <= index) add(null)
    set(index, value)
}
