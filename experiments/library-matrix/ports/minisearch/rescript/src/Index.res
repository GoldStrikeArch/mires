// Entry module of the ReScript port: what libraries/minisearch/reference.mjs does, on the port's
// `MiniSearch`. The boundary: the corpus input is typed as records, tagged and unboxed variants
// whose runtime representation is the JSON value itself, so reading it converts nothing; as the
// reference adapter does, the declarative `filter`, `boostDocument` and `boostTerm` become the
// callbacks they describe, `{ "wildcard": true }` becomes `Wildcard`, and the result is built as a
// JSON value (`JSON.t`, plain objects and arrays) in the key order CONTRACT.md's "Output" gives.

// The input types describe the corpus JSON: their constructors are matched, never built here.
@@warning("-37")

type filter = {field: string, @as("in") values: array<JSON.t>}

type searchOptions = {
  fields?: array<string>,
  boost?: dict<float>,
  prefix?: bool,
  fuzzy?: MiniSearch.fuzzy,
  maxFuzzy?: float,
  combineWith?: string,
  weights?: MiniSearch.weights,
  bm25?: MiniSearch.bm25Params,
  filter?: filter,
  boostDocument?: array<(MiniSearch.id, float)>,
  boostTerm?: array<(string, float)>,
}

@unboxed type rec query = Text(string) | Object(queryObject)
and queryObject = {...searchOptions, wildcard?: bool, queries?: array<query>}

@tag("op")
type operation =
  | @as("add") Add({document: MiniSearch.document})
  | @as("addAll") AddAll({documents: array<MiniSearch.document>})
  | @as("remove") Remove({document: MiniSearch.document})
  | @as("removeAll") RemoveAll({documents?: Null.t<array<MiniSearch.document>>})
  | @as("discard") Discard({id: MiniSearch.id})
  | @as("discardAll") DiscardAll({ids: array<MiniSearch.id>})
  | @as("replace") Replace({document: MiniSearch.document})
  | @as("vacuum") Vacuum({options?: MiniSearch.vacuumOptions})
  | @as("search") Search({query: query, options?: searchOptions})
  | @as("autoSuggest") AutoSuggest({query: string, options?: searchOptions})
  | @as("has") Has({id: MiniSearch.id})
  | @as("getStoredFields") GetStoredFields({id: MiniSearch.id})
  | @as("stats") Stats({})
  | @as("toJSON") ToJSON({})

type options = {
  fields: array<string>,
  storeFields?: array<string>,
  idField?: string,
  searchOptions?: searchOptions,
  autoSuggestOptions?: searchOptions,
  autoVacuum?: MiniSearch.autoVacuum,
}

type input = {options: options, operations: array<operation>}

let name = operation =>
  switch operation {
  | Add(_) => "add"
  | AddAll(_) => "addAll"
  | Remove(_) => "remove"
  | RemoveAll(_) => "removeAll"
  | Discard(_) => "discard"
  | DiscardAll(_) => "discardAll"
  | Replace(_) => "replace"
  | Vacuum(_) => "vacuum"
  | Search(_) => "search"
  | AutoSuggest(_) => "autoSuggest"
  | Has(_) => "has"
  | GetStoredFields(_) => "getStoredFields"
  | Stats(_) => "stats"
  | ToJSON(_) => "toJSON"
  }

let idJson = (id: MiniSearch.id) =>
  switch id {
  | String(text) => JSON.Encode.string(text)
  | Number(number) => JSON.Encode.float(number)
  }

