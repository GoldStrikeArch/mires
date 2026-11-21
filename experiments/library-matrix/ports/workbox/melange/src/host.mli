(** The host APIs Workbox drives: CacheStorage and Cache, Fetch ([Request], [Response],
    [Headers]), [URL] and its search parameters, [RegExp], timers, promises, [JSON], the location
    and the console. Workbox is a layer over these, so their semantics (URL parsing and
    serialisation, request and response construction, cache matching, integrity checks, regular
    expressions) are the engine's own. A call that can throw returns the thrown value as [Error];
    an asynchronous call answers with a promise of its outcome, which never rejects.

    This interface is shared, byte-identical, by the Melange and js_of_ocaml ports. Each has its
    own [host.ml]. *)

(** {1 Values} *)

type value
(** Any JS value: an input member, an output record, a thrown value, a plugin's state. *)

val null : value
val undefined : value
val of_string : string -> value
val of_int : int -> value
val array : value list -> value

val obj : (string * value) list -> value
(** A plain object with these members, in this order. *)

val empty : unit -> value
(** A fresh [{}]. *)

val get : value -> string -> value

val field : value -> string -> value option
(** A member that is not [undefined]. *)

val set : value -> string -> value -> unit
val push : value -> value -> unit

val to_string : value -> string option
(** The value, when it is a string. *)

val to_bool : value -> bool option
(** The value, when it is a boolean. *)

val to_list : value -> value list option
(** The elements, when the value is an array. *)

val truthy : value -> bool
(** JavaScript truthiness. *)

val string_of : value -> string
(** [String(value)]. *)

val stringify : value -> string
(** [JSON.stringify(value)]. *)

(** {1 Errors} *)

val is_error : value -> bool
(** [value instanceof Error]. *)

val error_name : value -> string
val error_message : value -> value
val new_error : string -> value
val type_error : string -> value

(** {1 Strings and environment} *)

val replace_all : string -> string -> string -> string
(** [replace_all text pattern replacement] is [text.split(pattern).join(replacement)]. *)

val location_href : unit -> string
val location_origin : unit -> string
val random_uuid : unit -> string
val warn : string -> unit

(** {1 Promises and timers} *)

type 'a promise
(** A JS promise: it starts when it is made and can be awaited any number of times. *)

val resolve : 'a -> 'a promise
val bind : 'a promise -> ('a -> 'b promise) -> 'b promise

val settle : value -> (value, value) result promise
(** The outcome of a JS promise given as a value: its value, or its rejection reason. *)

val all_settled : value list -> unit promise
(** [Promise.allSettled(promises)], for its settling. *)

val rejecting : ('a, value) result promise -> value
(** A JS promise that fulfils with the [Ok] value and rejects with the [Error] value. *)

val to_value : 'a promise -> value
(** The promise, as the JS value it is. *)

val deferred : unit -> unit promise * (unit -> unit)
(** A promise with its resolver. *)

val timeout : int -> unit promise
(** [new Promise(resolve => setTimeout(resolve, ms))]: one timer task. *)

(** {1 URL and RegExp} *)

type url

val url : string -> base:string option -> (url, value) result
(** [new URL(input, base)], or the TypeError it throws. *)

val href : url -> string
val origin : url -> string
val protocol : url -> string
val pathname : url -> string
val set_pathname : url -> string -> unit
val set_hash : url -> string -> unit

val copy_url : url -> url
(** [new URL(url.href)]. *)

val search_param_names : url -> string list
(** [[...url.searchParams.keys()]]. *)

val set_search_param : url -> string -> value -> unit
(** [url.searchParams.set(name, value)]: the host converts a number with Number-to-String. *)

val delete_search_param : url -> string -> unit
(** [url.searchParams.delete(name)], which re-serialises the query. *)

type regexp

val regexp : string -> (regexp, value) result
(** [new RegExp(source)], or the SyntaxError it throws. *)

val test : regexp -> string -> bool

(** {1 Fetch} *)

type headers
type request

type request_init = {
  method_ : string option;
  mode : string option;
  integrity : string option;
  cache : string option;
  credentials : string option;
  headers : headers option;
}
(** The members of a [RequestInit]; [None] leaves a member out. *)

val no_init : request_init

val request : string -> request_init -> (request, value) result
(** [new Request(url, init)], or the TypeError it throws. *)

val copy_request : request -> request_init -> (request, value) result
(** [new Request(request, init)]. *)

val clone_request : request -> request
val request_url : request -> string
val request_method : request -> string
val request_mode : request -> string
val request_integrity : request -> string
val request_headers : request -> headers
val request_value : request -> value

val as_request : value -> request option
(** The value, when it is [instanceof Request]. *)

val of_request : value -> request
(** A Request the caller supplied (service-worker mode). *)

type response
type blob

val response : value -> status:value -> response
(** [new Response(body, {status})]. *)

val copy_headers : headers -> headers
(** [new Headers(headers)]. *)

val copied_response : blob -> headers:headers -> status:int -> status_text:string -> response
(** [new Response(blob, {headers, status, statusText})]. *)

val clone_response : response -> response
val status : response -> int
val status_text : response -> string
val response_url : response -> string
val redirected : response -> bool
val response_type : response -> string
val response_headers : response -> headers
val text : response -> (value, value) result promise
val blob : response -> (blob, value) result promise

val mark_redirected : response -> response
(** The [redirected] fixture: the response gets an own [redirected: true] and an own [clone] that
    marks its result too, as a followed redirect leaves a response. *)

val fetch : request -> (response, value) result promise

(** {1 CacheStorage and Cache} *)

type cache

val caches_open : string -> (cache, value) result promise

val caches_match : request -> cache_name:string -> (response option, value) result promise
(** [caches.match(request, {cacheName})], which opens no cache. *)

val caches_keys : unit -> (string list, value) result promise
val caches_delete : string -> (bool, value) result promise
val cache_match : cache -> request -> (response option, value) result promise
val cache_match_url : cache -> string -> (response option, value) result promise
val cache_put : cache -> request -> response -> (unit, value) result promise
val cache_delete : cache -> request -> (bool, value) result promise
val cache_keys : cache -> (request list, value) result promise

val cache_keys_ignoring_search : cache -> request -> (request list, value) result promise
(** [cache.keys(request, {ignoreSearch: true})]. *)

(** {1 Events} *)

val event_type : value -> string
(** A native event's [type]. *)

val wait_until : value -> value -> unit
(** [event.waitUntil(promise)] on a native event. *)
