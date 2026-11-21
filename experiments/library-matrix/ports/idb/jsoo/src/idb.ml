(* idb 8.0.4 (build/index.js) for OCaml: IndexedDB requests become promises, a transaction gets a
   [done] promise, cursors move by returning a promise of the next position, and [Iterate] walks a
   store or an index.

   idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back.
   OCaml has no Proxies, so a wrapped object is the host object itself, and this module holds the
   methods idb changes: each calls the host method and wraps its result as idb's [get] trap would.
   A transaction is the one exception: it carries its [done] promise. Properties idb forwards
   unchanged ([name], [keyPath], [version], ...) are read with [Host] directly.

   idb's wrapped methods throw synchronously; here they return [Error]. A request is issued when
   the method is called, and its promise exists from then on.

   Shared, byte-identical, by the Melange and js_of_ocaml ports. *)

open Host

let ( let* ) = Result.bind

(* idb's [promisifyRequest]: settled once, with the request's result on [success] or its error on
   [error], after which both listeners are removed. A cursor's request fires once per move, so each
   move wraps the same request again. *)
let wrap request =
  promise (fun resolve reject ->
      let unlisten = ref ignore in
      let success = listener (fun () -> resolve (result request); !unlisten ())
      and error = listener (fun () -> reject (request_error request); !unlisten ()) in
      (unlisten :=
         fun () ->
           remove_request_listener request `Success success;
           remove_request_listener request `Error error);
      add_request_listener request `Success success;
      add_request_listener request `Error error)

