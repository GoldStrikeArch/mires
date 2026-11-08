open Host

let add_style_string (style : js) (source : string) : unit =
  let parts = string_split source ";" in
  let rec loop i =
    if i >= array_length parts then ()
    else
      let property = to_string (get_i parts i) in
      let re = new_re "^\\s*([\\w-]+)\\s*:\\s*(.+?)(\\s*!([\\w-]+))?\\s*$" "" in
      let match_ = re_exec re property in
      (if not (is_null match_) then
         let name = to_string (get_i match_ 1) in
         let value = to_string (get_i match_ 2) in
         let priority = get_i match_ 4 in
         let priority = if is_nullish priority then "" else to_string priority in
         ignore (invoke3 style "setProperty" (of_string name) (of_string value) (of_string priority)));
      loop (i + 1)
  in
  loop 0

let selector_to_element (selector : string) : js =
  let tag, attributes = Data.parse_simple_css_selector selector in
  let tag_name = if is_undefined tag then "div" else to_string tag in
  let element = invoke1 (document ()) "createElement" (of_string tag_name) in
  let names = object_keys attributes in
  let rec loop i =
    if i >= array_length names then ()
    else
      let name = to_string (get_i names i) in
      let value = array_join (get attributes name) " " in
      if name = "style" then add_style_string (get element "style") value
      else ignore (invoke2 element "setAttribute" (of_string name) (of_string value));
      loop (i + 1)
  in
  loop 0;
  element

let is_any_parent_cross_origin () : bool =
  let rec walk (current : js) : js =
    let parent = get current "parent" in
    if is_nullish parent || parent == current then of_bool false
    else
      try_host
        (fun () ->
          let parent_origin = get (get parent "location") "origin" in
          let current_origin = get (get current "location") "origin" in
          if not (strict_eq parent_origin current_origin) then of_bool true
          else walk parent)
        (fun error ->
          if is_error error && to_string (get error "name") = "SecurityError"
          then of_bool true
          else
            let throw : js -> js =
              [%mel.raw {| function(e) { throw e; } |}]
            in
            throw error)
  in
  to_bool (walk (window ()))

let with_iframe (action : js) (initial_html : js) (poll_interval : js) : js =
  let interval =
    if is_undefined poll_interval then 50. else to_float poll_interval
  in
  let rec wait_body () : js =
    let d = document () in
    if truthy (get d "body") then promise_resolve undefined
    else promise_then (wait interval) (fun _ -> wait_body ())
  in
  promise_then (wait_body ()) (fun _ ->
      let d = document () in
      let iframe = invoke1 d "createElement" (of_string "iframe") in
      let load_p =
        new_promise (fun resolve reject ->
            let is_complete = ref false in
            let resolve_fn _ =
              is_complete := true;
              ignore (apply1 resolve undefined);
              undefined
            in
            let reject_fn error =
              is_complete := true;
              ignore (apply1 reject error);
              undefined
            in
            set iframe "onload" (source0 "" resolve_fn);
            set iframe "onerror" (source1 "" reject_fn);
            let style = get iframe "style" in
            ignore
              (invoke3 style "setProperty" (of_string "display")
                 (of_string "block") (of_string "important"));
            set style "position" (of_string "absolute");
            set style "top" (of_string "0");
            set style "left" (of_string "0");
            set style "visibility" (of_string "hidden");
            if (not (is_undefined initial_html)) && has iframe "srcdoc" then
              set iframe "srcdoc" initial_html
            else set iframe "src" (of_string "about:blank");
            ignore (invoke1 (get d "body") "appendChild" iframe);
            let rec check_ready_state () : unit =
              if !is_complete then ()
              else
                let cw = get iframe "contentWindow" in
                let doc = if is_nullish cw then undefined else get cw "document" in
                let ready =
                  if is_nullish doc then undefined else get doc "readyState"
                in
                if (not (is_nullish ready)) && to_string ready = "complete" then
                  ignore (resolve_fn undefined)
                else
                  ignore
                    (set_timeout
                       (source0 "" (fun _ ->
                            check_ready_state ();
                            undefined))
                       10.)
            in
            check_ready_state ();
            undefined)
      in
      let after_load _ =
        let rec wait_iframe_body () : js =
          let cw = get iframe "contentWindow" in
          let doc = if is_nullish cw then undefined else get cw "document" in
          let body = if is_nullish doc then undefined else get doc "body" in
          if truthy body then promise_resolve undefined
          else promise_then (wait interval) (fun _ -> wait_iframe_body ())
        in
        promise_then (wait_iframe_body ()) (fun _ ->
            let cw = get iframe "contentWindow" in
            apply2 action iframe cw)
      in
      let ran = promise_then load_p after_load in
      let cleanup _ =
        let parent = get iframe "parentNode" in
        if not (is_nullish parent) then
          ignore (invoke1 parent "removeChild" iframe);
        undefined
      in
      let wrap : js -> (js -> js) -> js =
        [%mel.raw
          {|
          function(p, cleanup) {
            return p.then(function(v) { cleanup(); return v; }, function(e) { cleanup(); throw e; });
          }
        |}]
      in
      wrap ran cleanup)
