// The corpus input (CONTRACT "Input") as ReScript types. `decode` reads the corpus JSON once and
// rejects anything outside the contract's schema with a TypeError, before any database is opened.
// Names become ReScript strings; keys, values, versions, counts, directions, modes and options stay
// JSON values and reach IndexedDB as they are.

/** `store: None` is `tx.store`; `index: None` is the store itself. */
type target = {store: option<string>, index: option<string>}

/** A key range written as an object; its open flags stay JSON values, as IDBKeyRange converts
    them. An absent bound or flag is `None`. */
type range =
  | Only(JSON.t)
  | LowerBound(JSON.t, option<JSON.t>)
  | UpperBound(option<JSON.t>, option<JSON.t>)
  | Bound(JSON.t, JSON.t, option<JSON.t>, option<JSON.t>)

/** A positional argument: a query method's first argument is a key or a key range. */
type argument = Value(JSON.t) | Range(range)

type request = {method: Idb.method, target: target, args: array<argument>}

type action =
  | Continue(option<JSON.t>)
  | Advance(JSON.t)
  | ContinuePrimaryKey(JSON.t, JSON.t)
  | Update(JSON.t)
  | Delete
  | Break

type walkKind = Cursor | KeyCursor | Iterate

type walk = {
  kind: walkKind,
  target: target,
  args: array<argument>,
  positions: array<list<action>>,
  rest: list<action>,
}

type upgrade =
  | CreateStore({name: string, options: option<JSON.t>})
  | DeleteStore(string)
  | CreateIndex({store: string, name: string, keyPath: JSON.t, options: option<JSON.t>})
  | DeleteIndex({store: string, name: string})
  /** `put` or `add` in the upgrade transaction: issued, not awaited. */
  | Write({method: Idb.method, store: string, args: array<JSON.t>})
  | Abort

type blocking = LogBlocking | CloseSelf

/** `"log"`, or `{close: conn}`: log, then close that connection. */
type blocked = LogBlocked | CloseOther(string)

type openDB = {
  db: string,
  conn: string,
  version: option<JSON.t>,
  upgrade: option<array<upgrade>>,
  blocking: option<blocking>,
  blocked: option<blocked>,
}

type rec step = {op: op, catches: bool}
and op =
  | OpenDB(openDB)
  | Close(string)
  | DeleteDB({db: string, blocked: option<blocked>})
  | Transaction({conn: string, stores: JSON.t, mode: JSON.t, steps: array<step>})
  /** A database shortcut; `label` is its name, such as `getAllFromIndex`. */
  | Shortcut({
      conn: string,
      label: string,
      method: Idb.method,
      store: string,
      index: option<string>,
      args: array<argument>,
    })
  | Request(request)
  | Batch({requests: array<request>, abort: bool, done: bool})
  | Walk(walk)
  | AbortTransaction
  | Done
  | Store
  /** `wrap(unwrap(source)[method](...args))`. */
  | Wrap(request)

let invalid = what => JsError.TypeError.throwWithMessage("outside the idb contract: " ++ what)

let record = (json: JSON.t) =>
  switch json {
  | JSON.Object(fields) => fields
  | _ => invalid("not an object")
  }

let field = (fields: dict<JSON.t>, name): JSON.t =>
  switch fields->Dict.get(name) {
  | Some(value) => value
  | None => invalid("no " ++ name)
  }

let text = (fields, name) =>
  switch fields->field(name) {
  | String(text) => text
  | _ => invalid(name ++ " is not a string")
  }

let flag = (fields, name) =>
  switch fields->field(name) {
  | Boolean(flag) => flag
  | _ => invalid(name ++ " is not a boolean")
  }

let items = (json, name) =>
  switch json {
  | JSON.Array(items) => items
  | _ => invalid(name ++ " is not an array")
  }

let list = (fields, name) => fields->field(name)->items(name)

/** A name, or `null`. */
let nameOrNull = (fields, name) =>
  switch fields->field(name) {
  | String(text) => Some(text)
  | Null => None
  | _ => invalid(name ++ " is neither a name nor null")
  }

let method = name =>
  switch name {
  | "get" => Idb.Get
  | "getKey" => GetKey
  | "getAll" => GetAll
  | "getAllKeys" => GetAllKeys
  | "count" => Count
  | "put" => Put
  | "add" => Add
  | "delete" => Delete
  | "clear" => Clear
  | _ => invalid("not a request method: " ++ name)
  }

/** A key is passed as it is, `null` included; an object is a key range, its fields tested with
    `in` (the B9 decision: `{only: null}` is `only(null)`). */
let query = (key: JSON.t) =>
  switch key {
  | Object(range) =>
    let open_ = name => range->Dict.get(name)
    Range(
      switch (range->Dict.get("only"), range->Dict.get("lower"), range->Dict.get("upper")) {
      | (Some(only), _, _) => Only(only)
      | (None, Some(lower), Some(upper)) => Bound(lower, upper, open_("lowerOpen"), open_("upperOpen"))
      | (None, Some(lower), None) => LowerBound(lower, open_("lowerOpen"))
      | (None, None, upper) => UpperBound(upper, open_("upperOpen"))
      },
    )
  | _ => Value(key)
  }

