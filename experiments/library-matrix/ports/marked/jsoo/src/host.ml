(* js_of_ocaml host implementation for the shared marked@18.1.1 OCaml core.

   Markdown and HTML stay Js.js_string Js.t (UTF-16) throughout, Vec wraps Js.js_array Js.t,
   Re wraps Js.regExp Js.t with zero-allocation nullable match inspection and pre-wrapped
   strict-arity replacers, and StrMap wraps JS Map with zero-argument constructor typing. *)

open Js_of_ocaml

module Vec = struct
  type 'a t = 'a Js.js_array Js.t

  let create () = new%js Js.array_empty
  let length (a : 'a t) = a##.length
  let get (a : 'a t) (i : int) : 'a = Js.Unsafe.get a i
  let set (a : 'a t) (i : int) (v : 'a) = Js.Unsafe.set a i v
  let push (a : 'a t) (v : 'a) = ignore (a##push v)
  let pop_unit (a : 'a t) = ignore a##pop
  let unshift (a : 'a t) (v : 'a) = ignore (a##unshift v)
  let slice (a : 'a t) s e = a##slice s e
  let slice_from (a : 'a t) s = a##slice_end s
end

module Jstr = struct
  type t = Js.js_string Js.t

  class type ext_js_string = object
    inherit Js.js_string
    method charCodeAt_int : int -> int Js.meth
    method trimStart : Js.js_string Js.t Js.meth
    method trimEnd : Js.js_string Js.t Js.meth
    method includes : Js.js_string Js.t -> bool Js.t Js.meth
    method endsWith : Js.js_string Js.t -> bool Js.t Js.meth
    method repeat : int -> Js.js_string Js.t Js.meth
  end

  class type string_ext_constr = object
    method fromCodePoint : int -> Js.js_string Js.t Js.meth
  end

  class type number_ext_constr = object
    method parseInt : Js.js_string Js.t -> int -> int Js.meth
  end

  class type global_uri = object
    method encodeURI : Js.js_string Js.t -> Js.js_string Js.t Js.meth
  end

  let string_ext_constr : string_ext_constr Js.t = Js.Unsafe.global##._String
  let number_ext_constr : number_ext_constr Js.t = Js.Unsafe.global##._Number
  let global_uri : global_uri Js.t = Js.Unsafe.global
  let ext (s : t) : ext_js_string Js.t = Js.Unsafe.coerce s
  let of_string = Js.string
  let empty = Js.string ""
  let length (s : t) = s##.length
  let code_unit_at (s : t) (i : int) = (ext s)##charCodeAt_int i
  let equal (a : t) (b : t) = Js.strict_equals a b
  let concat (a : t) (b : t) = a##concat b
  let concat3 (a : t) (b : t) (c : t) = a##concat_2 b c
  let concat4 (a : t) (b : t) (c : t) (d : t) = a##concat_3 b c d
  let slice (s : t) start stop = s##slice start stop
  let slice_from (s : t) start = s##slice_end start
  let substring (s : t) start stop = s##substring start stop
  let substring_from (s : t) start = s##substring_toEnd start
  let trim (s : t) = s##trim
  let trim_start (s : t) = (ext s)##trimStart
  let trim_end (s : t) = (ext s)##trimEnd
  let to_lower (s : t) = s##toLowerCase
  let to_upper (s : t) = s##toUpperCase
  let includes (s : t) (sub : t) = Js.to_bool ((ext s)##includes sub)
  let index_of (s : t) (sub : t) = s##indexOf sub
  let index_of_from (s : t) (sub : t) (from : int) = s##indexOf_from sub from
  let last_index_of (s : t) (sub : t) = s##lastIndexOf sub
  let ends_with (s : t) (sub : t) = Js.to_bool ((ext s)##endsWith sub)
  let repeat (s : t) (n : int) = (ext s)##repeat n
  let split_str (s : t) (sep : t) = Js.str_array (s##split sep)
  let join (a : t Vec.t) (sep : t) = a##join sep
  let replace_str_lit (s : t) (search : t) (repl : t) = s##replace_string search repl
  let from_code_point (cp : int) = string_ext_constr##fromCodePoint cp
  let of_int (n : int) = (Js.number_of_float (float_of_int n))##toString
  let parse_int (s : t) (radix : int) = number_ext_constr##parseInt s radix
  let encode_uri (s : t) = try Some (global_uri##encodeURI s) with _ -> None
end

module Re = struct
  type t = Js.regExp Js.t
  type match_result = Js.match_result_handle Js.t Js.opt
  type replacer1 = (unit, Js.js_string Js.t -> Js.js_string Js.t) Js.meth_callback

  type replacer3 =
    ( unit,
      Js.js_string Js.t ->
      Js.js_string Js.t Js.optdef ->
      Js.js_string Js.t Js.optdef ->
      Js.js_string Js.t )
    Js.meth_callback

  class type replace_string = object
    method replace_fn1 : Js.regExp Js.t -> replacer1 -> Js.js_string Js.t Js.meth
    method replace_fn3 : Js.regExp Js.t -> replacer3 -> Js.js_string Js.t Js.meth
  end

  let key_index = Js.string "index"
  let create pat flags = new%js Js.regExp_withFlags pat flags
  let of_string pat flags = new%js Js.regExp_withFlags (Js.string pat) (Js.string flags)
  let source (re : t) = re##.source
  let test (re : t) (s : Jstr.t) = Js.to_bool (re##test s)
  let search (s : Jstr.t) (re : t) = s##search re
  let exec_raw (re : t) (s : Jstr.t) : match_result = re##exec s
  let is_match (m : match_result) = Js.Opt.test m
  let set_last_index (re : t) (i : int) = re##.lastIndex := i
  let match_index (m : match_result) : int = Js.Unsafe.get m key_index
  let match_full (m : match_result) : Jstr.t = Js.Unsafe.get m 0

  let match_group_or_empty (m : match_result) (i : int) : Jstr.t =
    let g : Jstr.t Js.optdef = Js.Unsafe.get m i in
    if Js.Optdef.test g then Js.Unsafe.get m i else Jstr.empty

  let match_group_is_nonempty (m : match_result) (i : int) : bool =
    let g : Jstr.t Js.optdef = Js.Unsafe.get m i in
    if Js.Optdef.test g then (Js.Unsafe.get m i : Jstr.t)##.length > 0 else false

  let split (s : Jstr.t) (re : t) = Js.str_array (s##split_regExp re)
  let replace_str (s : Jstr.t) (re : t) (repl : Jstr.t) = s##replace re repl
  let make_replacer1 (f : Jstr.t -> Jstr.t) : replacer1 = Js.Unsafe.callback_with_arity 1 f

  let replace_with1 (s : Jstr.t) (re : t) (r : replacer1) =
    (Js.Unsafe.coerce s : replace_string Js.t)##replace_fn1 re r

  let replace_fn1 (s : Jstr.t) (re : t) (f : Jstr.t -> Jstr.t) =
    (Js.Unsafe.coerce s : replace_string Js.t)##replace_fn1 re (Js.Unsafe.callback_with_arity 1 f)

  let empty_thunk () = Jstr.empty

  let make_replacer3 (f : Jstr.t -> Jstr.t -> Jstr.t -> Jstr.t) : replacer3 =
    Js.Unsafe.callback_with_arity 3 (fun m0 g1 g2 ->
        let s1 = Js.Optdef.get g1 empty_thunk in
        let s2 = Js.Optdef.get g2 empty_thunk in
        f m0 s1 s2)

  let replace_with3 (s : Jstr.t) (re : t) (r : replacer3) =
    (Js.Unsafe.coerce s : replace_string Js.t)##replace_fn3 re r
end

module StrMap = struct
  class type ['a] js_map = object
    method size : int Js.readonly_prop
    method has : Js.js_string Js.t -> bool Js.t Js.meth
    method get : Js.js_string Js.t -> 'a Js.meth
    method set : Js.js_string Js.t -> 'a -> unit Js.meth
  end

  type 'a t = 'a js_map Js.t

  let map_constr : Js.Unsafe.any js_map Js.t Js.constr = Js.Unsafe.global##._Map
  let create () = Js.Unsafe.coerce (new%js map_constr)
  let size (m : 'a t) = m##.size
  let has (m : 'a t) (k : Jstr.t) = Js.to_bool (m##has k)
  let get_exn (m : 'a t) (k : Jstr.t) = m##get k
  let set (m : 'a t) (k : Jstr.t) (v : 'a) = m##set k v
end

module Value = struct
  type t = Js.Unsafe.top Js.t

  let ty_string = Js.string "string"
  let ty_boolean = Js.string "boolean"

  let field (obj : t) name : t option =
    let v : t Js.optdef = Js.Unsafe.get obj (Js.string name) in
    match Js.Optdef.to_option v with
    | None -> None
    | Some x -> if Js.Unsafe.equals x Js.null then None else Some x

  let as_jstr (v : t) : Jstr.t =
    if Js.strict_equals (Js.typeof v) ty_string then Js.Unsafe.coerce v
    else invalid_arg "marked port: expected string"

  let as_string_name (v : t) = Js.to_string (as_jstr v)

  let as_bool (v : t) =
    if Js.strict_equals (Js.typeof v) ty_boolean then Js.to_bool (Js.Unsafe.coerce v)
    else invalid_arg "marked port: expected boolean"
end
