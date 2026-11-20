(* Melange host implementation for the shared marked@18.1.1 OCaml core.

   Melange strings are native JS UTF-16 strings, Vec wraps JS Array, Re wraps JS RegExp with
   zero-allocation nullable match inspection and pre-wrapped uncurried replacers, and StrMap
   wraps JS Map via [@@mel.new] / [@@mel.send] / [@@mel.get] / [@@mel.set] / [@@mel.get_index] /
   [@@mel.set_index] / [@@mel.scope] externals. *)

module Vec = struct
  type 'a t

  external create : unit -> 'a t = "Array" [@@mel.new]
  external length : 'a t -> int = "length" [@@mel.get]
  external get : 'a t -> int -> 'a = "" [@@mel.get_index]
  external set : 'a t -> int -> 'a -> unit = "" [@@mel.set_index]
  external push : 'a t -> 'a -> unit = "push" [@@mel.send]
  external pop_unit : 'a t -> unit = "pop" [@@mel.send]
  external unshift : 'a t -> 'a -> unit = "unshift" [@@mel.send]
  external slice : 'a t -> int -> int -> 'a t = "slice" [@@mel.send]
  external slice_from : 'a t -> int -> 'a t = "slice" [@@mel.send]
end

module Jstr = struct
  type t = string

  let of_string (s : string) : t = s
  let empty = ""
  let length = String.length

  external code_unit_at : t -> int -> int = "charCodeAt" [@@mel.send]

  let equal = String.equal
  let concat (a : t) (b : t) = a ^ b
  let concat3 (a : t) (b : t) (c : t) = a ^ b ^ c
  let concat4 (a : t) (b : t) (c : t) (d : t) = a ^ b ^ c ^ d

  external slice : t -> int -> int -> t = "slice" [@@mel.send]
  external slice_from : t -> int -> t = "slice" [@@mel.send]
  external substring : t -> int -> int -> t = "substring" [@@mel.send]
  external substring_from : t -> int -> t = "substring" [@@mel.send]
  external trim : t -> t = "trim" [@@mel.send]
  external trim_start : t -> t = "trimStart" [@@mel.send]
  external trim_end : t -> t = "trimEnd" [@@mel.send]
  external to_lower : t -> t = "toLowerCase" [@@mel.send]
  external to_upper : t -> t = "toUpperCase" [@@mel.send]
  external includes : t -> t -> bool = "includes" [@@mel.send]
  external index_of : t -> t -> int = "indexOf" [@@mel.send]
  external index_of_from : t -> t -> int -> int = "indexOf" [@@mel.send]
  external last_index_of : t -> t -> int = "lastIndexOf" [@@mel.send]
  external ends_with : t -> t -> bool = "endsWith" [@@mel.send]
  external repeat : t -> int -> t = "repeat" [@@mel.send]
  external split_str : t -> t -> t Vec.t = "split" [@@mel.send]
  external join : t Vec.t -> t -> t = "join" [@@mel.send]
  external replace_str_lit : t -> t -> t -> t = "replace" [@@mel.send]
  external from_code_point : int -> t = "fromCodePoint" [@@mel.scope "String"]
  external of_int : int -> t = "String"
  external parse_int : t -> int -> int = "parseInt" [@@mel.scope "Number"]
  external encode_uri_raw : t -> t = "encodeURI"

  let encode_uri s = try Some (encode_uri_raw s) with _ -> None
end

module Re = struct
  type t
  type raw_match
  type match_result = raw_match Js.nullable
  type replacer1 = (Jstr.t -> Jstr.t [@u])
  type replacer3 = (Jstr.t -> Jstr.t Js.undefined -> Jstr.t Js.undefined -> Jstr.t [@u])

  external create : Jstr.t -> Jstr.t -> t = "RegExp" [@@mel.new]

  let of_string pat flags = create pat flags

  external source : t -> Jstr.t = "source" [@@mel.get]
  external test : t -> Jstr.t -> bool = "test" [@@mel.send]
  external search : Jstr.t -> t -> int = "search" [@@mel.send]
  external exec_raw : t -> Jstr.t -> match_result = "exec" [@@mel.send]

  let is_match (m : match_result) = not (Js.Nullable.isNullable m)

  external set_last_index : t -> int -> unit = "lastIndex" [@@mel.set]
  external match_index : match_result -> int = "index" [@@mel.get]
  external match_at : match_result -> int -> Jstr.t = "" [@@mel.get_index]
  external match_at_undef : match_result -> int -> Jstr.t Js.undefined = "" [@@mel.get_index]

  let match_full m = match_at m 0

  let match_group_or_empty m i =
    let g = match_at_undef m i in
    if Js.Undefined.testAny g then Jstr.empty else match_at m i

  let match_group_is_nonempty m i =
    let g = match_at_undef m i in
    (not (Js.Undefined.testAny g)) && String.length (match_at m i) > 0

  external split : Jstr.t -> t -> Jstr.t Vec.t = "split" [@@mel.send]
  external replace_str : Jstr.t -> t -> Jstr.t -> Jstr.t = "replace" [@@mel.send]

  let make_replacer1 (f : Jstr.t -> Jstr.t) : replacer1 = fun [@u] m0 -> f m0

  external replace_with1 : Jstr.t -> t -> replacer1 -> Jstr.t = "replace" [@@mel.send]

  let replace_fn1 s re f = replace_with1 s re (fun [@u] m0 -> f m0)

  let make_replacer3 (f : Jstr.t -> Jstr.t -> Jstr.t -> Jstr.t) : replacer3 =
   fun [@u] m0 g1 g2 ->
    let s1 =
      if Js.Undefined.testAny g1 then Jstr.empty else Js.Undefined.getUnsafe g1
    in
    let s2 =
      if Js.Undefined.testAny g2 then Jstr.empty else Js.Undefined.getUnsafe g2
    in
    f m0 s1 s2

  external replace_with3 : Jstr.t -> t -> replacer3 -> Jstr.t = "replace" [@@mel.send]
end

module StrMap = struct
  type 'a t

  external create : unit -> 'a t = "Map" [@@mel.new]
  external size : 'a t -> int = "size" [@@mel.get]
  external has : 'a t -> Jstr.t -> bool = "has" [@@mel.send]
  external get_exn : 'a t -> Jstr.t -> 'a = "get" [@@mel.send]
  external set : 'a t -> Jstr.t -> 'a -> unit = "set" [@@mel.send]
end

module Value = struct
  type t = Js.Json.t

  external get_prop : t -> string -> t Js.nullable = "" [@@mel.get_index]

  let field obj name = Js.Nullable.toOption (get_prop obj name)

  let as_string_name v =
    match Js.Json.decodeString v with
    | Some s -> s
    | None -> invalid_arg "marked port: expected string"

  let as_jstr = as_string_name

  let as_bool v =
    match Js.Json.decodeBoolean v with
    | Some b -> b
    | None -> invalid_arg "marked port: expected boolean"
end
