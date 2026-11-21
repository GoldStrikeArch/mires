(* The browser APIs idb drives, for js_of_ocaml: its [Promise] bindings, and [Js.Unsafe] for the
   IndexedDB objects, which js_of_ocaml does not bind. OCaml strings are UTF-8 here: a name crosses
   with [Js.string] and [Js.to_string], which is exact for the well-formed UTF-16 the contract
   allows. Keys and values never cross: they stay JS values. A host method that can throw is called
   through [catching]. See host.mli. *)

open Js_of_ocaml

(* --- Values --- *)

type value = Js.Unsafe.any

let inject = Js.Unsafe.inject
let global name : value = Js.Unsafe.get Js.Unsafe.global (Js.string name)
let prop target name = Js.Unsafe.get target (Js.string name)
let field target name : value = prop target name
let call target name args : value = Js.Unsafe.meth_call target name (Array.of_list args)
let null = inject Js.null
let string text = inject (Js.string text)
let number number = inject (Js.number_of_float number)
let bool flag = inject (Js.bool flag)
let array values = inject (Js.array (Array.of_list values))
let record fields = Js.Unsafe.obj (Array.of_list fields)
let optional = function Some value -> value | None -> inject Js.undefined
let text_of value = Js.to_string (Js.Unsafe.coerce value)
let float_of value = Js.float_of_number (Js.Unsafe.coerce value)
let bool_of value = Js.to_bool (Js.Unsafe.coerce value)
let list_of value : value list = Array.to_list (Js.to_array (Js.Unsafe.coerce value))

type json = Null | Bool of bool | Number of float | String of string | Array of value list | Object

let type_error message : 'a =
  Js.Js_error.raise_ (Js.Js_error.of_error (Js.Unsafe.new_obj (global "TypeError") [| string message |]))

let classify value =
  match Js.to_string (Js.typeof value) with
  | "string" -> String (text_of value)
  | "number" -> Number (float_of value)
  | "boolean" -> Bool (bool_of value)
  | "object" when value == null -> Null
  | "object" -> if bool_of (call (global "Array") "isArray" [ value ]) then Array (list_of value) else Object
  | _ -> type_error "not a JSON value"

let member value name : value option = Js.Optdef.to_option (prop value name)
let random_uuid () = text_of (call (global "crypto") "randomUUID" [])

(* --- Errors --- *)

type error = value

let error_name error = text_of (field error "name")
let abort_error () = Js.Unsafe.new_obj (global "DOMException") [| string "AbortError"; string "AbortError" |]

(* A host call that can throw: its JS exception, as the value thrown. *)
let catching call = try Ok (call ()) with Js.Js_error.Exn thrown -> Error (inject (Js.Js_error.to_error thrown))

(* --- Promises --- *)

type 'a promise = 'a Promise.t

let resolve = Promise.resolve
let reject error = Promise.reject (Promise.error_of_any error)
let bind promise next = Promise.bind next promise
let map promise f = Promise.map f promise

let promise executor =
  Promise.make (fun ~resolve ~reject -> executor resolve (fun error -> reject (Promise.error_of_any error)))

let settle promise =
  promise
  |> Promise.map (fun value -> Ok value)
  |> Promise.catch (fun error -> Promise.resolve (Error (Promise.error_to_any error)))

(* The host's Promise.all, which follows both promises from now on and rejects with the first
   rejection. *)
let both a b =
  let all = call (global "Promise") "all" [ inject (Js.array [| Promise.to_any a; Promise.to_any b |]) ] in
  Promise.map (fun pair -> (Js.Unsafe.get pair 0, Js.Unsafe.get pair 1)) (Promise.of_any all)

let ignore_rejection promise = ignore (settle promise)
let export promise = Promise.to_any promise

(* --- IndexedDB objects --- *)

type database = value
type transaction = value
type object_store = value
type index = value
type cursor = value
type source = value

let store_source store = store
let index_source index = index

(* A request, and how its [result] is read. *)
type 'a request = { raw : value; read : value -> 'a }

let result request = request.read request.raw
let request_error request = field request.raw "error"
let values raw = { raw; read = (fun raw : value option -> Js.Optdef.to_option (prop raw "result")) }
let cursors raw = { raw; read = (fun raw : cursor option -> Js.Opt.to_option (prop raw "result")) }
let lists raw = { raw; read = (fun raw -> list_of (field raw "result")) }

type event = value

let old_version event = float_of (field event "oldVersion")
let new_version event = Option.map float_of (Js.Opt.to_option (prop event "newVersion"))
let event_database event = field event "target"

(* --- Listeners --- *)

type listener = value

