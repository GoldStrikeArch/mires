(* The host APIs Workbox drives, for Melange: externals onto the JS values themselves. OCaml strings
   are JS strings here, so every string crosses as it is. See host.mli. *)

type value

external cast : 'a -> value = "%identity"
external uncast : value -> 'a = "%identity"

let null = cast Js.null
let undefined = cast Js.undefined
let of_string (text : string) = cast text
let of_int (number : int) = cast number
let array (values : value list) = cast (Array.of_list values)

external empty : unit -> value = "Object" [@@mel.new]
external get : value -> string -> value = "" [@@mel.get_index]
external set : value -> string -> value -> unit = "" [@@mel.set_index]

let obj members =
  let target = empty () in
  List.iter (fun (key, value) -> set target key value) members;
  target

let field target key =
  let value = get target key in
  if Js.typeof value = "undefined" then None else Some value

external push : value -> value -> unit = "push" [@@mel.send]

let typed kind value = if Js.typeof value = kind then Some (uncast value) else None
let to_string value : string option = typed "string" value
let to_bool value : bool option = typed "boolean" value

external is_array : value -> bool = "isArray" [@@mel.scope "Array"]

let to_list value = if is_array value then Some (Array.to_list (uncast value : value array)) else None

external boolean : value -> bool = "Boolean"

let truthy value = boolean value

external string_of : value -> string = "String"
external stringify : value -> string = "stringify" [@@mel.scope "JSON"]
external is_error : value -> bool = "isPrototypeOf" [@@mel.scope "Error", "prototype"]

let error_name error : string = uncast (get error "name")
let error_message error = get error "message"

external new_error : string -> value = "Error" [@@mel.new]
external type_error : string -> value = "TypeError" [@@mel.new]
external replace_all_with : string -> string -> string -> string = "replaceAll" [@@mel.send]

let replace_all text pattern replacement = replace_all_with text pattern replacement

external location_href : string = "href" [@@mel.scope "location"]
external location_origin : string = "origin" [@@mel.scope "location"]

let location_href () = location_href
let location_origin () = location_origin

external random_uuid : unit -> string = "randomUUID" [@@mel.scope "crypto"]
external warn : string -> unit = "warn" [@@mel.scope "console"]

(* A JS exception, as the value thrown. *)
let catching f = try Ok (f ()) with Js.Exn.Error error -> Error (cast error)

type 'a promise = 'a Js.Promise.t

let resolve = Js.Promise.resolve
let bind promise f = Js.Promise.then_ f promise

