// Host primitives for native JSON values shared with the component modules.
type value
external box: 'a => value = "%identity"
@new external obj: unit => value = "Object"
@new external array: unit => value = "Array"
@get_index external get: (value, string) => value = ""
@set_index external set: (value, string, value) => unit = ""
@get_index external at: (value, int) => value = ""
@get external length: value => int = "length"
@send external push: (value, value) => unit = "push"
@scope("JSON") @val external stringify: value => value = "stringify"
@scope("JSON") @val external parse: value => value = "parse"
@send external join: (value, string) => value = "join"
@send external filter: (value, value => bool) => value = "filter"

let object = fields => {
  let out = obj()
  Array.forEach(fields, ((k, v)) => set(out, k, v))
  out
}
let map = (values, f) => {
  let out = array()
  for i in 0 to length(values) - 1 {
    push(out, f(at(values, i)))
  }
  out
}
// A transaction step on the Explorer store.
let transaction = (mode, step) =>
  object([
    ("op", box("tx")),
    ("conn", box("explorer")),
    ("stores", box("rows")),
    ("mode", box(mode)),
    ("steps", box([step])),
    ("catch", box(false)),
  ])
// One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one readwrite
// transaction, then walk the store with a cursor in primary-key order.
let persist = rows => {
  let schema = [
    object([("op", box("createStore")), ("name", box("rows")), ("options", object([("keyPath", box("id"))]))]),
    object([("op", box("createIndex")), ("store", box("rows")), ("name", box("byGroup")), ("keyPath", box("group"))]),
  ]
  let openDb = object([
    ("op", box("openDB")),
    ("db", box("explorer")),
    ("conn", box("explorer")),
    ("version", box(1)),
    ("upgrade", box(schema)),
    ("catch", box(false)),
  ])
  let put = row =>
    object([
      ("op", box("put")),
      ("store", box(Nullable.null)),
      ("index", box(Nullable.null)),
      ("args", box([row])),
      ("catch", box(false)),
    ])
  let write = object([
    ("op", box("batch")),
    ("requests", map(rows, put)),
    ("abort", box(false)),
    ("done", box(true)),
  ])
  let walk = object([
    ("op", box("openCursor")),
    ("store", box(Nullable.null)),
    ("index", box(Nullable.null)),
    ("args", array()),
    ("positions", array()),
    ("rest", box([["continue"]])),
    ("catch", box(false)),
  ])
  object([("steps", box([openDb, transaction("readwrite", write), transaction("readonly", walk)]))])
}
// The stored rows' `value` texts, from the cursor entries of the idb log.
let stored = h =>
  get(at(h, 0), "log")
  ->filter(entry => at(entry, 0) === box("cursor"))
  ->map(entry => get(at(entry, 3), "value"))
let hits = h => get(at(get(at(h, 1), "results"), 1), "hits")
let cmd = (name, argument) => box([box(name), argument])
let run = packet => {
  let input = at(packet, 0)
  let h = at(packet, 1)
  switch length(h) {
  | 0 =>
    let rows = map(get(input, "records"), doc =>
      object([("id", get(doc, "id")), ("group", box("documents")), ("value", stringify(doc))])
    )
    cmd("idb", persist(rows))
  | 1 =>
    let options = object([("fields", box(["title", "body"])), ("storeFields", box(["body"]))])
    let add = object([("op", box("addAll")), ("documents", map(stored(h), parse))])
    let query = object([("op", box("search")), ("query", get(input, "query"))])
    cmd("minisearch", object([("options", options), ("operations", box([add, query]))]))
  | 2 =>
    let found = hits(h)
    let markdown = length(found) == 0 ? box("") : get(at(found, 0), "body")
    cmd("marked", object([("op", box("parse")), ("markdown", markdown)]))
  | 3 => cmd("fflate", object([("op", box("strToU8")), ("text", join(stored(h), "\n"))]))
  | 4 =>
    let opts = object([("level", box(6)), ("mtime", box(0))])
    cmd(
      "fflate",
      object([("op", box("gzipSync")), ("data", object([("bytes", at(h, 3))])), ("options", opts)]),
    )
  | 5 => cmd("uplot", get(input, "chart"))
  | _ =>
    let ids = map(hits(h), hit => get(hit, "id"))
    box([box(Nullable.null), box([ids, at(h, 2), at(h, 4), box(length(at(h, 3))), at(h, 5)])])
  }
}
