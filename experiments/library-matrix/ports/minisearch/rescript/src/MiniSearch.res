// The `MiniSearch` class of minisearch/dist/es/index.js (7.2.0), for the API subset
// libraries/minisearch/CONTRACT.md defines, method by method and on the same data structures:
// JavaScript `Map`s (ReScript's `Map.t`) for every map MiniSearch keeps, plain objects
// (`dict`) where it uses objects (`_fieldIds`, a result's `match`, the field boosts), and arrays
// with holes for the field lengths.

// A document id: `Map` keys compare by SameValueZero, so `1` and `"1"` are different documents.
@unboxed type id = String(string) | Number(float)

// A document: a JSON object, of which MiniSearch reads the id, indexed and stored fields.
type document = dict<JSON.t>

type shortId = int
type fieldId = int

// A term's index data (`fieldsData`): field id to posting list, short id to term frequency.
type fieldTermData = Map.t<fieldId, Map.t<shortId, int>>

// `prefix`: a boolean, or a function of `(term, i, terms)` (autoSuggest's default).
@unboxed type prefix = Prefix(bool) | PrefixFunction((string, int, array<string>) => bool)

// `fuzzy`: a boolean, a fraction of the term length, or an edit distance.
@unboxed type fuzzy = Fuzzy(bool) | FuzzyNumber(float)

type weights = {fuzzy?: float, prefix?: float}
type bm25Params = {k: float, b: float, d: float}

type searchResult = {
  id: id,
  score: float,
  // The matched document terms, `Object.keys(match)`.
  terms: array<string>,
  // The query terms that matched.
  queryTerms: array<string>,
  // Document term to the fields it matched in.
  @as("match") match: dict<array<string>>,
  // What `Object.assign(result, storedFields)` adds to MiniSearch's result object.
  storedFields: option<dict<JSON.t>>,
}

type searchOptions = {
  fields?: array<string>,
  filter?: searchResult => bool,
  boost?: dict<float>,
  boostTerm?: (string, int, array<string>) => float,
  weights?: weights,
  boostDocument?: (id, string, option<dict<JSON.t>>) => float,
  prefix?: prefix,
  fuzzy?: fuzzy,
  maxFuzzy?: float,
  combineWith?: string,
  bm25?: bm25Params,
}

// A query: a string, `MiniSearch.wildcard`, or a combination with its own options.
type rec query =
  | Text(string)
  | Wildcard
  | Combination({options: searchOptions, queries: array<query>})

type vacuumOptions = {batchSize?: float, batchWait?: float}
type vacuumConditions = {minDirtCount?: float, minDirtFactor?: float}
type autoVacuumOptions = {...vacuumOptions, ...vacuumConditions}
@unboxed type autoVacuum = AutoVacuum(bool) | AutoVacuumOptions(autoVacuumOptions)

type options = {
  fields: array<string>,
  idField?: string,
  storeFields?: array<string>,
  logger?: (string, string, option<string>) => unit,
  autoVacuum?: autoVacuum,
  searchOptions?: searchOptions,
  autoSuggestOptions?: searchOptions,
}

// `this._options`: the constructor's options over the defaults. `autoVacuum` is `None` for
// `autoVacuum: false`.
type resolvedOptions = {
  fields: array<string>,
  idField: string,
  storeFields: array<string>,
  logger: (string, string, option<string>) => unit,
  mutable autoVacuum: option<autoVacuumOptions>,
  searchOptions: searchOptions,
  autoSuggestOptions: searchOptions,
}

// One document's accumulated result before quality, stored fields and sorting.
type rawResult = {mutable score: float, terms: array<string>, mutable match: dict<array<string>>}
type rawResults = Map.t<shortId, rawResult>

type querySpec = {term: string, fuzzy: fuzzy, prefix: bool, termBoost: float}

type suggestion = {suggestion: string, terms: array<string>, score: float}

// The value `toJSON` returns (`AsPlainObject`).
type asPlainObject = {
  documentCount: int,
  nextId: int,
  documentIds: dict<id>,
  fieldIds: dict<fieldId>,
  fieldLength: dict<array<int>>,
  averageFieldLength: array<float>,
  storedFields: dict<dict<JSON.t>>,
  dirtCount: int,
  index: array<(string, dict<dict<int>>)>,
  serializationVersion: int,
}

type t = {
  options: resolvedOptions,
  mutable index: SearchableMap.t<fieldTermData>,
  mutable documentCount: int,
  mutable documentIds: Map.t<shortId, id>,
  mutable idToShortId: Map.t<id, shortId>,
  fieldIds: dict<fieldId>,
  mutable fieldLength: Map.t<shortId, array<int>>,
  mutable avgFieldLength: array<float>,
  mutable nextId: shortId,
  mutable storedFields: Map.t<shortId, dict<JSON.t>>,
  mutable dirtCount: int,
  // `_currentVacuum != null`. MiniSearch clears it only after the vacuum's last `await`, which is
  // after any synchronous run has returned (CONTRACT.md, "Vacuuming in a synchronous run").
  mutable currentVacuum: bool,
}

