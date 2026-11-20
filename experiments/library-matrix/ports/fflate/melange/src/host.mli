(* The JavaScript host, as the shared fflate OCaml core sees it.

   Every module of this port except host.ml and the entry module (index.ml for Melange, main.ml
   for js_of_ocaml) is byte-identical between ports/fflate/melange/src/ and
   ports/fflate/jsoo/src/.

   It exposes typed arrays (Uint8Array, Uint16Array, Int32Array), JavaScript UTF-16 strings with
   TextEncoder / TextDecoder, in-place JSON input reading, and Date.now() for omitted GZIP mtime. *)

module U8 : sig
  type t

  val create : int -> t
  val of_array : int array -> t
  val length : t -> int
  val get : t -> int -> int
  val set : t -> int -> int -> unit
  val subarray : t -> int -> int -> t
  val set_from : t -> t -> int -> unit
  val copy_of : t -> t
end

module U16 : sig
  type t

  val create : int -> t
  val of_array : int array -> t
  val length : t -> int
  val get : t -> int -> int
  val set : t -> int -> int -> unit
  val subarray : t -> int -> int -> t
end

module I32 : sig
  type t

  val create : int -> t
  val of_array : int array -> t
  val length : t -> int
  val get : t -> int -> int
  val set : t -> int -> int -> unit
end

module Text : sig
  type t

  val empty : t
  val length : t -> int
  val unit_at : t -> int -> int
  val append : t -> t -> t
  val encode_utf8 : t -> U8.t
  val decode_utf8 : U8.t -> t
  val from_u8_chunk : U8.t -> t
end

module Value : sig
  type t

  val field : t -> string -> t option
  val as_string_name : t -> string
  val as_text : t -> Text.t
  val as_bool : t -> bool
  val as_int : t -> int
  val as_float : t -> float
  val as_array : t -> t array
  val as_u8_from_int_array : t -> U8.t
end

val date_now : unit -> float
