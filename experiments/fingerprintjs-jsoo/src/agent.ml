open Js_of_ocaml
open Js_host
open Data

let escape_component_key (key : js) : js =
  let s = meth key "split" [| js_string "\\" |] in
  let s = meth s "join" [| js_string "\\\\" |] in
  let s = meth s "split" [| js_string ":" |] in
  let s = meth s "join" [| js_string "\\:" |] in
  let s = meth s "split" [| js_string "|" |] in
  meth s "join" [| js_string "\\|" |]

let components_to_canonical_string (components : js) : js =
  let keys = object_keys components in
  ignore (meth keys "sort" [||]);
  let parts = array () in
  for i = 0 to array_length keys - 1 do
    let key = array_get keys i in
    let component = get_key components key in
    let value =
      if js_in_s "error" component then js_string "error"
      else
        let serialized = json_stringify (get component "value") in
        if is_undefined serialized then js_string "undefined" else serialized
    in
    let part = array_of_list [ escape_component_key key; js_string ":"; value ] in
    array_push parts (meth part "join" [| js_string "" |])
  done;
  meth parts "join" [| js_string "|" |]

let hash_components (components : js) : js =
  let canonical = components_to_canonical_string components in
  js_string (Hashing.x64hash128_js canonical 0l)

let components_to_debug_string (components : js) : js =
  let replacer =
    js_fun2 (fun _key value -> if is_error value then Misc.error_to_object value else value)
  in
  json_stringify_with components replacer (js_int 2)

let make_lazy_get_result (components : js) : js =
  let result = obj () in
  let cache = ref undefined in
  let getter =
    js_fun0 ~name:"get visitorId" (fun () ->
        if is_undefined !cache then cache := hash_components components;
        !cache)
  in
  let setter =
    js_fun1 ~name:"set visitorId" (fun value ->
        cache := value;
        undefined)
  in
  set_function_name getter "get visitorId";
  set_function_name setter "set visitorId";
  let descriptor =
    obj_fields
      [|
        "get", getter;
        "set", setter;
        "enumerable", js_bool true;
        "configurable", js_bool true;
      |]
  in
  define_property result "visitorId" descriptor;
  set result "confidence" (Confidence.get_confidence components);
  set result "components" components;
  set result "version" (js_string Sources.algorithm_version);
  result

let print_debug result time_between =
  let ua = to_string (get (navigator ()) "userAgent") in
  let visitor = to_string (get result "visitorId") in
  let comps = to_string (components_to_debug_string (get result "components")) in
  let msg =
    Printf.sprintf
      "Copy the text below to get the debug data:\n\n```\nversion: %s\nuserAgent: %s\ntimeBetweenLoadAndGet: %g\nvisitorId: %s\ncomponents: %s\n```"
      Sources.algorithm_version ua time_between visitor comps
  in
  console_log msg

let make_agent (get_components : js) (debug : js) : js =
  let creation_time = date_now () in
  let get_fn =
    js_fun ~name:"get" ~arity:1 (fun args ->
        let get_options = arg args 0 in
        let start_time = date_now () in
        then1 (fun_call get_components [||])
          (js_fun1 (fun components ->
               let result = make_lazy_get_result components in
               let debug_get =
                 (not (is_nullish get_options)) && is_truthy (get get_options "debug")
               in
               if is_truthy debug || debug_get then print_debug result (start_time -. creation_time);
               result)))
  in
  obj_fields [| "get", get_fn |]

let prepare_for_sources (delay_fallback : js) : js =
  let delay = if is_undefined delay_fallback then 50. else to_float delay_fallback in
  Async.request_idle_callback_if_available delay ~deadline_timeout:(delay *. 2.) ()

let monitor : unit -> unit =
  let f =
    Js.Unsafe.pure_js_expr
      {|
      function() {
        if (window.__fpjs_d_m || Math.random() >= 0.001) {
          return;
        }
        try {
          var request = new XMLHttpRequest();
          request.open("get", "https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring", true);
          request.send();
        } catch (error) {
          console.error(error);
        }
      }
    |}
  in
  fun () -> ignore (Js.Unsafe.fun_call f [||])

let load (options : js) : js =
  let options = if is_undefined options || is_null options then obj () else options in
  let delay = get options "delayFallback" in
  let debug = get options "debug" in
  let monitoring = get options "monitoring" in
  if is_undefined monitoring || is_truthy monitoring then monitor ();
  lwt_to_promise
    (let open Lwt.Infix in
     let waiter, wakener = Lwt.wait () in
     let prep = prepare_for_sources delay in
     ignore
       (then2 prep
          (js_fun0 (fun () ->
               Lwt.wakeup_later wakener ();
               undefined))
          (js_fun1 (fun e ->
               Lwt.wakeup_later_exn wakener (Js_value e);
               undefined)));
     waiter >>= fun () ->
     let source_options = obj_fields [| "cache", obj (); "debug", debug |] in
     let get_components =
       Entropy.load_sources (Sources.make_sources ()) source_options (array ()) undefined
     in
     Lwt.return (make_agent get_components debug))

let load_export (args : js array) = load (arg args 0)
let prepare_export (args : js array) = prepare_for_sources (arg args 0)
let hash_export (args : js array) = hash_components (arg args 0)
let debug_export (args : js array) = components_to_debug_string (arg args 0)
