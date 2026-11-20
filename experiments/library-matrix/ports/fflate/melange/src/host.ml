(* Melange host implementation for the shared fflate OCaml core.

   Melange strings are JavaScript UTF-16 strings and typed arrays map directly to JS Uint8Array,
   Uint16Array, and Int32Array via zero-cost [@@mel.new] / [@@mel.send] / [@@mel.get_index] /
   [@@mel.set_index] externals. *)

module U8 = struct
  type t = Js.Typed_array.Uint8Array.t

  external create : int -> t = "Uint8Array" [@@mel.new]
  external of_array : int array -> t = "Uint8Array" [@@mel.new]
  external length : t -> int = "length" [@@mel.get]
  external get : t -> int -> int = "" [@@mel.get_index]
  external set : t -> int -> int -> unit = "" [@@mel.set_index]
  external subarray : t -> int -> int -> t = "subarray" [@@mel.send]
  external set_from : t -> t -> int -> unit = "set" [@@mel.send]
  external copy_of : t -> t = "Uint8Array" [@@mel.new]
end

module U16 = struct
  type t = Js.Typed_array.Uint16Array.t

  external create : int -> t = "Uint16Array" [@@mel.new]
  external of_array : int array -> t = "Uint16Array" [@@mel.new]
  external length : t -> int = "length" [@@mel.get]
  external get : t -> int -> int = "" [@@mel.get_index]
  external set : t -> int -> int -> unit = "" [@@mel.set_index]
  external subarray : t -> int -> int -> t = "subarray" [@@mel.send]
end

module I32 = struct
  type t = Js.Typed_array.Int32Array.t

  external create : int -> t = "Int32Array" [@@mel.new]
  external of_array : int array -> t = "Int32Array" [@@mel.new]
  external length : t -> int = "length" [@@mel.get]
  external get : t -> int -> int = "" [@@mel.get_index]
  external set : t -> int -> int -> unit = "" [@@mel.set_index]
end

module Text = struct
  type t = string
  type text_encoder
  type text_decoder

  external make_text_encoder : unit -> text_encoder = "TextEncoder" [@@mel.new]
  external make_text_decoder : unit -> text_decoder = "TextDecoder" [@@mel.new]
  external te_encode : text_encoder -> t -> U8.t = "encode" [@@mel.send]
  external td_decode : text_decoder -> U8.t -> t = "decode" [@@mel.send]
  external from_char_code_apply : unit Js.null -> U8.t -> t = "apply" [@@mel.scope "String", "fromCharCode"]

  (* Stateless WHATWG UTF-8 codecs, matching fflate's module-level te / td instances. *)
  let text_encoder = make_text_encoder ()
  let text_decoder = make_text_decoder ()

  let empty = ""
  let length = String.length

  external unit_at : t -> int -> int = "charCodeAt" [@@mel.send]

  let append (a : t) (b : t) = a ^ b
  let encode_utf8 s = te_encode text_encoder s
  let decode_utf8 b = td_decode text_decoder b
  let from_u8_chunk chunk = from_char_code_apply Js.null chunk
end

module Value = struct
  type t = Js.Json.t

  external get_prop : t -> string -> t Js.nullable = "" [@@mel.get_index]
  external u8_of_json_array : t -> U8.t = "Uint8Array" [@@mel.new]

  let field obj name = Js.Nullable.toOption (get_prop obj name)

  let as_string_name v =
    match Js.Json.decodeString v with
    | Some s -> s
    | None -> invalid_arg "fflate port: expected string"

  let as_text = as_string_name

  let as_bool v =
    match Js.Json.decodeBoolean v with
    | Some b -> b
    | None -> invalid_arg "fflate port: expected boolean"

  let as_float v =
    match Js.Json.decodeNumber v with
    | Some x -> x
    | None -> invalid_arg "fflate port: expected number"

  let as_int v = int_of_float (as_float v)

  let as_array v =
    match Js.Json.decodeArray v with
    | Some a -> a
    | None -> invalid_arg "fflate port: expected array"

  let as_u8_from_int_array v =
    match Js.Json.decodeArray v with
    | Some _ -> u8_of_json_array v
    | None -> invalid_arg "fflate port: expected byte array"
end

external date_now : unit -> float = "now" [@@mel.scope "Date"]