let listener f = inject (Js.wrap_callback (fun (_ : value) -> f ()))
let listen target kind handle =
  ignore (call target "addEventListener" [ string kind; inject (Js.wrap_callback handle) ])
let request_event = function `Success -> "success" | `Error -> "error"
let transaction_event = function `Complete -> "complete" | `Error -> "error" | `Abort -> "abort"

let add_request_listener request kind listener =
  ignore (call request.raw "addEventListener" [ string (request_event kind); listener ])

let remove_request_listener request kind listener =
  ignore (call request.raw "removeEventListener" [ string (request_event kind); listener ])

let add_transaction_listener tx kind listener =
  ignore (call tx "addEventListener" [ string (transaction_event kind); listener ])

let remove_transaction_listener tx kind listener =
  ignore (call tx "removeEventListener" [ string (transaction_event kind); listener ])

let on_upgrade_needed request handle = listen request.raw "upgradeneeded" handle
let on_blocked request handle = listen request.raw "blocked" handle
let on_version_change db handle = listen db "versionchange" handle

(* --- IDBFactory --- *)

let open_database name version =
  catching (fun () ->
      let raw = call (global "indexedDB") "open" [ string name; optional version ] in
      { raw; read = (fun raw -> field raw "result") })

let delete_database name =
  catching (fun () -> { raw = call (global "indexedDB") "deleteDatabase" [ string name ]; read = ignore })

let upgrade_transaction request = field request.raw "transaction"

(* --- IDBKeyRange --- *)

let key_range factory args = catching (fun () -> call (global "IDBKeyRange") factory args)
let only key = key_range "only" [ key ]
let lower_bound lower open_ = key_range "lowerBound" [ lower; optional open_ ]
let upper_bound upper open_ = key_range "upperBound" [ optional upper; optional open_ ]
let bound lower upper lower_open upper_open =
  key_range "bound" [ lower; upper; optional lower_open; optional upper_open ]

(* --- IDBDatabase, DOMStringList --- *)

let names list = List.map text_of (list_of (call (global "Array") "from" [ list ]))
let version db = float_of (field db "version")
let object_store_names db = names (field db "objectStoreNames")

let create_object_store db name options =
  catching (fun () -> call db "createObjectStore" [ string name; optional options ])

let delete_object_store db name = catching (fun () -> ignore (call db "deleteObjectStore" [ string name ]))
let transaction db stores mode = catching (fun () -> call db "transaction" [ stores; optional mode ])
let close db = ignore (call db "close" [])

(* --- IDBTransaction --- *)

let scope tx = names (field tx "objectStoreNames")
let object_store tx name = catching (fun () -> call tx "objectStore" [ string name ])
let abort tx = catching (fun () -> ignore (call tx "abort" []))
let transaction_error tx : error option = Js.Opt.to_option (prop tx "error")

(* --- IDBObjectStore and IDBIndex --- *)

let store_name store = text_of (field store "name")
let store_key_path store = field store "keyPath"
let auto_increment store = bool_of (field store "autoIncrement")
let index_names store = names (field store "indexNames")
let index store name = catching (fun () -> call store "index" [ string name ])

let create_index store name key_path options =
  catching (fun () -> call store "createIndex" [ string name; key_path; optional options ])

let delete_index store name = catching (fun () -> ignore (call store "deleteIndex" [ string name ]))
let index_name index = text_of (field index "name")
let index_key_path index = field index "keyPath"
let unique index = bool_of (field index "unique")
let multi_entry index = bool_of (field index "multiEntry")

(* A request method called with the script's arguments, exactly as many as it gives. *)
let issue read name source args = catching (fun () -> read (call source name args))
let get = issue values "get"
let get_key = issue values "getKey"
let get_all = issue values "getAll"
let get_all_keys = issue values "getAllKeys"
let count = issue values "count"
let put = issue values "put"
let add = issue values "add"
let delete = issue values "delete"
let clear = issue values "clear"
let open_cursor = issue cursors "openCursor"
let open_key_cursor = issue cursors "openKeyCursor"
let all_keys store = issue lists "getAllKeys" store []
let all_values store = issue lists "getAll" store []

(* --- IDBCursor --- *)

let key cursor = field cursor "key"
let primary_key cursor = field cursor "primaryKey"
let cursor_value cursor = field cursor "value"
let cursor_request cursor = cursors (field cursor "request")
let move cursor name args = catching (fun () -> ignore (call cursor name args))
let advance cursor count = move cursor "advance" [ count ]
let continue cursor key = move cursor "continue" [ optional key ]
let continue_primary_key cursor key primary_key = move cursor "continuePrimaryKey" [ key; primary_key ]
let update cursor value = catching (fun () -> values (call cursor "update" [ value ]))
let delete_record cursor = catching (fun () -> values (call cursor "delete" []))
