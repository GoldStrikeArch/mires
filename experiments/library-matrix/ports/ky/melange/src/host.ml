(* The host APIs Ky drives, for Melange: externals onto the JS values themselves. OCaml strings
   are JS strings here, so every string crosses as it is. See host.mli. *)

type value

external cast : 'a -> value = "%identity"
external uncast : value -> 'a = "%identity"

let null = cast Js.null
let of_string (text : string) = cast text
let of_int (number : int) = cast number
let of_float (number : float) = cast number
let array (values : value list) = cast (Array.of_list values)

external get : value -> string -> value = "" [@@mel.get_index]

let field target key =
  let value = get target key in
  if Js.typeof value = "undefined" then None else Some value

external object_keys : value -> string array = "keys" [@@mel.scope "Object"]

let keys target = Array.to_list (object_keys target)
let typed kind value = if Js.typeof value = kind then Some (uncast value) else None
let to_string value : string option = typed "string" value
let to_number value : float option = typed "number" value
let to_boolean value : bool option = typed "boolean" value

external is_array : value -> bool = "isArray" [@@mel.scope "Array"]

let to_list value =
  if is_array value then Some (Array.to_list (uncast value : value array)) else None

external is_integer : value -> bool = "isInteger" [@@mel.scope "Number"]
external stringify : value -> string = "stringify" [@@mel.scope "JSON"]
external json_parse : string -> value = "parse" [@@mel.scope "JSON"]

(* A JS exception, as the value thrown. *)
let catching f = try Ok (f ()) with Js.Exn.Error error -> Error (cast error)
let parse text = catching (fun () -> json_parse text)

external instance_of_error : value -> bool = "isPrototypeOf" [@@mel.scope "Error", "prototype"]
external object_to_string : value -> string = "call" [@@mel.scope "Object", "prototype", "toString"]

