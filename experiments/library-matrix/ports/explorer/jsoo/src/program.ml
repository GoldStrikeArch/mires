open Host

let object_ fields =
  let value = obj () in
  List.iter (fun (key, field) -> set value key field) fields;
  value

let list items =
  let value = array () in
  List.iter (push value) items;
  value

let map f values =
  let output = array () in
  for i = 0 to length values - 1 do push output (f (at values i)) done;
  output

(* One idb script (idb CONTRACT.md, "Input"): create the store, put every row in one readwrite
   transaction, then walk the store with a cursor in primary-key order. *)
let persist rows =
  let schema =
    list
      [ object_ ["op", str "createStore"; "name", str "rows"; "options", object_ ["keyPath", str "id"]];
        object_ ["op", str "createIndex"; "store", str "rows"; "name", str "byGroup"; "keyPath", str "group"] ]
  in
  let open_db =
    object_
      [ "op", str "openDB"; "db", str "explorer"; "conn", str "explorer"; "version", int 1;
        "upgrade", schema; "catch", bool false ]
  in
  let put row =
    object_ ["op", str "put"; "store", nil; "index", nil; "args", list [row]; "catch", bool false]
  in
  let write = object_ ["op", str "batch"; "requests", map put rows; "abort", bool false; "done", bool true] in
  let walk =
    object_
      [ "op", str "openCursor"; "store", nil; "index", nil; "args", array (); "positions", array ();
        "rest", list [list [str "continue"]]; "catch", bool false ]
  in
  let transaction mode step =
    object_
      [ "op", str "tx"; "conn", str "explorer"; "stores", str "rows"; "mode", str mode;
        "steps", list [step]; "catch", bool false ]
  in
  object_ ["steps", list [open_db; transaction "readwrite" write; transaction "readonly" walk]]

(* The stored rows' [value] texts, from the cursor entries of the idb log. *)
let stored history =
  let log = get (at history 0) "log" in
  List.init (length log) (at log)
  |> List.filter (fun entry -> text (at entry 0) = "cursor")
  |> List.map (fun entry -> get (at entry 3) "value")
  |> list

let hits history = get (at (get (at history 1) "results") 1) "hits"
let command name argument = list [str name; argument]

let run packet =
  let input = at packet 0 and history = at packet 1 in
  match length history with
  | 0 ->
      let row doc = object_ ["id", get doc "id"; "group", str "documents"; "value", json doc] in
      command "idb" (persist (map row (get input "records")))
  | 1 ->
      let options = object_ ["fields", list [str "title"; str "body"]; "storeFields", list [str "body"]] in
      let add = object_ ["op", str "addAll"; "documents", map parse (stored history)] in
      let query = object_ ["op", str "search"; "query", get input "query"] in
      command "minisearch" (object_ ["options", options; "operations", list [add; query]])
  | 2 ->
      let found = hits history in
      let markdown = if length found = 0 then str "" else get (at found 0) "body" in
      command "marked" (object_ ["op", str "parse"; "markdown", markdown])
  | 3 ->
      command "fflate" (object_ ["op", str "strToU8"; "text", join (stored history) (str "\n")])
  | 4 ->
      let data = object_ ["bytes", at history 3] in
      let options = object_ ["level", int 6; "mtime", int 0] in
      command "fflate" (object_ ["op", str "gzipSync"; "data", data; "options", options])
  | 5 -> command "uplot" (get input "chart")
  | _ ->
      let ids = map (fun hit -> get hit "id") (hits history) in
      list [nil; list [ids; at history 2; at history 4; int (length (at history 3)); at history 5]]
