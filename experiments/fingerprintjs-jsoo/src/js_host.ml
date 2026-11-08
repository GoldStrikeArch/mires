(* Thin host bindings. Browser globals are read only when a function runs. *)

open Js_of_ocaml

type js = Js.Unsafe.any

exception Js_value of js

let global () : js = Js.Unsafe.inject Js.Unsafe.global

let undefined : js = Js.Unsafe.inject Js.undefined
let null : js = Js.Unsafe.inject Js.null

let inject = Js.Unsafe.inject

let get (obj : js) (key : string) : js = Js.Unsafe.get obj key
let set (obj : js) (key : string) (value : js) : unit = Js.Unsafe.set obj key value
let delete (obj : js) (key : string) : unit = Js.Unsafe.delete obj key

let get_key (obj : js) (key : js) : js = Js.Unsafe.get obj key
let set_key (obj : js) (key : js) (value : js) : unit = Js.Unsafe.set obj key value

let obj () : js = Js.Unsafe.obj [||]
let obj_fields fields : js = Js.Unsafe.obj fields

let js_string (s : string) : js = inject (Js.string s)
let js_bool (b : bool) : js = inject (Js.bool b)
let js_float (f : float) : js = inject f
let js_int (i : int) : js = inject i

let to_string (v : js) : string = Js.to_string (Js.Unsafe.coerce v)
let to_bool (v : js) : bool = Js.to_bool (Js.Unsafe.coerce v)
let to_float (v : js) : float = Js.float_of_number (Js.Unsafe.coerce v)

let typeof (v : js) : string =
  Js.to_string (Js.typeof (Js.Unsafe.coerce v))

let is_undefined (v : js) = typeof v = "undefined"
let is_null (v : js) =
  Js.to_bool
    (Js.Unsafe.fun_call
       (Js.Unsafe.pure_js_expr "(function(v){return v === null;})")
       [| v |])

let is_nullish (v : js) = is_undefined v || is_null v

let js_equals (a : js) (b : js) = Js.strict_equals (Js.Unsafe.coerce a) (Js.Unsafe.coerce b)

let is_truthy : js -> bool =
  let f = Js.Unsafe.pure_js_expr "(function(v){return !!v;})" in
  fun v -> Js.to_bool (Js.Unsafe.fun_call f [| v |])

let js_in : js -> js -> bool =
  let f = Js.Unsafe.pure_js_expr "(function(key,obj){return key in obj;})" in
  fun key obj -> Js.to_bool (Js.Unsafe.fun_call f [| key; obj |])

let js_in_s key obj = js_in (js_string key) obj

let instanceof : js -> js -> bool =
  let f = Js.Unsafe.pure_js_expr "(function(v,C){return v instanceof C;})" in
  fun v ctor -> Js.to_bool (Js.Unsafe.fun_call f [| v; ctor |])

let window () = get (global ()) "window"
let document () = get (global ()) "document"
let navigator () = get (global ()) "navigator"
let screen () = get (global ()) "screen"

let host_object name = get (global ()) name

let call_this (fn : js) (this : js) (args : js array) : js =
  Js.Unsafe.call fn this args

let call_null (fn : js) (args : js array) : js = call_this fn null args

let fun_call (fn : js) (args : js array) : js = Js.Unsafe.fun_call fn args

let meth (obj : js) (name : string) (args : js array) : js =
  Js.Unsafe.meth_call obj name args

let new_obj (ctor : js) (args : js array) : js = Js.Unsafe.new_obj ctor args

let new_named name args = new_obj (host_object name) args

let array () : js = new_obj (host_object "Array") [||]

let array_of_list (items : js list) : js =
  let a = array () in
  List.iter (fun item -> ignore (meth a "push" [| item |])) items;
  a

let array_length (a : js) : int =
  int_of_float (to_float (get a "length"))

let array_get (a : js) (i : int) : js = Js.Unsafe.get a i
let array_set (a : js) (i : int) (v : js) : unit = Js.Unsafe.set a i v

let array_push (a : js) (v : js) : unit = ignore (meth a "push" [| v |])

let js_array_of_ocaml (items : js array) : js =
  let a = array () in
  Array.iter (array_push a) items;
  a

let arguments_get (args : js) (i : int) : js =
  if i < array_length args then array_get args i else undefined

let arg (args : js array) (i : int) : js =
  if i < Array.length args then args.(i) else undefined

let object_ctor () = host_object "Object"

let object_keys (v : js) : js = meth (object_ctor ()) "keys" [| v |]

let define_property (target : js) (key : string) (desc : js) : unit =
  ignore (meth (object_ctor ()) "defineProperty" [| target; js_string key; desc |])

let set_function_name (fn : js) (name : string) : unit =
  let desc =
    obj_fields
      [|
        "value", js_string name;
        "configurable", js_bool true;
      |]
  in
  define_property fn "name" desc

(* Build a JS function whose `.length` is [arity] but that still sees every
   passed argument (needed for default-parameter exports like [load]). *)
