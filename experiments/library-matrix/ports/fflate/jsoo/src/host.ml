(* js_of_ocaml host implementation for the shared fflate OCaml core.

   Text stays a JavaScript UTF-16 string (Js.js_string Js.t), typed arrays map directly to
   Typed_array.uint8Array / uint16Array / int32Array with zero-overhead indexed reads/writes,
   and outputs return directly as JS Uint8Array with zero copy. *)

open Js_of_ocaml

module U8 = struct
  type t = Typed_array.uint8Array Js.t

  let create n = new%js Typed_array.uint8Array n
  let of_array arr = new%js Typed_array.uint8Array_fromArray (Js.array arr)
  let length (a : t) = a##.length
  let get (a : t) (i : int) : int = Js.Unsafe.get a i
  let set (a : t) (i : int) (v : int) = Js.Unsafe.set a i v
  let subarray (a : t) s e = a##subarray s e
  let set_from (dst : t) (src : t) off = dst##set_fromTypedArray src off
  let copy_of (a : t) = new%js Typed_array.uint8Array_fromTypedArray a
end

module U16 = struct
  type t = Typed_array.uint16Array Js.t

  let create n = new%js Typed_array.uint16Array n
  let of_array arr = new%js Typed_array.uint16Array_fromArray (Js.array arr)
  let length (a : t) = a##.length
  let get (a : t) (i : int) : int = Js.Unsafe.get a i
  let set (a : t) (i : int) (v : int) = Js.Unsafe.set a i v
  let subarray (a : t) s e = a##subarray s e
end

module I32 = struct
  type t = Typed_array.int32Array Js.t

  let create n = new%js Typed_array.int32Array n

  let of_array (arr : int array) =
    new%js Typed_array.int32Array_fromArray (Js.Unsafe.coerce (Js.array arr))

  let length (a : t) = a##.length
  let get (a : t) (i : int) : int = Js.Unsafe.get a i
  let set (a : t) (i : int) (v : int) = Js.Unsafe.set a i v
end

module Text = struct
  type t = Js.js_string Js.t

  class type code_units = object
    method charCodeAt : int -> int Js.meth
  end

  class type text_encoder = object
    method encode : t -> U8.t Js.meth
  end

  class type text_decoder = object
    method decode : U8.t -> t Js.meth
  end

  class type from_char_code_fn = object
    method apply : unit Js.opt -> U8.t -> t Js.meth
  end

  let te_constr : text_encoder Js.t Js.constr = Js.Unsafe.global##._TextEncoder
  let td_constr : text_decoder Js.t Js.constr = Js.Unsafe.global##._TextDecoder
  let from_char_code : from_char_code_fn Js.t = (Js.Unsafe.global##._String)##.fromCharCode

  (* Stateless WHATWG UTF-8 codecs, matching fflate's module-level te / td instances. *)
  let text_encoder = new%js te_constr
  let text_decoder = new%js td_constr

  let empty = Js.string ""
  let length (s : t) = s##.length
  let unit_at (s : t) (i : int) = (Js.Unsafe.coerce s : code_units Js.t)##charCodeAt i
  let append (a : t) (b : t) = a##concat b
  let encode_utf8 s = text_encoder##encode s
  let decode_utf8 b = text_decoder##decode b
  let from_u8_chunk chunk = from_char_code##apply Js.null chunk
end

module Value = struct
  type t = Js.Unsafe.top Js.t

  class type array_constructor = object
    method isArray : t -> bool Js.t Js.meth
  end

  let array_constructor : array_constructor Js.t = Js.Unsafe.global##._Array
  let ty_string = Js.string "string"
  let ty_boolean = Js.string "boolean"
  let ty_number = Js.string "number"

  let field (obj : t) name : t option =
    let v : t Js.optdef = Js.Unsafe.get obj (Js.string name) in
    match Js.Optdef.to_option v with
    | None -> None
    | Some x -> if Js.Unsafe.equals x Js.null then None else Some x

  let as_text (v : t) : Text.t =
    if Js.strict_equals (Js.typeof v) ty_string then Js.Unsafe.coerce v
    else invalid_arg "fflate port: expected string"

  let as_string_name (v : t) = Js.to_string (as_text v)

  let as_bool (v : t) =
    if Js.strict_equals (Js.typeof v) ty_boolean then Js.to_bool (Js.Unsafe.coerce v)
    else invalid_arg "fflate port: expected boolean"

  let as_float (v : t) =
    if Js.strict_equals (Js.typeof v) ty_number then Js.float_of_number (Js.Unsafe.coerce v)
    else invalid_arg "fflate port: expected number"

  let as_int (v : t) = int_of_float (as_float v)

  let as_array (v : t) : t array =
    if Js.to_bool (array_constructor##isArray v) then Js.to_array (Js.Unsafe.coerce v)
    else invalid_arg "fflate port: expected array"

  let as_u8_from_int_array (v : t) : U8.t =
    if Js.to_bool (array_constructor##isArray v) then
      new%js Typed_array.uint8Array_fromArray (Js.Unsafe.coerce v)
    else invalid_arg "fflate port: expected byte array"
end

let date_now () = Js.float_of_number Js.date##now
