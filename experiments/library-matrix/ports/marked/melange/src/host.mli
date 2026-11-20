(* The JavaScript host, as the shared marked@18.1.1 OCaml core sees it.

   Every module of this port except host.ml and the entry module (index.ml for Melange, main.ml
   for js_of_ocaml) is byte-identical between ports/marked/melange/src/ and
   ports/marked/jsoo/src/.

   Markdown and HTML stay native JavaScript UTF-16 strings (Jstr.t) end-to-end with zero UTF-8
   round-tripping, regexes bind directly to host JS RegExp (with zero-allocation match inspection
   and pre-wrapped stateless replacers), dynamic arrays bind to JS Array (Vec), and link
   reference tables bind to JS Map (StrMap). *)

module Vec : sig
  type 'a t

  val create : unit -> 'a t
  val length : 'a t -> int
  val get : 'a t -> int -> 'a
  val set : 'a t -> int -> 'a -> unit
  val push : 'a t -> 'a -> unit
  val pop_unit : 'a t -> unit
  val unshift : 'a t -> 'a -> unit
  val slice : 'a t -> int -> int -> 'a t
  val slice_from : 'a t -> int -> 'a t
end

module Jstr : sig
  type t

  val of_string : string -> t
  (** Pure ASCII compile-time literal to JS string. Never pass non-ASCII bytes here; use
      {!from_code_point} for Unicode code points so Melange and js_of_ocaml agree identically. *)

  val empty : t
  val length : t -> int
  val code_unit_at : t -> int -> int
  val equal : t -> t -> bool
  val concat : t -> t -> t
  val concat3 : t -> t -> t -> t
  val concat4 : t -> t -> t -> t -> t
  val slice : t -> int -> int -> t
  val slice_from : t -> int -> t
  val substring : t -> int -> int -> t
  val substring_from : t -> int -> t
  val trim : t -> t
  val trim_start : t -> t
  val trim_end : t -> t
  val to_lower : t -> t
  val to_upper : t -> t
  val includes : t -> t -> bool
  val index_of : t -> t -> int
  val index_of_from : t -> t -> int -> int
  val last_index_of : t -> t -> int
  val ends_with : t -> t -> bool
  val repeat : t -> int -> t
  val split_str : t -> t -> t Vec.t
  val join : t Vec.t -> t -> t
  val replace_str_lit : t -> t -> t -> t
  val from_code_point : int -> t
  val of_int : int -> t
  val parse_int : t -> int -> int
  val encode_uri : t -> t option
end

module Re : sig
  type t
  type match_result
  type replacer1
  type replacer3

  val create : Jstr.t -> Jstr.t -> t
  val of_string : string -> string -> t
  val source : t -> Jstr.t
  val test : t -> Jstr.t -> bool
  val search : Jstr.t -> t -> int
  val exec_raw : t -> Jstr.t -> match_result
  val is_match : match_result -> bool
  val set_last_index : t -> int -> unit
  val match_index : match_result -> int
  val match_full : match_result -> Jstr.t
  val match_group_or_empty : match_result -> int -> Jstr.t
  val match_group_is_nonempty : match_result -> int -> bool
  val split : Jstr.t -> t -> Jstr.t Vec.t
  val replace_str : Jstr.t -> t -> Jstr.t -> Jstr.t
  val make_replacer1 : (Jstr.t -> Jstr.t) -> replacer1
  val replace_with1 : Jstr.t -> t -> replacer1 -> Jstr.t
  val replace_fn1 : Jstr.t -> t -> (Jstr.t -> Jstr.t) -> Jstr.t

  val make_replacer3 : (Jstr.t -> Jstr.t -> Jstr.t -> Jstr.t) -> replacer3
  (** Wraps a 3-arg regex replacer once; undefined capture groups arrive as [Jstr.empty]. *)

  val replace_with3 : Jstr.t -> t -> replacer3 -> Jstr.t
end

module StrMap : sig
  type 'a t

  val create : unit -> 'a t
  val size : 'a t -> int
  val has : 'a t -> Jstr.t -> bool
  val get_exn : 'a t -> Jstr.t -> 'a
  val set : 'a t -> Jstr.t -> 'a -> unit
end

module Value : sig
  type t

  val field : t -> string -> t option
  val as_string_name : t -> string
  val as_jstr : t -> Jstr.t
  val as_bool : t -> bool
end
