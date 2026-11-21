(* Plays a case's script through the port's idb ([Idb]), as libraries/idb/reference.mjs drives npm
   idb, then dumps the databases it left, and closes and deletes every database it touched, also
   after a failure.

   A step's outcome is a promise of a result: [Error Failed] when it failed, the failure already
   logged by the step where it happened. An input outside the contract that only shows while the
   script runs (a connection no earlier step opened, [store: null] in a transaction with several
   stores, a transaction step outside a transaction) throws a TypeError, which is never logged and
   rejects [run] after cleanup, as the reference rejects it.

   Shared, byte-identical, by the Melange and js_of_ocaml ports. *)

open Host
open Script

type failed = Failed
type 'a outcome = ('a, failed) result promise

let return value : _ outcome = resolve (Ok value)

let ( let* ) (outcome : 'a outcome) (next : 'a -> 'b outcome) : 'b outcome =
  bind outcome (function Ok value -> next value | Error Failed -> resolve (Error Failed))

type session = {
  prefix : string;
  mutable log : value list;  (** newest first *)
  mutable connections : (string * database) list;
  mutable opened : database list;  (** newest first *)
  mutable attempted : string list;
  mutable live : string list;
}

let remember names name = if List.mem name names then names else names @ [ name ]

let connection session conn =
  match List.assoc_opt conn session.connections with
  | Some db -> db
  | None -> invalid ("no connection named " ^ conn)

let inside = function Some tx -> tx | None -> invalid "a transaction step outside a transaction"

(* The connection a [blocked] callback closes, resolved when the step starts. *)
let closing session = function
  | Some (Close_other conn) -> Some (connection session conn)
  | Some Log_blocked | None -> None

(* --- The log --- *)

let push session entry = session.log <- array entry :: session.log

(* [[label, "ok"]] for an undefined result, [[label, "ok", value]] otherwise. *)
let ok session label = function
  | None -> push session [ string label; string "ok" ]
  | Some value -> push session [ string label; string "ok"; value ]

let error session label error = push session [ string label; string "error"; string (error_name error) ]

(* Logs a failure where it happens; the [Failed] that propagates says it is logged. *)
let fail session label failure =
  error session label failure;
  resolve (Error Failed)

(* A synchronous call; its failure is logged with [label] and ends the step. *)
let check_now session label = function Ok value -> return value | Error failure -> fail session label failure

(* A call that returns a promise: its synchronous failure or its rejection is logged. *)
let check session label issued =
  match issued with
  | Error failure -> fail session label failure
  | Ok promise ->
      bind (settle promise) (function Ok value -> return value | Error failure -> fail session label failure)

(* One call that is awaited, then logged. *)
let attempt session label issued =
  let* value = check session label issued in
  ok session label value;
  return ()

let version = function Some version -> number version | None -> null
let name_list names = array (List.map string names)

(* --- Sources and arguments --- *)

(* Chains the synchronous host calls that issue a request; each can fail. *)
let ( let*? ) = Result.bind

(* [tx.objectStore(store)], or [tx.store], then [.index(index)]. *)
let source tx { store; index } =
  let*? store =
    match store with
    | Some name -> Idb.Transaction.object_store tx name
    | None ->
        Result.map
          (function Some store -> store | None -> invalid "store: null in a transaction with several stores")
          (Idb.Transaction.store tx)
  in
  match index with
  | Some name -> Result.map index_source (Host.index store name)
  | None -> Ok (store_source store)

(* Builds a key range written as an object: a DataError here is the step's failure. *)
let rec host_args = function
  | [] -> Ok []
  | argument :: rest ->
      let*? value =
        match argument with
        | Value value -> Ok value
        | Range (Only key) -> only key
        | Range (Lower_bound (lower, open_)) -> lower_bound lower open_
        | Range (Upper_bound (upper, open_)) -> upper_bound upper open_
        | Range (Bound (lower, upper, lower_open, upper_open)) -> bound lower upper lower_open upper_open
      in
      let*? values = host_args rest in
      Ok (value :: values)

(* [source[method](...args)]: the source is resolved first, then the arguments are built. *)
let request tx { meth; target; args } =
  let*? source = source tx target in
  let*? args = host_args args in
  Idb.request source meth args

(* A call's outcome, its synchronous failure included, followed from now on as
   [Promise.allSettled] follows it. *)
let settled = function Ok promise -> settle promise | Error failure -> resolve (Error failure)

(* --- Upgrades --- *)

(* Runs inside the upgrade callback: synchronous calls, each logged; a failure does not stop it. *)
let upgrade_step session op db tx =
  let logged label = function Ok value -> ok session label value | Error failure -> error session label failure in
  match op with
  | Create_store { name; options } ->
      logged "createStore"
        (Result.map (fun store -> Some (string (store_name store))) (create_object_store db name options))
  | Delete_store name -> logged "deleteStore" (Result.map (fun () -> None) (delete_object_store db name))
  | Create_index { store; name; key_path; options } ->
      logged "createIndex"
        (let*? store = Idb.Transaction.object_store tx store in
         let*? index = create_index store name key_path options in
         Ok (Some (string (index_name index))))
  | Delete_index { store; name } ->
      logged "deleteIndex"
        (let*? store = Idb.Transaction.object_store tx store in
         Result.map (fun () -> None) (delete_index store name))
  | Abort -> logged "abort" (Result.map (fun () -> None) (Idb.Transaction.abort tx))
  | Write { meth; store; values } -> (
      (* Issued, not awaited: a failed write aborts the upgrade, which rejects the open. *)
      let issued =
        let*? store = Idb.Transaction.object_store tx store in
        Idb.request (store_source store) meth values
      in
      match issued with
      | Ok written -> ignore_rejection written
      | Error failure -> error session (Idb.name meth) failure)

(* --- Cursor walks --- *)

let position session kind cursor =
  push session
    (match kind with
    | Key_cursor -> [ string "cursor"; key cursor; primary_key cursor ]
    | Cursor | Iterate -> [ string "cursor"; key cursor; primary_key cursor; cursor_value cursor ])

let actions_at walk at = if at < Array.length walk.positions then walk.positions.(at) else walk.rest

let write session cursor = function
  | Update value -> attempt session "update" (Idb.Cursor.update cursor value)
  | _ -> attempt session "delete" (Idb.Cursor.delete cursor)

(* A position's actions in a walk, one at a time: the move to await next, or [None] when the walk
   stops here (no move, or a [break]). *)
let rec walk_actions session cursor actions moved =
  match actions with
  | [] -> return moved
  | Break :: _ -> return None
  | ((Update _ | Delete) as action) :: rest ->
      let* () = write session cursor action in
      walk_actions session cursor rest moved
  | Continue key :: rest ->
      let* next = check_now session "continue" (Idb.Cursor.continue cursor key) in
      walk_actions session cursor rest (Some ("continue", next))
  | Advance count :: rest ->
      let* next = check_now session "advance" (Idb.Cursor.advance cursor count) in
      walk_actions session cursor rest (Some ("advance", next))
  | Continue_primary_key (key, primary_key) :: rest ->
      let moved = Idb.Cursor.continue_primary_key cursor key primary_key in
      let* next = check_now session "continuePrimaryKey" moved in
      walk_actions session cursor rest (Some ("continuePrimaryKey", next))

(* A walk with the promise-returning moves: each position's move is awaited after its actions. *)
let rec walk_from session walk cursor at =
  match cursor with
  | None ->
      push session [ string "end" ];
      return ()
  | Some cursor -> (
      position session walk.kind cursor;
      let* moved = walk_actions session cursor (actions_at walk at) None in
      match moved with
      | None -> return ()
      | Some (label, next) ->
          let* cursor = check session label (Ok next) in
          walk_from session walk cursor (at + 1))

(* A position's actions in the iterator. A move only records the next step; [false] on [break]. *)
let rec iterate_actions session (yielded : Idb.Iterate.yielded) actions =
  match actions with
  | [] -> return true
  | Break :: _ -> return false
  | ((Update _ | Delete) as action) :: rest ->
      let* () = write session yielded.cursor action in
      iterate_actions session yielded rest
  | Continue key :: rest ->
      let* () = check_now session "continue" (Idb.Iterate.continue yielded key) in
      iterate_actions session yielded rest
  | Advance count :: rest ->
      let* () = check_now session "advance" (Idb.Iterate.advance yielded count) in
      iterate_actions session yielded rest
  | Continue_primary_key (key, primary_key) :: rest ->
      let recorded = Idb.Iterate.continue_primary_key yielded key primary_key in
      let* () = check_now session "continuePrimaryKey" recorded in
      iterate_actions session yielded rest

(* The async iterator: a position without a move continues. *)
let rec iterate_from session walk iteration at =
  let* yielded = check session "iterate" (Ok (Idb.Iterate.next iteration)) in
  match yielded with
  | None ->
      push session [ string "end" ];
      return ()
  | Some (yielded : Idb.Iterate.yielded) ->
      position session walk.kind yielded.cursor;
      let* continues = iterate_actions session yielded (actions_at walk at) in
      if continues then iterate_from session walk iteration (at + 1) else return ()

let cursor_walk session tx walk =
  match walk.kind with
  | Iterate ->
      let* iteration =
        check_now session "iterate"
          (let*? source = source tx walk.source in
           let*? args = host_args walk.query in
           Ok (Idb.Iterate.make source args))
      in
      iterate_from session walk iteration 0
  | Cursor | Key_cursor ->
      let label, open_ =
        if walk.kind = Key_cursor then ("openKeyCursor", Idb.open_key_cursor) else ("openCursor", Idb.open_cursor)
      in
      let* cursor =
        check session label
          (let*? source = source tx walk.source in
           let*? args = host_args walk.query in
           open_ source args)
      in
      walk_from session walk cursor 0

(* --- Batches --- *)

(* Issues every request before awaiting any, each call returning before the next is made, then
   logs each outcome in issue order. Never fails. *)
let batch session tx requests ~abort ~done_ =
  let pending =
    List.fold_left
      (fun pending (r : Script.request) -> (Idb.name r.meth, settled (request tx r)) :: pending)
      [] requests
    |> List.rev
  in
  if abort then (
    match Idb.Transaction.abort tx with
    | Ok () -> ok session "abort" None
    | Error failure -> error session "abort" failure);
  let pending =
    if done_ then pending @ [ ("done", map (settle tx.done_) (Result.map (fun () -> None))) ] else pending
  in
  let rec log = function
    | [] -> return ()
    | (label, outcome) :: rest ->
        bind outcome (fun outcome ->
            (match outcome with Ok value -> ok session label value | Error failure -> error session label failure);
            log rest)
  in
  log pending

(* --- Steps --- *)

let rec perform session step tx =
  bind (execute session step.op tx) (function
    | Error Failed when step.catches -> return ()
    | outcome -> resolve outcome)

and perform_all session steps tx =
  List.fold_left (fun previous step -> let* () = previous in perform session step tx) (return ()) steps

and execute session op tx =
  match op with
  | Open_db step -> open_db session step
  | Close conn ->
      close (connection session conn);
      ok session "close" None;
      return ()
  | Delete_db { db; blocked } -> delete_db session db blocked
  | Transaction { conn; stores; mode; steps } ->
      let db = connection session conn in
      let* tx = check_now session "tx" (Idb.Database.transaction db stores (Some mode)) in
      perform_all session steps (Some tx)
  | Shortcut { conn; label; meth; store; index; args } ->
      let db = connection session conn in
      attempt session label
        (Result.map (fun args -> Idb.Database.shortcut db meth store index args) (host_args args))
  | Request r -> attempt session (Idb.name r.meth) (request (inside tx) r)
  | Batch { requests; abort; done_ } -> batch session (inside tx) requests ~abort ~done_
  | Walk walk -> cursor_walk session (inside tx) walk
  | Abort_transaction ->
      let tx = inside tx in
      let* () = check_now session "abort" (Idb.Transaction.abort tx) in
      ok session "abort" None;
      return ()
  | Done ->
      let tx = inside tx in
      let* () = check session "done" (Ok tx.done_) in
      ok session "done" None;
      return ()
  | Store ->
      let tx = inside tx in
      let* store = check_now session "store" (Idb.Transaction.store tx) in
      ok session "store" (Option.map (fun store -> string (store_name store)) store);
      return ()
  | Wrap { meth; target; args } ->
      let tx = inside tx in
      (* A wrapped store or index is the host object, so [unwrap(source)[method]] is the host
         method. *)
      attempt session "wrap"
        (let*? source = source tx target in
         let*? args = host_args args in
         Result.map Idb.wrap (Idb.call source meth args))

and open_db session { db; conn; version = requested; upgrade = ops; blocking; blocked } =
  session.attempted <- remember session.attempted db;
  let closing = closing session blocked in
  let upgrade =
    Option.map
      (fun ops database old_version new_version tx ->
        push session [ string "upgrade"; number old_version; version new_version ];
        List.iter (fun op -> upgrade_step session op database tx) ops)
      ops
  in
  let blocked =
    Option.map
      (fun _ current blocked _ ->
        push session [ string "blocked"; number current; version blocked ];
        Option.iter close closing)
      blocked
  in
  let blocking =
    Option.map
      (fun blocking current blocked event ->
        push session [ string "blocking"; string conn; number current; version blocked ];
        if blocking = Close_self then close (event_database event))
      blocking
  in
  let* opened = check session "openDB" (Idb.open_db ?upgrade ?blocked ?blocking (session.prefix ^ db) requested) in
  ok session "openDB" (Some (array [ number (Host.version opened); name_list (object_store_names opened) ]));
  session.connections <- (conn, opened) :: List.remove_assoc conn session.connections;
  session.opened <- opened :: session.opened;
  session.live <- remember session.live db;
  return ()

and delete_db session db blocked =
  session.attempted <- remember session.attempted db;
  let closing = closing session blocked in
  let blocked =
    Option.map
      (fun _ current _ ->
        push session [ string "blocked"; number current ];
        Option.iter close closing)
      blocked
  in
  let* () = check session "deleteDB" (Idb.delete_db ?blocked (session.prefix ^ db)) in
  ok session "deleteDB" None;
  session.live <- List.filter (fun name -> name <> db) session.live;
  return ()

(* --- Dump and cleanup --- *)

(* They work on plain promises: a failure there rejects the call. *)
let ( let> ) = bind

let now = function Ok value -> resolve value | Error failure -> reject failure
let sent = function Ok promise -> promise | Error failure -> reject failure

let rec indexes store = function
  | [] -> Ok []
  | name :: rest ->
      let*? index = Host.index store name in
      let*? entries = indexes store rest in
      Ok (array [ string name; index_key_path index; bool (unique index); bool (multi_entry index) ] :: entries)

(* [[storeName, keyPath, autoIncrement, indexes, records]]. *)
let dump_store tx name =
  let> store = now (Idb.Transaction.object_store tx name) in
  let> indexes = now (indexes store (index_names store)) in
  let> keys = sent (Result.map Idb.wrap (all_keys store)) in
  let> values = sent (Result.map Idb.wrap (all_values store)) in
  let records = List.map2 (fun key value -> array [ key; value ]) keys values in
  resolve (array [ string name; store_key_path store; bool (auto_increment store); array indexes; array records ])

let rec dump_stores tx dumped = function
  | [] ->
      let> () = tx.Idb.Transaction.done_ in
      resolve (List.rev dumped)
  | name :: rest ->
      let> store = dump_store tx name in
      dump_stores tx (store :: dumped) rest

(* Every store, in one readonly transaction. *)
let stores db =
  match object_store_names db with
  | [] -> resolve []
  | names ->
      let> tx = now (Idb.Database.transaction db (name_list names) None) in
      dump_stores tx [] names

(* [[db, version, stores]], read through a connection of its own, closed also after a failure. *)
let dump session name =
  let> db = sent (Idb.open_db (session.prefix ^ name) None) in
  let> stores = settle (stores db) in
  close db;
  match stores with
  | Ok stores -> resolve (array [ string name; number (Host.version db); array stores ])
  | Error failure -> reject failure

let rec dump_all session dumps = function
  | [] -> resolve (List.rev dumps)
  | name :: rest ->
      let> dump = dump session name in
      dump_all session (dump :: dumps) rest

let play session script =
  (* A logged failure ends the script there; anything else rejects the call. *)
  let> (_ : (unit, failed) result) = perform_all session script None in
  let> databases = dump_all session [] session.live in
  resolve (record [ ("log", array (List.rev session.log)); ("databases", array databases) ])

let rec delete_all session = function
  | [] -> resolve ()
  | name :: rest ->
      let> () = sent (Idb.delete_db (session.prefix ^ name)) in
      delete_all session rest

let cleanup session =
  List.iter close (List.rev session.opened);
  delete_all session session.attempted

let run input =
  let> () = resolve () in
  let script = decode input in
  let session =
    {
      prefix = "mires-matrix-idb-" ^ random_uuid () ^ "-";
      log = [];
      connections = [];
      opened = [];
      attempted = [];
      live = [];
    }
  in
  let> played = settle (let> () = resolve () in play session script) in
  (* As the reference's [finally]: cleanup always runs, and its own failure replaces the result. *)
  let> () = cleanup session in
  match played with Ok output -> resolve output | Error failure -> reject failure
