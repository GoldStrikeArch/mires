open Host

let wait_ms (duration_ms : float) : js = wait duration_ms

let request_idle_callback_if_available (fallback_timeout : float)
    (deadline_timeout : float) : js =
  let w = window () in
  let ric = get w "requestIdleCallback" in
  if not (truthy ric) then wait (math_min fallback_timeout deadline_timeout)
  else
    new_promise (fun resolve _reject ->
        let cb =
          source0 "" (fun _ ->
              ignore (apply1 resolve undefined);
              undefined)
        in
        let options = object_ () in
        set options "timeout" (of_float deadline_timeout);
        ignore (call2 ric w cb options);
        undefined)

let await_if_async (action : unit -> js) (callback : bool -> js -> js) : unit =
  let run : (unit -> js) -> (bool -> js -> js) -> unit =
    [%mel.raw
      {|
      function(action, callback) {
        function callCb(ok, value) {
          if (typeof callback === "function" && callback.length >= 2) return callback(ok, value);
          return callback(ok)(value);
        }
        try {
          var returned = action();
          if (returned && typeof returned.then === "function") {
            returned.then(
              function(result) { callCb(true, result); },
              function(error) { callCb(false, error); }
            );
          } else {
            callCb(true, returned);
          }
        } catch (error) {
          callCb(false, error);
        }
      }
    |}]
  in
  ignore (apply2 (hide run) (hide action) (hide callback))

let map_with_breaks (items : js) (callback : js -> int -> js)
    (loop_release_interval : float) : js =
  let rec go (i : int) (results : js) (last : float) : js =
    if i >= array_length items then promise_resolve results
    else (
      set_i results i
        (apply_curried2 (hide callback) (get_i items i) (of_int i));
      let now = date_now () in
      if now >= last +. loop_release_interval then
        promise_then (release_event_loop ()) (fun _ -> go (i + 1) results now)
      else go (i + 1) results last)
  in
  let results = array_ () in
  let rec alloc i =
    if i >= array_length items then ()
    else (
      ignore (push results undefined);
      alloc (i + 1))
  in
  alloc 0;
  go 0 results (date_now ())

let prepare_for_sources (delay_fallback : js) : js =
  let delay =
    if is_undefined delay_fallback then 50. else to_float delay_fallback
  in
  request_idle_callback_if_available delay (delay *. 2.)
