open Js_of_ocaml
open Js_host
open Data

let add_style_string (style : js) (source : string) : unit =
  let parts = meth (js_string source) "split" [| js_string ";" |] in
  let n = array_length parts in
  let re = new%js Js.regExp (Js.string {|^\s*([\w-]+)\s*:\s*(.+?)(\s*!([\w-]+))?\s*$|}) in
  for i = 0 to n - 1 do
    let property = array_get parts i in
    match Js.Opt.to_option (re##exec (Js.Unsafe.coerce property)) with
    | None -> ()
    | Some handle ->
      let m = Js.match_result handle in
      let name = Js.to_string (Js.Optdef.get (Js.array_get m 1) (fun () -> Js.string "")) in
      let value = Js.to_string (Js.Optdef.get (Js.array_get m 2) (fun () -> Js.string "")) in
      let priority =
        match Js.Optdef.to_option (Js.array_get m 4) with
        | None -> ""
        | Some p -> Js.to_string p
      in
      ignore (meth style "setProperty" [| js_string name; js_string value; js_string priority |])
  done

let selector_to_element (selector : string) : js =
  let tag, attributes = parse_simple_css_selector selector in
  let element = create_element (Option.value tag ~default:"div") in
  List.iter
    (fun (name, values) ->
      let value = String.concat " " values in
      if name = "style" then add_style_string (get element "style") value
      else ignore (meth element "setAttribute" [| js_string name; js_string value |]))
    attributes;
  element

let rec is_any_parent_cross_origin ?(current = window ()) () : bool =
  let parent_w = get current "parent" in
  if (not (is_truthy parent_w)) || js_equals parent_w current then false
  else
    try
      let parent_origin = get (get parent_w "location") "origin" in
      let current_origin = get (get current "location") "origin" in
      if not (js_equals parent_origin current_origin) then true
      else is_any_parent_cross_origin ~current:parent_w ()
    with exn ->
      let err = js_error_value exn in
      if is_error err && to_string (get err "name") = "SecurityError" then true else raise exn

let with_iframe (action : js -> js -> js) ?(initial_html = undefined)
    ?(poll_interval = 50.) () : js =
  let d = document () in
  let rec wait_body () =
    if is_truthy (get d "body") then Lwt.return_unit
    else Lwt.bind (Async.wait_lwt poll_interval) wait_body
  in
  let work () =
    let iframe = create_element "iframe" in
    let cleanup () =
      let parent = get iframe "parentNode" in
      if is_truthy parent then ignore (meth parent "removeChild" [| iframe |])
    in
    Lwt.finalize
      (fun () ->
        let open Lwt.Infix in
        wait_body () >>= fun () ->
        let load_p, load_waker = Lwt.wait () in
        let is_complete = ref false in
        let resolve () =
          if not !is_complete then (
            is_complete := true;
            if Lwt.is_sleeping load_p then Lwt.wakeup_later load_waker ())
        in
        let reject err =
          if not !is_complete then (
            is_complete := true;
            if Lwt.is_sleeping load_p then Lwt.wakeup_later_exn load_waker err)
        in
        set iframe "onload" (js_fun0 (fun () -> resolve (); undefined));
        set iframe "onerror"
          (js_fun1 (fun error ->
               reject (Failure "iframe error");
               ignore error;
               undefined));
        let style = get iframe "style" in
        ignore (meth style "setProperty" [| js_string "display"; js_string "block"; js_string "important" |]);
        set style "position" (js_string "absolute");
        set style "top" (js_string "0");
        set style "left" (js_string "0");
        set style "visibility" (js_string "hidden");
        if (not (is_undefined initial_html)) && is_truthy initial_html && js_in_s "srcdoc" iframe
        then set iframe "srcdoc" initial_html
        else set iframe "src" (js_string "about:blank");
        ignore (meth (get d "body") "appendChild" [| iframe |]);
        let rec check_ready_state () =
          if !is_complete then ()
          else
            let cw = get iframe "contentWindow" in
            let ready =
              is_truthy cw
              &&
              let doc = get cw "document" in
              is_truthy doc && to_string (get doc "readyState") = "complete"
            in
            if ready then resolve ()
            else ignore (set_timeout check_ready_state 10.)
        in
        check_ready_state ();
        load_p >>= fun () ->
        let rec wait_iframe_body () =
          let cw = get iframe "contentWindow" in
          let body =
            is_truthy cw
            &&
            let doc = get cw "document" in
            is_truthy doc && is_truthy (get doc "body")
          in
          if body then Lwt.return_unit
          else Lwt.bind (Async.wait_lwt poll_interval) wait_iframe_body
        in
        wait_iframe_body () >>= fun () ->
        let result = action iframe (get iframe "contentWindow") in
        if is_promise result then
          let waiter, wakener = Lwt.wait () in
          ignore
            (then2 result
               (js_fun1 (fun v ->
                    Lwt.wakeup_later wakener v;
                    undefined))
               (js_fun1 (fun err ->
                    Lwt.wakeup_later_exn wakener (Js_value err);
                    undefined)));
          waiter
        else Lwt.return result)
      (fun () -> cleanup (); Lwt.return_unit)
  in
  lwt_to_promise (work ())

let with_iframe_export (args : js array) : js =
  let action = arg args 0 in
  let initial_html = arg args 1 in
  let poll = arg args 2 in
  let poll_interval =
    if is_undefined poll then 50. else to_float poll
  in
  with_iframe
    (fun iframe iwindow -> call_this action undefined [| iframe; iwindow |])
    ~initial_html ~poll_interval ()
