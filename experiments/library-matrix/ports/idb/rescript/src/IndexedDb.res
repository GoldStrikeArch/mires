// The browser's IndexedDB, as idb drives it: IDBFactory, IDBRequest, IDBDatabase, IDBTransaction,
// IDBObjectStore, IDBIndex, IDBCursor, IDBKeyRange and DOMStringList, with their events. A method
// that can throw throws its JS exception (a DOMException or a TypeError), which callers catch as
// `JsExn`. Keys and values are JSON values (CONTRACT "Parity domain") and reach IndexedDB as they
// are, so `null` stays a value and numbers are never narrowed.

type database
type transaction
type objectStore
type index
type cursor
type keyRange
type request<'result>

/** An IDBObjectStore or an IDBIndex: they share their read methods and cursors. */
type source

/** A method argument: a JSON value or a key range, as IDL `any` accepts both. */
type argument

external value: JSON.t => argument = "%identity"
external range: keyRange => argument = "%identity"

/** A list of names: `objectStoreNames` and `indexNames`. */
type domStringList = Array.arrayLike<string>

/** `blocked`, `upgradeneeded` and `versionchange`. A deletion has no new version. */
type versionChangeEvent<'target> = {oldVersion: float, newVersion: Null.t<float>, target: 'target}

module DOMException = {
  @new external make: (string, string) => JsExn.t = "DOMException"
  @get external name: JsExn.t => string = "name"
}

module Factory = {
  type t
  @val external indexedDB: t = "indexedDB"
  @send external openDatabase: (t, string, option<JSON.t>) => request<database> = "open"
  @send external deleteDatabase: (t, string) => request<unit> = "deleteDatabase"
}

module KeyRange = {
  @scope("IDBKeyRange") @val external only: JSON.t => keyRange = "only"
  @scope("IDBKeyRange") @val
  external lowerBound: (JSON.t, option<JSON.t>) => keyRange = "lowerBound"
  @scope("IDBKeyRange") @val
  external upperBound: (option<JSON.t>, option<JSON.t>) => keyRange = "upperBound"
  @scope("IDBKeyRange") @val
  external bound: (JSON.t, JSON.t, option<JSON.t>, option<JSON.t>) => keyRange = "bound"
}

module Request = {
  @get external result: request<'result> => 'result = "result"
  @get external error: request<'result> => JsExn.t = "error"
  @send
  external addEventListener: (request<'result>, [#success | #error], unit => unit) => unit =
    "addEventListener"
  @send
  external removeEventListener: (request<'result>, [#success | #error], unit => unit) => unit =
    "removeEventListener"
}

module OpenRequest = {
  type t = request<database>
  /** The versionchange transaction, while `upgradeneeded` is dispatched. */
  @get external transaction: t => transaction = "transaction"
  @send
  external onUpgradeNeeded: (t, @as("upgradeneeded") _, versionChangeEvent<t> => unit) => unit =
    "addEventListener"
  @send
  external onBlocked: (t, @as("blocked") _, versionChangeEvent<t> => unit) => unit =
    "addEventListener"
}

module DeleteRequest = {
  @send
  external onBlocked: (request<unit>, @as("blocked") _, versionChangeEvent<request<unit>> => unit) => unit =
    "addEventListener"
}

module Database = {
  @get external version: database => float = "version"
  @get external objectStoreNames: database => domStringList = "objectStoreNames"
  @send external createObjectStore: (database, string, option<JSON.t>) => objectStore = "createObjectStore"
  @send external deleteObjectStore: (database, string) => unit = "deleteObjectStore"
  /** `stores` is a name or an array of names; an absent `mode` is readonly. */
  @send external transaction: (database, JSON.t, option<JSON.t>) => transaction = "transaction"
  @send external close: database => unit = "close"
  @send
  external onVersionChange: (database, @as("versionchange") _, versionChangeEvent<database> => unit) => unit =
    "addEventListener"
}

module Transaction = {
  @get external objectStoreNames: transaction => domStringList = "objectStoreNames"
  @get external error: transaction => Null.t<JsExn.t> = "error"
  @send external objectStore: (transaction, string) => objectStore = "objectStore"
  @send external abort: transaction => unit = "abort"
  @send
  external addEventListener: (transaction, [#complete | #error | #abort], unit => unit) => unit =
    "addEventListener"
  @send
  external removeEventListener: (transaction, [#complete | #error | #abort], unit => unit) => unit =
    "removeEventListener"
}

module ObjectStore = {
  external asSource: objectStore => source = "%identity"
  @get external name: objectStore => string = "name"
  @get external keyPath: objectStore => JSON.t = "keyPath"
  @get external autoIncrement: objectStore => bool = "autoIncrement"
  @get external indexNames: objectStore => domStringList = "indexNames"
  @send external index: (objectStore, string) => index = "index"
  @send external createIndex: (objectStore, string, JSON.t, option<JSON.t>) => index = "createIndex"
  @send external deleteIndex: (objectStore, string) => unit = "deleteIndex"
  @send external getAllKeys: objectStore => request<array<JSON.t>> = "getAllKeys"
  @send external getAll: objectStore => request<array<JSON.t>> = "getAll"
}

module StoreIndex = {
  external asSource: index => source = "%identity"
  @get external name: index => string = "name"
  @get external keyPath: index => JSON.t = "keyPath"
  @get external unique: index => bool = "unique"
  @get external multiEntry: index => bool = "multiEntry"
}

/** The request methods, called with the script's arguments. A result is any JSON value, `None`
    when it is `undefined`. The write methods exist on stores only. */
module Source = {
  @send @variadic external get: (source, array<argument>) => request<option<JSON.t>> = "get"
  @send @variadic external getKey: (source, array<argument>) => request<option<JSON.t>> = "getKey"
  @send @variadic external getAll: (source, array<argument>) => request<option<JSON.t>> = "getAll"
  @send @variadic
  external getAllKeys: (source, array<argument>) => request<option<JSON.t>> = "getAllKeys"
  @send @variadic external count: (source, array<argument>) => request<option<JSON.t>> = "count"
  @send @variadic external put: (source, array<argument>) => request<option<JSON.t>> = "put"
  @send @variadic external add: (source, array<argument>) => request<option<JSON.t>> = "add"
  @send @variadic external delete: (source, array<argument>) => request<option<JSON.t>> = "delete"
  @send @variadic external clear: (source, array<argument>) => request<option<JSON.t>> = "clear"
  @send @variadic
  external openCursor: (source, array<argument>) => request<Null.t<cursor>> = "openCursor"
  @send @variadic
  external openKeyCursor: (source, array<argument>) => request<Null.t<cursor>> = "openKeyCursor"
}

module Cursor = {
  @get external key: cursor => JSON.t = "key"
  @get external primaryKey: cursor => JSON.t = "primaryKey"
  @get external value: cursor => JSON.t = "value"
  /** The request that opened the cursor: it fires once more after each move. */
  @get external request: cursor => request<Null.t<cursor>> = "request"
  @send external advance: (cursor, JSON.t) => unit = "advance"
  /** `continue()` when the key is `None`. */
  @send external continue: (cursor, option<JSON.t>) => unit = "continue"
  @send external continuePrimaryKey: (cursor, JSON.t, JSON.t) => unit = "continuePrimaryKey"
  @send external update: (cursor, JSON.t) => request<option<JSON.t>> = "update"
  @send external delete: cursor => request<option<JSON.t>> = "delete"
}