// What MiniSearch throws (`new Error(message)`).
exception MiniSearchError(string)

// ---------------------------------------------------------------------------------------------
// Defaults and helpers.

let defaultBM25params = {k: 1.2, b: 0.7, d: 0.5}
let defaultFuzzyWeight = 0.45
let defaultPrefixWeight = 0.375

let defaultSearchOptions: searchOptions = {
  combineWith: "or",
  prefix: Prefix(false),
  fuzzy: Fuzzy(false),
  maxFuzzy: 6.,
  boost: dict{},
  weights: {fuzzy: defaultFuzzyWeight, prefix: defaultPrefixWeight},
  bm25: defaultBM25params,
}

let defaultAutoSuggestOptions: searchOptions = {
  combineWith: "and",
  prefix: PrefixFunction((_term, i, terms) => i == terms->Array.length - 1),
}

// `defaultVacuumOptions` and `defaultVacuumConditions`.
let defaultBatchSize = 1000.
let defaultBatchWait = 10.
let defaultMinDirtFactor = 0.1
let defaultMinDirtCount = 20.
let defaultAutoVacuumOptions: autoVacuumOptions = {
  batchSize: defaultBatchSize,
  batchWait: defaultBatchWait,
  minDirtFactor: defaultMinDirtFactor,
  minDirtCount: defaultMinDirtCount,
}

let defaultLogger = (level, message, _code) =>
  switch level {
  | "error" => Console.error(message)
  | "warn" => Console.warn(message)
  | _ => Console.info(message)
  }

// `{ ...base, ...over }`: a key `over` has replaces `base`'s value whole.
let spread = (base: searchOptions, over: searchOptions): searchOptions => {
  fields: ?(over.fields->Option.orElse(base.fields)),
  filter: ?(over.filter->Option.orElse(base.filter)),
  boost: ?(over.boost->Option.orElse(base.boost)),
  boostTerm: ?(over.boostTerm->Option.orElse(base.boostTerm)),
  weights: ?(over.weights->Option.orElse(base.weights)),
  boostDocument: ?(over.boostDocument->Option.orElse(base.boostDocument)),
  prefix: ?(over.prefix->Option.orElse(base.prefix)),
  fuzzy: ?(over.fuzzy->Option.orElse(base.fuzzy)),
  maxFuzzy: ?(over.maxFuzzy->Option.orElse(base.maxFuzzy)),
  combineWith: ?(over.combineWith->Option.orElse(base.combineWith)),
  bm25: ?(over.bm25->Option.orElse(base.bm25)),
}

// `value || fallback` for a number: `0` and `NaN` (and `undefined`) are falsy.
let truthyOr = (value: option<float>, fallback) =>
  switch value {
  | Some(value) if value != 0. && !Float.isNaN(value) => value
  | _ => fallback
  }

let orNaN = (value: option<float>) =>
  switch value {
  | Some(value) => value
  | None => Float.Constants.nan
  }

