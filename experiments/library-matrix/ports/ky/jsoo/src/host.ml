(* The host APIs Ky drives, for js_of_ocaml: its Fetch, Abort and Promise bindings, and
   Js.Unsafe for what they leave out (AbortSignal.any, TextDecoder, timers, untyped option
   values). OCaml strings are UTF-8 here, so a string crosses with Js.string and Js.to_string;
   the strings Ky reads (header values, URLs, bodies) are well-formed, so the conversion is exact.
   See host.mli. *)

open Js_of_ocaml

type value = Js.Unsafe.any

let inject = Js.Unsafe.inject
let global name : value = Js.Unsafe.get Js.Unsafe.global (Js.string name)
let call target name args : value = Js.Unsafe.meth_call target name args
let construct name args : value = Js.Unsafe.new_obj (global name) args
let null = inject Js.null
let of_string text = inject (Js.string text)
let of_int (number : int) = inject number
let of_float number = inject (Js.number_of_float number)
let array values = inject (Js.array (Array.of_list values))
let get target key : value = Js.Unsafe.get target (Js.string key)
let typeof value = Js.to_string (Js.typeof value)

let field target key =
  let value = get target key in
  if typeof value = "undefined" then None else Some value

let keys target = Array.to_list (Array.map Js.to_string (Js.to_array (Js.object_keys target)))
let to_string value = if typeof value = "string" then Some (Js.to_string (Js.Unsafe.coerce value)) else None
let to_number value = if typeof value = "number" then Some (Js.float_of_number (Js.Unsafe.coerce value)) else None
let to_boolean value = if typeof value = "boolean" then Some (Js.to_bool (Js.Unsafe.coerce value)) else None
let truthy value = Js.to_bool (Js.Unsafe.coerce value)

let to_list value =
  if truthy (call (global "Array") "isArray" [| value |]) then
    Some (Array.to_list (Js.to_array (Js.Unsafe.coerce value : value Js.js_array Js.t)))
  else None

