(** The browser APIs idb drives: IndexedDB (IDBFactory, IDBRequest, IDBDatabase, IDBTransaction,
    IDBObjectStore, IDBIndex, IDBCursor, IDBKeyRange and DOMStringList) with its events,
    DOMException, Promise, and the JS values a script carries. Keys, values, versions, counts,
    directions, modes and options are [value]s and reach IndexedDB as they are, so [null] stays a
    value and numbers are never narrowed. A call that can throw returns its exception as [Error].

    This interface is shared, byte-identical, by the Melange and js_of_ocaml ports. Each has its
    own [host.ml]. *)

(** {1 Values} *)

type value
(** Any JS value. *)

val null : value
val string : string -> value
val number : float -> value
val bool : bool -> value
val array : value list -> value

val record : (string * value) list -> value
(** A plain object, its properties in this order. *)

(** A JSON value, one level deep. *)
type json = Null | Bool of bool | Number of float | String of string | Array of value list | Object

val classify : value -> json

val member : value -> string -> value option
(** An object's property, [None] when it is absent (JSON has no [undefined]). *)

val random_uuid : unit -> string
(** [crypto.randomUUID()]. *)

(** {1 Errors} *)

type error
(** A thrown or rejected value: a DOMException, or a TypeError from IDL conversions. *)

val error_name : error -> string
val abort_error : unit -> error
(** [new DOMException('AbortError', 'AbortError')]. *)

val type_error : string -> 'a
(** Throws a JS TypeError: an input outside the contract. *)

(** {1 Promises} *)

type 'a promise
(** A JS Promise. Its callbacks run as microtasks, while an IndexedDB transaction is still active. *)

val resolve : 'a -> 'a promise
val reject : error -> 'a promise
val bind : 'a promise -> ('a -> 'b promise) -> 'b promise
val map : 'a promise -> ('a -> 'b) -> 'b promise

val promise : (('a -> unit) -> (error -> unit) -> unit) -> 'a promise
(** [new Promise(executor)]. *)

val settle : 'a promise -> ('a, error) result promise
(** Follows the promise from now on: its value, or its rejection as [Error]. *)

val both : 'a promise -> 'b promise -> ('a * 'b) promise
(** [Promise.all([a, b])]. *)

val ignore_rejection : 'a promise -> unit
(** [promise.catch(() => {})]. *)

val export : value promise -> value
(** The promise, as the JS value it is. *)

(** {1 IndexedDB objects} *)

type database
type transaction
type object_store
type index
type cursor

type source
(** An IDBObjectStore or an IDBIndex: they share their read methods and cursors. *)

val store_source : object_store -> source
val index_source : index -> source

type 'a request
(** An IDBRequest whose [result] is read as an ['a]. *)

val result : 'a request -> 'a
val request_error : 'a request -> error

type event
(** An IDBVersionChangeEvent: [blocked], [upgradeneeded] or [versionchange]. *)

val old_version : event -> float

val new_version : event -> float option
(** [None] for a deletion. *)

val event_database : event -> database
(** [event.target] of a [versionchange] at a connection. *)

(** {1 Listeners} *)

type listener

val listener : (unit -> unit) -> listener
(** A JS event listener, to add and later remove. *)

val add_request_listener : 'a request -> [ `Success | `Error ] -> listener -> unit
val remove_request_listener : 'a request -> [ `Success | `Error ] -> listener -> unit
val add_transaction_listener : transaction -> [ `Complete | `Error | `Abort ] -> listener -> unit
val remove_transaction_listener : transaction -> [ `Complete | `Error | `Abort ] -> listener -> unit
val on_upgrade_needed : database request -> (event -> unit) -> unit
val on_blocked : 'a request -> (event -> unit) -> unit
val on_version_change : database -> (event -> unit) -> unit

(** {1 IDBFactory} *)

val open_database : string -> value option -> (database request, error) result
(** [indexedDB.open(name, version)]; an absent version is [undefined]. *)

val delete_database : string -> (unit request, error) result

val upgrade_transaction : database request -> transaction
(** The open request's versionchange transaction, while [upgradeneeded] is dispatched. *)

(** {1 IDBKeyRange} *)

val only : value -> (value, error) result
val lower_bound : value -> value option -> (value, error) result
val upper_bound : value option -> value option -> (value, error) result
val bound : value -> value -> value option -> value option -> (value, error) result

(** {1 IDBDatabase} *)

val version : database -> float
val object_store_names : database -> string list
val create_object_store : database -> string -> value option -> (object_store, error) result
val delete_object_store : database -> string -> (unit, error) result

val transaction : database -> value -> value option -> (transaction, error) result
(** [db.transaction(stores, mode)]: a name or an array of names; an absent mode is readonly. *)

val close : database -> unit

(** {1 IDBTransaction} *)

val scope : transaction -> string list
(** [objectStoreNames]. *)

val object_store : transaction -> string -> (object_store, error) result
val abort : transaction -> (unit, error) result
val transaction_error : transaction -> error option

(** {1 IDBObjectStore and IDBIndex} *)

val store_name : object_store -> string
val store_key_path : object_store -> value
val auto_increment : object_store -> bool
val index_names : object_store -> string list
val index : object_store -> string -> (index, error) result
val create_index : object_store -> string -> value -> value option -> (index, error) result
val delete_index : object_store -> string -> (unit, error) result
val index_name : index -> string
val index_key_path : index -> value
val unique : index -> bool
val multi_entry : index -> bool

(** The request methods, called with the script's arguments. A result is any value, [None] when
    it is [undefined]. The write methods exist on stores only. *)

val get : source -> value list -> (value option request, error) result
val get_key : source -> value list -> (value option request, error) result
val get_all : source -> value list -> (value option request, error) result
val get_all_keys : source -> value list -> (value option request, error) result
val count : source -> value list -> (value option request, error) result
val put : source -> value list -> (value option request, error) result
val add : source -> value list -> (value option request, error) result
val delete : source -> value list -> (value option request, error) result
val clear : source -> value list -> (value option request, error) result

val open_cursor : source -> value list -> (cursor option request, error) result
(** A cursor request: its cursor, or [None] past the end. *)

val open_key_cursor : source -> value list -> (cursor option request, error) result

val all_keys : object_store -> (value list request, error) result
(** [getAllKeys()], read as a list. *)

val all_values : object_store -> (value list request, error) result
(** [getAll()], read as a list. *)

(** {1 IDBCursor} *)

val key : cursor -> value
val primary_key : cursor -> value
val cursor_value : cursor -> value

val cursor_request : cursor -> cursor option request
(** The request that opened the cursor: it fires once more after each move. *)

val advance : cursor -> value -> (unit, error) result

val continue : cursor -> value option -> (unit, error) result
(** [continue()] when the key is [None]. *)

val continue_primary_key : cursor -> value -> value -> (unit, error) result
val update : cursor -> value -> (value option request, error) result
val delete_record : cursor -> (value option request, error) result