/** The first argument of a query method is a query; `put`, `add` and `clear` take plain arguments. */
let queryArgs = args => args->Array.mapWithIndex((arg, i) => i == 0 ? query(arg) : Value(arg))
let args = (method: Idb.method, args) =>
  switch method {
  | Put | Add | Clear => args->Array.map(arg => Value(arg))
  | Get | GetKey | GetAll | GetAllKeys | Count | Delete => queryArgs(args)
  }

let target = fields => {store: fields->nameOrNull("store"), index: fields->nameOrNull("index")}

let request = (fields, method) => {method, target: target(fields), args: args(method, fields->list("args"))}

let action = (json: JSON.t) =>
  switch json {
  | JSON.Array([String("continue")]) => Continue(None)
  | Array([String("continue"), key]) => Continue(Some(key))
  | Array([String("advance"), count]) => Advance(count)
  | Array([String("continuePrimaryKey"), key, primaryKey]) => ContinuePrimaryKey(key, primaryKey)
  | Array([String("update"), value]) => Update(value)
  | Array([String("delete")]) => Delete
  | Array([String("break")]) => Break
  | _ => invalid("not a cursor action")
  }

let actions = json => json->items("a position")->List.fromArray->List.map(action)

let walk = (fields, kind) => {
  kind,
  target: target(fields),
  args: queryArgs(fields->list("args")),
  positions: fields->list("positions")->Array.map(actions),
  rest: actions(fields->field("rest")),
}

let blocked = (fields: dict<JSON.t>) =>
  fields
  ->Dict.get("blocked")
  ->Option.map(blocked =>
    switch blocked {
    | JSON.String("log") => LogBlocked
    | Object(close) => CloseOther(close->text("close"))
    | _ => invalid("blocked")
    }
  )

let upgrade = (json: JSON.t) => {
  let fields = record(json)
  switch fields->text("op") {
  | "createStore" => CreateStore({name: fields->text("name"), options: fields->Dict.get("options")})
  | "deleteStore" => DeleteStore(fields->text("name"))
  | "createIndex" =>
    CreateIndex({
      store: fields->text("store"),
      name: fields->text("name"),
      keyPath: fields->field("keyPath"),
      options: fields->Dict.get("options"),
    })
  | "deleteIndex" => DeleteIndex({store: fields->text("store"), name: fields->text("name")})
  | "put" => Write({method: Put, store: fields->text("store"), args: fields->list("args")})
  | "add" => Write({method: Add, store: fields->text("store"), args: fields->list("args")})
  | "abort" => Abort
  | op => invalid("not an upgrade step: " ++ op)
  }
}

let rec step = (json: JSON.t) => {
  let fields = record(json)
  let conn = () => fields->text("conn")
  let op = switch fields->text("op") {
  | "openDB" =>
    OpenDB({
      db: fields->text("db"),
      conn: conn(),
      version: fields->Dict.get("version"),
      upgrade: fields->Dict.get("upgrade")->Option.map(ops => ops->items("upgrade")->Array.map(upgrade)),
      blocking: fields
      ->Dict.get("blocking")
      ->Option.map(blocking =>
        switch blocking {
        | JSON.String("log") => LogBlocking
        | String("close") => CloseSelf
        | _ => invalid("blocking")
        }
      ),
      blocked: blocked(fields),
    })
  | "close" => Close(conn())
  | "deleteDB" => DeleteDB({db: fields->text("db"), blocked: blocked(fields)})
  | "tx" =>
    Transaction({
      conn: conn(),
      stores: fields->field("stores"),
      mode: fields->field("mode"),
      steps: fields->list("steps")->Array.map(step),
    })
  | "call" =>
    // database-extras' getMethod: `prop.replace(/FromIndex$/, '')`; only read methods have an index form.
    let label = fields->text("method")
    let index = label->String.endsWith("FromIndex")
    let method = method(index ? label->String.slice(~start=0, ~end=-9) : label)
    if index && Idb.isWrite(method) {
      invalid("not a database shortcut: " ++ label)
    }
    Shortcut({
      conn: conn(),
      label,
      method,
      store: fields->text("store"),
      index: fields->nameOrNull("index"),
      args: args(method, fields->list("args")),
    })
  | "batch" =>
    Batch({
      requests: fields
      ->list("requests")
      ->Array.map(json => {
        let fields = record(json)
        request(fields, method(fields->text("op")))
      }),
      abort: fields->flag("abort"),
      done: fields->flag("done"),
    })
  | "openCursor" => Walk(walk(fields, Cursor))
  | "openKeyCursor" => Walk(walk(fields, KeyCursor))
  | "iterate" => Walk(walk(fields, Iterate))
  | "abort" => AbortTransaction
  | "done" => Done
  | "store" => Store
  | "wrap" => Wrap(request(fields, method(fields->text("method"))))
  | op => Request(request(fields, method(op)))
  }
  let catches = switch fields->Dict.get("catch") {
  | None => false
  | Some(Boolean(catches)) => catches
  | Some(_) => invalid("catch is not a boolean")
  }
  {op, catches}
}

let decode = json => record(json)->list("steps")->Array.map(step)
