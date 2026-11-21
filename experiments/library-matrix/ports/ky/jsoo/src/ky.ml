(* Ky 2.2.0 (core/Ky.js, core/constants.js, utils/normalize.js, utils/merge.js, utils/timeout.js,
   utils/is-network-error.js and errors/*.js), the subset in libraries/ky/CONTRACT.md.

   Ky's async methods are promises of results here: a call that Ky would throw out of resolves
   with [Error failure]. *)

(* errors/*.js: Ky's own errors, or what a host API, the transport or one of Ky's option checks
   threw (a TypeError, RangeError, SyntaxError or abort reason), as the value it is. *)
type failure =
  | Http_error of { response : Host.response; request : Host.request; data : Host.value option }
  | Network_error of { request : Host.request; cause : Host.value }
  | Timeout_error of { request : Host.request }
  | Thrown of Host.value

let error_name = function
  | Http_error _ -> "HTTPError"
  | Network_error _ -> "NetworkError"
  | Timeout_error _ -> "TimeoutError"
  | Thrown value -> Host.error_name value

type hooks = {
  init : (unit -> unit) list;
  before_request : (request:Host.request -> retry_count:int -> unit) list;
  after_response : (request:Host.request -> response:Host.response -> retry_count:int -> unit) list;
  before_retry : (request:Host.request -> error:failure -> retry_count:int -> unit) list;
  before_error : (request:Host.request -> error:failure -> retry_count:int -> unit) list;
}

(* The caller's retry options, as the JS values given (None where a value is undefined);
   normalize_retry_options validates them. *)
type retry_options = {
  limit : Host.value option;
  methods : Host.value option;
  status_codes : Host.value option;
  max_retry_after : Host.value option;
  backoff_limit : Host.value option;
  retry_on_timeout : Host.value option;
  delay : int -> float;
}

type options = {
  method_ : string option;
  headers : Host.value option;
  json : Host.value option;
  retry : retry_options;
  timeout : Host.value option;
  throw_http_errors : Host.value option;
  hooks : hooks;
  fetch : Host.request -> Host.response Host.promise;
  signal : Host.abort_signal option;
}

type retry = {
  limit : float;
  methods : string list;
  status_codes : float list;
  after_status_codes : float list;
  max_retry_after : float;
  backoff_limit : float;
  delay : int -> float;
  retry_on_timeout : bool;
}

let ( let* ) = Host.bind
let return value = Host.resolve (Ok value)
let fail error = Host.resolve (Error error)

(* Binds a promise of a result, and a result, inside a promise of a result. *)
let ( let*? ) outcome f = Host.bind outcome (function Ok value -> f value | Error error -> fail error)
let ( let*! ) result f = match result with Ok value -> f value | Error error -> fail error
let thrown result = Result.map_error (fun value -> Thrown value) result
let type_error message = Error (Thrown (Host.type_error message))

(* core/constants.js and utils/normalize.js *)
let request_methods = [ "get"; "post"; "put"; "patch"; "head"; "delete"; "query" ]
let case_insensitive_methods = request_methods @ [ "options"; "trace" ]
let retry_methods = [ "get"; "put"; "head"; "delete"; "options"; "trace"; "query" ]
let retry_status_codes = [ 408.; 413.; 429.; 500.; 502.; 503.; 504. ]
let retry_after_status_codes = [ 413.; 429.; 503. ]

(* The maximum value of a 32bit int (see issue #117) *)
let max_safe_timeout = 2_147_483_647.
let max_error_response_body_size = 10 * 1024 * 1024

let normalize_request_method input =
  if List.mem (Host.lowercase input) request_methods then Host.uppercase input else input

(* Common methods for retry matching are case-insensitive; custom ones keep their case. *)
let normalize_retry_method method_ =
  let lowercase = Host.lowercase method_ in
  if List.mem lowercase case_insensitive_methods then lowercase else method_

let non_negative value = Option.bind (Host.to_number value) (fun n -> if n >= 0. then Some n else None)

(* The defaults for undefined values, and Ky's checks, in Ky's order. *)
let normalize_retry_options (retry : retry_options) =
  let ( let*! ) = Result.bind in
  let*! limit =
    match retry.limit with
    | None -> Ok 2.
    | Some value when Host.is_integer value && Option.get (Host.to_number value) >= 0. ->
        Ok (Option.get (Host.to_number value))
    | Some _ -> type_error "`retry.limit` must be a finite, non-negative integer"
  in
  let array message = function
    | None -> Ok None
    | Some value -> (
        match Host.to_list value with
        | Some values -> Ok (Some values)
        | None -> Error (Thrown (Host.new_error message)))
  in
  let*! methods = array "retry.methods must be an array" retry.methods in
  let*! status_codes = array "retry.statusCodes must be an array" retry.status_codes in
  (* A mistyped entry only ever fails to match, so it is reported instead. *)
  let every check message values =
    let rec loop acc = function
      | [] -> Ok (List.rev acc)
      | value :: rest -> (
          match check value with
          | Some item -> loop (item :: acc) rest
          | None -> type_error message)
    in
    loop [] values
  in
  let*! methods =
    match methods with
    | None -> Ok retry_methods
    | Some values -> every Host.to_string "`retry.methods` must only contain strings" values
  in
  let*! status_codes =
    match status_codes with
    | None -> Ok retry_status_codes
    | Some values ->
        every
          (fun value -> if Host.is_integer value then Host.to_number value else None)
          "`retry.statusCodes` must only contain numbers" values
  in
  let limit_option message = function
    | None -> Ok Float.infinity
    | Some value -> Option.fold (non_negative value) ~none:(type_error message) ~some:Result.ok
  in
  let*! max_retry_after =
    limit_option "`retry.maxRetryAfter` must be a non-negative number or `Infinity`" retry.max_retry_after
  in
  let*! backoff_limit =
    limit_option "`retry.backoffLimit` must be a non-negative number or `Infinity`" retry.backoff_limit
  in
  let*! retry_on_timeout =
    match retry.retry_on_timeout with
    | None -> Ok false
    | Some value -> (
        match Host.to_boolean value with
        | Some flag -> Ok flag
        | None -> type_error "`retry.retryOnTimeout` must be a boolean")
  in
  Ok
    {
      limit;
      methods = List.map normalize_retry_method methods;
      status_codes;
      after_status_codes = retry_after_status_codes;
      max_retry_after;
      backoff_limit;
      delay = retry.delay;
      retry_on_timeout;
    }

(* Milliseconds, or None for `false`. *)
let validate_timeout_option value =
  match (Host.to_boolean value, non_negative value) with
  | Some false, _ -> Ok None
  (* Infinity lands here too, since it is not a usable delay either. *)
  | _, Some milliseconds when milliseconds > max_safe_timeout ->
      Error (Thrown (Host.range_error "The `timeout` option cannot be greater than 2147483647"))
  | _, Some milliseconds -> Ok (Some milliseconds)
  | _ -> type_error "The `timeout` option must be a non-negative number or `false`"

(* utils/merge.js: option merging keys the headers by their lowercased name, the last spelling of
   a name winning at the place of its first, and mergeHeaders sets each one on a fresh Headers. *)
let lowercase_headers headers =
  let seen = Hashtbl.create 16 in
  let order = ref [] in
  List.iter
    (fun key ->
      let value = Host.get headers key and name = Host.lowercase key in
      match Hashtbl.find_opt seen name with
      | Some cell -> cell := value
      | None ->
          let cell = ref value in
          Hashtbl.add seen name cell;
          order := (name, cell) :: !order)
    (Host.keys headers);
  List.rev_map (fun (name, cell) -> (name, !cell)) !order

let merge_headers headers =
  let result = Host.headers () in
  let rec set_each = function
    | [] -> Ok result
    | (name, value) :: rest -> (
        match thrown (Host.set_header result name value) with
        | Ok () -> set_each rest
        | Error _ as error -> error)
  in
  set_each headers

(* utils/is-network-error.js, inlined by Ky from is-network-error 1.3.2. *)
let network_error_messages =
  [
    "network error";
    "NetworkError when attempting to fetch resource.";
    "The Internet connection appears to be offline.";
    "Network request failed";
    "fetch failed";
    "terminated";
    " A network error occurred.";
    "Network connection lost";
  ]

let is_raw_network_error error =
  match Host.error_message error with
  | Some message when Host.is_error error && Host.error_name error = "TypeError" ->
      let wrapped prefix = String.starts_with ~prefix message && String.ends_with ~suffix:")" message in
      if message = "Load failed" || wrapped "Load failed (" then
        (* Safari 17+ has generic message but no stack for network errors *)
        (not (Host.has_stack error)) || Host.has_property error "__sentry_captured__"
      else
        String.starts_with ~prefix:"error sending request for url" message
        || message = "Failed to fetch" || wrapped "Failed to fetch ("
        || List.mem message network_error_messages
  | _ -> false

(* Matches JSON subtypes like `json`, `problem+json` and `vnd.api+json`, written out rather than
   matched with a pattern, as Ky does. *)
let is_json_content_type content_type =
  let first = match String.index_opt content_type ';' with Some i -> String.sub content_type 0 i | None -> content_type in
  let mime_type = Host.lowercase (Host.trim first) in
  let length = String.length mime_type in
  if not (String.ends_with ~suffix:"json" mime_type) then false
  else
    (* mimeType.at(-5): the character before `json`, which is ASCII whenever it matters. *)
    match if length >= 5 then Some mime_type.[length - 5] else None with
    | Some '/' -> true
    (* The separator is not the slash, so any slash in the type comes before it. *)
    | Some ('.' | '+' | '-') -> String.contains mime_type '/'
    | _ -> false

let charset_pattern = Host.regexp {|;\s*charset\s*=\s*(?:"([^"]+)"|([^;,\s]+))|} "i"

let create_text_decoder content_type =
  let charset =
    Option.bind (Host.exec charset_pattern content_type) (fun groups ->
        match groups.(1) with Some _ as quoted -> quoted | None -> groups.(2))
  in
  match charset with
  | Some label when label <> "" -> (
      (* An unknown label falls back to UTF-8. *)
      match Host.text_decoder label with Ok decoder -> decoder | Error _ -> Host.utf8_decoder ())
  | _ -> Host.utf8_decoder ()

(* An error body as text, or None when it cannot be read or is larger than
   maxErrorResponseBodySize. JSON uses UTF-8 regardless of the charset parameter (RFC 8259). *)
let read_response_text response =
  if not (Host.has_body response) then
    let* text = Host.settle (Host.response_text response) in
    Host.resolve (Result.to_option text)
  else
    let content_type = Option.value (Host.header (Host.response_headers response) "content-type") ~default:"" in
    let decoder =
      if is_json_content_type content_type then Host.utf8_decoder () else create_text_decoder content_type
    in
    let* bytes = Host.settle (Host.array_buffer response) in
    Host.resolve
      (match bytes with
      | Ok bytes when Host.byte_length bytes <= max_error_response_body_size -> Some (Host.decode decoder bytes)
      | _ -> None)

(* #getResponseData: the parsed JSON or the text of an error body, or None when it is empty,
   unreadable or unparsable. *)
let get_response_data response =
  let* text = read_response_text response in
  Host.resolve
    (match text with
    | None | Some "" -> None
    | Some text ->
        let content_type = Option.value (Host.header (Host.response_headers response) "content-type") ~default:"" in
        if not (is_json_content_type content_type) then Some (Host.of_string text)
        else Result.to_option (Host.parse text))

type t = {
  mutable request : Host.request;
  mutable abort_controller : Host.abort_controller;
  mutable signal : Host.abort_signal;  (* #options.signal: the user's and the controller's *)
  mutable retry_count : int;
  mutable original_request : Host.request option;
  user_provided_abort_signal : Host.abort_signal option;
  retry : retry;
  timeout : float option;
  throw_http_errors : bool;
  hooks : hooks;
  fetch : Host.request -> Host.response Host.promise;
}

let create_managed_signal user_provided_abort_signal abort_controller =
  match user_provided_abort_signal with
  | Some signal -> Host.any_signal [ signal; Host.controller_signal abort_controller ]
  | None -> Host.controller_signal abort_controller

let create input (options : options) =
  let ( let*! ) = Result.bind in
  let headers = Option.fold options.headers ~none:[] ~some:lowercase_headers in
  let*! request_headers = merge_headers headers in
  let method_ = normalize_request_method (Option.value options.method_ ~default:"GET") in
  let*! retry = normalize_retry_options options.retry in
  (* Defaults only replace undefined, so a null reaches the validation and is reported. *)
  let*! throw_http_errors =
    match options.throw_http_errors with
    | None -> Ok true
    | Some value -> (
        match Host.to_boolean value with
        | Some flag -> Ok flag
        | None -> type_error "The `throwHttpErrors` option must be a boolean or a function")
  in
  let*! timeout = Option.fold options.timeout ~none:(Ok (Some 10_000.)) ~some:validate_timeout_option in
  let abort_controller = Host.abort_controller () in
  let signal = create_managed_signal options.signal abort_controller in
  (* A content-type from options.headers counts as user-provided. *)
  let user_provided_content_type = List.mem_assoc "content-type" headers in
  let body = Option.map Host.stringify options.json in
  let*! () =
    if Option.is_some body && not user_provided_content_type then
      thrown (Host.set_header request_headers "content-type" (Host.of_string "application/json"))
    else Ok ()
  in
  let*! request = thrown (Host.request input ~method_ ~headers:request_headers ~body ~signal) in
  Ok
    {
      request;
      abort_controller;
      signal;
      retry_count = 0;
      original_request = None;
      user_provided_abort_signal = options.signal;
      retry;
      timeout;
      throw_http_errors;
      hooks = options.hooks;
      fetch = options.fetch;
    }

(* #fetch replaces the request with the clone it prepares for a possible retry, so the request
   that was actually sent is reported. *)
let sent_request ky = Option.value ky.original_request ~default:ky.request

(* Chromium reports a user abort during a fetch as a network error; the abort reason is surfaced
   instead. *)
let throw_if_aborted_by_user ky =
  match ky.user_provided_abort_signal with
  | Some signal -> thrown (Host.throw_if_aborted signal)
  | None -> Ok ()

(* A hook runs inside #raceWithTotalTimeout, which first throws a user abort. *)
let run_hooks ky hooks call =
  List.fold_left
    (fun outcome hook -> Result.bind outcome (fun () -> Result.map (fun () -> call hook) (throw_if_aborted_by_user ky)))
    (Ok ()) hooks

(* utils/timeout.js: the transport raced against a timer that aborts the controller and rejects.
   [timed_out] is Ky's own flag: once it is set, the outcome is the TimeoutError, and a response
   arriving later is discarded. *)
let timeout ky request milliseconds timed_out =
  let timer = ref None in
  let race =
    Host.promise (fun resolve reject ->
        timer :=
          Some
            (Host.set_timeout
               (fun () ->
                 timed_out := true;
                 Host.abort ky.abort_controller;
                 reject Host.null)
               milliseconds);
        ignore
          (Host.bind (Host.settle (ky.fetch request)) (fun result ->
               (match result with
               | Ok response -> if not !timed_out then resolve response
               | Error error -> reject error);
               Host.resolve ())))
  in
  Host.bind (Host.settle race) (fun result ->
      Option.iter Host.clear_timeout !timer;
      match result with Ok response -> Host.resolve response | Error error -> Host.reject error)

let fetch ky =
  (* Cloning is done here to prepare in advance for retries, and skipped when they are disabled. *)
  let*! retry_request =
    if ky.retry.limit > 0. then
      Result.map Option.some
        (Result.bind (Host.clone_request ky.request) (fun clone -> Host.with_signal clone ky.signal) |> thrown)
    else Ok None
  in
  let request = ky.request in
  ky.original_request <- Some request;
  Option.iter (fun retry_request -> ky.request <- retry_request) retry_request;
  let timed_out = ref false in
  let sent =
    match ky.timeout with
    | None -> ky.fetch request
    | Some milliseconds -> timeout ky request milliseconds timed_out
  in
  let* result = Host.settle sent in
  match result with
  | Ok response -> return response
  | Error error ->
      let*! () = throw_if_aborted_by_user ky in
      if !timed_out then fail (Timeout_error { request })
      else if is_raw_network_error error then fail (Network_error { request; cause = error })
      else fail (Thrown error)

(* #calculateDelay: the retry.delay backoff, capped by retry.backoffLimit. *)
let calculate_delay ky =
  let retry_delay = ky.retry.delay (ky.retry_count + 1) in
  if not (retry_delay >= 0.) then type_error "`retry.delay` must return a non-negative number or `Infinity`"
  else Ok (Some (Host.math_min ky.retry.backoff_limit retry_delay))

(* #calculateRetryDelay: the wait before retrying the error, or None when it is not retried and
   fails the call as it is. *)
let calculate_retry_delay ky error =
  if float_of_int ky.retry_count >= ky.retry.limit then Ok None
  else if not (List.mem (normalize_retry_method (Host.request_method ky.request)) ky.retry.methods) then Ok None
  else
    match error with
    | Timeout_error _ when ky.retry.retry_on_timeout -> calculate_delay ky
    | Http_error { response; _ } -> (
        let status = float_of_int (Host.status response) in
        if not (List.mem status ky.retry.status_codes) then Ok None
        else
          match Retry_timing.get_retry_timing_header (Host.response_headers response) with
          | Some header when List.mem status ky.retry.after_status_codes -> (
              match Retry_timing.calculate_retry_timing_delay header with
              (* Malformed retry timing headers should not disable retries; they only lose their
                 server-provided timing. *)
              | None -> calculate_delay ky
              (* No jitter when the server provides explicit retry timing. *)
              | Some after -> Ok (Some (Host.math_min ky.retry.max_retry_after after)))
          | _ when status = 413. -> Ok None
          | _ -> calculate_delay ky)
    | Network_error _ -> calculate_delay ky
    (* Only known retriable error types are retried. *)
    | _ -> Ok None

(* #retry and #retryFromError, which call each other as in Ky. *)
let rec retry ky =
  let* result = fetch ky in
  match result with Ok response -> return response | Error error -> retry_from_error ky error

and retry_from_error ky error =
  let*! () = throw_if_aborted_by_user ky in
  let*! delay = calculate_retry_delay ky error in
  match delay with
  | None -> fail error
  | Some delay ->
      let* () = Host.sleep (Host.math_min delay max_safe_timeout) in
      (* Reset the abort controller if it was aborted (on a timeout retry), so hooks and the
         retried request get a fresh signal. *)
      let*! () =
        if Host.aborted (Host.controller_signal ky.abort_controller) then (
          ky.abort_controller <- Host.abort_controller ();
          ky.signal <- create_managed_signal ky.user_provided_abort_signal ky.abort_controller;
          Result.map (fun request -> ky.request <- request) (thrown (Host.with_signal ky.request ky.signal)))
        else Ok ()
      in
      let*! () =
        run_hooks ky ky.hooks.before_retry (fun hook ->
            hook ~request:ky.request ~error ~retry_count:(ky.retry_count + 1))
      in
      ky.retry_count <- ky.retry_count + 1;
      retry ky

let run ky =
  let*! () = run_hooks ky ky.hooks.before_request (fun hook -> hook ~request:ky.request ~retry_count:0) in
  let rec handle response =
    let*! () =
      run_hooks ky ky.hooks.after_response (fun hook ->
          hook ~request:ky.request ~response ~retry_count:ky.retry_count)
    in
    if Host.ok response || not ky.throw_http_errors then return response
    else
      let* data = get_response_data response in
      let error = Http_error { response; request = sent_request ky; data } in
      let*! () = throw_if_aborted_by_user ky in
      let*? response = retry_from_error ky error in
      handle response
  in
  let*? response = retry ky in
  handle response

(* #throwProcessedError: the beforeError hooks see the error, which then fails the call. *)
let throw_processed_error ky error =
  List.iter (fun hook -> hook ~request:(sent_request ky) ~error ~retry_count:ky.retry_count) ky.hooks.before_error;
  fail error

(* The body shortcuts, and the media type each one advertises. *)
type shortcut = Json | Text

let mime_type = function Json -> "application/json" | Text -> "text/*"

(* ky(input, options)[shortcut](): the init hooks, the request, the run and the body read. A
   construction error fails without running the beforeError hooks, and so does a body that is not
   JSON. *)
let request input (options : options) shortcut =
  List.iter (fun hook -> hook ()) options.hooks.init;
  let*! ky = create input options in
  let headers = Host.request_headers ky.request in
  let accept =
    match Host.header headers "accept" with Some accept when accept <> "" -> accept | _ -> mime_type shortcut
  in
  let*! () = thrown (Host.set_header headers "accept" (Host.of_string accept)) in
  let* result = run ky in
  match result with
  | Error error -> throw_processed_error ky error
  | Ok response -> (
      let* text = Host.settle (Host.response_text response) in
      match (text, shortcut) with
      | Error error, _ -> fail (Thrown error)
      | Ok text, Text -> return (Host.of_string text)
      | Ok text, Json -> Host.resolve (thrown (Host.parse text)))
