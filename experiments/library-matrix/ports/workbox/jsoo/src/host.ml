(* The host APIs Workbox drives, for js_of_ocaml: its Promise binding, Js.regExp, and Js.Unsafe for
   CacheStorage, Fetch, URL and the plain values the script and the output are made of. OCaml
   strings are UTF-8 here, so a string crosses with Js.string and Js.to_string; the strings that
   cross (URLs, cache names, method names, search parameter names, messages) are well-formed, so
   the conversion is exact. Bodies and revisions stay JS values. See host.mli. *)

open Js_of_ocaml

type value = Js.Unsafe.any

let inject = Js.Unsafe.inject
let global name : value = Js.Unsafe.get Js.Unsafe.global (Js.string name)
let call target name args : value = Js.Unsafe.meth_call target name args
let construct name args : value = Js.Unsafe.new_obj (global name) args
let null = inject Js.null
let undefined = inject Js.undefined
let of_string text = inject (Js.string text)
let of_int (number : int) = inject number
let array values = inject (Js.array (Array.of_list values))
let empty () : value = Js.Unsafe.obj [||]
let get target key : value = Js.Unsafe.get target (Js.string key)
let set target key (value : value) = Js.Unsafe.set target (Js.string key) value

let obj members =
  let target = empty () in
  List.iter (fun (key, value) -> set target key value) members;
  target

let typeof value = Js.to_string (Js.typeof value)
let js_string value = Js.to_string (Js.Unsafe.coerce value)

let field target key =
  let value = get target key in
  if typeof value = "undefined" then None else Some value