@val external hasOwnProperty: (dict<'a>, string) => bool = "Object.prototype.hasOwnProperty.call"

let getOwnProperty = (object: dict<'a>, property) =>
  hasOwnProperty(object, property) ? object->Dict.get(property) : None

// `Object.fromEntries(map)` of a map keyed by short or field ids.
@val external objectFromEntries: Map.t<int, 'a> => dict<'a> = "Object.fromEntries"

// `${id}` in a template literal.
let idText = (id: id) =>
  switch id {
  | String(text) => text
  | Number(number) => Float.toString(number)
  }

// `extractField(document, idField)` as an id, `None` for the `null` and `undefined` MiniSearch
// rejects.
let idOf = (value: option<JSON.t>): option<id> =>
  switch value {
  | None | Some(Null) => None
  | Some(String(text)) => Some(String(text))
  | Some(Number(number)) => Some(Number(number))
  | Some(_) => panic("MiniSearch (ReScript port): ids are strings or numbers")
  }

// `stringifyField`: `fieldValue.toString()`.
let stringifyField = (fieldValue: JSON.t) =>
  switch fieldValue {
  | String(text) => text
  | Number(number) => Float.toString(number)
  | Boolean(flag) => Bool.toString(flag)
  | _ => panic("MiniSearch (ReScript port): field values are strings, numbers or null")
  }

// The default tokenizer, `text.split(SPACE_OR_PUNCTUATION)`, on the host `RegExp` with the `u`
// flag (CONTRACT.md, "Tokenizer"), and the default term processing, `term.toLowerCase()`.
let spaceOrPunctuation = /[\n\r\p{Z}\p{P}]+/u
@send external splitByRegExp: (string, RegExp.t) => array<string> = "split"

let tokenize = text => text->splitByRegExp(spaceOrPunctuation)
let processTerm = term => term->String.toLowerCase

let createMap = () => Map.make()

let assignUniqueTerm = (target, term) =>
  if !(target->Array.includes(term)) {
    target->Array.push(term)
  }

let assignUniqueTerms = (target, source) =>
  source->Array.forEach(term => assignUniqueTerm(target, term))

let byScore = (a: searchResult, b: searchResult) => b.score -. a.score

// ---------------------------------------------------------------------------------------------
// The index.

let addFields = (t, fields) =>
  fields->Array.forEachWithIndex((field, i) => t.fieldIds->Dict.set(field, i))

let make = (options: options): t => {
  let autoVacuum = switch options.autoVacuum {
  | None | Some(AutoVacuum(true)) => Some(defaultAutoVacuumOptions)
  | Some(AutoVacuum(false)) => None
  | Some(AutoVacuumOptions(autoVacuum)) => Some(autoVacuum)
  }
  let t = {
    options: {
      fields: options.fields,
      idField: options.idField->Option.getOr("id"),
      storeFields: options.storeFields->Option.getOr([]),
      logger: options.logger->Option.getOr(defaultLogger),
      autoVacuum,
      searchOptions: spread(defaultSearchOptions, options.searchOptions->Option.getOr({})),
      autoSuggestOptions: spread(
        defaultAutoSuggestOptions,
        options.autoSuggestOptions->Option.getOr({}),
      ),
    },
    index: SearchableMap.make(),
    documentCount: 0,
    documentIds: Map.make(),
    idToShortId: Map.make(),
    fieldIds: Dict.make(),
    fieldLength: Map.make(),
    avgFieldLength: [],
    nextId: 0,
    storedFields: Map.make(),
    dirtCount: 0,
    currentVacuum: false,
  }
  addFields(t, options.fields)
  t
}

let documentCount = t => t.documentCount
let termCount = t => t.index->SearchableMap.size
let dirtCount = t => t.dirtCount
let dirtFactor = t =>
  Int.toFloat(t.dirtCount) /. (1. +. Int.toFloat(t.documentCount) +. Int.toFloat(t.dirtCount))
let isVacuuming = t => t.currentVacuum

let addDocumentId = (t, documentId) => {
  let shortDocumentId = t.nextId
  t.idToShortId->Map.set(documentId, shortDocumentId)
  t.documentIds->Map.set(shortDocumentId, documentId)
  t.documentCount = t.documentCount + 1
  t.nextId = t.nextId + 1
  shortDocumentId
}

// The field lengths of a document are an array with holes for the fields it does not have, and
// the averages an array with holes for fields no document ever had (`null` in `toJSON`).
let addFieldLength = (t, documentId, fieldId, count, length) => {
  let fieldLengths = switch t.fieldLength->Map.get(documentId) {
  | Some(fieldLengths) => fieldLengths
  | None =>
    let fieldLengths = []
    t.fieldLength->Map.set(documentId, fieldLengths)
    fieldLengths
  }
  fieldLengths->Array.setUnsafe(fieldId, length)
  let averageFieldLength = t.avgFieldLength->Array.get(fieldId)->truthyOr(0.)
  let totalFieldLength = averageFieldLength *. Int.toFloat(count) +. Int.toFloat(length)
  t.avgFieldLength->Array.setUnsafe(fieldId, totalFieldLength /. Int.toFloat(count + 1))
}

// A hole reads as `undefined`, which makes the average `NaN`.
let removeFieldLength = (t, _documentId, fieldId, count, length) =>
  if count == 1 {
    t.avgFieldLength->Array.setUnsafe(fieldId, 0.)
  } else {
    let totalFieldLength =
      t.avgFieldLength->Array.get(fieldId)->orNaN *. Int.toFloat(count) -. Int.toFloat(length)
    t.avgFieldLength->Array.setUnsafe(fieldId, totalFieldLength /. Int.toFloat(count - 1))
  }

let saveStoredFields = (t, documentId, doc: document) => {
  let storeFields = t.options.storeFields
  if storeFields->Array.length > 0 {
    let documentFields = switch t.storedFields->Map.get(documentId) {
    | Some(documentFields) => documentFields
    | None =>
      let documentFields = Dict.make()
      t.storedFields->Map.set(documentId, documentFields)
      documentFields
    }
    storeFields->Array.forEach(fieldName =>
      switch doc->Dict.get(fieldName) {
      | Some(fieldValue) => documentFields->Dict.set(fieldName, fieldValue)
      | None => ()
      }
    )
  }
}

let addTerm = (t, fieldId, documentId, term) => {
  let indexData = t.index->SearchableMap.fetch(term, createMap)
  switch indexData->Map.get(fieldId) {
  | None =>
    let fieldIndex = Map.make()
    fieldIndex->Map.set(documentId, 1)
    indexData->Map.set(fieldId, fieldIndex)
  | Some(fieldIndex) =>
    let docs = switch fieldIndex->Map.get(documentId) {
    | Some(docs) => docs
    | None => 0
    }
    fieldIndex->Map.set(documentId, docs + 1)
  }
}

let warnDocumentChanged = (t, shortDocumentId, fieldId, term) =>
  switch t.fieldIds
  ->Dict.keysToArray
  ->Array.find(fieldName => t.fieldIds->Dict.getUnsafe(fieldName) == fieldId) {
  | Some(fieldName) =>
    let id = t.documentIds->Map.get(shortDocumentId)->Option.mapOr("undefined", idText)
    t.options.logger(
      "warn",
      `MiniSearch: document with ID ${id} has changed before removal: term "${term}" was not present in field "${fieldName}". Removing a document after it has changed can corrupt the index!`,
      Some("version_conflict"),
    )
  | None => ()
  }

let removeTerm = (t, fieldId, documentId, term) =>
  if !(t.index->SearchableMap.has(term)) {
    warnDocumentChanged(t, documentId, fieldId, term)
  } else {
    let indexData = t.index->SearchableMap.fetch(term, createMap)
    switch indexData->Map.get(fieldId) {
    | Some(fieldIndex) =>
      switch fieldIndex->Map.get(documentId) {
      | Some(frequency) if frequency <= 1 =>
        if fieldIndex->Map.size <= 1 {
          indexData->Map.delete(fieldId)->ignore
        } else {
          fieldIndex->Map.delete(documentId)->ignore
        }
      | Some(frequency) => fieldIndex->Map.set(documentId, frequency - 1)
      | None => warnDocumentChanged(t, documentId, fieldId, term)
      }
    | None => warnDocumentChanged(t, documentId, fieldId, term)
    }
    switch t.index->SearchableMap.get(term) {
    | Some(data) if data->Map.size == 0 => t.index->SearchableMap.delete(term)
    | _ => ()
    }
  }

// ---------------------------------------------------------------------------------------------
// Adding, removing and discarding documents.

let add = (t, document: document) => {
  let {fields, idField} = t.options
  let id = switch idOf(document->Dict.get(idField)) {
  | Some(id) => id
  | None => throw(MiniSearchError(`MiniSearch: document does not have ID field "${idField}"`))
  }
  if t.idToShortId->Map.has(id) {
    throw(MiniSearchError(`MiniSearch: duplicate ID ${idText(id)}`))
  }
  let shortDocumentId = addDocumentId(t, id)
  saveStoredFields(t, shortDocumentId, document)
  fields->Array.forEach(field =>
    switch document->Dict.get(field) {
    | None | Some(Null) => ()
    | Some(fieldValue) =>
      let tokens = tokenize(stringifyField(fieldValue))
      let fieldId = t.fieldIds->Dict.getUnsafe(field)
      let uniqueTerms = Set.fromArray(tokens)->Set.size
      addFieldLength(t, shortDocumentId, fieldId, t.documentCount - 1, uniqueTerms)
      tokens->Array.forEach(term => {
        let processedTerm = processTerm(term)
        if processedTerm != "" {
          addTerm(t, fieldId, shortDocumentId, processedTerm)
        }
      })
    }
  )
}

let addAll = (t, documents) => documents->Array.forEach(document => add(t, document))

let remove = (t, document: document) => {
  let {fields, idField} = t.options
  let id = switch idOf(document->Dict.get(idField)) {
  | Some(id) => id
  | None => throw(MiniSearchError(`MiniSearch: document does not have ID field "${idField}"`))
  }
  let shortId = switch t.idToShortId->Map.get(id) {
  | Some(shortId) => shortId
  | None =>
    throw(
      MiniSearchError(
        `MiniSearch: cannot remove document with ID ${idText(id)}: it is not in the index`,
      ),
    )
  }
  fields->Array.forEach(field =>
    switch document->Dict.get(field) {
    | None | Some(Null) => ()
    | Some(fieldValue) =>
      let tokens = tokenize(stringifyField(fieldValue))
      let fieldId = t.fieldIds->Dict.getUnsafe(field)
      let uniqueTerms = Set.fromArray(tokens)->Set.size
      removeFieldLength(t, shortId, fieldId, t.documentCount, uniqueTerms)
      tokens->Array.forEach(term => {
        let processedTerm = processTerm(term)
        if processedTerm != "" {
          removeTerm(t, fieldId, shortId, processedTerm)
        }
      })
    }
  )
  t.storedFields->Map.delete(shortId)->ignore
  t.documentIds->Map.delete(shortId)->ignore
  t.idToShortId->Map.delete(id)->ignore
  t.fieldLength->Map.delete(shortId)->ignore
  t.documentCount = t.documentCount - 1
}

// `removeAll(documents)`; with an explicit `null`, an error; with no argument at all, a fresh index
// that keeps `dirtCount` and the vacuum state.
let removeAll = (t, ~documents: option<Null.t<array<document>>>=?) =>
  switch documents {
  | Some(documents) =>
    switch documents->Null.toOption {
    | Some(documents) => documents->Array.forEach(document => remove(t, document))
    | None =>
      throw(
        MiniSearchError(
          "Expected documents to be present. Omit the argument to remove all documents.",
        ),
      )
    }
  | None =>
    t.index = SearchableMap.make()
    t.documentCount = 0
    t.documentIds = Map.make()
    t.idToShortId = Map.make()
    t.fieldLength = Map.make()
    t.avgFieldLength = []
    t.storedFields = Map.make()
    t.nextId = 0
  }

// ---------------------------------------------------------------------------------------------
// Vacuuming.

let vacuumConditionsMet = (t, conditions: option<vacuumConditions>) =>
  switch conditions {
  | None => true
  | Some({?minDirtCount, ?minDirtFactor}) =>
    let minDirtCount = minDirtCount->truthyOr(defaultMinDirtCount)
    let minDirtFactor = minDirtFactor->truthyOr(defaultMinDirtFactor)
    Int.toFloat(t.dirtCount) >= minDirtCount && dirtFactor(t) >= minDirtFactor
  }

// The synchronous part of `performVacuuming`: the terms in iteration order up to the first batch
// boundary, where MiniSearch awaits a timer and the rest happens after the synchronous run (a walk
// that ends first also gives back the dirt count).
let performVacuuming = (t, options: vacuumOptions, conditions) => {
  let initialDirtCount = t.dirtCount
  if vacuumConditionsMet(t, conditions) {
    let batchSize = options.batchSize->truthyOr(defaultBatchSize)
    let iterator = t.index->SearchableMap.entries
    let rec batch = i =>
      switch iterator->SearchableMap.next {
      | None => true
      | Some((term, fieldsData)) =>
        fieldsData->Map.forEachWithKey((fieldIndex, fieldId) =>
          fieldIndex->Map.forEachWithKey((_, shortId) =>
            if !(t.documentIds->Map.has(shortId)) {
              if fieldIndex->Map.size <= 1 {
                fieldsData->Map.delete(fieldId)->ignore
              } else {
                fieldIndex->Map.delete(shortId)->ignore
              }
            }
          )
        )
        switch t.index->SearchableMap.get(term) {
        | Some(data) if data->Map.size == 0 => t.index->SearchableMap.delete(term)
        | _ => ()
        }
        Float.mod(Int.toFloat(i), batchSize) == 0. ? false : batch(i + 1)
      }
    if batch(1) {
      t.dirtCount = t.dirtCount - initialDirtCount
    }
  }
}

// A vacuum requested while one is current is chained after it (`_enqueuedVacuum`), so it would
// start after the current one's last `await`: never inside a synchronous run.
let conditionalVacuum = (t, options, conditions) =>
  if !t.currentVacuum && vacuumConditionsMet(t, conditions) {
    performVacuuming(t, options, None)
    t.currentVacuum = true
  }

let vacuum = (t, ~options: vacuumOptions={}) => conditionalVacuum(t, options, None)

let maybeAutoVacuum = t =>
  switch t.options.autoVacuum {
  | None => ()
  | Some({?minDirtFactor, ?minDirtCount, ?batchSize, ?batchWait}) =>
    conditionalVacuum(t, {?batchSize, ?batchWait}, Some({?minDirtCount, ?minDirtFactor}))
  }

let discard = (t, id) => {
  let shortId = switch t.idToShortId->Map.get(id) {
  | Some(shortId) => shortId
  | None =>
    throw(
      MiniSearchError(
        `MiniSearch: cannot discard document with ID ${idText(id)}: it is not in the index`,
      ),
    )
  }
  t.idToShortId->Map.delete(id)->ignore
  t.documentIds->Map.delete(shortId)->ignore
  t.storedFields->Map.delete(shortId)->ignore
  // `forEach` skips the holes: fields the document does not have.
  t.fieldLength
  ->Map.get(shortId)
  ->Option.getOr([])
  ->Array.forEachWithIndex((fieldLength, fieldId) =>
    removeFieldLength(t, shortId, fieldId, t.documentCount, fieldLength)
  )
  t.fieldLength->Map.delete(shortId)->ignore
  t.documentCount = t.documentCount - 1
  t.dirtCount = t.dirtCount + 1
  maybeAutoVacuum(t)
}

// Automatic vacuuming is off while the ids are discarded (and back on even when one fails), then
// considered once.
let discardAll = (t, ids) => {
  let autoVacuum = t.options.autoVacuum
  t.options.autoVacuum = None
  switch ids->Array.forEach(id => discard(t, id)) {
  | () => t.options.autoVacuum = autoVacuum
  | exception error =>
    t.options.autoVacuum = autoVacuum
    throw(error)
  }
  maybeAutoVacuum(t)
}

let replace = (t, updatedDocument: document) => {
  let id = updatedDocument->Dict.get(t.options.idField)
  switch idOf(id) {
  | Some(id) => discard(t, id)
  | None =>
    let text = id->Option.isNone ? "undefined" : "null"
    throw(
      MiniSearchError(
        `MiniSearch: cannot discard document with ID ${text}: it is not in the index`,
      ),
    )
  }
  add(t, updatedDocument)
}

let has = (t, id) => t.idToShortId->Map.has(id)

let getStoredFields = (t, id) =>
  t.idToShortId->Map.get(id)->Option.flatMap(shortId => t.storedFields->Map.get(shortId))

// ---------------------------------------------------------------------------------------------
// Searching.

let calcBM25Score = (
  termFreq,
  matchingCount,
  totalCount,
  fieldLength,
  avgFieldLength,
  {k, b, d}: bm25Params,
) => {
  let invDocFreq = Math.log(1. +. (totalCount -. matchingCount +. 0.5) /. (matchingCount +. 0.5))
  invDocFreq *.
  (d +. termFreq *. (k +. 1.) /. (termFreq +. k *. (1. -. b +. b *. fieldLength /. avgFieldLength)))
}

// BM25+ for every posting of `fieldTermData` in the boosted fields, added to `results`. A posting
// of a discarded document is removed as it is met (`removeTerm`), and lowers `matchingFields` for
// the postings after it.
let termResults = (
  t,
  sourceTerm,
  derivedTerm,
  termWeight,
  termBoost,
  fieldTermData: option<fieldTermData>,
  fieldBoosts: dict<float>,
  boostDocumentFn,
  bm25params,
  results: rawResults,
) => {
  switch fieldTermData {
  | None => ()
  | Some(fieldTermData) =>
    fieldBoosts
    ->Dict.keysToArray
    ->Array.forEach(field => {
      let fieldBoost = fieldBoosts->Dict.getUnsafe(field)
      switch t.fieldIds->Dict.get(field) {
      | None => ()
      | Some(fieldId) =>
        switch fieldTermData->Map.get(fieldId) {
        | None => ()
        | Some(fieldTermFreqs) =>
          let matchingFields = ref(fieldTermFreqs->Map.size)
          let avgFieldLength = t.avgFieldLength->Array.get(fieldId)->orNaN
          fieldTermFreqs->Map.forEachWithKey((termFreq, docId) =>
            switch t.documentIds->Map.get(docId) {
            | None =>
              removeTerm(t, fieldId, docId, derivedTerm)
              matchingFields := matchingFields.contents - 1
            | Some(id) =>
              let docBoost = switch boostDocumentFn {
              | Some(boostDocument) =>
                boostDocument(id, derivedTerm, t.storedFields->Map.get(docId))
              | None => 1.
              }
              if docBoost != 0. && !Float.isNaN(docBoost) {
                let fieldLength = switch t.fieldLength->Map.get(docId) {
                | Some(lengths) =>
                  switch lengths[fieldId] {
                  | Some(length) => Int.toFloat(length)
                  | None => Float.Constants.nan
                  }
                | None => Float.Constants.nan
                }
                let rawScore = calcBM25Score(
                  Int.toFloat(termFreq),
                  Int.toFloat(matchingFields.contents),
                  Int.toFloat(t.documentCount),
                  fieldLength,
                  avgFieldLength,
                  bm25params,
                )
                let weightedScore = termWeight *. termBoost *. fieldBoost *. docBoost *. rawScore
                switch results->Map.get(docId) {
                | Some(result) =>
                  result.score = result.score +. weightedScore
                  assignUniqueTerm(result.terms, sourceTerm)
                  switch result.match->getOwnProperty(derivedTerm) {
                  | Some(match) => match->Array.push(field)
                  | None => result.match->Dict.set(derivedTerm, [field])
                  }
                | None =>
                  let match = Dict.make()
                  match->Dict.set(derivedTerm, [field])
                  results->Map.set(docId, {score: weightedScore, terms: [sourceTerm], match})
                }
              }
            }
          )
        }
      }
    })
  }
  results
}

// The exact term, then its prefix matches, then its fuzzy matches. Scoring the exact term may
// delete it (and merge radix nodes) before the prefix view and the fuzzy walk are taken.
let executeQuerySpec = (t, query: querySpec, searchOptions) => {
  let options = spread(t.options.searchOptions, searchOptions)
  let boosts =
    options.fields
    ->Option.getOr(t.options.fields)
    ->Array.reduce(Dict.make(), (boosts, field) => {
      let boost = options.boost->Option.flatMap(boost => boost->getOwnProperty(field))
      boosts->Dict.set(field, boost->truthyOr(1.))
      boosts
    })
  let boostDocument = options.boostDocument
  // `{ ...defaultSearchOptions.weights, ...weights }`
  let weights = options.weights->Option.getOr({})
  let fuzzyWeight = weights.fuzzy->Option.getOr(defaultFuzzyWeight)
  let prefixWeight = weights.prefix->Option.getOr(defaultPrefixWeight)
  let bm25params = options.bm25->Option.getOrThrow
  let data = t.index->SearchableMap.get(query.term)
  let results = termResults(
    t,
    query.term,
    query.term,
    1.,
    query.termBoost,
    data,
    boosts,
    boostDocument,
    bm25params,
    Map.make(),
  )
  let prefixMatches = query.prefix ? Some(t.index->SearchableMap.atPrefix(query.term)) : None
  let fuzzyMatches = switch query.fuzzy {
  | Fuzzy(false) => None
  | fuzzy =>
    let fuzzy = switch fuzzy {
    | FuzzyNumber(fuzzy) => fuzzy
    | Fuzzy(_) => 0.2
    }
    let maxDistance =
      fuzzy < 1.
        ? Math.min(
            options.maxFuzzy->orNaN,
            Math.round(Int.toFloat(query.term->String.length) *. fuzzy),
          )
        : fuzzy
    maxDistance != 0. && !Float.isNaN(maxDistance)
      ? Some(t.index->SearchableMap.fuzzyGet(query.term, maxDistance->Float.toInt))
      : None
  }
  switch prefixMatches {
  | None => ()
  | Some(prefixMatches) =>
    let iterator = prefixMatches->SearchableMap.entries
    let rec loop = () =>
      switch iterator->SearchableMap.next {
      | None => ()
      | Some((term, data)) =>
        let length = term->String.length
        let distance = length - query.term->String.length
        if distance != 0 {
          // A term that is also a fuzzy match is scored as a prefix match only.
          fuzzyMatches->Option.forEach(fuzzyMatches => fuzzyMatches->Map.delete(term)->ignore)
          let weight =
            prefixWeight *.
            Int.toFloat(length) /.
            (Int.toFloat(length) +. 0.3 *. Int.toFloat(distance))
          termResults(
            t,
            query.term,
            term,
            weight,
            query.termBoost,
            Some(data),
            boosts,
            boostDocument,
            bm25params,
            results,
          )->ignore
        }
        loop()
      }
    loop()
  }
  switch fuzzyMatches {
  | None => ()
  | Some(fuzzyMatches) =>
    fuzzyMatches->Map.forEachWithKey(((data, distance), term) =>
      if distance != 0 {
        let length = Int.toFloat(term->String.length)
        let weight = fuzzyWeight *. length /. (length +. Int.toFloat(distance))
        termResults(
          t,
          query.term,
          term,
          weight,
          query.termBoost,
          Some(data),
          boosts,
          boostDocument,
          bm25params,
          results,
        )->ignore
      }
    )
  }
  results
}

let executeWildcardQuery = (t, searchOptions) => {
  let results: rawResults = Map.make()
  let options = spread(t.options.searchOptions, searchOptions)
  t.documentIds->Map.forEachWithKey((id, shortId) => {
    let score = switch options.boostDocument {
    | Some(boostDocument) => boostDocument(id, "", t.storedFields->Map.get(shortId))
    | None => 1.
    }
    results->Map.set(shortId, {score, terms: [], match: Dict.make()})
  })
  results
}

// OR: `b`'s documents merged into `a`, in `b`'s order after `a`'s.
let combineOr = (a: rawResults, b: rawResults) => {
  b->Map.forEachWithKey((incoming, docId) =>
    switch a->Map.get(docId) {
    | None => a->Map.set(docId, incoming)
    | Some(existing) =>
      let {score, terms, match} = incoming
      existing.score = existing.score +. score
      existing.match = Dict.assign(existing.match, match)
      assignUniqueTerms(existing.terms, terms)
    }
  )
  a
}

// AND: a new map, in `b`'s order, of the documents in both.
let combineAnd = (a: rawResults, b: rawResults) => {
  let combined: rawResults = Map.make()
  b->Map.forEachWithKey((incoming, docId) =>
    switch a->Map.get(docId) {
    | None => ()
    | Some(existing) =>
      let {score, terms, match} = incoming
      assignUniqueTerms(existing.terms, terms)
      let result: rawResult = {
        score: existing.score +. score,
        terms: existing.terms,
        match: Dict.assign(existing.match, match),
      }
      combined->Map.set(docId, result)
    }
  )
  combined
}

let combineAndNot = (a: rawResults, b: rawResults) => {
  b->Map.forEachWithKey((_, docId) => a->Map.delete(docId)->ignore)
  a
}

// No results is an empty map, whatever the operator; otherwise the operator is looked up in lower
// case and the results folded from the left.
let combineResults = (results: array<rawResults>, ~combineWith="or") =>
  if results->Array.length == 0 {
    Map.make()
  } else {
    let combinator = switch combineWith->String.toLowerCase {
    | "or" => combineOr
    | "and" => combineAnd
    | "and_not" => combineAndNot
    | _ => throw(MiniSearchError(`Invalid combination operator: ${combineWith}`))
    }
    results->Array.slice(~start=1)->Array.reduce(results->Array.getUnsafe(0), combinator)
  }

let termToQuerySpec = (options: searchOptions, term, i, terms) => {
  // `options.fuzzy || false`
  let fuzzy = switch options.fuzzy {
  | Some(FuzzyNumber(fuzzy)) if fuzzy != 0. && !Float.isNaN(fuzzy) => FuzzyNumber(fuzzy)
  | Some(Fuzzy(true)) => Fuzzy(true)
  | _ => Fuzzy(false)
  }
  let prefix = switch options.prefix {
  | Some(PrefixFunction(prefix)) => prefix(term, i, terms)
  | Some(Prefix(prefix)) => prefix
  | None => false
  }
  let termBoost = switch options.boostTerm {
  | Some(boostTerm) => boostTerm(term, i, terms)
  | None => 1.
  }
  {term, fuzzy, prefix, termBoost}
}

// A combination's options are the call's options under its own (`{ ...searchOptions, ...query }`):
// the constructor's search options are not merged in at this level.
let rec executeQuery = (t, query, searchOptions) =>
  switch query {
  | Wildcard => executeWildcardQuery(t, searchOptions)
  | Combination({options: own, queries}) =>
    let options = spread(searchOptions, own)
    let results = queries->Array.map(subquery => executeQuery(t, subquery, options))
    combineResults(results, ~combineWith=?options.combineWith)
  | Text(query) =>
    let options = spread(t.options.searchOptions, searchOptions)
    let terms = tokenize(query)->Array.map(processTerm)->Array.filter(term => term != "")
    let queries = terms->Array.mapWithIndex((term, i) => termToQuerySpec(options, term, i, terms))
    let results = queries->Array.map(query => executeQuerySpec(t, query, options))
    combineResults(results, ~combineWith=?options.combineWith)
  }

let search = (t, query, ~searchOptions: searchOptions={}) => {
  let searchOptionsWithDefaults = spread(t.options.searchOptions, searchOptions)
  let rawResults = executeQuery(t, query, searchOptions)
  let results = []
  rawResults->Map.forEachWithKey(({score, terms, match}, docId) => {
    // The quality is the number of distinct matched query terms.
    let quality = terms->Array.length > 0 ? terms->Array.length : 1
    let result = {
      // Raw results only hold present documents.
      id: t.documentIds->Map.get(docId)->Option.getUnsafe,
      score: score *. Int.toFloat(quality),
      terms: match->Dict.keysToArray,
      queryTerms: terms,
      match,
      storedFields: t.storedFields->Map.get(docId),
    }
    switch searchOptionsWithDefaults.filter {
    | Some(filter) if !filter(result) => ()
    | _ => results->Array.push(result)
    }
  })
  // A wildcard query without boostDocument is not sorted: every score is 1.
  switch query {
  | Wildcard if searchOptionsWithDefaults.boostDocument->Option.isNone => ()
  | _ => results->Array.sort(byScore)
  }
  results
}

type suggestionGroup = {mutable score: float, terms: array<string>, mutable count: int}

let autoSuggest = (t, queryString, ~options: searchOptions={}) => {
  let options = spread(t.options.autoSuggestOptions, options)
  let suggestions: Map.t<string, suggestionGroup> = Map.make()
  search(t, Text(queryString), ~searchOptions=options)->Array.forEach(({score, terms}) => {
    let phrase = terms->Array.join(" ")
    switch suggestions->Map.get(phrase) {
    | Some(suggestion) =>
      suggestion.score = suggestion.score +. score
      suggestion.count = suggestion.count + 1
    | None => suggestions->Map.set(phrase, {score, terms, count: 1})
    }
  })
  let results = []
  suggestions->Map.forEachWithKey(({score, terms, count}, suggestion) =>
    results->Array.push({suggestion, terms, score: score /. Int.toFloat(count)})
  )
  results->Array.sort((a: suggestion, b: suggestion) => b.score -. a.score)
  results
}

// ---------------------------------------------------------------------------------------------
// Serialization.

let toJSON = (t): asPlainObject => {
  let index = []
  let iterator = t.index->SearchableMap.entries
  let rec loop = () =>
    switch iterator->SearchableMap.next {
    | None => ()
    | Some((term, fieldIndex)) =>
      let data = Dict.make()
      fieldIndex->Map.forEachWithKey((freqs, fieldId) =>
        data->Dict.set(Int.toString(fieldId), objectFromEntries(freqs))
      )
      index->Array.push((term, data))
      loop()
    }
  loop()
  {
    documentCount: t.documentCount,
    nextId: t.nextId,
    documentIds: objectFromEntries(t.documentIds),
    fieldIds: t.fieldIds,
    fieldLength: objectFromEntries(t.fieldLength),
    averageFieldLength: t.avgFieldLength,
    storedFields: objectFromEntries(t.storedFields),
    dirtCount: t.dirtCount,
    index,
    serializationVersion: 2,
  }
}