(* Ky's isError: an Error, or an Error of another realm by its internal brand. *)
let is_error value = instance_of_error value || object_to_string value = "[object Error]"

let error_name value = Option.value (to_string (get value "name")) ~default:"Error"
let error_message value = to_string (get value "message")
let has_stack value = Option.is_some (field value "stack")

external reflect_has : value -> string -> bool = "has" [@@mel.scope "Reflect"]

let has_property value key = reflect_has value key

external new_error : string -> value = "Error" [@@mel.new]
external type_error : string -> value = "TypeError" [@@mel.new]
external range_error : string -> value = "RangeError" [@@mel.new]
external set : value -> string -> value -> unit = "" [@@mel.set_index]

let set_name error name = set error "name" (of_string name)

external lowercase : string -> string = "toLowerCase" [@@mel.send]
external uppercase : string -> string = "toUpperCase" [@@mel.send]
external trim : string -> string = "trim" [@@mel.send]
external number : string -> float = "Number"

type regexp = Js.Re.t

let regexp pattern flags = Js.Re.fromStringWithFlags pattern ~flags

let exec pattern text =
  Js.Re.exec ~str:text pattern
  |> Option.map (fun result -> Array.map Js.Nullable.toOption (Js.Re.captures result))

let test pattern text = Js.Re.test ~str:text pattern

external math_min : float -> float -> float = "min" [@@mel.scope "Math"]
external math_max : float -> float -> float = "max" [@@mel.scope "Math"]
external math_pow : float -> float -> float = "pow" [@@mel.scope "Math"]
external now : unit -> float = "now" [@@mel.scope "Date"]

type 'a promise = 'a Js.Promise.t

let resolve = Js.Promise.resolve
let bind promise f = Js.Promise.then_ f promise
let reject (error : value) = Js.Promise.reject (uncast error)

let settle promise =
  promise
  |> Js.Promise.then_ (fun value -> Js.Promise.resolve (Ok value))
  |> Js.Promise.catch (fun (error : Js.Promise.error) -> Js.Promise.resolve (Error (cast error)))

let promise executor =
  Js.Promise.make (fun ~resolve ~reject ->
      executor (fun value -> resolve value [@u]) (fun error -> reject (uncast error) [@u]))

let to_value (promise : value promise) = cast promise

type timer

external set_timeout : ((unit -> unit)[@mel.uncurry]) -> float -> timer = "setTimeout"
external clear_timeout : timer -> unit = "clearTimeout"

let sleep milliseconds = promise (fun resolve _ -> ignore (set_timeout resolve milliseconds))

type abort_signal
type abort_controller

external abort_controller : unit -> abort_controller = "AbortController" [@@mel.new]
external controller_signal : abort_controller -> abort_signal = "signal" [@@mel.get]
external abort : abort_controller -> unit = "abort" [@@mel.send]
external abort_with : abort_controller -> value -> unit = "abort" [@@mel.send]
external signal_any : abort_signal array -> abort_signal = "any" [@@mel.scope "AbortSignal"]

let any_signal signals = signal_any (Array.of_list signals)

external aborted : abort_signal -> bool = "aborted" [@@mel.get]
external reason : abort_signal -> value = "reason" [@@mel.get]
external signal_throw_if_aborted : abort_signal -> unit = "throwIfAborted" [@@mel.send]

let throw_if_aborted signal = catching (fun () -> signal_throw_if_aborted signal)

external add_event_listener :
  abort_signal -> string -> ((unit -> unit)[@mel.uncurry]) -> < once : bool > Js.t -> unit
  = "addEventListener"
[@@mel.send]

let on_abort signal listener = add_event_listener signal "abort" listener [%mel.obj { once = true }]

type headers

external headers : unit -> headers = "Headers" [@@mel.new]
external headers_set : headers -> string -> value -> unit = "set" [@@mel.send]
external headers_get : headers -> string -> string Js.null = "get" [@@mel.send]

let set_header headers name value = resolve (catching (fun () -> headers_set headers name value))
let header headers name = Js.Null.toOption (headers_get headers name)

type request

type request_init = {
  method_ : string; [@mel.as "method"]
  headers : headers;
  body : string option; [@mel.optional]
  signal : abort_signal;
}
[@@deriving jsProperties] [@@warning "-69"]

external new_request : string -> request_init -> request = "Request" [@@mel.new]
external copy_request : request -> < signal : abort_signal > Js.t -> request = "Request" [@@mel.new]

let request input ~method_ ~headers ~body ~signal =
  resolve (catching (fun () -> new_request input (request_init ~method_ ~headers ?body ~signal ())))

let with_signal request signal = catching (fun () -> copy_request request [%mel.obj { signal }])

external request_clone : request -> request = "clone" [@@mel.send]

let clone_request request = catching (fun () -> request_clone request)

external request_method : request -> string = "method" [@@mel.get]
external request_url : request -> string = "url" [@@mel.get]
external request_headers : request -> headers = "headers" [@@mel.get]
external request_signal : request -> abort_signal = "signal" [@@mel.get]
external request_text : request -> string promise = "text" [@@mel.send]

type response

external new_response : value -> < status : value ; headers : value > Js.t -> response
  = "Response"
[@@mel.new]

let response body ~status ~headers =
  resolve (catching (fun () -> new_response body [%mel.obj { status; headers }]))

external status : response -> int = "status" [@@mel.get]
external ok : response -> bool = "ok" [@@mel.get]
external response_headers : response -> headers = "headers" [@@mel.get]
external response_body : response -> value Js.null = "body" [@@mel.get]

let has_body response = Option.is_some (Js.Null.toOption (response_body response))

external response_text : response -> string promise = "text" [@@mel.send]

type bytes

external array_buffer : response -> bytes promise = "arrayBuffer" [@@mel.send]
external byte_length : bytes -> int = "byteLength" [@@mel.get]

type text_decoder

external utf8_decoder : unit -> text_decoder = "TextDecoder" [@@mel.new]
external labelled_decoder : string -> text_decoder = "TextDecoder" [@@mel.new]

let text_decoder label = resolve (catching (fun () -> labelled_decoder label))

external decode : text_decoder -> bytes -> string = "decode" [@@mel.send]
