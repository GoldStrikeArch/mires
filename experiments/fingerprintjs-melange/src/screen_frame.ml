open Host

let screen_frame_check_interval = 2500.
let rounding_precision = 10.

let screen_frame_backup : js ref = ref undefined
let screen_frame_size_timeout_id : js ref = ref undefined

let get_current_screen_frame () : js =
  let s = screen () in
  let replace value replacement = Data.replace_nan value replacement in
  let avail_top = replace (of_float (Data.to_float_js (get s "availTop"))) null in
  let right =
    replace
      (of_float
         (Data.to_float_js (get s "width")
         -. Data.to_float_js (get s "availWidth")
         -. to_float (replace (of_float (Data.to_float_js (get s "availLeft"))) (of_float 0.))))
      null
  in
  let bottom =
    replace
      (of_float
         (Data.to_float_js (get s "height")
         -. Data.to_float_js (get s "availHeight")
         -. to_float (replace (of_float (Data.to_float_js (get s "availTop"))) (of_float 0.))))
      null
  in
  let left = replace (of_float (Data.to_float_js (get s "availLeft"))) null in
  let frame = array_ () in
  ignore (push frame avail_top);
  ignore (push frame right);
  ignore (push frame bottom);
  ignore (push frame left);
  frame

let is_frame_size_null (frame : js) : bool =
  let rec loop i =
    if i >= 4 then true
    else if truthy (get_i frame i) then false
    else loop (i + 1)
  in
  loop 0

let rec check_screen_frame () : unit =
  let frame_size = get_current_screen_frame () in
  if is_frame_size_null frame_size then
    screen_frame_size_timeout_id :=
      of_float
        (set_timeout
           (source0 "" (fun _ ->
                check_screen_frame ();
                undefined))
           screen_frame_check_interval)
  else (
    screen_frame_backup := frame_size;
    screen_frame_size_timeout_id := undefined)

let watch_screen_frame () : unit =
  if not (is_undefined !screen_frame_size_timeout_id) then ()
  else check_screen_frame ()

let get_unstable_screen_frame () : js =
  watch_screen_frame ();
  source0 "" (fun _ ->
      let rec finish (frame_size : js) : js =
        if is_frame_size_null frame_size then
          if not (is_undefined !screen_frame_backup) then
            promise_resolve (array_slice !screen_frame_backup 0)
          else if truthy (Browser.get_fullscreen_element ()) then
            promise_then (Browser.exit_fullscreen ()) (fun _ ->
                finish (get_current_screen_frame ()))
          else done_frame frame_size
        else done_frame frame_size
      and done_frame frame_size =
        if not (is_frame_size_null frame_size) then
          screen_frame_backup := frame_size;
        promise_resolve frame_size
      in
      finish (get_current_screen_frame ()))

let get_screen_frame (_ : js) : js =
  let safari =
    Browser.is_webkit () && Browser.is_webkit_616_or_newer ()
    && Browser.is_safari_webkit ()
  in
  let firefox = Browser.is_gecko () && Browser.is_gecko_143_or_newer () in
  if safari || firefox then source0 "" (fun _ -> promise_resolve undefined)
  else
    let getter = get_unstable_screen_frame () in
    source0 "" (fun _ ->
        promise_then (apply0 getter) (fun frame_size ->
            let process side =
              if is_null side then null
              else of_float (Data.round (to_float side) rounding_precision)
            in
            let out = array_ () in
            ignore (push out (process (get_i frame_size 0)));
            ignore (push out (process (get_i frame_size 1)));
            ignore (push out (process (get_i frame_size 2)));
            ignore (push out (process (get_i frame_size 3)));
            out))
