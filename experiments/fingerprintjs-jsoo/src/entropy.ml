open Js_host
open Data
open Async

let component ~ok value duration =
  let result = obj () in
  set result (if ok then "value" else "error") value;
  set result "duration" (js_float duration);
  result

let load_source (source : js) (source_options : js) : js =
  let load_start = date_now () in
  let source_load_promise =
    new_promise (fun resolve_load _reject ->
        await_if_async
          (fun () -> call_null source [| source_options |])
          (fun ok load_result ->
            let load_duration = date_now () -. load_start in
            if not ok then
              ignore
                (fun_call resolve_load
                   [|
                     js_fun0 (fun () ->
                         component ~ok:false load_result load_duration);
                   |])
            else if typeof load_result = "function" then
              ignore
                (fun_call resolve_load
                   [|
                     js_fun0 (fun () ->
                         new_promise (fun resolve_get _ ->
                             let get_start = date_now () in
                             await_if_async
                               (fun () -> call_this load_result undefined [||])
                               (fun gok gvalue ->
                                 let duration =
                                   load_duration +. date_now () -. get_start
                                 in
                                 ignore
                                   (fun_call resolve_get
                                      [| component ~ok:gok gvalue duration |]))));
                   |])
            else
              ignore
                (fun_call resolve_load
                   [|
                     js_fun0 (fun () ->
                         component ~ok:true load_result load_duration);
                   |])))
  in
  ignore (Async.suppress_unhandled_rejection source_load_promise);
  js_fun0 (fun () ->
      then1 source_load_promise
        (js_fun1 (fun finalize -> fun_call finalize [||])))

let load_sources (sources : js) (source_options : js) (exclude_sources : js)
    (loop_release_interval : js) : js =
  let keys = object_keys sources in
  let included = array () in
  let n = array_length keys in
  for i = 0 to n - 1 do
    let key = array_get keys i in
    if is_undefined exclude_sources || is_null exclude_sources || not (includes exclude_sources key)
    then array_push included key
  done;
  let interval =
    if is_undefined loop_release_interval then Async.loop_release_interval
    else to_float loop_release_interval
  in
  let source_getters_promise =
    Async.suppress_unhandled_rejection
      (lwt_to_promise
         (Async.map_with_breaks included
            (fun source_key _ -> load_source (get_key sources source_key) source_options)
            ~interval ()))
  in
  js_fun0 (fun () ->
      lwt_to_promise
        (let open Lwt.Infix in
         let await_js p =
           let waiter, wakener = Lwt.wait () in
           ignore
             (then2 p
                (js_fun1 (fun v ->
                     Lwt.wakeup_later wakener v;
                     undefined))
                (js_fun1 (fun e ->
                     Lwt.wakeup_later_exn wakener (Js_value e);
                     undefined)));
           waiter
         in
         await_js source_getters_promise >>= fun source_getters ->
         Async.map_with_breaks source_getters
           (fun getter _ -> Async.suppress_unhandled_rejection (fun_call getter [||]))
           ~interval ()
         >>= fun component_promises ->
         await_js (promise_all component_promises) >>= fun component_array ->
         let components = obj () in
         let len = array_length included in
         for i = 0 to len - 1 do
           set_key components (array_get included i) (array_get component_array i)
         done;
         Lwt.return components))

let load_sources_export (args : js array) : js =
  load_sources (arg args 0) (arg args 1) (arg args 2) (arg args 3)

let is_final_result_loaded (load_result : js) = typeof load_result <> "function"

let transform_source (source : js) (transform_value : js) : js =
  let rec transform_maybe_promise value allow_stage =
    if is_promise value then
      then1 value
        (js_fun1 (fun v -> transform_loaded v allow_stage))
    else transform_loaded value allow_stage
  and transform_loaded value allow_stage =
    if allow_stage && typeof value = "function" then
      js_fun0 (fun () ->
          let result = fun_call value [||] in
          transform_maybe_promise result false)
    else fun_call transform_value [| value |]
  in
  js_fun ~arity:1 (fun args ->
      let options = arg args 0 in
      let load_result = call_this source undefined [| options |] in
      transform_maybe_promise load_result true)

let transform_source_export (args : js array) : js =
  transform_source (arg args 0) (arg args 1)
