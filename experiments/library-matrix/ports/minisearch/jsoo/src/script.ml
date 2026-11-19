(* [run(input)]: the operation script of libraries/minisearch/reference.mjs. One index per call, the
   operations in order, one result record per operation (CONTRACT.md, "Input" and "Output").

   The input is read in place through [Host.Value] (documents lazily: MiniSearch's [extractField]
   is [document[name]]), and never written; the result is built through [Host.Json]. *)

open Host
open Options

let field = Value.field

let required value name =
  match Value.field value name with Some item -> item | None -> invalid_arg ("MiniSearch port: missing " ^ name)

let array_of value =
  match Value.view value with Array items -> items | _ -> invalid_arg "MiniSearch port: an array was expected"

let number_of value =
  match Value.view value with Number number -> number | _ -> invalid_arg "MiniSearch port: a number was expected"

let text_of value =
  match Value.view value with String text -> text | _ -> invalid_arg "MiniSearch port: a string was expected"

let names value = Array.map Value.name (array_of value)
let optional value name read = Option.map read (field value name)

(* A value MiniSearch reads from a document, or a filter value. *)
let value_of item =
  match Value.view item with
  | Null -> Null
  | Bool value -> Bool value
  | Number number -> Number number
  | String text -> String text
  | Array _ | Object -> invalid_arg "MiniSearch port: document values are strings, numbers or null"

let doc_id_of item =
  match Value.view item with
  | Number number -> Number_id number
  | String text -> String_id text
  | _ -> invalid_arg "MiniSearch port: ids are strings or numbers"

let document item : Minisearch.document = fun name -> Option.map value_of (field item name)
let pair item = match array_of item with [| key; factor |] -> (key, number_of factor) | _ -> invalid_arg "MiniSearch port: a pair was expected"

(* SearchOptions, the declarative callbacks included (CONTRACT.md, "Input"). *)
let search_options item =
  {
    fields = optional item "fields" (fun fields -> Array.to_list (names fields));
    boost =
      optional item "boost" (fun boost ->
          Array.to_list (Array.map (fun name -> (name, number_of (required boost name))) (Value.keys boost)));
    (* [options.prefix === true] *)
    prefix = optional item "prefix" (fun prefix -> Prefix_bool (match Value.view prefix with Bool true -> true | _ -> false));
    fuzzy =
      optional item "fuzzy" (fun fuzzy ->
          match Value.view fuzzy with
          | Bool value -> Fuzzy_bool value
          | Number number -> Fuzzy_number number
          | _ -> invalid_arg "MiniSearch port: fuzzy is a boolean or a number");
    max_fuzzy = optional item "maxFuzzy" number_of;
    combine_with = optional item "combineWith" text_of;
    weights =
      optional item "weights" (fun weights ->
          { fuzzy_weight = optional weights "fuzzy" number_of; prefix_weight = optional weights "prefix" number_of });
    bm25 =
      optional item "bm25" (fun bm25 ->
          let parameter name = Option.value (optional bm25 name number_of) ~default:Float.nan in
          { k = parameter "k"; b = parameter "b"; d = parameter "d" });
    filter =
      optional item "filter" (fun filter ->
          { field = Value.name (required filter "field"); values = Array.to_list (Array.map value_of (array_of (required filter "in"))) });
    boost_document =
      optional item "boostDocument" (fun pairs ->
          Array.to_list (Array.map (fun item -> let key, factor = pair item in (doc_id_of key, factor)) (array_of pairs)));
    boost_term =
      optional item "boostTerm" (fun pairs ->
          Array.to_list (Array.map (fun item -> let key, factor = pair item in (text_of key, factor)) (array_of pairs)));
  }

let call_options operation = match field operation "options" with Some item -> search_options item | None -> no_options

(* [{ wildcard: true }] is [MiniSearch.wildcard]; another object is a query tree node, whose
   options are its keys other than [queries]. *)
let rec query item =
  match Value.view item with
  | String text -> Minisearch.Text_query text
  | Object -> (
      match optional item "wildcard" Value.view with
      | Some (Bool true) -> Minisearch.Wildcard
      | _ -> Minisearch.Tree (search_options item, Array.to_list (Array.map query (array_of (required item "queries")))))
  | _ -> invalid_arg "MiniSearch port: a query is a string or an object"

let index_options item : Options.index =
  let field_names =
    match field item "fields" with
    | Some fields -> names fields
    | None -> failwith "MiniSearch: option \"fields\" must be provided"
  in
  let constructor name = match field item name with Some options -> search_options options | None -> no_options in
  {
    field_names;
    id_field = Option.value (optional item "idField" Value.name) ~default:"id";
    store_fields = Option.value (optional item "storeFields" names) ~default:[||];
    search_options = over library_defaults (constructor "searchOptions");
    auto_suggest_options = over auto_suggest_defaults (constructor "autoSuggestOptions");
    auto_vacuum =
      (match field item "autoVacuum" with
      | None -> Some default_auto_vacuum
      | Some auto -> (
          match Value.view auto with
          | Null | Bool true -> Some default_auto_vacuum
          | Bool false -> None
          | _ ->
              Some
                {
                  batch_size = optional auto "batchSize" number_of;
                  min_dirt_count = optional auto "minDirtCount" number_of;
                  min_dirt_factor = optional auto "minDirtFactor" number_of;
                }));
  }