// The factor of the first pair whose key is `key` (`===`), else 1.
let factor = (pairs: array<('key, float)>, key: 'key) =>
  switch pairs->Array.find(((candidate, _)) => candidate === key) {
  | Some((_, factor)) => factor
  | None => 1.
  }

// `result[field]` on MiniSearch's result object, which `Object.assign` gave the stored fields: a
// stored field, else the id (a filter names a stored field or `id`, CONTRACT.md "Parity domain").
let resultField = (result: MiniSearch.searchResult, field) =>
  switch result.storedFields->Option.flatMap(stored => stored->MiniSearch.getOwnProperty(field)) {
  | Some(value) => Some(value)
  | None => field == "id" ? Some(idJson(result.id)) : None
  }

let callbacks = (json: searchOptions): MiniSearch.searchOptions => {
  fields: ?json.fields,
  // (result) => values.includes(result[field])
  filter: ?(
    json.filter->Option.map(({field, values}) =>
      result =>
        switch resultField(result, field) {
        | Some(value) => values->Array.includes(value)
        | None => false
        }
    )
  ),
  boost: ?json.boost,
  boostTerm: ?(json.boostTerm->Option.map(pairs => (term, _i, _terms) => factor(pairs, term))),
  weights: ?json.weights,
  boostDocument: ?(
    json.boostDocument->Option.map(pairs => (id, _term, _storedFields) => factor(pairs, id))
  ),
  prefix: ?(json.prefix->Option.map(prefix => MiniSearch.Prefix(prefix))),
  fuzzy: ?json.fuzzy,
  maxFuzzy: ?json.maxFuzzy,
  combineWith: ?json.combineWith,
  bm25: ?json.bm25,
}

let rec query = (json: query): MiniSearch.query =>
  switch json {
  | Text(text) => Text(text)
  | Object({wildcard: true}) => Wildcard
  | Object(node) =>
    Combination({
      options: callbacks((node :> searchOptions)),
      queries: node.queries->Option.getOrThrow->Array.map(query),
    })
  }

// A hit: MiniSearch's result object with `match` as `[term, fields]` pairs, and the stored fields
// after it as `Object.assign` adds them (a stored `id` replaces the id in place).
let hit = (result: MiniSearch.searchResult) => {
  let match =
    result.match
    ->Dict.keysToArray
    ->Array.map(term =>
      JSON.Encode.array([
        JSON.Encode.string(term),
        JSON.Encode.stringArray(result.match->Dict.getUnsafe(term)),
      ])
    )
  let out = dict{
    "id": idJson(result.id),
    "score": JSON.Encode.float(result.score),
    "terms": JSON.Encode.stringArray(result.terms),
    "queryTerms": JSON.Encode.stringArray(result.queryTerms),
    "match": JSON.Encode.array(match),
  }
  result.storedFields->Option.forEach(stored => out->Dict.assign(stored)->ignore)
  JSON.Encode.object(out)
}

let suggestion = ({suggestion, terms, score}: MiniSearch.suggestion) =>
  JSON.Encode.object(
    dict{
      "suggestion": JSON.Encode.string(suggestion),
      "terms": JSON.Encode.stringArray(terms),
      "score": JSON.Encode.float(score),
    },
  )

// Runs one operation, setting its payload on `record`; MiniSearch's errors propagate.
let execute = (index, operation, record) =>
  switch operation {
  | Add({document}) => index->MiniSearch.add(document)
  | AddAll({documents}) => index->MiniSearch.addAll(documents)
  | Remove({document}) => index->MiniSearch.remove(document)
  | RemoveAll({?documents}) => index->MiniSearch.removeAll(~documents?)
  | Discard({id}) => index->MiniSearch.discard(id)
  | DiscardAll({ids}) => index->MiniSearch.discardAll(ids)
  | Replace({document}) => index->MiniSearch.replace(document)
  | Vacuum({?options}) => index->MiniSearch.vacuum(~options?)
  | Search({query: json, ?options}) =>
    let hits = index->MiniSearch.search(query(json), ~searchOptions=?options->Option.map(callbacks))
    record->Dict.set("hits", JSON.Encode.array(hits->Array.map(hit)))
  | AutoSuggest({query, ?options}) =>
    let suggestions = index->MiniSearch.autoSuggest(query, ~options=?options->Option.map(callbacks))
    record->Dict.set("suggestions", JSON.Encode.array(suggestions->Array.map(suggestion)))
  | Has({id}) => record->Dict.set("value", JSON.Encode.bool(index->MiniSearch.has(id)))
  | GetStoredFields({id}) =>
    let value = switch index->MiniSearch.getStoredFields(id) {
    | Some(stored) => JSON.Encode.object(Dict.copy(stored))
    | None => JSON.Encode.null
    }
    record->Dict.set("value", value)
  | Stats(_) =>
    record->Dict.set("documentCount", JSON.Encode.int(index->MiniSearch.documentCount))
    record->Dict.set("termCount", JSON.Encode.int(index->MiniSearch.termCount))
    record->Dict.set("dirtCount", JSON.Encode.int(index->MiniSearch.dirtCount))
    record->Dict.set("dirtFactor", JSON.Encode.float(index->MiniSearch.dirtFactor))
    record->Dict.set("isVacuuming", JSON.Encode.bool(index->MiniSearch.isVacuuming))
  | ToJSON(_) =>
    // JSON.parse(JSON.stringify(index)): holes and non-finite numbers become null.
    let text = index->MiniSearch.toJSON->JSON.stringifyAny->Option.getOrThrow
    record->Dict.set("value", JSON.parseOrThrow(text))
  }

let run = ({options: json, operations}: input) => {
  let warnings = ref([])
  let options: MiniSearch.options = {
    fields: json.fields,
    idField: ?json.idField,
    storeFields: ?json.storeFields,
    logger: (_level, message, _code) => warnings.contents->Array.push(message),
    autoVacuum: ?json.autoVacuum,
    searchOptions: ?(json.searchOptions->Option.map(callbacks)),
    autoSuggestOptions: ?(json.autoSuggestOptions->Option.map(callbacks)),
  }
  let index = MiniSearch.make(options)
  let results = operations->Array.map(operation => {
    let record = dict{"op": JSON.Encode.string(name(operation))}
    warnings := []
    let error = switch execute(index, operation, record) {
    | () => None
    | exception MiniSearch.MiniSearchError(message) => Some(message)
    }
    if warnings.contents->Array.length > 0 {
      record->Dict.set("warnings", JSON.Encode.stringArray(warnings.contents))
    }
    error->Option.forEach(message => record->Dict.set("error", JSON.Encode.string(message)))
    JSON.Encode.object(record)
  })
  JSON.Encode.object(dict{"results": JSON.Encode.array(results)})
}