(* The request methods idb promisifies, and database-extras' [readMethods] and [writeMethods]. *)
type meth = Get | Get_key | Get_all | Get_all_keys | Count | Put | Add | Delete | Clear

let name = function
  | Get -> "get"
  | Get_key -> "getKey"
  | Get_all -> "getAll"
  | Get_all_keys -> "getAllKeys"
  | Count -> "count"
  | Put -> "put"
  | Add -> "add"
  | Delete -> "delete"
  | Clear -> "clear"

let is_write = function
  | Put | Add | Delete | Clear -> true
  | Get | Get_key | Get_all | Get_all_keys | Count -> false

(* The unwrapped host call, which returns the raw request. *)
let call source meth args =
  match meth with
  | Get -> get source args
  | Get_key -> get_key source args
  | Get_all -> get_all source args
  | Get_all_keys -> get_all_keys source args
  | Count -> count source args
  | Put -> put source args
  | Add -> add source args
  | Delete -> delete source args
  | Clear -> clear source args

(* A wrapped request method. *)
let request source meth args = Result.map wrap (call source meth args)
let open_cursor source args = Result.map wrap (Host.open_cursor source args)
let open_key_cursor source args = Result.map wrap (Host.open_key_cursor source args)

module Transaction = struct
  type t = { raw : transaction; done_ : unit promise }

  (* idb's [cacheDonePromiseForTransaction]: settled by the first [complete], [error] or [abort].
     A request error bubbles to the transaction before the transaction records it, so [done] then
     rejects with a new AbortError. *)
  let cache_done tx =
    promise (fun resolve reject ->
        let unlisten = ref ignore in
        let complete = listener (fun () -> resolve (); !unlisten ())
        and error =
          listener (fun () ->
              reject (match transaction_error tx with Some error -> error | None -> abort_error ());
              !unlisten ())
        in
        (unlisten :=
           fun () ->
             remove_transaction_listener tx `Complete complete;
             remove_transaction_listener tx `Error error;
             remove_transaction_listener tx `Abort error);
        add_transaction_listener tx `Complete complete;
        add_transaction_listener tx `Error error;
        add_transaction_listener tx `Abort error)

  (* idb creates [done] when it first wraps a transaction, so [done] also settles for a caller that
     only asks after the transaction finished. *)
  let wrap raw = { raw; done_ = cache_done raw }
  let object_store tx name = Host.object_store tx.raw name
  let abort tx = Host.abort tx.raw

  (* idb's [tx.store] trap, [objectStoreNames[1] ? undefined : objectStore(objectStoreNames[0])].
     A transaction's scope is never empty. On a finished transaction with one store,
     [objectStore] throws InvalidStateError. *)
  let store tx =
    match scope tx.raw with
    | [ only ] -> Result.map Option.some (object_store tx only)
    | _ -> Ok None
end

module Database = struct
  let transaction db stores mode = Result.map Transaction.wrap (Host.transaction db stores mode)

  (* The database shortcuts ([db.get(store, ...)], [db.getAllFromIndex(store, index, ...)], ...),
     database-extras' [getMethod]: one transaction on [tx.store] (or its index) per call, then
     [Promise.all([request, isWrite && tx.done])]. So a write resolves only after [done], the
     request's error comes first, and both promises are handled. The method is async: a
     synchronous failure rejects the call. *)
  let shortcut db meth store index args =
    let write = is_write meth in
    let issued =
      let* tx = transaction db (string store) (Some (string (if write then "readwrite" else "readonly"))) in
      let* target = Transaction.store tx in
      let target = Option.get target in
      let* source =
        match index with
        | Some name -> Result.map index_source (Host.index target name)
        | None -> Ok (store_source target)
      in
      let* request = request source meth args in
      Ok (both request (if write then tx.done_ else resolve ()))
    in
    match issued with Ok settled -> map settled fst | Error error -> reject error
end

(* idb's cursor advance methods ([wrapFunction]): the host move, then the cursor's request wrapped
   again. The promise resolves with the same cursor, or with [None] past the end. *)
module Cursor = struct
  let moved cursor called = Result.map (fun () -> wrap (cursor_request cursor)) called
  let advance cursor count = moved cursor (Host.advance cursor count)
  let continue cursor key = moved cursor (Host.continue cursor key)
  let continue_primary_key cursor key primary_key = moved cursor (Host.continue_primary_key cursor key primary_key)
  let update cursor value = Result.map wrap (Host.update cursor value)
  let delete cursor = Result.map wrap (delete_record cursor)
end

(* idb's [iterate], the async iterator of a store or an index. The first [next] opens
   [openCursor(...args)]. Each later one awaits the move the loop body made on the yielded cursor,
   or calls [continue()] when the body made none. *)
module Iterate = struct
  type state = Unopened | At of cursor | Finished

  type t = {
    source : source;
    args : value list;
    mutable state : state;
    mutable advanced : cursor option promise option;
  }

  (* The cursor an iteration yields, idb's proxied cursor: its moves are intercepted. They call the
     host move and record its promise for the next [next], as [cursorIteratorTraps] do. *)
  type yielded = { iteration : t; cursor : cursor }

  let make source args = { source; args; state = Unopened; advanced = None }

  let next iteration =
    let state = iteration.state in
    iteration.state <- Finished;
    let moved =
      match state with
      | Unopened -> open_cursor iteration.source iteration.args
      | At cursor -> (
          match iteration.advanced with Some moved -> Ok moved | None -> Cursor.continue cursor None)
      | Finished -> Ok (resolve None)
    in
    match moved with
    | Error error -> reject error
    | Ok moved ->
        map moved (fun cursor ->
            iteration.advanced <- None;
            Option.map
              (fun cursor ->
                iteration.state <- At cursor;
                { iteration; cursor })
              cursor)

  let record { iteration; cursor } move = Result.map (fun moved -> iteration.advanced <- Some moved) (move cursor)
  let advance yielded count = record yielded (fun cursor -> Cursor.advance cursor count)
  let continue yielded key = record yielded (fun cursor -> Cursor.continue cursor key)

  let continue_primary_key yielded key primary_key =
    record yielded (fun cursor -> Cursor.continue_primary_key cursor key primary_key)
end

(* idb's [openDB(name, version, { upgrade, blocked, blocking })]. [upgrade] gets
   [(db, oldVersion, newVersion, transaction)], [blocked] and [blocking]
   [(currentVersion, blockedVersion, event)]. An absent version opens the current version, or
   version 1 for a new database; an invalid one fails now. [blocking] is attached to the
   connection once it is open. *)
let open_db ?upgrade ?blocked ?blocking name version =
  let* request = open_database name version in
  let open_promise = wrap request in
  Option.iter
    (fun upgrade ->
      on_upgrade_needed request (fun event ->
          upgrade (result request) (old_version event) (new_version event)
            (Transaction.wrap (upgrade_transaction request))))
    upgrade;
  Option.iter
    (fun blocked -> on_blocked request (fun event -> blocked (old_version event) (new_version event) event))
    blocked;
  ignore_rejection
    (map open_promise (fun db ->
         Option.iter
           (fun blocking ->
             on_version_change db (fun event -> blocking (old_version event) (new_version event) event))
           blocking));
  Ok open_promise

(* idb's [deleteDB(name, { blocked })]: [blocked(currentVersion, event)]; resolves undefined. *)
let delete_db ?blocked name =
  let* request = delete_database name in
  Option.iter (fun blocked -> on_blocked request (fun event -> blocked (old_version event) event)) blocked;
  Ok (map (wrap request) ignore)