(* A host promise's outcome; a synchronous throw is its rejection. *)
let attempt (f : unit -> 'a Js.Promise.t) : ('a, value) result promise =
  match catching f with
  | Error error -> Js.Promise.resolve (Error error)
  | Ok promise ->
      promise
      |> Js.Promise.then_ (fun value -> Js.Promise.resolve (Ok value))
      |> Js.Promise.catch (fun (error : Js.Promise.error) -> Js.Promise.resolve (Error (cast error)))

let settle (promise : value) : (value, value) result promise = attempt (fun () -> uncast promise)

external all_settled : value array -> unit Js.Promise.t = "allSettled" [@@mel.scope "Promise"]

let all_settled promises = Js.Promise.then_ (fun _ -> Js.Promise.resolve ()) (all_settled (Array.of_list promises))

let rejecting outcome =
  cast
    (Js.Promise.then_
       (function Ok value -> Js.Promise.resolve value | Error error -> Js.Promise.reject (uncast error))
       outcome)

let to_value promise = cast promise

(* `f () [@u]` would call an uncurried function of no arguments; these resolvers take one. *)
let nothing = ()

let deferred () =
  let resolver = ref (fun () -> ()) in
  let promise = Js.Promise.make (fun ~resolve ~reject:_ -> resolver := fun () -> resolve nothing [@u]) in
  (promise, fun () -> !resolver ())

external set_timeout : ((unit -> unit)[@mel.uncurry]) -> int -> unit = "setTimeout"

let timeout milliseconds =
  Js.Promise.make (fun ~resolve ~reject:_ -> set_timeout (fun () -> resolve nothing [@u]) milliseconds)

type url
type search_params

external new_url : string -> url = "URL" [@@mel.new]
external new_url_with_base : string -> string -> url = "URL" [@@mel.new]

let url input ~base =
  catching (fun () -> match base with Some base -> new_url_with_base input base | None -> new_url input)

external href : url -> string = "href" [@@mel.get]
external origin : url -> string = "origin" [@@mel.get]
external protocol : url -> string = "protocol" [@@mel.get]
external pathname : url -> string = "pathname" [@@mel.get]
external set_pathname : url -> string -> unit = "pathname" [@@mel.set]
external set_hash : url -> string -> unit = "hash" [@@mel.set]

let copy_url url = new_url (href url)

external search_params : url -> search_params = "searchParams" [@@mel.get]
external search_param_keys : search_params -> string Js.iterator = "keys" [@@mel.send]
external array_from : string Js.iterator -> string array = "from" [@@mel.scope "Array"]
external search_params_set : search_params -> string -> value -> unit = "set" [@@mel.send]
external search_params_delete : search_params -> string -> unit = "delete" [@@mel.send]

let search_param_names url = Array.to_list (array_from (search_param_keys (search_params url)))
let set_search_param url name value = search_params_set (search_params url) name value
let delete_search_param url name = search_params_delete (search_params url) name

type regexp = Js.Re.t

let regexp source = catching (fun () -> Js.Re.fromString source)
let test regexp text = Js.Re.test ~str:text regexp

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

let no_init = { method_ = None; mode = None; integrity = None; cache = None; credentials = None; headers = None }

(* The RequestInit object, with only the members given. *)
let init_object { method_; mode; integrity; cache; credentials; headers } =
  let members key value = Option.to_list (Option.map (fun value -> (key, value)) value) in
  obj
    (List.concat
       [
         members "method" (Option.map of_string method_);
         members "mode" (Option.map of_string mode);
         members "integrity" (Option.map of_string integrity);
         members "cache" (Option.map of_string cache);
         members "credentials" (Option.map of_string credentials);
         members "headers" (Option.map cast headers);
       ])

external new_request : string -> value -> request = "Request" [@@mel.new]
external new_request_of_request : request -> value -> request = "Request" [@@mel.new]

let request url init = catching (fun () -> new_request url (init_object init))
let copy_request request init = catching (fun () -> new_request_of_request request (init_object init))

external clone_request : request -> request = "clone" [@@mel.send]
external request_url : request -> string = "url" [@@mel.get]
external request_method : request -> string = "method" [@@mel.get]
external request_mode : request -> string = "mode" [@@mel.get]
external request_integrity : request -> string = "integrity" [@@mel.get]
external request_headers : request -> headers = "headers" [@@mel.get]

let request_value request = cast request

external request_prototype : value = "prototype" [@@mel.scope "Request"]
external is_prototype_of : value -> value -> bool = "isPrototypeOf" [@@mel.send]

let as_request value = if is_prototype_of request_prototype value then Some (uncast value : request) else None
let of_request value : request = uncast value

type response
type blob

external new_response : value -> value -> response = "Response" [@@mel.new]

let response body ~status = new_response body (obj [ ("status", status) ])

external copy_headers : headers -> headers = "Headers" [@@mel.new]
external new_copied_response : blob -> value -> response = "Response" [@@mel.new]

let copied_response body ~headers ~status ~status_text =
  let init = obj [ ("headers", cast headers); ("status", of_int status); ("statusText", of_string status_text) ] in
  new_copied_response body init

external clone_response : response -> response = "clone" [@@mel.send]
external status : response -> int = "status" [@@mel.get]
external status_text : response -> string = "statusText" [@@mel.get]
external response_url : response -> string = "url" [@@mel.get]
external redirected : response -> bool = "redirected" [@@mel.get]
external response_type : response -> string = "type" [@@mel.get]
external response_headers : response -> headers = "headers" [@@mel.get]
external response_text : response -> value Js.Promise.t = "text" [@@mel.send]
external response_blob : response -> blob Js.Promise.t = "blob" [@@mel.send]

let text response = attempt (fun () -> response_text response)
let blob response = attempt (fun () -> response_blob response)

type property = { value : value } [@@warning "-69"]

external bind_clone : value -> response -> value = "bind" [@@mel.send]
external call_clone : value -> response = "call" [@@mel.send]
external define_property : response -> string -> property -> unit = "defineProperty" [@@mel.scope "Object"]

let rec mark_redirected response =
  let clone = bind_clone (get (cast response) "clone") response in
  define_property response "redirected" { value = cast true };
  define_property response "clone" { value = cast (fun () -> mark_redirected (call_clone clone)) };
  response

external host_fetch : request -> response Js.Promise.t = "fetch"

let fetch request = attempt (fun () -> host_fetch request)

type cache

external caches_open : string -> cache Js.Promise.t = "open" [@@mel.scope "caches"]
external caches_match : request -> value -> response Js.undefined Js.Promise.t = "match" [@@mel.scope "caches"]
external caches_keys : unit -> string array Js.Promise.t = "keys" [@@mel.scope "caches"]
external caches_delete : string -> bool Js.Promise.t = "delete" [@@mel.scope "caches"]
external cache_match : cache -> request -> response Js.undefined Js.Promise.t = "match" [@@mel.send]
external cache_match_url : cache -> string -> response Js.undefined Js.Promise.t = "match" [@@mel.send]
external cache_put : cache -> request -> response -> unit Js.Promise.t = "put" [@@mel.send]
external cache_delete : cache -> request -> bool Js.Promise.t = "delete" [@@mel.send]
external cache_keys : cache -> request array Js.Promise.t = "keys" [@@mel.send]
external cache_keys_matching : cache -> request -> value -> request array Js.Promise.t = "keys" [@@mel.send]

let found promise = Js.Promise.then_ (fun response -> Js.Promise.resolve (Js.Undefined.toOption response)) promise
let listed promise = Js.Promise.then_ (fun items -> Js.Promise.resolve (Array.to_list items)) promise
let caches_open name = attempt (fun () -> caches_open name)
let caches_match request ~cache_name =
  attempt (fun () -> found (caches_match request (obj [ ("cacheName", of_string cache_name) ])))
let caches_keys () = attempt (fun () -> listed (caches_keys ()))
let caches_delete name = attempt (fun () -> caches_delete name)
let cache_match cache request = attempt (fun () -> found (cache_match cache request))
let cache_match_url cache url = attempt (fun () -> found (cache_match_url cache url))
let cache_put cache request response = attempt (fun () -> cache_put cache request response)
let cache_delete cache request = attempt (fun () -> cache_delete cache request)
let cache_keys cache = attempt (fun () -> listed (cache_keys cache))

let cache_keys_ignoring_search cache request =
  attempt (fun () -> listed (cache_keys_matching cache request (obj [ ("ignoreSearch", cast true) ])))

let event_type event : string = uncast (get event "type")

external event_wait_until : value -> value -> unit = "waitUntil" [@@mel.send]

let wait_until event promise = event_wait_until event promise