(* ---------------------------------------------------------------------------------------------
   The result. *)

let json_value = function
  | Null -> Json.null
  | Bool value -> Json.bool value
  | Number number -> Json.number number
  | String text -> Json.text text

let json_id = function Number_id number -> Json.number number | String_id text -> Json.text text
let json_texts texts = Json.array (List.map Json.text texts)
let json_stored stored = Json.obj (List.map (fun (name, value) -> (name, json_value value)) stored)

(* MiniSearch's result object with [match] as pairs, followed by the stored fields as
   [Object.assign(result, storedFields)] adds them: a stored [id] replaces the id in place. *)
let json_hit index (hit : Minisearch.hit) =
  let stored = Minisearch.stored_fields index hit.short_id in
  let id =
    match List.find_opt (fun (name, _) -> String.equal name "id") stored with
    | Some (_, value) -> json_value value
    | None -> json_id hit.id
  in
  Json.obj
    ([
       ("id", id);
       ("score", Json.number hit.score);
       ("terms", json_texts (List.map fst hit.matches));
       ("queryTerms", json_texts hit.query_terms);
       ( "match",
         Json.array
           (List.map
              (fun (term, fields) -> Json.array [ Json.text term; Json.array (List.map (fun name -> Json.text (Text.of_string name)) fields) ])
              hit.matches) );
     ]
    @ List.filter_map (fun (name, value) -> if String.equal name "id" then None else Some (name, json_value value)) stored)

let json_suggestion (suggestion : Minisearch.suggestion) =
  Json.obj
    [
      ("suggestion", Json.text suggestion.suggestion);
      ("terms", json_texts suggestion.suggestion_terms);
      ("score", Json.number suggestion.suggestion_score);
    ]

(* One operation: its payload fields, in order. *)
let perform index operation =
  match Value.name (required operation "op") with
  | "add" ->
      Minisearch.add index (document (required operation "document"));
      []
  | "addAll" ->
      Minisearch.add_all index (Array.to_list (Array.map document (array_of (required operation "documents"))));
      []
  | "remove" ->
      Minisearch.remove index (document (required operation "document"));
      []
  | "removeAll" ->
      Minisearch.remove_all index
        (match field operation "documents" with
        | None -> Minisearch.Everything
        | Some documents -> (
            match Value.view documents with
            | Array documents -> Minisearch.Documents (Array.to_list (Array.map document documents))
            | _ -> Minisearch.Null_argument));
      []
  | "discard" ->
      Minisearch.discard index (doc_id_of (required operation "id"));
      []
  | "discardAll" ->
      Minisearch.discard_all index (Array.to_list (Array.map doc_id_of (array_of (required operation "ids"))));
      []
  | "replace" ->
      Minisearch.replace index (document (required operation "document"));
      []
  | "vacuum" ->
      (* The returned Promise settles after the synchronous run (CONTRACT.md, "Vacuuming"). *)
      Minisearch.vacuum index (Option.bind (field operation "options") (fun options -> optional options "batchSize" number_of));
      []
  | "search" ->
      let hits = Minisearch.search index (query (required operation "query")) (call_options operation) in
      [ ("hits", Json.array (List.map (json_hit index) hits)) ]
  | "autoSuggest" ->
      let suggestions = Minisearch.auto_suggest index (text_of (required operation "query")) (call_options operation) in
      [ ("suggestions", Json.array (List.map json_suggestion suggestions)) ]
  | "has" -> [ ("value", Json.bool (Minisearch.has index (doc_id_of (required operation "id")))) ]
  | "getStoredFields" ->
      let stored = Minisearch.get_stored_fields index (doc_id_of (required operation "id")) in
      [ ("value", match stored with Some stored -> json_stored stored | None -> Json.null) ]
  | "stats" ->
      [
        ("documentCount", Json.int index.Minisearch.document_count);
        ("termCount", Json.int (Minisearch.term_count index));
        ("dirtCount", Json.int index.Minisearch.dirt_count);
        ("dirtFactor", Json.number (Minisearch.dirt_factor index));
        ("isVacuuming", Json.bool index.Minisearch.vacuuming);
      ]
  | "toJSON" -> [ ("value", Minisearch.to_json index) ]
  | name -> raise (Minisearch.Error (Text.of_string ("unknown operation " ^ name)))

(* A record: the operation name, then the payload when the operation returned, its warnings, and
   MiniSearch's message when it threw (the script goes on with the index as MiniSearch left it). *)
let record index operation =
  let op = ("op", Json.text (text_of (required operation "op"))) in
  let outcome = try Ok (perform index operation) with Minisearch.Error message -> Error message in
  let warnings = match Minisearch.take_warnings index with [] -> [] | warnings -> [ ("warnings", json_texts warnings) ] in
  match outcome with
  | Ok payload -> Json.obj ((op :: payload) @ warnings)
  | Error message -> Json.obj ((op :: warnings) @ [ ("error", Json.text message) ])

let run input =
  let index = Minisearch.create (index_options (required input "options")) in
  let records = Array.map (record index) (array_of (required input "operations")) in
  Json.obj [ ("results", Json.array (Array.to_list records)) ]