let push target value = ignore (call target "push" [| value |])
let to_string value = if typeof value = "string" then Some (js_string value) else None
let to_bool value = if typeof value = "boolean" then Some (Js.to_bool (Js.Unsafe.coerce value)) else None
let truthy value = Js.to_bool (Js.Unsafe.fun_call (global "Boolean") [| value |])
let elements value : value list = Array.to_list (Js.to_array (Js.Unsafe.coerce value))
let to_list value = if truthy (call (global "Array") "isArray" [| value |]) then Some (elements value) else None
let string_of value = js_string (Js.Unsafe.fun_call (global "String") [| value |])
let stringify value = Js.to_string (Js._JSON##stringify value)
let is_error value = Js.instanceof value Js.error_constr
let error_name error = js_string (get error "name")
let error_message error = get error "message"
let new_error message = inject (new%js Js.error_constr (Js.string message))
let type_error message = construct "TypeError" [| of_string message |]
let replace_all text pattern replacement =
  js_string (call (of_string text) "replaceAll" [| of_string pattern; of_string replacement |])
let location_href () = js_string (get (global "location") "href")
let location_origin () = js_string (get (global "location") "origin")
let random_uuid () = js_string (call (global "crypto") "randomUUID" [||])
let warn message = ignore (call (global "console") "warn" [| of_string message |])

(* A JS exception, as the value thrown. js_of_ocaml raises Js_error.Exn for a value that is an
   Error of this realm, which every host error is here. *)
let catching f = try Ok (f ()) with Js_error.Exn error -> Error (inject (Js_error.to_error error))

type 'a promise = 'a Promise.t

let resolve = Promise.resolve
let bind promise f = Promise.bind f promise

(* A host promise's outcome; a synchronous throw is its rejection. *)
let attempt (f : unit -> value) : ('a, value) result promise =
  match catching f with
  | Error error -> Promise.resolve (Error error)
  | Ok promise ->
      Promise.of_any promise
      |> Promise.map (fun value -> Ok value)
      |> Promise.catch (fun error -> Promise.resolve (Error (Promise.error_to_any error)))

let settle (promise : value) : (value, value) result promise = attempt (fun () -> promise)
let all_settled promises =
  Promise.map ignore (Promise.of_any (call (global "Promise") "allSettled" [| array promises |]))

let rejecting outcome =
  Promise.to_any
    (Promise.bind
       (function Ok value -> Promise.resolve value | Error error -> Promise.reject (Promise.error_of_any error))
       outcome)

let to_value promise = Promise.to_any promise

let deferred () =
  let resolver = ref (fun () -> ()) in
  let promise = Promise.make (fun ~resolve ~reject:_ -> resolver := resolve) in
  (promise, fun () -> !resolver ())

let timeout milliseconds =
  Promise.make (fun ~resolve ~reject:_ ->
      let callback = inject (Js.wrap_callback (fun () -> resolve ())) in
      ignore (Js.Unsafe.fun_call (global "setTimeout") [| callback; of_int milliseconds |]))

type url = value

let url input ~base =
  let arguments =
    match base with Some base -> [| of_string input; of_string base |] | None -> [| of_string input |]
  in
  catching (fun () -> construct "URL" arguments)

let href url = js_string (get url "href")
let origin url = js_string (get url "origin")
let protocol url = js_string (get url "protocol")
let pathname url = js_string (get url "pathname")
let set_pathname url pathname = set url "pathname" (of_string pathname)
let set_hash url hash = set url "hash" (of_string hash)
let copy_url url = construct "URL" [| get url "href" |]
let search_params url = get url "searchParams"

let search_param_names url =
  List.map js_string (elements (call (global "Array") "from" [| call (search_params url) "keys" [||] |]))

let set_search_param url name value = ignore (call (search_params url) "set" [| of_string name; value |])
let delete_search_param url name = ignore (call (search_params url) "delete" [| of_string name |])

type regexp = Js.regExp Js.t

let regexp source = catching (fun () -> new%js Js.regExp (Js.string source))
let test (regexp : regexp) text = Js.to_bool (regexp##test (Js.string text))

type headers = value
type request = value

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
         members "headers" headers;
       ])

let request url init = catching (fun () -> construct "Request" [| of_string url; init_object init |])
let copy_request request init = catching (fun () -> construct "Request" [| request; init_object init |])
let clone_request request = call request "clone" [||]
let request_url request = js_string (get request "url")
let request_method request = js_string (get request "method")
let request_mode request = js_string (get request "mode")
let request_integrity request = js_string (get request "integrity")
let request_headers request = get request "headers"
let request_value request = request
let as_request value =
  if truthy (call (get (global "Request") "prototype") "isPrototypeOf" [| value |]) then Some value else None
let of_request value = value

type response = value
type blob = value

let response body ~status = construct "Response" [| body; obj [ ("status", status) ] |]
let copy_headers headers = construct "Headers" [| headers |]

let copied_response body ~headers ~status ~status_text =
  let init = obj [ ("headers", headers); ("status", of_int status); ("statusText", of_string status_text) ] in
  construct "Response" [| body; init |]

let clone_response response = call response "clone" [||]
let status response : int = Js.Unsafe.get response (Js.string "status")
let status_text response = js_string (get response "statusText")
let response_url response = js_string (get response "url")
let redirected response = truthy (get response "redirected")
let response_type response = js_string (get response "type")
let response_headers response = get response "headers"
let text response = attempt (fun () -> call response "text" [||])
let blob response = attempt (fun () -> call response "blob" [||])

let rec mark_redirected response =
  let clone = call (get response "clone") "bind" [| response |] in
  let define name value =
    ignore (call (global "Object") "defineProperty" [| response; of_string name; obj [ ("value", value) ] |])
  in
  define "redirected" (inject Js._true);
  define "clone" (inject (Js.wrap_callback (fun () -> mark_redirected (Js.Unsafe.fun_call clone [||]))));
  response

let fetch request = attempt (fun () -> call Js.Unsafe.global "fetch" [| request |])

type cache = value

let caches () = global "caches"
let found promise =
  let present response = if typeof response = "undefined" then None else Some response in
  Promise.map (Result.map present) promise
let listed promise = Promise.map (Result.map elements) promise
let caches_open name = attempt (fun () -> call (caches ()) "open" [| of_string name |])
let caches_match request ~cache_name =
  found (attempt (fun () -> call (caches ()) "match" [| request; obj [ ("cacheName", of_string cache_name) ] |]))

let caches_keys () = Promise.map (Result.map (List.map js_string)) (listed (attempt (fun () -> call (caches ()) "keys" [||])))

let caches_delete name =
  Promise.map (Result.map truthy) (attempt (fun () -> call (caches ()) "delete" [| of_string name |]))

let cache_match cache request = found (attempt (fun () -> call cache "match" [| request |]))
let cache_match_url cache url = found (attempt (fun () -> call cache "match" [| of_string url |]))
let cache_put cache request response =
  Promise.map (Result.map ignore) (attempt (fun () -> call cache "put" [| request; response |]))

let cache_delete cache request = Promise.map (Result.map truthy) (attempt (fun () -> call cache "delete" [| request |]))
let cache_keys cache = listed (attempt (fun () -> call cache "keys" [||]))

let cache_keys_ignoring_search cache request =
  listed (attempt (fun () -> call cache "keys" [| request; obj [ ("ignoreSearch", inject Js._true) ] |]))

let event_type event = js_string (get event "type")
let wait_until event promise = ignore (call event "waitUntil" [| promise |])
