(* The JavaScript host, as the port's core sees it.

   The core (every module of this port except this one's implementation and the entry module) is
   the same OCaml source in the Melange and the js_of_ocaml port. This interface is shared too;
   only host.ml differs: Melange externals in one directory, js_of_ocaml's Js API in the other.

   It holds exactly what the core needs from JavaScript: the string primitives MiniSearch's code
   uses (strings are JavaScript strings, UTF-16 code units, never OCaml byte strings), the default
   tokenizer and term processor, which are host calls in MiniSearch too, and the two sides of the
   boundary (reading the corpus input in place, building the result). *)

(** A JavaScript string: a sequence of UTF-16 code units, lone surrogates included. *)
module Text : sig
  type t

  val of_string : string -> t
  (** An OCaml string as a JavaScript string: an ASCII literal, or a name read by {!Value.name}. *)

  val length : t -> int
  (** [s.length]: the number of code units. *)

  val unit_at : t -> int -> int
  (** [s.charCodeAt(i)], for [0 <= i < length s]. *)

  val slice : t -> int -> int -> t
  (** [s.slice(start, end_)], for [0 <= start <= end_ <= length s]. *)

  val append : t -> t -> t
  (** [a + b]. *)

  val equal : t -> t -> bool
  (** [a === b]. *)

  val lower : t -> t
  (** [s.toLowerCase()]: MiniSearch's default [processTerm]. *)

  val split : t -> t array
  (** [s.split(/[\n\r\p{Z}\p{P}]+/u)]: MiniSearch's default [tokenize]. *)

  val of_number : float -> t
  (** [String(x)]: what [x.toString()] and a template literal's [${x}] write. *)
end

(** A value of the input: a JSON-compatible JavaScript value, read in place, never written. *)
module Value : sig
  type t

  type view = Null | Bool of bool | Number of float | String of Text.t | Array of t array | Object

  val view : t -> view

  val field : t -> string -> t option
  (** [o[name]] of an object, [None] when it is [undefined]. *)

  val keys : t -> string array
  (** [Object.keys(o)] of an object, as names (see {!name}). *)

  val name : t -> string
  (** A string holding a name (a field, a stored field, an option, an operation) as an OCaml
      string. Names are never indexed text: the parity domain makes them ASCII identifiers. *)
end

(** A value of the result, built as JavaScript values. *)
module Json : sig
  type t

  val null : t
  val bool : bool -> t
  val int : int -> t
  val number : float -> t
  val text : Text.t -> t
  val array : t list -> t

  val obj : (string * t) list -> t
  (** A plain object with these keys, set in this order. *)
end
