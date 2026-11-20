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

let stored history =
  let cursor = at (at history 0) 3 in
  map (fun row -> at row 2) cursor

let hits history = get (at (get (at history 1) "results") 1) "hits"
let command name argument = list [str name; argument]

let run packet =
  let input = at packet 0 and history = at packet 1 in
  match length history with
  | 0 ->
      let row doc = object_ ["id", get doc "id"; "group", str "documents"; "value", json doc] in
      let batch = object_ ["rows", map row (get input "records")] in
      command "idb" (object_ ["batches", list [batch]; "keys", array (); "groups", array (); "step", int 1])
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
