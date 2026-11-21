(* workbox-core 7.4.1: WorkboxError and its production messages, copyResponse,
   cacheMatchIgnoreParams, timeout and waitUntil, the ExtendableEvent a strategy runs under, and
   the promise operators of this port.

   Workbox's async functions are promises of results here: a call that Workbox would throw out of
   resolves with [Error error], and no OCaml exception crosses a promise. *)

(* A thrown value: a WorkboxError this port made, or whatever a host API or a plugin threw. Both
   keep the JS value, so a plugin, a waiting event and an error's details see the object
   JavaScript would. *)
type error = Workbox_error of Host.value | Thrown of Host.value

let error_value = function Workbox_error value | Thrown value -> value

(* `error instanceof Error`: Workbox tests it before it keeps or reports a caught value, because a
   plugin may throw anything. *)
let is_error = function Workbox_error _ -> true | Thrown value -> Host.is_error value

type 'a outcome = ('a, error) result Host.promise

let ( let* ) = Host.bind
let return value = Host.resolve (Ok value)
let fail error = Host.resolve (Error error)

(* Binds an outcome, and a result, inside an outcome. *)
let ( let*? ) outcome f = Host.bind outcome (function Ok value -> f value | Error error -> fail error)
let ( let*! ) result f = match result with Ok value -> f value | Error error -> fail error
let thrown result = Result.map_error (fun value -> Thrown value) result

(* A host call's outcome, with what it threw as [Thrown]. *)
let host promise = Host.bind promise (fun result -> Host.resolve (thrown result))

(* `new WorkboxError(code, details)` of the production build: an Error whose own `name` is the code
   and own `details` the details object. The production messageGenerator prints the argument list
   as JSON, so absent details print as `[null]`, and a nested error prints its own enumerable
   properties. *)
let workbox_error ?(details = Host.undefined) code =
  let error = Host.new_error (code ^ " :: " ^ Host.stringify (Host.array [ details ])) in
  Host.set error "name" (Host.of_string code);
  Host.set error "details" details;
  Workbox_error error

(* The event a strategy, install or activate runs under: a native ExtendableEvent, or the stand-in
   a page supplies. `wait_until` takes a JS promise. *)
type event = { event_type : string; wait_until : Host.value -> unit }

(* `waitUntil(event, asyncFn)`: the started promise extends the event's lifetime, rejecting where
   Workbox's would, and is returned. *)
let wait_until event async_fn =
  let return_promise = async_fn () in
  let settled = Host.bind return_promise (fun outcome -> Host.resolve (Result.map_error error_value outcome)) in
  event.wait_until (Host.rejecting settled);
  return_promise

(* `timeout(ms)`: one timer task. *)
let timeout = Host.timeout

(* `copyResponse(response)`: refuses a response that is not from this origin (a URL-less one has
   origin null), then builds a new Response from a clone's headers, status, status text and body.
   `canConstructResponseFromBodyStream()` always answers false in 7.4.1, so the body is a Blob. *)
let copy_response response =
  let origin =
    match Host.response_url response with
    | "" -> Ok None
    | url -> Result.map (fun url -> Some (Host.origin url)) (thrown (Host.url url ~base:None))
  in
  let*! origin = origin in
  if origin <> Some (Host.location_origin ()) then
    let origin = Option.fold origin ~none:Host.null ~some:Host.of_string in
    fail (workbox_error "cross-origin-copy-response" ~details:(Host.obj [ ("origin", origin) ]))
  else
    let cloned_response = Host.clone_response response in
    let headers = Host.copy_headers (Host.response_headers cloned_response) in
    let status = Host.status cloned_response in
    let status_text = Host.status_text cloned_response in
    let*? body = host (Host.blob cloned_response) in
    let*! copied = thrown (Host.copied_response body ~headers ~status ~status_text) in
    return copied

let strip_params full_url ignore_params =
  Host.url full_url ~base:None
  |> Result.map (fun stripped_url ->
         List.iter (Host.delete_search_param stripped_url) ignore_params;
         Host.href stripped_url)
  |> thrown

(* `cacheMatchIgnoreParams(cache, request, ignoreParams)`: an ordinary match when stripping the
   parameters changes nothing (deleting re-serialises the query), else the first key that strips
   to the same URL. *)
let cache_match_ignore_params cache request ignore_params =
  let*! stripped_request_url = strip_params (Host.request_url request) ignore_params in
  if Host.request_url request = stripped_request_url then host (Host.cache_match cache request)
  else
    let*? cache_keys = host (Host.cache_keys_ignoring_search cache request) in
    let rec find = function
      | [] -> return None
      | cache_key :: rest -> (
          let*! stripped = strip_params (Host.request_url cache_key) ignore_params in
          if stripped = stripped_request_url then host (Host.cache_match cache cache_key) else find rest)
    in
    find cache_keys
