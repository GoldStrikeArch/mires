(** The host APIs Ky drives: Fetch ([Headers], [Request], [Response]), [AbortController] and
    [AbortSignal], [TextDecoder], timers, promises, [JSON], [RegExp], the clock, and the JS string
    operations Ky applies to header values. Ky is a thin layer over these, so their semantics
    (method and URL normalisation, header validation, body encoding, encoding labels, regular
    expressions) are the engine's own. A call that can throw returns the thrown value as [Error].

    This interface is shared, byte-identical, by the Melange and js_of_ocaml ports. Each has its
    own [host.ml]. *)

(** {1 Values} *)

type value
(** Any JS value: an option value, a header value, parsed JSON, a thrown error. *)

val null : value
val of_string : string -> value
val of_int : int -> value
val of_float : float -> value

val array : value list -> value
(** A JS array. *)

val get : value -> string -> value

val field : value -> string -> value option
(** A property that is not [undefined]. *)

val keys : value -> string list
(** [Object.keys]. *)

val to_string : value -> string option
val to_number : value -> float option
val to_boolean : value -> bool option

val to_list : value -> value list option
(** The elements of an array ([Array.isArray]). *)

val is_integer : value -> bool
(** [Number.isInteger]. *)

val stringify : value -> string
(** [JSON.stringify]. *)

val parse : string -> (value, value) result
(** [JSON.parse], or the SyntaxError it throws. *)

(** {1 Errors} *)

val is_error : value -> bool
val error_name : value -> string
val error_message : value -> string option
val has_stack : value -> bool
val has_property : value -> string -> bool
val new_error : string -> value
val type_error : string -> value
val range_error : string -> value
val set_name : value -> string -> unit

(** {1 Strings and numbers, with JS semantics} *)

val lowercase : string -> string
(** [String.prototype.toLowerCase]. *)

val uppercase : string -> string
(** [String.prototype.toUpperCase]. *)

val trim : string -> string
(** [String.prototype.trim]. *)

val number : string -> float
(** [Number(text)]. *)

type regexp

val regexp : string -> string -> regexp
(** [new RegExp(pattern, flags)]. *)

val exec : regexp -> string -> string option array option
(** The match and its groups, [None] for a group that did not take part. *)

val test : regexp -> string -> bool
val math_min : float -> float -> float
val math_max : float -> float -> float
val math_pow : float -> float -> float
val now : unit -> float

(** {1 Promises and timers} *)

type 'a promise

val resolve : 'a -> 'a promise
val reject : value -> 'a promise
val bind : 'a promise -> ('a -> 'b promise) -> 'b promise

val settle : 'a promise -> ('a, value) result promise
(** The promise's value, or its rejection reason as [Error]. *)

val promise : (('a -> unit) -> (value -> unit) -> unit) -> 'a promise
(** [new Promise(executor)]. *)

val to_value : value promise -> value
(** The promise, as the JS value it is. *)

type timer

val set_timeout : (unit -> unit) -> float -> timer
val clear_timeout : timer -> unit
val sleep : float -> unit promise

(** {1 Aborting} *)

type abort_signal
type abort_controller

val abort_controller : unit -> abort_controller
val controller_signal : abort_controller -> abort_signal

val abort : abort_controller -> unit
(** [abort()], whose reason is the platform's AbortError [DOMException]. *)

val abort_with : abort_controller -> value -> unit

val any_signal : abort_signal list -> abort_signal
(** [AbortSignal.any]: aborted as soon as one of the signals is, with that one's reason. *)

val aborted : abort_signal -> bool
val reason : abort_signal -> value

val throw_if_aborted : abort_signal -> (unit, value) result
(** [throwIfAborted()]: the reason, as the error it throws. *)

val on_abort : abort_signal -> (unit -> unit) -> unit
(** Calls the listener once, when the signal aborts. *)

(** {1 Fetch} *)

type headers

val headers : unit -> headers

val set_header : headers -> string -> value -> (unit, value) result
(** [Headers.set]: it validates the name and the value (a TypeError) and stores the value's
    ByteString conversion, so any JSON value is accepted as JS accepts it. *)

val header : headers -> string -> string option

type request

val request :
  string ->
  method_:string ->
  headers:headers ->
  body:string option ->
  signal:abort_signal ->
  (request, value) result
(** [new Request(input, {method, headers, body, signal})], or the TypeError it throws. *)

val with_signal : request -> abort_signal -> (request, value) result
(** [new Request(request, {signal})]. *)

val clone_request : request -> (request, value) result
val request_method : request -> string
val request_url : request -> string
val request_headers : request -> headers
val request_signal : request -> abort_signal
val request_text : request -> string promise

type response

val response : value -> status:value -> headers:value -> (response, value) result
(** [new Response(body, {status, headers})], or the RangeError or TypeError it throws. *)

val status : response -> int
val ok : response -> bool
val response_headers : response -> headers
val has_body : response -> bool
val response_text : response -> string promise

(** {1 Decoding} *)

type bytes

val array_buffer : response -> bytes promise
val byte_length : bytes -> int

type text_decoder

val utf8_decoder : unit -> text_decoder

val text_decoder : string -> (text_decoder, value) result
(** A decoder for an Encoding Standard label, or the RangeError for an unknown one. *)

val decode : text_decoder -> bytes -> string
