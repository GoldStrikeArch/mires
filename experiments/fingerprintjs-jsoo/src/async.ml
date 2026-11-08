open Js_host

let wait (duration_ms : float) ?(resolve_with = undefined) () : js =
  new_promise (fun resolve _reject ->
      ignore
        (set_timeout
           (fun () -> ignore (fun_call resolve [| resolve_with |]))
           duration_ms))

let wait_lwt (duration_ms : float) : unit Lwt.t =
  let waiter, wakener = Lwt.wait () in
  ignore (set_timeout (fun () -> Lwt.wakeup_later wakener ()) duration_ms);
  waiter

let release_event_loop () : unit Lwt.t =
  let p =
    new_promise (fun resolve _reject ->
        let channel = new_named "MessageChannel" [||] in
        let port1 = get channel "port1" in
        let port2 = get channel "port2" in
        set port1 "onmessage" (js_fun0 (fun () -> fun_call resolve [||]));
        ignore (meth port2 "postMessage" [| null |]))
  in
  let waiter, wakener = Lwt.wait () in
  ignore
    (then2 p
       (js_fun1 (fun _ ->
            Lwt.wakeup_later wakener ();
            undefined))
       (js_fun1 (fun err ->
            Lwt.wakeup_later_exn wakener (Failure (to_string (json_stringify err)));
            undefined)));
  waiter

let request_idle_callback_if_available (fallback_timeout : float)
    ?(deadline_timeout = infinity) () : js =
  let w = window () in
  let ric = get w "requestIdleCallback" in
  if is_truthy ric then
    new_promise (fun resolve _reject ->
        let options = obj_fields [| "timeout", js_float deadline_timeout |] in
        let cb = js_fun0 (fun () -> fun_call resolve [||]) in
        ignore (call_this ric w [| cb; options |]))
  else wait (js_min fallback_timeout deadline_timeout) ()

let await_if_async (action : unit -> js) (callback : bool -> js -> unit) : unit =
  try
    let returned = action () in
    if is_promise returned then
      ignore
        (then2 returned
           (js_fun1 (fun result ->
                callback true result;
                undefined))
           (js_fun1 (fun error ->
                callback false error;
                undefined)))
    else callback true returned
  with exn -> callback false (js_error_value exn)

let loop_release_interval = 16.

let map_with_breaks (items : js) (callback : js -> int -> js)
    ?(interval = loop_release_interval) () : js Lwt.t =
  let len = array_length items in
  let results = array () in
  let rec loop i last =
    if i >= len then Lwt.return results
    else (
      array_set results i (callback (array_get items i) i);
      let now = date_now () in
      if now >= last +. interval then
        let last = now in
        Lwt.bind (release_event_loop ()) (fun () -> loop (i + 1) last)
      else loop (i + 1) last)
  in
  loop 0 (date_now ())

let suppress_unhandled_rejection (promise : js) : js =
  ignore (then2 promise undefined (js_fun0 (fun () -> undefined)));
  promise
