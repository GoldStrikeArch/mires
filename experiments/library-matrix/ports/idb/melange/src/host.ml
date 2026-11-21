(* The browser APIs idb drives, for Melange: externals onto the JS objects themselves. OCaml
   strings are JS strings and floats are JS numbers here, so both cross as they are. A host method
   that can throw is called through [catching]. See host.mli. *)

(* --- Values --- *)

type value

external of_null : 'a Js.null -> value = "%identity"
external string : string -> value = "%identity"
external number : float -> value = "%identity"
external bool : bool -> value = "%identity"
external of_array : value array -> value = "%identity"
external of_dict : value Js.Dict.t -> value = "%identity"

let null = of_null Js.null
let array values = of_array (Array.of_list values)
let record fields = of_dict (Js.Dict.fromList fields)

type json = Null | Bool of bool | Number of float | String of string | Array of value list | Object

external is_array : value -> bool = "isArray" [@@mel.scope "Array"]
external elements : value -> value array = "from" [@@mel.scope "Array"]

let classify value =
  match Js.Types.classify value with
  | JSNull -> Null
  | JSFalse -> Bool false
  | JSTrue -> Bool true
  | JSNumber number -> Number number
  | JSString text -> String text
  | JSObject _ -> if is_array value then Array (Array.to_list (elements value)) else Object
  | JSUndefined | JSFunction _ | JSSymbol _ | JSBigInt _ -> Js.Exn.raiseTypeError "not a JSON value"

external get : value -> string -> value Js.undefined = "" [@@mel.get_index]

let member value name = Js.Undefined.toOption (get value name)

external random_uuid : unit -> string = "randomUUID" [@@mel.scope "crypto"]

(* --- Errors --- *)

type error

external of_exn : Js.Exn.t -> error = "%identity"
external of_promise_error : Js.Promise.error -> error = "%identity"
external error_name : error -> string = "name" [@@mel.get]
external dom_exception : string -> string -> error = "DOMException" [@@mel.new]

let abort_error () = dom_exception "AbortError" "AbortError"
let type_error message = Js.Exn.raiseTypeError message

(* A host call that can throw: its JS exception, as the value thrown. *)
let catching call = try Ok (call ()) with Js.Exn.Error thrown -> Error (of_exn thrown)

(* --- Promises --- *)

type 'a promise = 'a Js.Promise.t

external resolve : 'a -> 'a promise = "resolve" [@@mel.scope "Promise"]
external reject : error -> 'a promise = "reject" [@@mel.scope "Promise"]

