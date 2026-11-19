(* The host for js_of_ocaml. Text stays a JavaScript string ([Js.js_string Js.t]) from the input
   to the result and is never converted to an OCaml string: [Js.to_string] would turn the UTF-16
   into UTF-8, losing lone surrogates and changing every length, slice and edit distance
   (CONTRACT.md, "Strings are UTF-16"). The string primitives are js_of_ocaml's typed method
   calls; the input is read with [Js.typeof] and [Js.Unsafe.get], the result built with
   [Js.Unsafe.obj] and [Js.array]. Only names (fields, options) become OCaml strings. *)

open Js_of_ocaml

module Text = struct
  type t = Js.js_string Js.t

  (* [charCodeAt] returning an [int]: a code unit is an integer, which is what a js_of_ocaml
     [int] is at run time (a JavaScript number). [Js.js_string]'s own binding returns a
     [Js.number]. *)
  class type code_units = object
    method charCodeAt : int -> int Js.meth
  end

  let of_string = Js.string
  let length (text : t) = text##.length
  let unit_at (text : t) index = (Js.Unsafe.coerce text : code_units Js.t)##charCodeAt index
  let slice (text : t) start end_ = text##slice start end_
  let append (a : t) (b : t) = a##concat b
  let equal (a : t) (b : t) = Js.strict_equals a b
  let lower (text : t) = text##toLowerCase

  (* MiniSearch's SPACE_OR_PUNCTUATION, a constant of its module as here. [split] does not use
     or change its [lastIndex] (the [u] flag alone makes it neither global nor sticky). *)
  let space_or_punctuation = new%js Js.regExp_withFlags (Js.string "[\\n\\r\\p{Z}\\p{P}]+") (Js.string "u")
  let split (text : t) = Js.to_array (Js.str_array (text##split_regExp space_or_punctuation))
  let of_number number = (Js.number_of_float number)##toString
end

module Value = struct
  type t = Js.Unsafe.top Js.t
  type view = Null | Bool of bool | Number of float | String of Text.t | Array of t array | Object

  class type array_constructor = object
    method isArray : t -> bool Js.t Js.meth
  end

  let array_constructor : array_constructor Js.t = Js.Unsafe.global##._Array

  let view (value : t) =
    match Js.to_string (Js.typeof value) with
    | "string" -> String (Js.Unsafe.coerce value)
    | "number" -> Number (Js.float_of_number (Js.Unsafe.coerce value))
    | "boolean" -> Bool (Js.to_bool (Js.Unsafe.coerce value))
    | _ ->
        if Js.Unsafe.equals value Js.null then Null
        else if Js.to_bool (array_constructor##isArray value) then Array (Js.to_array (Js.Unsafe.coerce value))
        else Object

  let field (value : t) name : t option = Js.Optdef.to_option (Js.Unsafe.get value (Js.string name))
  let keys (value : t) = Array.map Js.to_string (Js.to_array (Js.object_keys value))

  let name value =
    match view value with String text -> Js.to_string text | _ -> invalid_arg "MiniSearch port: a name was expected"
end

module Json = struct
  type t = Js.Unsafe.any

  let null = Js.Unsafe.inject Js.null
  let bool value = Js.Unsafe.inject (Js.bool value)
  let int (number : int) = Js.Unsafe.inject number
  let number (number : float) = Js.Unsafe.inject (Js.number_of_float number)
  let text (text : Text.t) = Js.Unsafe.inject text
  let array items = Js.Unsafe.inject (Js.array (Array.of_list items))
  let obj fields : t = Js.Unsafe.obj (Array.of_list fields)
end
