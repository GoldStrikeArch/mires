(* Index options, search options and their merging.

   MiniSearch merges option objects with spread ([{ ...a, ...b }]): a key [b] holds replaces [a]'s
   whole value, so object options ([boost], [weights], [bm25]) are replaced, never merged key by
   key. Every search option is an [option] here, [None] meaning "not given", and [over] is that
   spread. *)

open Host

(* A JSON scalar a document holds in a field MiniSearch reads. *)
type value = Null | Bool of bool | Number of float | String of Text.t

(* A document id: the keys of MiniSearch's id map compare by SameValueZero, so [1] and ["1"] are
   different ids. *)
type doc_id = Number_id of float | String_id of Text.t

let doc_id_equal a b =
  match (a, b) with
  | Number_id a, Number_id b -> a = b
  | String_id a, String_id b -> Text.equal a b
  | Number_id _, String_id _ | String_id _, Number_id _ -> false

(* [prefix]: a boolean, or autoSuggest's default [(term, i, terms) => i === terms.length - 1]. *)
type prefix = Prefix_bool of bool | Prefix_last_term

(* [fuzzy]: a boolean or a number. *)
type fuzzy = Fuzzy_bool of bool | Fuzzy_number of float

type weights = { fuzzy_weight : float option; prefix_weight : float option }
type bm25 = { k : float; b : float; d : float }

(* The declarative [filter]: [(result) => values.includes(result[field])]. *)
type filter = { field : string; values : value list }

type search = {
  fields : string list option;
  boost : (string * float) list option;  (** the own keys of the [boost] object, with their values *)
  prefix : prefix option;
  fuzzy : fuzzy option;
  max_fuzzy : float option;
  combine_with : Text.t option;
  weights : weights option;
  bm25 : bm25 option;
  filter : filter option;
  boost_document : (doc_id * float) list option;
      (** the declarative [boostDocument]: [(id) => factor of the first pair for id, else 1] *)
  boost_term : (Text.t * float) list option;
      (** the declarative [boostTerm]: [(term) => factor of the first pair for term, else 1] *)
}

let no_options =
  {
    fields = None;
    boost = None;
    prefix = None;
    fuzzy = None;
    max_fuzzy = None;
    combine_with = None;
    weights = None;
    bm25 = None;
    filter = None;
    boost_document = None;
    boost_term = None;
  }

(* MiniSearch's [defaultSearchOptions]. *)
let library_defaults =
  {
    no_options with
    combine_with = Some (Text.of_string "or");
    prefix = Some (Prefix_bool false);
    fuzzy = Some (Fuzzy_bool false);
    max_fuzzy = Some 6.;
    boost = Some [];
    weights = Some { fuzzy_weight = Some 0.45; prefix_weight = Some 0.375 };
    bm25 = Some { k = 1.2; b = 0.7; d = 0.5 };
  }

(* MiniSearch's [defaultAutoSuggestOptions]. *)
let auto_suggest_defaults =
  { no_options with combine_with = Some (Text.of_string "and"); prefix = Some Prefix_last_term }

let pick top base = match top with Some _ -> top | None -> base

(* [{ ...base, ...top }]. *)
let over base top =
  {
    fields = pick top.fields base.fields;
    boost = pick top.boost base.boost;
    prefix = pick top.prefix base.prefix;
    fuzzy = pick top.fuzzy base.fuzzy;
    max_fuzzy = pick top.max_fuzzy base.max_fuzzy;
    combine_with = pick top.combine_with base.combine_with;
    weights = pick top.weights base.weights;
    bm25 = pick top.bm25 base.bm25;
    filter = pick top.filter base.filter;
    boost_document = pick top.boost_document base.boost_document;
    boost_term = pick top.boost_term base.boost_term;
  }

(* [value || fallback] for a number. *)
let or_default value fallback =
  match value with Some number when number <> 0. && not (Float.is_nan number) -> number | Some _ | None -> fallback

(* [getOwnProperty(options.boost, field) || 1]. *)
let field_boost options field =
  match options.boost with
  | None -> 1.
  | Some boosts -> (
      match List.find_opt (fun (name, _) -> String.equal name field) boosts with
      | Some (_, boost) -> or_default (Some boost) 1.
      | None -> 1.)

(* [autoVacuum] when it is not [false]. Missing (or zero) values fall back to the defaults where
   MiniSearch reads them ([batchSize || 1000], [minDirtCount || 20], [minDirtFactor || 0.1]). *)
type auto_vacuum = { batch_size : float option; min_dirt_count : float option; min_dirt_factor : float option }

(* [defaultAutoVacuumOptions]. *)
let default_auto_vacuum = { batch_size = Some 1000.; min_dirt_count = Some 20.; min_dirt_factor = Some 0.1 }

type index = {
  field_names : string array;  (** [fields]; a field's id is its position *)
  id_field : string;
  store_fields : string array;
  search_options : search;  (** [defaultSearchOptions] under the constructor's [searchOptions] *)
  auto_suggest_options : search;  (** [defaultAutoSuggestOptions] under the constructor's [autoSuggestOptions] *)
  auto_vacuum : auto_vacuum option;  (** [None] for [autoVacuum: false] *)
}