let make_js_fn : js -> js -> js =
  Js.Unsafe.pure_js_expr
    {|
    (function(arity, impl) {
      var names = [];
      for (var i = 0; i < arity; i++) names.push("a" + i);
      return new Function(
        "impl",
        "return function(" + names.join(",") + "){var a=[];for(var i=0;i<arguments.length;i++)a[i]=arguments[i];return impl(a);}"
      )(impl);
    })
  |}

let js_fun ?(name = "") ?(arity = 0) (f : js array -> js) : js =
  let impl =
    inject
      (Js.Unsafe.callback_with_arity 1 (fun args ->
           let n = array_length args in
           f (Array.init n (fun i -> array_get args i))))
  in
  let fn = Js.Unsafe.fun_call make_js_fn [| js_int arity; impl |] in
  if name <> "" then set_function_name fn name;
  fn

let js_fun0 ?name f = js_fun ?name ~arity:0 (fun _ -> f ())
let js_fun1 ?name f = js_fun ?name ~arity:1 (fun args -> f (arg args 0))
let js_fun2 ?name f = js_fun ?name ~arity:2 (fun args -> f (arg args 0) (arg args 1))

let date_now () : float =
  to_float (fun_call (get (host_object "Date") "now") [||])

let json () = host_object "JSON"

let json_stringify (v : js) : js = meth (json ()) "stringify" [| v |]

let json_stringify_with (v : js) (replacer : js) (space : js) : js =
  meth (json ()) "stringify" [| v; replacer; space |]

let promise_ctor () = host_object "Promise"

let new_promise (executor : js -> js -> unit) : js =
  let fn =
    Js.Unsafe.callback_with_arity 2 (fun resolve reject ->
        executor resolve reject;
        undefined)
  in
  new_obj (promise_ctor ()) [| inject fn |]

let promise_resolve (v : js) : js = meth (promise_ctor ()) "resolve" [| v |]
let promise_reject (v : js) : js = meth (promise_ctor ()) "reject" [| v |]
let promise_all (values : js) : js = meth (promise_ctor ()) "all" [| values |]

let then1 (p : js) (on_ok : js) : js = meth p "then" [| on_ok |]
let then2 (p : js) (on_ok : js) (on_err : js) : js = meth p "then" [| on_ok; on_err |]

let is_promise (v : js) : bool =
  (not (is_nullish v)) && typeof (get v "then") = "function"

let js_error_value (exn : exn) : js =
  match exn with
  | Js_value v -> v
  | _ ->
    (match Js.Js_error.of_exn exn with
    | Some err -> inject (Js.Js_error.to_error err)
    | None ->
      (match exn with
      | Failure msg -> new_obj (host_object "Error") [| js_string msg |]
      | _ -> js_string (Printexc.to_string exn)))

let lwt_to_promise (t : js Lwt.t) : js =
  new_promise (fun resolve reject ->
      Lwt.on_any t
        (fun value -> ignore (fun_call resolve [| value |]))
        (fun exn -> ignore (fun_call reject [| js_error_value exn |])))

let set_timeout (callback : unit -> unit) (ms : float) : js =
  let fn = js_fun0 (fun () -> callback (); undefined) in
  fun_call (get (global ()) "setTimeout") [| fn; js_float ms |]

let clear_timeout (id : js) : unit =
  ignore (fun_call (get (global ()) "clearTimeout") [| id |])

let match_media (query : string) : js =
  meth (window ()) "matchMedia" [| js_string query |]

let match_media_matches (query : string) : bool =
  is_truthy (get (match_media query) "matches")

let console_log (message : string) : unit =
  ignore (meth (host_object "console") "log" [| js_string message |])

let atob (s : string) : string =
  to_string (fun_call (get (global ()) "atob") [| js_string s |])

let string_from_char_code codes =
  let ctor = host_object "String" in
  let args = Array.map js_int codes in
  to_string (meth ctor "fromCharCode" args)

let is_error (v : js) : bool = instanceof v (host_object "Error")

let is_dom_exception (v : js) : bool = instanceof v (host_object "DOMException")

let parse_int (v : js) : float =
  to_float (fun_call (get (global ()) "parseInt") [| v |])

let parse_float (v : js) : float =
  to_float (fun_call (get (global ()) "parseFloat") [| v |])

let math () = host_object "Math"

let math_call name args = meth (math ()) name args

let js_round (value : float) : float =
  to_float (math_call "round" [| js_float value |])

let js_floor (value : float) : float =
  to_float (math_call "floor" [| js_float value |])

let js_abs (value : float) : float =
  to_float (math_call "abs" [| js_float value |])

let js_min a b = to_float (math_call "min" [| js_float a; js_float b |])
let js_max a b = to_float (math_call "max" [| js_float a; js_float b |])
let js_pow a b = to_float (math_call "pow" [| js_float a; js_float b |])

let text_encoder_encode (s : js) : js =
  let ctor = host_object "TextEncoder" in
  let enc = new_obj ctor [||] in
  meth enc "encode" [| s |]

let create_element (tag : string) : js =
  meth (document ()) "createElement" [| js_string tag |]
