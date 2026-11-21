(* The corpus input (CONTRACT "Input") as OCaml types. [decode] reads the input once and rejects
   anything outside the contract's schema with a TypeError, before any database is opened. Names
   become OCaml strings; keys, values, versions, counts, directions, modes and options stay
   [Host.value]s and reach IndexedDB as they are.

   Shared, byte-identical, by the Melange and js_of_ocaml ports. *)

open Host

(* [store = None] is [tx.store]; [index = None] is the store itself. *)
type target = { store : string option; index : string option }

(* A key range written as an object; its open flags stay values, as IDBKeyRange converts them. An
   absent bound or flag is [None]. *)
type range =
  | Only of value
  | Lower_bound of value * value option
  | Upper_bound of value option * value option
  | Bound of value * value * value option * value option

(* A positional argument: a query method's first argument is a key or a key range. *)
type argument = Value of value | Range of range

type request = { meth : Idb.meth; target : target; args : argument list }

type action =
  | Continue of value option
  | Advance of value
  | Continue_primary_key of value * value
  | Update of value
  | Delete
  | Break

type walk_kind = Cursor | Key_cursor | Iterate

type walk = {
  kind : walk_kind;
  source : target;
  query : argument list;
  positions : action list array;
  rest : action list;
}

type upgrade =
  | Create_store of { name : string; options : value option }
  | Delete_store of string
  | Create_index of { store : string; name : string; key_path : value; options : value option }
  | Delete_index of { store : string; name : string }
  (* [put] or [add] in the upgrade transaction: issued, not awaited. *)
  | Write of { meth : Idb.meth; store : string; values : value list }
  | Abort

type blocking = Log_blocking | Close_self

(* ["log"], or [{close: conn}]: log, then close that connection. *)
type blocked = Log_blocked | Close_other of string

type open_db = {
  db : string;
  conn : string;
  version : value option;
  upgrade : upgrade list option;
  blocking : blocking option;
  blocked : blocked option;
}

type step = { op : op; catches : bool }

and op =
  | Open_db of open_db
  | Close of string
  | Delete_db of { db : string; blocked : blocked option }
  | Transaction of { conn : string; stores : value; mode : value; steps : step list }
  (* A database shortcut; [label] is its name, such as [getAllFromIndex]. *)
  | Shortcut of {
      conn : string;
      label : string;
      meth : Idb.meth;
      store : string;
      index : string option;
      args : argument list;
    }
  | Request of request
  | Batch of { requests : request list; abort : bool; done_ : bool }
  | Walk of walk
  | Abort_transaction
  | Done
  | Store
  (* [wrap(unwrap(source)[method](...args))]. *)
  | Wrap of request

let invalid what = type_error ("outside the idb contract: " ^ what)

let field json name =
  match member json name with Some value -> value | None -> invalid ("no " ^ name)

let text_of name value = match classify value with String text -> text | _ -> invalid (name ^ " is not a string")
let items_of name value = match classify value with Array items -> items | _ -> invalid (name ^ " is not an array")
let text json name = text_of name (field json name)
let items json name = items_of name (field json name)
let flag json name =
  match classify (field json name) with Bool flag -> flag | _ -> invalid (name ^ " is not a boolean")

(* A name, or [null]. *)
let name_or_null json name =
  match classify (field json name) with
  | String text -> Some text
  | Null -> None
  | _ -> invalid (name ^ " is neither a name nor null")

let meth = function
  | "get" -> Idb.Get
  | "getKey" -> Get_key
  | "getAll" -> Get_all
  | "getAllKeys" -> Get_all_keys
  | "count" -> Count
  | "put" -> Put
  | "add" -> Add
  | "delete" -> Delete
  | "clear" -> Clear
  | name -> invalid ("not a request method: " ^ name)

(* A key is passed as it is, [null] included; an object is a key range, its fields tested with
   [in] (the B9 decision: [{only: null}] is [only(null)]). *)
let query key =
  match classify key with
  | Object -> (
      let flag name = member key name in
      Range
        (match (member key "only", member key "lower", member key "upper") with
        | Some only, _, _ -> Only only
        | None, Some lower, Some upper -> Bound (lower, upper, flag "lowerOpen", flag "upperOpen")
        | None, Some lower, None -> Lower_bound (lower, flag "lowerOpen")
        | None, None, upper -> Upper_bound (upper, flag "upperOpen")))
  | Null | Bool _ | Number _ | String _ | Array _ -> Value key

(* The first argument of a query method is a query; [put], [add] and [clear] take plain arguments. *)
let query_args = function first :: rest -> query first :: List.map (fun arg -> Value arg) rest | [] -> []

let args (meth : Idb.meth) values =
  match meth with
  | Put | Add | Clear -> List.map (fun value -> Value value) values
  | Get | Get_key | Get_all | Get_all_keys | Count | Delete -> query_args values

let target json = { store = name_or_null json "store"; index = name_or_null json "index" }
let request json meth = { meth; target = target json; args = args meth (items json "args") }

let action json =
  match items_of "an action" json with
  | name :: values -> (
      match (text_of "an action" name, values) with
      | "continue", [] -> Continue None
      | "continue", [ key ] -> Continue (Some key)
      | "advance", [ count ] -> Advance count
      | "continuePrimaryKey", [ key; primary_key ] -> Continue_primary_key (key, primary_key)
      | "update", [ value ] -> Update value
      | "delete", [] -> Delete
      | "break", [] -> Break
      | name, _ -> invalid ("not a cursor action: " ^ name))
  | [] -> invalid "an empty cursor action"

let actions json = List.map action (items_of "a position" json)

let walk json kind =
  {
    kind;
    source = target json;
    query = query_args (items json "args");
    positions = Array.of_list (List.map actions (items json "positions"));
    rest = actions (field json "rest");
  }

let blocked json =
  Option.map
    (fun blocked ->
      match classify blocked with
      | String "log" -> Log_blocked
      | Object -> Close_other (text blocked "close")
      | _ -> invalid "blocked")
    (member json "blocked")

let upgrade json =
  match text json "op" with
  | "createStore" -> Create_store { name = text json "name"; options = member json "options" }
  | "deleteStore" -> Delete_store (text json "name")
  | "createIndex" ->
      Create_index
        {
          store = text json "store";
          name = text json "name";
          key_path = field json "keyPath";
          options = member json "options";
        }
  | "deleteIndex" -> Delete_index { store = text json "store"; name = text json "name" }
  | "put" -> Write { meth = Put; store = text json "store"; values = items json "args" }
  | "add" -> Write { meth = Add; store = text json "store"; values = items json "args" }
  | "abort" -> Abort
  | op -> invalid ("not an upgrade step: " ^ op)

let rec step json =
  let conn () = text json "conn" in
  let op =
    match text json "op" with
    | "openDB" ->
        Open_db
          {
            db = text json "db";
            conn = conn ();
            version = member json "version";
            upgrade = Option.map (fun ops -> List.map upgrade (items_of "upgrade" ops)) (member json "upgrade");
            blocking =
              Option.map
                (fun blocking ->
                  match classify blocking with
                  | String "log" -> Log_blocking
                  | String "close" -> Close_self
                  | _ -> invalid "blocking")
                (member json "blocking");
            blocked = blocked json;
          }
    | "close" -> Close (conn ())
    | "deleteDB" -> Delete_db { db = text json "db"; blocked = blocked json }
    | "tx" ->
        Transaction
          {
            conn = conn ();
            stores = field json "stores";
            mode = field json "mode";
            steps = List.map step (items json "steps");
          }
    | "call" ->
        (* database-extras' getMethod: [prop.replace(/FromIndex$/, '')]; only read methods have an
           index form. *)
        let label = text json "method" in
        let base = String.length label - String.length "FromIndex" in
        let from_index = base > 0 && String.sub label base (String.length "FromIndex") = "FromIndex" in
        let meth = meth (if from_index then String.sub label 0 base else label) in
        if from_index && Idb.is_write meth then invalid ("not a database shortcut: " ^ label);
        Shortcut
          {
            conn = conn ();
            label;
            meth;
            store = text json "store";
            index = name_or_null json "index";
            args = args meth (items json "args");
          }
    | "batch" ->
        Batch
          {
            requests = List.map (fun json -> request json (meth (text json "op"))) (items json "requests");
            abort = flag json "abort";
            done_ = flag json "done";
          }
    | "openCursor" -> Walk (walk json Cursor)
    | "openKeyCursor" -> Walk (walk json Key_cursor)
    | "iterate" -> Walk (walk json Iterate)
    | "abort" -> Abort_transaction
    | "done" -> Done
    | "store" -> Store
    | "wrap" -> Wrap (request json (meth (text json "method")))
    | op -> Request (request json (meth op))
  in
  let catches =
    match Option.map classify (member json "catch") with
    | None -> false
    | Some (Bool catches) -> catches
    | Some _ -> invalid "catch is not a boolean"
  in
  { op; catches }

let decode input = List.map step (items input "steps")