external make : ((('a -> unit)[@u]) -> ((error -> unit)[@u]) -> unit[@mel.uncurry]) -> 'a promise = "Promise"
[@@mel.new]

external both_ : 'a promise * 'b promise -> ('a * 'b) promise = "all" [@@mel.scope "Promise"]
external export : value promise -> value = "%identity"

let bind promise next = Js.Promise.then_ next promise
let map promise f = bind promise (fun value -> resolve (f value))
let promise executor =
  make (fun resolve reject -> executor (fun value -> resolve value [@u]) (fun error -> reject error [@u]))
let both a b = both_ (a, b)

let settle promise =
  promise
  |> Js.Promise.then_ (fun value -> resolve (Ok value))
  |> Js.Promise.catch (fun error -> resolve (Error (of_promise_error error)))

let ignore_rejection promise = ignore (settle promise)

(* --- IndexedDB objects --- *)

type database
type transaction
type object_store
type index
type cursor
type source

external store_source : object_store -> source = "%identity"
external index_source : index -> source = "%identity"

(* A request, and how its [result] is read. *)
type raw_request
type 'a request = { raw : raw_request; read : raw_request -> 'a }

let result request = request.read request.raw

external request_error : raw_request -> error = "error" [@@mel.get]

let request_error request = request_error request.raw

external any_result : raw_request -> value Js.undefined = "result" [@@mel.get]
external cursor_result : raw_request -> cursor Js.null = "result" [@@mel.get]
external database_result : raw_request -> database = "result" [@@mel.get]
external array_result : raw_request -> value array = "result" [@@mel.get]

let values raw = { raw; read = (fun raw -> Js.Undefined.toOption (any_result raw)) }
let cursors raw = { raw; read = (fun raw -> Js.Null.toOption (cursor_result raw)) }
let lists raw = { raw; read = (fun raw -> Array.to_list (array_result raw)) }

type event

external old_version : event -> float = "oldVersion" [@@mel.get]
external new_version : event -> float Js.null = "newVersion" [@@mel.get]
external event_database : event -> database = "target" [@@mel.get]

let new_version event = Js.Null.toOption (new_version event)

(* --- Listeners --- *)

type listener = (unit -> unit[@u])

let listener f : listener = fun [@u] () -> f ()

external add_listener : raw_request -> string -> listener -> unit = "addEventListener" [@@mel.send]
external remove_listener : raw_request -> string -> listener -> unit = "removeEventListener" [@@mel.send]
external add_tx_listener : transaction -> string -> listener -> unit = "addEventListener" [@@mel.send]
external remove_tx_listener : transaction -> string -> listener -> unit = "removeEventListener" [@@mel.send]

let request_event = function `Success -> "success" | `Error -> "error"
let transaction_event = function `Complete -> "complete" | `Error -> "error" | `Abort -> "abort"
let add_request_listener request kind listener = add_listener request.raw (request_event kind) listener
let remove_request_listener request kind listener = remove_listener request.raw (request_event kind) listener
let add_transaction_listener tx kind listener = add_tx_listener tx (transaction_event kind) listener
let remove_transaction_listener tx kind listener = remove_tx_listener tx (transaction_event kind) listener

external on_request_event : raw_request -> string -> ((event -> unit)[@mel.uncurry]) -> unit = "addEventListener"
[@@mel.send]

external on_database_event : database -> string -> ((event -> unit)[@mel.uncurry]) -> unit = "addEventListener"
[@@mel.send]

let on_upgrade_needed request handle = on_request_event request.raw "upgradeneeded" handle
let on_blocked request handle = on_request_event request.raw "blocked" handle
let on_version_change db handle = on_database_event db "versionchange" handle

(* --- IDBFactory --- *)

type factory

external indexed_db : factory = "indexedDB"
external open_ : factory -> string -> value Js.undefined -> raw_request = "open" [@@mel.send]
external delete_database : factory -> string -> raw_request = "deleteDatabase" [@@mel.send]
external upgrade_transaction : raw_request -> transaction = "transaction" [@@mel.get]

let open_database name version =
  catching (fun () ->
      { raw = open_ indexed_db name (Js.Undefined.fromOption version); read = database_result })

let delete_database name = catching (fun () -> { raw = delete_database indexed_db name; read = ignore })
let upgrade_transaction request = upgrade_transaction request.raw

(* --- IDBKeyRange --- *)

external only : value -> value = "only" [@@mel.scope "IDBKeyRange"]
external lower_bound : value -> value Js.undefined -> value = "lowerBound" [@@mel.scope "IDBKeyRange"]
external upper_bound : value Js.undefined -> value Js.undefined -> value = "upperBound" [@@mel.scope "IDBKeyRange"]

external bound : value -> value -> value Js.undefined -> value Js.undefined -> value = "bound"
[@@mel.scope "IDBKeyRange"]

let only key = catching (fun () -> only key)
let lower_bound lower open_ = catching (fun () -> lower_bound lower (Js.Undefined.fromOption open_))

let upper_bound upper open_ =
  catching (fun () -> upper_bound (Js.Undefined.fromOption upper) (Js.Undefined.fromOption open_))

let bound lower upper lower_open upper_open =
  catching (fun () -> bound lower upper (Js.Undefined.fromOption lower_open) (Js.Undefined.fromOption upper_open))

(* --- IDBDatabase, DOMStringList --- *)

type string_list

external names : string_list -> string array = "from" [@@mel.scope "Array"]
external version : database -> float = "version" [@@mel.get]
external object_store_names : database -> string_list = "objectStoreNames" [@@mel.get]

external create_object_store : database -> string -> value Js.undefined -> object_store = "createObjectStore"
[@@mel.send]

external delete_object_store : database -> string -> unit = "deleteObjectStore" [@@mel.send]
external transaction : database -> value -> value Js.undefined -> transaction = "transaction" [@@mel.send]
external close : database -> unit = "close" [@@mel.send]

let object_store_names db = Array.to_list (names (object_store_names db))

let create_object_store db name options =
  catching (fun () -> create_object_store db name (Js.Undefined.fromOption options))

let delete_object_store db name = catching (fun () -> delete_object_store db name)
let transaction db stores mode = catching (fun () -> transaction db stores (Js.Undefined.fromOption mode))

(* --- IDBTransaction --- *)

external scope : transaction -> string_list = "objectStoreNames" [@@mel.get]
external object_store : transaction -> string -> object_store = "objectStore" [@@mel.send]
external abort : transaction -> unit = "abort" [@@mel.send]
external transaction_error : transaction -> error Js.null = "error" [@@mel.get]

let scope tx = Array.to_list (names (scope tx))
let object_store tx name = catching (fun () -> object_store tx name)
let abort tx = catching (fun () -> abort tx)
let transaction_error tx = Js.Null.toOption (transaction_error tx)

(* --- IDBObjectStore and IDBIndex --- *)

external store_name : object_store -> string = "name" [@@mel.get]
external store_key_path : object_store -> value = "keyPath" [@@mel.get]
external auto_increment : object_store -> bool = "autoIncrement" [@@mel.get]
external index_names : object_store -> string_list = "indexNames" [@@mel.get]
external index : object_store -> string -> index = "index" [@@mel.send]

external create_index : object_store -> string -> value -> value Js.undefined -> index = "createIndex"
[@@mel.send]

external delete_index : object_store -> string -> unit = "deleteIndex" [@@mel.send]
external index_name : index -> string = "name" [@@mel.get]
external index_key_path : index -> value = "keyPath" [@@mel.get]
external unique : index -> bool = "unique" [@@mel.get]
external multi_entry : index -> bool = "multiEntry" [@@mel.get]

let index_names store = Array.to_list (names (index_names store))
let index store name = catching (fun () -> index store name)

let create_index store name key_path options =
  catching (fun () -> create_index store name key_path (Js.Undefined.fromOption options))

let delete_index store name = catching (fun () -> delete_index store name)

external get : source -> value array -> raw_request = "get" [@@mel.send] [@@mel.variadic]
external get_key : source -> value array -> raw_request = "getKey" [@@mel.send] [@@mel.variadic]
external get_all : source -> value array -> raw_request = "getAll" [@@mel.send] [@@mel.variadic]
external get_all_keys : source -> value array -> raw_request = "getAllKeys" [@@mel.send] [@@mel.variadic]
external count : source -> value array -> raw_request = "count" [@@mel.send] [@@mel.variadic]
external put : source -> value array -> raw_request = "put" [@@mel.send] [@@mel.variadic]
external add : source -> value array -> raw_request = "add" [@@mel.send] [@@mel.variadic]
external delete : source -> value array -> raw_request = "delete" [@@mel.send] [@@mel.variadic]
external clear : source -> value array -> raw_request = "clear" [@@mel.send] [@@mel.variadic]
external open_cursor : source -> value array -> raw_request = "openCursor" [@@mel.send] [@@mel.variadic]
external open_key_cursor : source -> value array -> raw_request = "openKeyCursor" [@@mel.send] [@@mel.variadic]
external store_get_all_keys : object_store -> raw_request = "getAllKeys" [@@mel.send]
external store_get_all : object_store -> raw_request = "getAll" [@@mel.send]

(* A request method called with the script's arguments, exactly as many as it gives. *)
let issue call read source args = catching (fun () -> read (call source (Array.of_list args)))
let get = issue get values
let get_key = issue get_key values
let get_all = issue get_all values
let get_all_keys = issue get_all_keys values
let count = issue count values
let put = issue put values
let add = issue add values
let delete = issue delete values
let clear = issue clear values
let open_cursor = issue open_cursor cursors
let open_key_cursor = issue open_key_cursor cursors
let all_keys store = catching (fun () -> lists (store_get_all_keys store))
let all_values store = catching (fun () -> lists (store_get_all store))

(* --- IDBCursor --- *)

external key : cursor -> value = "key" [@@mel.get]
external primary_key : cursor -> value = "primaryKey" [@@mel.get]
external cursor_value : cursor -> value = "value" [@@mel.get]
external cursor_request : cursor -> raw_request = "request" [@@mel.get]
external advance : cursor -> value -> unit = "advance" [@@mel.send]
external continue : cursor -> value Js.undefined -> unit = "continue" [@@mel.send]
external continue_primary_key : cursor -> value -> value -> unit = "continuePrimaryKey" [@@mel.send]
external update : cursor -> value -> raw_request = "update" [@@mel.send]
external delete_record : cursor -> raw_request = "delete" [@@mel.send]

let cursor_request cursor = cursors (cursor_request cursor)
let advance cursor count = catching (fun () -> advance cursor count)
let continue cursor key = catching (fun () -> continue cursor (Js.Undefined.fromOption key))
let continue_primary_key cursor key primary_key = catching (fun () -> continue_primary_key cursor key primary_key)
let update cursor value = catching (fun () -> values (update cursor value))
let delete_record cursor = catching (fun () -> values (delete_record cursor))
