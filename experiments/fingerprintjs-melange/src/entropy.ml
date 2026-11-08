open Host

let call_source (source : js) (options : js) : js =
  let f : js -> js -> js =
    [%mel.raw {| function(f, a) { return f.call(null, a); } |}]
  in
  f source options

let resolved_component (ok : bool) (value : js) (duration : float) : js =
  let component = object_ () in
  if ok then set component "value" value else set component "error" value;
  set component "duration" (of_float duration);
  promise_resolve component

let component (ok : bool) (value : js) (duration : float) : js =
  let result = object_ () in
  if ok then set result "value" value else set result "error" value;
  set result "duration" (of_float duration);
  result

let stage_getter (load_result : js) (load_duration : float) : js =
  new_promise (fun resolve_get _ ->
      let get_start = date_now () in
      Fp_async.await_if_async
        (fun () -> apply0 load_result)
        (fun ok get_arg ->
          let duration = load_duration +. date_now () -. get_start in
          ignore (apply1 resolve_get (component ok get_arg duration));
          undefined);
      undefined)

let load_source (source : js) (source_options : js) : js =
  let source_load_promise =
    suppress
      (new_promise (fun resolve_load _reject ->
           let load_start = date_now () in
           Fp_async.await_if_async
             (fun () -> call_source source source_options)
             (fun ok load_arg ->
               let load_duration = date_now () -. load_start in
               let finalize =
                 if not ok then
                   source0 "" (fun _ ->
                       resolved_component false load_arg load_duration)
                 else if is_function load_arg then
                   source0 "" (fun _ -> stage_getter load_arg load_duration)
                 else
                   source0 "" (fun _ ->
                       resolved_component true load_arg load_duration)
               in
               ignore (apply1 resolve_load finalize);
               undefined);
           undefined))
  in
  source0 "" (fun _ ->
      promise_then source_load_promise (fun finalize -> apply0 finalize))

let load_sources (sources : js) (source_options : js) (exclude_sources : js)
    (loop_release_interval : js) : js =
  let interval =
    if is_undefined loop_release_interval then 16.
    else to_float loop_release_interval
  in
  let keys = object_keys sources in
  let included = array_ () in
  let rec filter i =
    if i >= array_length keys then ()
    else
      let key = get_i keys i in
      if Data.excludes exclude_sources key then ignore (push included key);
      filter (i + 1)
  in
  filter 0;
  let source_getters_promise =
    suppress
      (Fp_async.map_with_breaks included
         (fun source_key _ -> load_source (get sources (to_string source_key)) source_options)
         interval)
  in
  source0 "" (fun _ ->
      promise_then source_getters_promise (fun source_getters ->
          promise_then
            (Fp_async.map_with_breaks source_getters
               (fun getter _ -> suppress (apply0 getter))
               interval)
            (fun component_promises ->
              promise_then (promise_all component_promises) (fun component_array ->
                  let components = object_ () in
                  let rec assign i =
                    if i >= array_length included then ()
                    else (
                      set components
                        (to_string (get_i included i))
                        (get_i component_array i);
                      assign (i + 1))
                  in
                  assign 0;
                  components))))

let is_final_result_loaded (load_result : js) : bool = not (is_function load_result)

let transform_source (source : js) (transform_value : js) : js =
  let rec transform_maybe_promise (value : js) (allow_stage : bool) : js =
    if is_promise value then
      promise_then value (fun v -> transform_loaded v allow_stage)
    else transform_loaded value allow_stage
  and transform_loaded (value : js) (allow_stage : bool) : js =
    if allow_stage && is_function value then
      source0 "" (fun _ ->
          let result = apply0 value in
          transform_maybe_promise result false)
    else apply1 transform_value value
  in
  source1 "" (fun options ->
      let load_result = apply1 source options in
      transform_maybe_promise load_result true)