let is_integer value = truthy (call (global "Number") "isInteger" [| value |])
let stringify value = Js.to_string (Js._JSON##stringify value)

(* A JS exception, as the value thrown. *)
let catching f = try Ok (f ()) with Js_error.Exn error -> Error (inject (Js_error.to_error error))
let parse text = catching (fun () -> (Js._JSON##parse (Js.string text) : value))
(* Ky's isError: an Error, or an Error of another realm by its internal brand. *)
let is_error value =
  Js.instanceof value Js.error_constr
  || Js.to_string (Js.Unsafe.coerce (call (get (get (global "Object") "prototype") "toString") "call" [| value |]))
     = "[object Error]"
let error_name value = Option.value (to_string (get value "name")) ~default:"Error"
let error_message value = to_string (get value "message")
let has_stack value = Option.is_some (field value "stack")
let has_property value key = truthy (call (global "Reflect") "has" [| value; of_string key |])
let new_error message = inject (new%js Js.error_constr (Js.string message))
let type_error message = construct "TypeError" [| of_string message |]
let range_error message = construct "RangeError" [| of_string message |]
let set_name error name = Js.Unsafe.set error (Js.string "name") (of_string name)
let lowercase text = Js.to_string (Js.string text)##toLowerCase
let uppercase text = Js.to_string (Js.string text)##toUpperCase
let trim text = Js.to_string (Js.string text)##trim
let number text = Js.float_of_number (Js.Unsafe.coerce (Js.Unsafe.fun_call (global "Number") [| of_string text |]))

type regexp = Js.regExp Js.t

let regexp pattern flags = new%js Js.regExp_withFlags (Js.string pattern) (Js.string flags)

let exec (pattern : regexp) text =
  Js.Opt.to_option (pattern##exec (Js.string text))
  |> Option.map (fun handle ->
         let groups = Js.match_result handle in
         Array.init groups##.length (fun index ->
             Js.Optdef.to_option (Js.array_get groups index) |> Option.map Js.to_string))

let test (pattern : regexp) text = Js.to_bool (pattern##test (Js.string text))
let number_of_float = Js.number_of_float
let math_min a b = Js.float_of_number (Js.math##min (number_of_float a) (number_of_float b))
let math_max a b = Js.float_of_number (Js.math##max (number_of_float a) (number_of_float b))
let math_pow a b = Js.float_of_number (Js.math##pow (number_of_float a) (number_of_float b))
let now () = Js.float_of_number (Js.Unsafe.coerce (call (global "Date") "now" [||]))

type 'a promise = 'a Promise.t

let resolve = Promise.resolve
let reject error = Promise.reject (Promise.error_of_any error)
let bind promise f = Promise.bind f promise

let settle promise =
  promise
  |> Promise.map (fun value -> Ok value)
  |> Promise.catch (fun error -> Promise.resolve (Error (Promise.error_to_any error)))

let promise executor =
  Promise.make (fun ~resolve ~reject -> executor resolve (fun error -> reject (Promise.error_of_any error)))

let to_value promise = Promise.to_any promise

(* A host call that can throw an error of another realm, run inside a JS Promise executor: no
   OCaml [try] stands between the call and the executor, so the rejection is the thrown value
   itself, where js_of_ocaml would catch it as a [Failure] holding its text. *)
let attempt f = settle (Promise.make (fun ~resolve ~reject:_ -> resolve (f ())))

type timer = value

let set_timeout callback milliseconds =
  Js.Unsafe.fun_call (global "setTimeout")
    [| inject (Js.wrap_callback callback); of_float milliseconds |]

let clear_timeout (timer : timer) = ignore (Js.Unsafe.fun_call (global "clearTimeout") [| timer |] : value)
let sleep milliseconds = promise (fun resolve _ -> ignore (set_timeout resolve milliseconds))

type abort_signal = Abort.signal Js.t
type abort_controller = Abort.controller Js.t

let abort_controller () = new%js Abort.controller
let controller_signal (controller : abort_controller) = controller##.signal
let abort (controller : abort_controller) = controller##abort
let abort_with (controller : abort_controller) reason = controller##abort_reason reason

let any_signal signals : abort_signal =
  Js.Unsafe.coerce (call (global "AbortSignal") "any" [| inject (Js.array (Array.of_list signals)) |])

let aborted (signal : abort_signal) = Js.to_bool signal##.aborted
let reason (signal : abort_signal) = signal##.reason
let throw_if_aborted (signal : abort_signal) = catching (fun () -> signal##throwIfAborted)

let on_abort (signal : abort_signal) listener =
  let once = Js.Unsafe.obj [| ("once", inject Js._true) |] in
  ignore (call signal "addEventListener" [| of_string "abort"; inject (Js.wrap_callback listener); once |])

type headers = Fetch.headers Js.t

let headers () = new%js Fetch.headers

let set_header (headers : headers) name value =
  attempt (fun () -> ignore (call headers "set" [| of_string name; value |]))

let header (headers : headers) name = Js.Opt.to_option (headers##get (Js.string name)) |> Option.map Js.to_string

type request = Fetch.request Js.t

let request input ~method_ ~headers ~body ~signal =
  let init = Fetch.empty_request_init () in
  init##._method := Js.string method_;
  init##.headers := headers;
  Option.iter (fun body -> init##.body := of_string body) body;
  init##.signal := signal;
  attempt (fun () -> new%js Fetch.request_with_init (Js.string input) init)

let with_signal (request : request) signal =
  let init = Fetch.empty_request_init () in
  init##.signal := signal;
  catching (fun () -> new%js Fetch.request_of_request_with_init request init)

let clone_request (request : request) = catching (fun () -> request##clone)
let request_method (request : request) = Js.to_string request##._method
let request_url (request : request) = Js.to_string request##.url
let request_headers (request : request) = request##.headers
let request_signal (request : request) = request##.signal
let request_text (request : request) = Promise.map Js.to_string request##text

type response = Fetch.response Js.t

let response body ~status ~headers : (response, value) result promise =
  let init = Js.Unsafe.obj [| ("status", status); ("headers", headers) |] in
  attempt (fun () -> Js.Unsafe.coerce (construct "Response" [| body; init |]))

let status (response : response) = response##.status
let ok (response : response) = Js.to_bool response##.ok
let response_headers (response : response) = response##.headers
let has_body (response : response) = Js.Opt.test (Js.Unsafe.get response (Js.string "body") : value Js.opt)
let response_text (response : response) = Promise.map Js.to_string response##text

type bytes = Typed_array.arrayBuffer Js.t

let array_buffer (response : response) = response##arrayBuffer
let byte_length (bytes : bytes) = bytes##.byteLength

type text_decoder = value

let utf8_decoder () = construct "TextDecoder" [||]
let text_decoder label = attempt (fun () -> construct "TextDecoder" [| of_string label |])
let decode decoder (bytes : bytes) = Js.to_string (Js.Unsafe.coerce (call decoder "decode" [| inject bytes |]))
