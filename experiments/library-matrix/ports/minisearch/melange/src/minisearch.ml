(* The MiniSearch class: documents, the inverted index, search, autoSuggest and vacuuming,
   following minisearch/dist/es/index.js (7.2.0) method by method. *)

open Host
open Collections
open Options

(* An error MiniSearch throws, with its message. *)
exception Error of Text.t

(* A posting list ([fieldIndex]: short id to term frequency). A [Map] in MiniSearch whose keys are
   always inserted in increasing order: a document only ever adds postings under its own short
   id, which is larger than every short id before it. So the list is kept sorted by short id,
   which is that [Map]'s insertion order, in two parallel growable arrays. *)
module Posting = struct
  type t = { mutable docs : int array; mutable freqs : int array; mutable size : int }

  let singleton doc = { docs = [| doc |]; freqs = [| 1 |]; size = 1 }

  (* The position of [doc], or -1. *)
  let find posting doc =
    let rec search low high =
      if low >= high then -1
      else
        let middle = (low + high) / 2 in
        let current = posting.docs.(middle) in
        if current = doc then middle else if current < doc then search (middle + 1) high else search low middle
    in
    search 0 posting.size

  (* [set(doc, freq)] of a short id larger than every one in the list. *)
  let push posting doc freq =
    if posting.size = Array.length posting.docs then begin
      let capacity = Int.max 4 (2 * posting.size) in
      let docs = Array.make capacity 0 and freqs = Array.make capacity 0 in
      Array.blit posting.docs 0 docs 0 posting.size;
      Array.blit posting.freqs 0 freqs 0 posting.size;
      posting.docs <- docs;
      posting.freqs <- freqs
    end;
    posting.docs.(posting.size) <- doc;
    posting.freqs.(posting.size) <- freq;
    posting.size <- posting.size + 1

  (* [delete] of the entry at [at]. *)
  let remove_at posting at =
    let tail = posting.size - at - 1 in
    Array.blit posting.docs (at + 1) posting.docs at tail;
    Array.blit posting.freqs (at + 1) posting.freqs at tail;
    posting.size <- posting.size - 1

  (* Keeps the entries whose short id satisfies [keep], in order. *)
  let retain posting keep =
    let kept = ref 0 in
    for at = 0 to posting.size - 1 do
      if keep posting.docs.(at) then begin
        posting.docs.(!kept) <- posting.docs.(at);
        posting.freqs.(!kept) <- posting.freqs.(at);
        incr kept
      end
    done;
    posting.size <- !kept
end

(* A term's index data ([fieldsData]): field id to posting list, in insertion order. *)
type term_data = { mutable postings : (int * Posting.t) list }

let new_term_data () = { postings = [] }

(* [fieldsData.get(fieldId)]. *)
let posting_of data field_id =
  let rec find = function [] -> None | (id, posting) :: later -> if id = field_id then Some posting else find later in
  find data.postings

(* A [match] entry: a derived (document) term and the fields it matched in. *)
type matched = { term : Text.t; mutable in_fields : string list }

(* One document's accumulated result, [{ score, terms, match }]: the query terms that matched, and
   [match] in insertion order. *)
type raw_result = { mutable score : float; mutable terms : Text.t list; mutable matches : matched list }

(* [Map<shortId, result>]: its insertion order is the result order before sorting (and the order
   of equal scores after it). Short ids are dense (0 to nextId - 1, the same for every map of one
   search), so lookups go through an array indexed by short id, as the per-document tables do,
   and the order is the list of slots, as in [Ordered]. *)
module Results = struct
  type 'a slot = { short_id : int; mutable value : 'a; mutable live : bool }

  (* [order] holds every slot ever added, the most recent first; a deleted slot is marked dead. *)
  type 'a t = { slots : 'a slot option array; mutable order : 'a slot list }

  let create capacity = { slots = Array.make capacity None; order = [] }
  let capacity results = Array.length results.slots

  let find_opt results short_id =
    match results.slots.(short_id) with Some slot -> Some slot.value | None -> None

  (* [results.set(shortId, value)]. *)
  let set results short_id value =
    match results.slots.(short_id) with
    | Some slot -> slot.value <- value
    | None ->
        let slot = { short_id; value; live = true } in
        results.slots.(short_id) <- Some slot;
        results.order <- slot :: results.order

  (* [results.delete(shortId)]. *)
  let delete results short_id =
    match results.slots.(short_id) with
    | Some slot ->
        slot.live <- false;
        results.slots.(short_id) <- None
    | None -> ()

  (* The live entries in insertion order. *)
  let iter f results = List.iter (fun slot -> if slot.live then f slot.short_id slot.value) (List.rev results.order)

  let to_list results =
    List.fold_left
      (fun entries slot -> if slot.live then (slot.short_id, slot.value) :: entries else entries)
      [] results.order
end

(* [Map<string, _>]: [fuzzyGet]'s results (term to data and distance), autoSuggest's groups. *)
module Text_ordered = Ordered (Text_key)

(* [_idToShortId]: ids are [Map] keys, compared by SameValueZero, which this order agrees with
   ([Float.compare] makes [-0] equal to [0]; numbers and strings never compare equal). *)
module Id_map = Map.Make (struct
  type t = doc_id

  let compare a b =
    match (a, b) with
    | Number_id a, Number_id b -> Float.compare a b
    | String_id a, String_id b -> text_compare a b
    | Number_id _, String_id _ -> -1
    | String_id _, Number_id _ -> 1
end)

(* [document[name]]: [None] when it is undefined. *)
type document = string -> value option

type t = {
  options : Options.index;
  mutable index : term_data Searchable_map.t;
  mutable document_count : int;
  mutable document_ids : doc_id option Vec.t;
      (** [_documentIds], indexed by short id; a removed or discarded document is [None] *)
  mutable id_to_short_id : int Id_map.t;
  mutable field_length : int option array option Vec.t;
      (** [_fieldLength], indexed by short id: each field's unique token count, holes for absent
          fields (an array as long as one past the highest field set) *)
  mutable avg_field_length : float option array;  (** [_avgFieldLength], holes included *)
  mutable next_id : int;
  mutable stored_fields : (string * value) list option Vec.t;  (** [_storedFields], by short id *)
  mutable dirt_count : int;
  mutable vacuuming : bool;
      (** [_currentVacuum != null]. Once a vacuum starts, MiniSearch clears this only in a later
          microtask, so for the rest of a synchronous run it stays set (CONTRACT.md, "Vacuuming"). *)
  mutable warnings : Text.t list;  (** what the [logger] received, the most recent first *)
}

let create options =
  {
    options;
    index = Searchable_map.create ();
    document_count = 0;
    document_ids = Vec.create ();
    id_to_short_id = Id_map.empty;
    field_length = Vec.create ();
    avg_field_length = [||];
    next_id = 0;
    stored_fields = Vec.create ();
    dirt_count = 0;
    vacuuming = false;
    warnings = [];
  }

let take_warnings index =
  let warnings = List.rev index.warnings in
  index.warnings <- [];
  warnings

(* ---------------------------------------------------------------------------------------------
   Helpers. *)

let text = Text.of_string

(* A template literal. *)
let concat parts = List.fold_left Text.append (text "") parts

(* [${id}]. *)
let id_text = function Number_id number -> Text.of_number number | String_id text -> text

let error_with_id before id after = Error (concat [ text before; id_text id; text after ])
let is_present index short_id = Option.is_some (Vec.get index.document_ids short_id)

let stored_fields index short_id =
  match Vec.get index.stored_fields short_id with Some stored -> stored | None -> []

(* [array[index]]: [None] past the end, as for a hole. *)
let slot slots index = if index < Array.length slots then slots.(index) else None

(* [array[index] = value], growing the array with holes. *)
let set_slot slots index value =
  let slots =
    if index < Array.length slots then slots
    else begin
      let grown = Array.make (index + 1) None in
      Array.blit slots 0 grown 0 (Array.length slots);
      grown
    end
  in
  slots.(index) <- Some value;
  slots

(* [stringifyField]: [fieldValue.toString()], or [None] for the [null] and absent values
   MiniSearch skips. *)
let field_text = function
  | None | Some Null -> None
  | Some (String text) -> Some text
  | Some (Number number) -> Some (Text.of_number number)
  | Some (Bool true) -> Some (text "true")
  | Some (Bool false) -> Some (text "false")

(* [new Set(tokens).size]: distinct raw tokens, before lower-casing, the empty token included. *)
let unique_count tokens =
  Text_map.cardinal (Array.fold_left (fun seen token -> Text_map.add token () seen) Text_map.empty tokens)

(* [extractField(document, idField)], an error when it is null or absent. *)
let document_id index (document : document) =
  match document index.options.id_field with
  | Some (Number number) -> Number_id number
  | Some (String text) -> String_id text
  | None | Some Null ->
      raise (Error (concat [ text "MiniSearch: document does not have ID field \""; text index.options.id_field; text "\"" ]))
  | Some (Bool _) -> invalid_arg "MiniSearch port: document ids are strings or numbers"

(* ---------------------------------------------------------------------------------------------
   Field lengths, in doubles, exactly in MiniSearch's order. *)

let add_field_length index short_id field_id count length =
  let lengths = match Vec.get index.field_length short_id with Some lengths -> lengths | None -> [||] in
  Vec.set index.field_length short_id (Some (set_slot lengths field_id length));
  (* [this._avgFieldLength[fieldId] || 0] *)
  let average = or_default (slot index.avg_field_length field_id) 0. in
  let total = (average *. float_of_int count) +. float_of_int length in
  index.avg_field_length <- set_slot index.avg_field_length field_id (total /. float_of_int (count + 1))

let remove_field_length index field_id count length =
  if count = 1 then index.avg_field_length <- set_slot index.avg_field_length field_id 0.
  else begin
    (* A hole reads as undefined, which makes the average NaN. *)
    let average = match slot index.avg_field_length field_id with Some average -> average | None -> Float.nan in
    let total = (average *. float_of_int count) -. float_of_int length in
    index.avg_field_length <- set_slot index.avg_field_length field_id (total /. float_of_int (count - 1))
  end

(* ---------------------------------------------------------------------------------------------
   Terms. *)

let add_term index field_id short_id term =
  let data = Searchable_map.fetch index.index term new_term_data in
  match posting_of data field_id with
  | None -> data.postings <- data.postings @ [ (field_id, Posting.singleton short_id) ]
  | Some posting -> (
      match Posting.find posting short_id with
      | -1 -> Posting.push posting short_id 1
      | at -> posting.freqs.(at) <- posting.freqs.(at) + 1)

let warn_document_changed index short_id field_id term =
  (* The field name whose id is [fieldId] (field names are distinct). *)
  if field_id < Array.length index.options.field_names then begin
    let id = match Vec.get index.document_ids short_id with Some id -> id_text id | None -> text "undefined" in
    let message =
      concat
        [
          text "MiniSearch: document with ID ";
          id;
          text " has changed before removal: term \"";
          term;
          text "\" was not present in field \"";
          text index.options.field_names.(field_id);
          text "\". Removing a document after it has changed can corrupt the index!";
        ]
    in
    index.warnings <- message :: index.warnings
  end

let remove_term index field_id short_id term =
  if not (Searchable_map.has index.index term) then warn_document_changed index short_id field_id term
  else begin
    let data = Searchable_map.fetch index.index term new_term_data in
    (match posting_of data field_id with
    | None -> warn_document_changed index short_id field_id term
    | Some posting -> (
        match Posting.find posting short_id with
        | -1 -> warn_document_changed index short_id field_id term
        | at ->
            if posting.freqs.(at) <= 1 then begin
              if posting.size <= 1 then data.postings <- List.filter (fun (id, _) -> id <> field_id) data.postings
              else Posting.remove_at posting at
            end
            else posting.freqs.(at) <- posting.freqs.(at) - 1));
    match Searchable_map.get index.index term with
    | Some { postings = [] } -> Searchable_map.delete index.index term
    | Some _ | None -> ()
  end

(* ---------------------------------------------------------------------------------------------
   Adding and removing documents. *)

let add_document_id index id =
  let short_id = index.next_id in
  index.id_to_short_id <- Id_map.add id short_id index.id_to_short_id;
  Vec.push index.document_ids (Some id);
  Vec.push index.field_length None;
  Vec.push index.stored_fields None;
  index.document_count <- index.document_count + 1;
  index.next_id <- index.next_id + 1;
  short_id

let save_stored_fields index short_id (document : document) =
  if Array.length index.options.store_fields > 0 then begin
    let store stored name =
      match document name with
      | None -> stored
      | Some value ->
          if List.exists (fun (key, _) -> String.equal key name) stored then
            List.map (fun (key, old) -> if String.equal key name then (key, value) else (key, old)) stored
          else stored @ [ (name, value) ]
    in
    Vec.set index.stored_fields short_id (Some (Array.fold_left store [] index.options.store_fields))
  end

let add index (document : document) =
  let id = document_id index document in
  if Id_map.mem id index.id_to_short_id then raise (error_with_id "MiniSearch: duplicate ID " id "");
  let short_id = add_document_id index id in
  save_stored_fields index short_id document;
  Array.iteri
    (fun field_id field ->
      match field_text (document field) with
      | None -> ()
      | Some field_value ->
          let tokens = Text.split field_value in
          add_field_length index short_id field_id (index.document_count - 1) (unique_count tokens);
          Array.iter
            (fun token ->
              let term = Text.lower token in
              if Text.length term > 0 then add_term index field_id short_id term)
            tokens)
    index.options.field_names

let add_all index documents = List.iter (add index) documents

let remove index (document : document) =
  let id = document_id index document in
  let short_id =
    match Id_map.find_opt id index.id_to_short_id with
    | Some short_id -> short_id
    | None -> raise (error_with_id "MiniSearch: cannot remove document with ID " id ": it is not in the index")
  in
  Array.iteri
    (fun field_id field ->
      match field_text (document field) with
      | None -> ()
      | Some field_value ->
          let tokens = Text.split field_value in
          remove_field_length index field_id index.document_count (unique_count tokens);
          Array.iter
            (fun token ->
              let term = Text.lower token in
              if Text.length term > 0 then remove_term index field_id short_id term)
            tokens)
    index.options.field_names;
  Vec.set index.stored_fields short_id None;
  Vec.set index.document_ids short_id None;
  index.id_to_short_id <- Id_map.remove id index.id_to_short_id;
  Vec.set index.field_length short_id None;
  index.document_count <- index.document_count - 1

(* What [removeAll] is called with: documents, an explicit [null], or no argument at all. *)
type remove_all = Documents of document list | Null_argument | Everything

(* [removeAll()] makes a fresh index that keeps [dirtCount] and the vacuum state. *)
let remove_all index = function
  | Documents documents -> List.iter (remove index) documents
  | Null_argument -> raise (Error (text "Expected documents to be present. Omit the argument to remove all documents."))
  | Everything ->
      index.index <- Searchable_map.create ();
      index.document_count <- 0;
      index.document_ids <- Vec.create ();
      index.id_to_short_id <- Id_map.empty;
      index.field_length <- Vec.create ();
      index.avg_field_length <- [||];
      index.stored_fields <- Vec.create ();
      index.next_id <- 0

(* ---------------------------------------------------------------------------------------------
   Vacuuming. *)

let dirt_factor index =
  float_of_int index.dirt_count /. (1. +. float_of_int index.document_count +. float_of_int index.dirt_count)

let vacuum_conditions_met index conditions =
  let min_dirt_count = or_default conditions.min_dirt_count 20. in
  let min_dirt_factor = or_default conditions.min_dirt_factor 0.1 in
  float_of_int index.dirt_count >= min_dirt_count && dirt_factor index >= min_dirt_factor

(* The synchronous part of [performVacuuming]: the terms in iteration order until the first batch
   boundary, where MiniSearch awaits a timer and the rest happens after the run. Only a walk that
   ends without stopping reduces [dirtCount]. *)
let perform_vacuuming index batch_size =
  let initial_dirt_count = index.dirt_count in
  let batch_size = or_default batch_size 1000. in
  let rec walk position = function
    | [] -> index.dirt_count <- index.dirt_count - initial_dirt_count
    | (term, data) :: rest ->
        (* Every posting of a document no longer present goes; a field left without postings goes. *)
        data.postings <-
          List.filter
            (fun (_, posting) ->
              Posting.retain posting (is_present index);
              posting.Posting.size > 0)
            data.postings;
        (match Searchable_map.get index.index term with
        | Some { postings = [] } -> Searchable_map.delete index.index term
        | Some _ | None -> ());
        if Float.rem (float_of_int position) batch_size <> 0. then walk (position + 1) rest
  in
  walk 1 (Searchable_map.entries index.index)

(* [conditionalVacuum(options, conditions)]. A request made while a vacuum is current is chained
   after it, which in a synchronous run is never. *)
let conditional_vacuum index batch_size conditions =
  if not index.vacuuming then
    match conditions with
    | Some conditions when not (vacuum_conditions_met index conditions) -> ()
    | Some _ | None ->
        index.vacuuming <- true;
        perform_vacuuming index batch_size

let maybe_auto_vacuum index =
  match index.options.auto_vacuum with
  | None -> ()
  | Some auto -> conditional_vacuum index auto.batch_size (Some auto)

(* [vacuum(options)]: unconditional, unless a vacuum is current. *)
let vacuum index batch_size = conditional_vacuum index batch_size None

let discard_document index id =
  let short_id =
    match Id_map.find_opt id index.id_to_short_id with
    | Some short_id -> short_id
    | None -> raise (error_with_id "MiniSearch: cannot discard document with ID " id ": it is not in the index")
  in
  index.id_to_short_id <- Id_map.remove id index.id_to_short_id;
  Vec.set index.document_ids short_id None;
  Vec.set index.stored_fields short_id None;
  (* [forEach] over the array skips its holes: fields the document did not have. *)
  (match Vec.get index.field_length short_id with
  | Some lengths ->
      Array.iteri
        (fun field_id length ->
          match length with
          | Some length -> remove_field_length index field_id index.document_count length
          | None -> ())
        lengths
  | None -> ());
  Vec.set index.field_length short_id None;
  index.document_count <- index.document_count - 1;
  index.dirt_count <- index.dirt_count + 1

let discard index id =
  discard_document index id;
  maybe_auto_vacuum index

(* Automatic vacuuming is off while the ids are discarded, then considered once; a failing
   discard ends it there, without that vacuum. *)
let discard_all index ids =
  List.iter (discard_document index) ids;
  maybe_auto_vacuum index

(* [${value}] of an id that is not in the index. *)
let value_text = function
  | None -> text "undefined"
  | Some Null -> text "null"
  | Some (Bool value) -> text (if value then "true" else "false")
  | Some (Number number) -> Text.of_number number
  | Some (String text) -> text

let replace index (document : document) =
  match document index.options.id_field with
  | Some (Number number) ->
      discard index (Number_id number);
      add index document
  | Some (String text) ->
      discard index (String_id text);
      add index document
  | id -> raise (Error (concat [ text "MiniSearch: cannot discard document with ID "; value_text id; text ": it is not in the index" ]))

let has index id = Id_map.mem id index.id_to_short_id

let get_stored_fields index id =
  match Id_map.find_opt id index.id_to_short_id with
  | Some short_id -> Vec.get index.stored_fields short_id
  | None -> None

let term_count index = Searchable_map.size index.index

(* ---------------------------------------------------------------------------------------------
   Search. *)

type query = Text_query of Text.t | Wildcard | Tree of search * query list

(* One query term with its options ([termToQuerySpec]). *)
type spec = { spec_term : Text.t; spec_fuzzy : fuzzy option; spec_prefix : bool; term_boost : float }

(* What [termResults] needs besides the term and its data. *)
type scoring = {
  boosts : (string * int option * float) list;  (** field name, field id, field boost *)
  boost_document : (doc_id * float) list option;
  bm25 : bm25;
  scoring_term_boost : float;
}

let or_text = text "or"
let and_text = text "and"
let and_not_text = text "and_not"
let space = text " "

(* [parts.join(separator)]. *)
let join parts separator =
  match parts with
  | [] -> text ""
  | first :: rest -> List.fold_left (fun joined part -> Text.append (Text.append joined separator) part) first rest

let calc_bm25_score term_freq matching_count total_count field_length avg_field_length { k; b; d } =
  let inv_doc_freq = Float.log (1. +. ((total_count -. matching_count +. 0.5) /. (matching_count +. 0.5))) in
  inv_doc_freq *. (d +. (term_freq *. (k +. 1.) /. (term_freq +. (k *. (1. -. b +. (b *. field_length /. avg_field_length))))))

(* [Math.round] of a non-negative number: halves round up. *)
let js_round number =
  let floor = Float.floor number in
  if number -. floor >= 0.5 then floor +. 1. else floor

(* [byScore], [(a, b) => b.score - a.score], for a stable sort. *)
let by_score a b =
  let difference = b -. a in
  if difference > 0. then 1 else if difference < 0. then -1 else 0

let document_boost pairs id =
  match List.find_opt (fun (key, _) -> doc_id_equal key id) pairs with Some (_, factor) -> factor | None -> 1.

let fuzzy_truthy = function Fuzzy_bool value -> value | Fuzzy_number number -> number <> 0. && not (Float.is_nan number)

(* The value of a key that is an array index: the canonical decimal spelling of an integer from
   0 to 2^32 - 2. *)
let array_index term =
  let length = Text.length term in
  if length = 0 || length > 10 || (length > 1 && Text.unit_at term 0 = 0x30) then None
  else begin
    let value = ref 0. and digits = ref true in
    for index = 0 to length - 1 do
      let unit = Text.unit_at term index in
      if unit < 0x30 || unit > 0x39 then digits := false
      else value := (!value *. 10.) +. float_of_int (unit - 0x30)
    done;
    if !digits && !value <= 4294967294. then Some !value else None
  end

(* [Object.keys(match)] order: array-index keys first, in ascending numeric order, then every
   other key in insertion order. *)
let object_keys_order matches =
  let indexed = List.filter_map (fun entry -> Option.map (fun value -> (value, entry)) (array_index entry.term)) matches in
  let others = List.filter (fun entry -> Option.is_none (array_index entry.term)) matches in
  List.map (fun entry -> (entry.term, entry.in_fields))
    (List.map snd (List.stable_sort (fun (a, _) (b, _) -> Float.compare a b) indexed) @ others)

let rec has_text text = function [] -> false | first :: later -> Text.equal first text || has_text text later

(* [getOwnProperty(match, term)]. *)
let rec match_entry term = function
  | [] -> None
  | entry :: later -> if Text.equal entry.term term then Some entry else match_entry term later

(* [assignUniqueTerm(target, term)]. *)
let assign_unique_term result term = if not (has_text term result.terms) then result.terms <- result.terms @ [ term ]

let assign_unique_terms result terms = List.iter (assign_unique_term result) terms

(* [Object.assign(target, source)] on two [match] objects: an existing key keeps its place and
   takes the new value, a new key is added last. *)
let assign_matches target source =
  List.fold_left
    (fun target entry ->
      match match_entry entry.term target with
      | Some existing ->
          existing.in_fields <- entry.in_fields;
          target
      | None -> target @ [ entry ])
    target source

let combine_or a b =
  Results.iter
    (fun short_id incoming ->
      match Results.find_opt a short_id with
      | None -> Results.set a short_id incoming
      | Some existing ->
          existing.score <- existing.score +. incoming.score;
          existing.matches <- assign_matches existing.matches incoming.matches;
          assign_unique_terms existing incoming.terms)
    b;
  a

(* AND keeps [b]'s order: the combined map is built by walking [b]. *)
let combine_and a b =
  let combined = Results.create (Results.capacity b) in
  Results.iter
    (fun short_id incoming ->
      match Results.find_opt a short_id with
      | None -> ()
      | Some existing ->
          assign_unique_terms existing incoming.terms;
          Results.set combined short_id
            {
              score = existing.score +. incoming.score;
              terms = existing.terms;
              matches = assign_matches existing.matches incoming.matches;
            })
    b;
  combined

let combine_and_not a b =
  Results.iter (fun short_id _ -> Results.delete a short_id) b;
  a

(* [combineResults(results, combineWith = 'or')]: no results is an empty map whatever the
   operator; otherwise the operator is looked up in lower case, and the results are folded from
   the left. *)
let combine_results index results combine_with =
  match results with
  | [] -> Results.create index.next_id
  | first :: rest ->
      let combine_with = match combine_with with Some combine_with -> combine_with | None -> or_text in
      let operator = Text.lower combine_with in
      let combinator =
        if Text.equal operator or_text then combine_or
        else if Text.equal operator and_text then combine_and
        else if Text.equal operator and_not_text then combine_and_not
        else raise (Error (Text.append (text "Invalid combination operator: ") combine_with))
      in
      List.fold_left combinator first rest

(* [this._fieldIds[name]]. *)
let field_id index name =
  let found = ref None in
  Array.iteri (fun field_id field -> if String.equal field name then found := Some field_id) index.options.field_names;
  !found

(* The fields to score, in order, with their boost: an object built from the field list, so a
   repeated field keeps its first position. *)
let field_boosts index options =
  let fields = match options.fields with Some fields -> fields | None -> Array.to_list index.options.field_names in
  let unique = List.fold_left (fun unique field -> if List.exists (String.equal field) unique then unique else unique @ [ field ]) [] fields in
  List.map (fun field -> (field, field_id index field, field_boost options field)) unique

(* [termResults]: BM25+ for every posting of [data] in the boosted fields, into [results]. A
   posting of a document no longer present is skipped, lowers [matchingFields] for the postings
   after it, and is removed from the index ([removeTerm]). MiniSearch removes it while it iterates
   the posting list; here the removals of a list are applied right after it is scored. That is
   the same sequence of index changes, and nothing in between reads what they change (the
   iteration of a JavaScript [Map] goes on past a deleted entry). *)
let term_results index scoring source derived term_weight data results =
  let score_posting field field_id field_boost posting =
    let matching = ref posting.Posting.size in
    let average = match slot index.avg_field_length field_id with Some average -> average | None -> Float.nan in
    let gone = ref [] in
    for at = 0 to posting.Posting.size - 1 do
      let short_id = posting.Posting.docs.(at) in
      match Vec.get index.document_ids short_id with
      | None ->
          gone := short_id :: !gone;
          decr matching
      | Some id ->
          let doc_boost = match scoring.boost_document with Some pairs -> document_boost pairs id | None -> 1. in
          if doc_boost <> 0. && not (Float.is_nan doc_boost) then begin
            let term_freq = float_of_int posting.Posting.freqs.(at) in
            let field_length =
              match Option.bind (Vec.get index.field_length short_id) (fun lengths -> slot lengths field_id) with
              | Some length -> float_of_int length
              | None -> Float.nan
            in
            let raw_score =
              calc_bm25_score term_freq (float_of_int !matching) (float_of_int index.document_count) field_length average
                scoring.bm25
            in
            let weighted = term_weight *. scoring.scoring_term_boost *. field_boost *. doc_boost *. raw_score in
            match Results.find_opt results short_id with
            | Some result -> (
                result.score <- result.score +. weighted;
                assign_unique_term result source;
                match match_entry derived result.matches with
                | Some entry -> entry.in_fields <- entry.in_fields @ [ field ]
                | None -> result.matches <- result.matches @ [ { term = derived; in_fields = [ field ] } ])
            | None ->
                Results.set results short_id { score = weighted; terms = [ source ]; matches = [ { term = derived; in_fields = [ field ] } ] }
          end
    done;
    List.iter (fun short_id -> remove_term index field_id short_id derived) (List.rev !gone)
  in
  match data with
  | None -> ()
  | Some data ->
      List.iter
        (fun (field, field_id, field_boost) ->
          match field_id with
          | None -> ()
          | Some field_id -> Option.iter (score_posting field field_id field_boost) (posting_of data field_id))
        scoring.boosts

(* [executeQuerySpec]: the exact term, then prefix matches, then fuzzy matches. *)
let execute_query_spec index spec search_options =
  let options = over index.options.search_options search_options in
  let weights = match options.weights with Some weights -> weights | None -> { fuzzy_weight = None; prefix_weight = None } in
  let fuzzy_weight = match weights.fuzzy_weight with Some weight -> weight | None -> 0.45 in
  let prefix_weight = match weights.prefix_weight with Some weight -> weight | None -> 0.375 in
  let scoring =
    {
      boosts = field_boosts index options;
      boost_document = options.boost_document;
      bm25 = (match options.bm25 with Some bm25 -> bm25 | None -> { k = Float.nan; b = Float.nan; d = Float.nan });
      scoring_term_boost = spec.term_boost;
    }
  in
  let term = spec.spec_term in
  let results = Results.create index.next_id in
  term_results index scoring term term 1. (Searchable_map.get index.index term) results;
  (* After the exact term's cleanup, which may have deleted it and merged nodes: that changes
     the order of the walks below (discard-exact-term-merge). *)
  let prefix_matches = if spec.spec_prefix then Some (Searchable_map.prefix_entries index.index term) else None in
  let fuzzy_matches =
    match spec.spec_fuzzy with
    | None -> None
    | Some fuzzy ->
        let fuzzy = match fuzzy with Fuzzy_bool _ -> 0.2 | Fuzzy_number number -> number in
        let max_fuzzy = match options.max_fuzzy with Some max_fuzzy -> max_fuzzy | None -> Float.nan in
        let max_distance =
          if fuzzy < 1. then Float.min max_fuzzy (js_round (float_of_int (Text.length term) *. fuzzy)) else fuzzy
        in
        if max_distance = 0. || Float.is_nan max_distance then None
        else begin
          let matches = Text_ordered.create () in
          List.iter
            (fun (key, data, distance) -> Text_ordered.set matches key (data, distance))
            (Searchable_map.fuzzy_get index.index term (int_of_float max_distance));
          Some matches
        end
  in
  let term_length = Text.length term in
  Option.iter
    (List.iter (fun (derived, data) ->
         let distance = Text.length derived - term_length in
         if distance <> 0 then begin
           Option.iter (fun matches -> Text_ordered.delete matches derived) fuzzy_matches;
           let length = float_of_int (Text.length derived) in
           let weight = prefix_weight *. length /. (length +. (0.3 *. float_of_int distance)) in
           term_results index scoring term derived weight (Some data) results
         end))
    prefix_matches;
  Option.iter
    (Text_ordered.iter (fun derived (data, distance) ->
         if distance <> 0 then begin
           let length = float_of_int (Text.length derived) in
           let weight = fuzzy_weight *. length /. (length +. float_of_int distance) in
           term_results index scoring term derived weight (Some data) results
         end))
    fuzzy_matches;
  results

let execute_wildcard_query index search_options =
  let options = over index.options.search_options search_options in
  let results = Results.create index.next_id in
  Vec.iteri
    (fun short_id id ->
      match id with
      | None -> ()
      | Some id ->
          let score = match options.boost_document with Some pairs -> document_boost pairs id | None -> 1. in
          Results.set results short_id { score; terms = []; matches = [] })
    index.document_ids;
  results

(* [termToQuerySpec]. *)
let term_to_query_spec options term position count =
  {
    spec_term = term;
    (* [options.fuzzy || false]: a falsy fuzzy is [false]. *)
    spec_fuzzy = (match options.fuzzy with Some fuzzy when fuzzy_truthy fuzzy -> Some fuzzy | Some _ | None -> None);
    spec_prefix =
      (match options.prefix with
      | Some Prefix_last_term -> position = count - 1
      | Some (Prefix_bool value) -> value
      | None -> false);
    term_boost =
      (match options.boost_term with
      | Some pairs -> (
          match List.find_opt (fun (key, _) -> Text.equal key term) pairs with Some (_, factor) -> factor | None -> 1.)
      | None -> 1.);
  }

let rec execute_query index query options =
  match query with
  | Wildcard -> execute_wildcard_query index options
  | Tree (node, queries) ->
      (* [{ ...searchOptions, ...query }]: the constructor's options are not merged in, so a node
         without combineWith (here or above) combines with OR. *)
      let options = over options node in
      let results = List.map (fun subquery -> execute_query index subquery options) queries in
      combine_results index results options.combine_with
  | Text_query query ->
      let options = over index.options.search_options options in
      let terms =
        List.filter (fun term -> Text.length term > 0) (List.map Text.lower (Array.to_list (Text.split query)))
      in
      let count = List.length terms in
      let results =
        List.mapi (fun position term -> execute_query_spec index (term_to_query_spec options term position count) options) terms
      in
      combine_results index results options.combine_with

(* A search result: [{ id, score, terms, queryTerms, match }], [match] (and so [terms]) in
   [Object.keys] order. The stored fields are those of [short_id]. *)
type hit = {
  id : doc_id;
  short_id : int;
  score : float;
  matches : (Text.t * string list) list;
  query_terms : Text.t list;
}

type suggestion = { suggestion : Text.t; suggestion_terms : Text.t list; suggestion_score : float }

(* [result[field]] on MiniSearch's result object: a stored field ([Object.assign]), else the id. *)
let result_property index hit field =
  match List.find_opt (fun (name, _) -> String.equal name field) (stored_fields index hit.short_id) with
  | Some (_, value) -> Some value
  | None ->
      if String.equal field "id" then
        Some (match hit.id with Number_id number -> Number number | String_id text -> String text)
      else None

(* SameValueZero, as [Array.prototype.includes] compares. *)
let same_value_zero a b =
  match (a, b) with
  | Null, Null -> true
  | Bool a, Bool b -> a = b
  | Number a, Number b -> a = b || (Float.is_nan a && Float.is_nan b)
  | String a, String b -> Text.equal a b
  | (Null | Bool _ | Number _ | String _), _ -> false

let search index query options =
  let with_defaults = over index.options.search_options options in
  let raw = execute_query index query options in
  let hits =
    List.filter_map
      (fun (short_id, result) ->
        let quality = match result.terms with [] -> 1 | terms -> List.length terms in
        let id = match Vec.get index.document_ids short_id with Some id -> id | None -> assert false in
        let hit =
          {
            id;
            short_id;
            score = result.score *. float_of_int quality;
            matches = object_keys_order result.matches;
            query_terms = result.terms;
          }
        in
        let keep =
          match with_defaults.filter with
          | None -> true
          | Some { field; values } -> (
              match result_property index hit field with
              | Some value -> List.exists (same_value_zero value) values
              | None -> false)
        in
        if keep then Some hit else None)
      (Results.to_list raw)
  in
  (* A wildcard query without boostDocument is left unsorted: every score is 1, and the sort is
     stable, so sorting would not change it either. *)
  match (query, with_defaults.boost_document) with
  | Wildcard, None -> hits
  | _ -> List.stable_sort (fun a b -> by_score a.score b.score) hits

let auto_suggest index query options =
  let options = over index.options.auto_suggest_options options in
  let groups = Text_ordered.create () in
  List.iter
    (fun hit ->
      let terms = List.map fst hit.matches in
      let phrase = join terms space in
      match Text_ordered.find_opt groups phrase with
      | Some (score, first_terms, count) -> Text_ordered.set groups phrase (score +. hit.score, first_terms, count + 1)
      | None -> Text_ordered.set groups phrase (hit.score, terms, 1))
    (search index (Text_query query) options);
  let suggestions =
    List.map
      (fun (suggestion, (score, terms, count)) ->
        { suggestion; suggestion_terms = terms; suggestion_score = score /. float_of_int count })
      (Text_ordered.to_list groups)
  in
  List.stable_sort (fun a b -> by_score a.suggestion_score b.suggestion_score) suggestions

(* ---------------------------------------------------------------------------------------------
   Serialization: [JSON.parse(JSON.stringify(index))]. *)

(* A number through JSON: non-finite numbers are [null], [-0] is [0]. *)
let json_number number =
  if Float.is_finite number then Json.number (if number = 0. then 0. else number) else Json.null

let json_value = function
  | Null -> Json.null
  | Bool value -> Json.bool value
  | Number number -> json_number number
  | String text -> Json.text text

let json_id = function Number_id number -> json_number number | String_id text -> Json.text text

(* An object keyed by short id: integer keys, so ascending. *)
let by_short_id vec value =
  let entries = ref [] in
  Vec.iteri
    (fun short_id slot ->
      match slot with Some item -> entries := (string_of_int short_id, value item) :: !entries | None -> ())
    vec;
  Json.obj (List.rev !entries)

let to_json index =
  let fields = index.options.field_names in
  let index_entries =
    List.map
      (fun (term, data) ->
        (* [Object.fromEntries] keyed by field id: integer keys, so ascending field id. *)
        let by_field = List.sort (fun (a, _) (b, _) -> Int.compare a b) data.postings in
        let postings =
          List.map
            (fun (field_id, posting) ->
              let pairs = ref [] in
              for at = posting.Posting.size - 1 downto 0 do
                pairs := (string_of_int posting.Posting.docs.(at), Json.int posting.Posting.freqs.(at)) :: !pairs
              done;
              (string_of_int field_id, Json.obj !pairs))
            by_field
        in
        Json.array [ Json.text term; Json.obj postings ])
      (Searchable_map.entries index.index)
  in
  let holes slots value = Json.array (Array.to_list (Array.map (function Some item -> value item | None -> Json.null) slots)) in
  Json.obj
    [
      ("documentCount", Json.int index.document_count);
      ("nextId", Json.int index.next_id);
      ("documentIds", by_short_id index.document_ids json_id);
      ("fieldIds", Json.obj (Array.to_list (Array.mapi (fun field_id name -> (name, Json.int field_id)) fields)));
      ("fieldLength", by_short_id index.field_length (fun lengths -> holes lengths Json.int));
      ("averageFieldLength", holes index.avg_field_length json_number);
      ( "storedFields",
        by_short_id index.stored_fields (fun stored -> Json.obj (List.map (fun (name, value) -> (name, json_value value)) stored)) );
      ("dirtCount", Json.int index.dirt_count);
      ("index", Json.array index_entries);
      ("serializationVersion", Json.int 2);
    ]
