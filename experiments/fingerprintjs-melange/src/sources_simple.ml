open Host

let get_os_cpu (_ : js) : js = get (navigator ()) "oscpu"

let get_languages (_ : js) : js =
  let n = navigator () in
  let result = array_ () in
  let language = get n "language" in
  let language =
    if truthy language then language
    else
      let a = get n "userLanguage" in
      if truthy a then a
      else
        let b = get n "browserLanguage" in
        if truthy b then b else get n "systemLanguage"
  in
  if not (is_undefined language) then
    ignore (push result (let one = array_ () in ignore (push one language); one));
  let languages = get n "languages" in
  if array_is_array languages then (
    if not (Browser.is_chromium () && Browser.is_chromium_86_or_newer ()) then
      ignore (push result languages))
  else if is_string languages then
    if to_string languages <> "" then
      ignore (push result (string_split (to_string languages) ","));
  result

let get_color_depth (_ : js) : js = get (get (window ()) "screen") "colorDepth"

let get_device_memory (_ : js) : js =
  Data.replace_nan
    (of_float (Data.to_float_js (get (navigator ()) "deviceMemory")))
    undefined

let get_unstable_screen_resolution () : js =
  let s = screen () in
  let parse_dimension value = Data.replace_nan (of_float (Data.to_int_js value)) null in
  let dimensions = array_ () in
  ignore (push dimensions (parse_dimension (get s "width")));
  ignore (push dimensions (parse_dimension (get s "height")));
  ignore (array_reverse (array_sort dimensions));
  dimensions

let get_screen_resolution (_ : js) : js =
  if Browser.is_webkit () && Browser.is_webkit_616_or_newer () && Browser.is_safari_webkit ()
  then undefined
  else get_unstable_screen_resolution ()

let get_unstable_hardware_concurrency () : js =
  Data.replace_nan
    (of_float (Data.to_int_js (get (navigator ()) "hardwareConcurrency")))
    undefined

let get_hardware_concurrency (_ : js) : js =
  let value = get_unstable_hardware_concurrency () in
  if (not (is_undefined value)) && Browser.is_gecko () && Browser.is_gecko_143_or_newer ()
  then of_int (if to_float value >= 8. then 8 else 4)
  else value

let get_timezone_offset () : float =
  let date_ctor = get (global_this ()) "Date" in
  let now = construct0 date_ctor in
  let year = invoke0 now "getFullYear" in
  let jan = construct3 date_ctor year (of_int 0) (of_int 1) in
  let jul = construct3 date_ctor year (of_int 6) (of_int 1) in
  math_max
    (Data.to_float_js (invoke0 jan "getTimezoneOffset"))
    (Data.to_float_js (invoke0 jul "getTimezoneOffset"))

let get_timezone (_ : js) : js =
  let intl = get (window ()) "Intl" in
  let dtf = if is_nullish intl then undefined else get intl "DateTimeFormat" in
  if truthy dtf then
    let timezone = get (invoke0 (construct0 dtf) "resolvedOptions") "timeZone" in
    if truthy timezone then timezone
    else
      let offset = -.get_timezone_offset () in
      of_string
        ("UTC"
        ^ (if offset >= 0. then "+" else "")
        ^ number_to_string offset)
  else
    let offset = -.get_timezone_offset () in
    of_string
      ("UTC"
      ^ (if offset >= 0. then "+" else "")
      ^ number_to_string offset)

let get_session_storage (_ : js) : js =
  try_host
    (fun () -> of_bool (truthy (get (window ()) "sessionStorage")))
    (fun _ -> of_bool true)

let get_local_storage (_ : js) : js =
  try_host
    (fun () -> of_bool (truthy (get (window ()) "localStorage")))
    (fun _ -> of_bool true)

let get_indexed_db (_ : js) : js =
  if Browser.is_trident () || Browser.is_edge_html () then undefined
  else
    try_host
      (fun () -> of_bool (truthy (get (window ()) "indexedDB")))
      (fun _ -> of_bool true)

let get_open_database (_ : js) : js =
  of_bool (truthy (get (window ()) "openDatabase"))

let is_source_loaded = object_ ()

let get_cpu_class (_ : js) : js =
  set is_source_loaded "x" (of_bool true);
  get (navigator ()) "cpuClass"

let get_platform (_ : js) : js =
  let platform = get (navigator ()) "platform" in
  if to_string platform = "MacIntel" then
    if Browser.is_webkit () && not (Browser.is_desktop_webkit ()) then
      of_string (if Browser.is_ipad () then "iPad" else "iPhone")
    else platform
  else platform

let get_vendor (_ : js) : js =
  let v = get (navigator ()) "vendor" in
  if truthy v then v else of_string ""

let get_plugins (_ : js) : js =
  let raw = get (navigator ()) "plugins" in
  if not (truthy raw) then undefined
  else
    let plugins = array_ () in
    let rec loop i =
      if i >= array_length raw then ()
      else
        let plugin = get_i raw i in
        if not (truthy plugin) then loop (i + 1)
        else
          let mime_types = array_ () in
          let rec loop_mime j =
            if j >= array_length plugin then ()
            else
              let mime = get_i plugin j in
              let item = object_ () in
              set item "type" (get mime "type");
              set item "suffixes" (get mime "suffixes");
              ignore (push mime_types item);
              loop_mime (j + 1)
          in
          loop_mime 0;
          let item = object_ () in
          set item "name" (get plugin "name");
          set item "description" (get plugin "description");
          set item "mimeTypes" mime_types;
          ignore (push plugins item);
          loop (i + 1)
    in
    loop 0;
    plugins

let get_touch_support (_ : js) : js =
  let n = navigator () in
  let max_touch =
    if not (is_undefined (get n "maxTouchPoints")) then
      Data.to_int_js (get n "maxTouchPoints")
    else if not (is_undefined (get n "msMaxTouchPoints")) then
      to_float (get n "msMaxTouchPoints")
    else 0.
  in
  let touch_event =
    try_host
      (fun () ->
        ignore (invoke1 (document ()) "createEvent" (of_string "TouchEvent"));
        of_bool true)
      (fun _ -> of_bool false)
  in
  let result = object_ () in
  set result "maxTouchPoints" (of_float max_touch);
  set result "touchEvent" touch_event;
  set result "touchStart" (of_bool (has (window ()) "ontouchstart"));
  result

let flavor_keys =
  [|
    "chrome";
    "safari";
    "__crWeb";
    "__gCrWeb";
    "yandex";
    "__yb";
    "__ybro";
    "__firefox__";
    "__edgeTrackingPreventionStatistics";
    "webkit";
    "oprt";
    "samsungAr";
    "ucweb";
    "UCShellJava";
    "puffinDevice";
  |]

let get_vendor_flavors (_ : js) : js =
  let flavors = array_ () in
  let w = window () in
  let n = Js.Array.length flavor_keys in
  let rec loop i =
    if i >= n then ()
    else
      let key = Js.Array.unsafe_get flavor_keys i in
      let value = get w key in
      if truthy value && is_object value then ignore (push flavors (of_string key));
      loop (i + 1)
  in
  loop 0;
  ignore (array_sort flavors);
  flavors

let are_cookies_enabled (_ : js) : js =
  let d = document () in
  try_host
    (fun () ->
      set d "cookie" (of_string "cookietest=1; SameSite=Strict;");
      let result =
        string_index_of (to_string (get d "cookie")) "cookietest=" <> -1
      in
      set d "cookie"
        (of_string "cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT");
      of_bool result)
    (fun _ -> of_bool false)

let get_pdf_viewer_enabled (_ : js) : js = get (navigator ()) "pdfViewerEnabled"

let get_architecture (_ : js) : js =
  let f = new_float32_array 1 in
  let u8 = new_uint8_from_buffer (get f "buffer") in
  set_i f 0 ([%mel.raw {| Infinity |}] : js);
  set_i f 0 (of_float (to_float (get_i f 0) -. to_float (get_i f 0)));
  get_i u8 3

let get_apple_pay_state (_ : js) : js =
  let session = get (window ()) "ApplePaySession" in
  let can = if is_nullish session then undefined else get session "canMakePayments" in
  if not (is_function can) then of_int (-1)
  else if Dom.is_any_parent_cross_origin () then of_int (-3)
  else
    try_host
      (fun () ->
        of_int (if truthy (invoke0 session "canMakePayments") then 1 else 0))
      (fun error ->
        if
          is_error error
          && to_string (get error "name") = "InvalidAccessError"
          && re_test (new_re "\\bfrom\\b.*\\binsecure\\b" "i")
               (string_value (get error "message"))
        then of_int (-2)
        else
          let throw : js -> js =
            [%mel.raw {| function(e) { throw e; } |}]
          in
          throw error)

let get_private_click_measurement (_ : js) : js =
  let link = invoke1 (document ()) "createElement" (of_string "a") in
  let source_id = get link "attributionSourceId" in
  let source_id =
    if is_undefined source_id then get link "attributionsourceid" else source_id
  in
  if is_undefined source_id then undefined else js_string source_id

let get_audio_base_latency (_ : js) : js =
  if not (Browser.is_android () || Browser.is_webkit ()) then of_int (-2)
  else
    let ctor = get (window ()) "AudioContext" in
    if not (truthy ctor) then of_int (-1)
    else
      let latency = get (construct0 ctor) "baseLatency" in
      if is_nullish latency then of_int (-1)
      else if not (is_finite (to_float latency)) then of_int (-3)
      else latency

let get_date_time_locale (_ : js) : js =
  let intl = get (window ()) "Intl" in
  if not (truthy intl) then of_int (-1)
  else
    let dtf = get intl "DateTimeFormat" in
    if not (truthy dtf) then of_int (-2)
    else
      let locale = get (invoke0 (apply0 dtf) "resolvedOptions") "locale" in
      if (not (truthy locale)) && not (strict_eq locale (of_string "")) then
        of_int (-3)
      else locale

let math_or_zero (name : string) (arg : float) : float =
  let fn = math_fn name in
  if is_function fn then to_float (apply1 fn (of_float arg)) else 0.

let get_math_fingerprint (_ : js) : js =
  let pow_pi value = math_pow math_pi value in
  let acosh_pf value = math_log (value +. math_sqrt ((value *. value) -. 1.)) in
  let asinh_pf value = math_log (value +. math_sqrt ((value *. value) +. 1.)) in
  let atanh_pf value = math_log ((1. +. value) /. (1. -. value)) /. 2. in
  let sinh_pf value = math_exp value -. 1. /. math_exp value /. 2. in
  let cosh_pf value = (math_exp value +. 1. /. math_exp value) /. 2. in
  let expm1_pf value = math_exp value -. 1. in
  let tanh_pf value =
    (math_exp (2. *. value) -. 1.) /. (math_exp (2. *. value) +. 1.)
  in
  let log1p_pf value = math_log (1. +. value) in
  let result = object_ () in
  set result "acos" (of_float (math_or_zero "acos" 0.123124234234234242));
  set result "acosh" (of_float (math_or_zero "acosh" 1e308));
  set result "acoshPf" (of_float (acosh_pf 1e154));
  set result "asin" (of_float (math_or_zero "asin" 0.123124234234234242));
  set result "asinh" (of_float (math_or_zero "asinh" 1.));
  set result "asinhPf" (of_float (asinh_pf 1.));
  set result "atanh" (of_float (math_or_zero "atanh" 0.5));
  set result "atanhPf" (of_float (atanh_pf 0.5));
  set result "atan" (of_float (math_or_zero "atan" 0.5));
  set result "sin" (of_float (math_or_zero "sin" (-1e300)));
  set result "sinh" (of_float (math_or_zero "sinh" 1.));
  set result "sinhPf" (of_float (sinh_pf 1.));
  set result "cos" (of_float (math_or_zero "cos" 10.000000000123));
  set result "cosh" (of_float (math_or_zero "cosh" 1.));
  set result "coshPf" (of_float (cosh_pf 1.));
  set result "tan" (of_float (math_or_zero "tan" (-1e300)));
  set result "tanh" (of_float (math_or_zero "tanh" 1.));
  set result "tanhPf" (of_float (tanh_pf 1.));
  set result "exp" (of_float (math_or_zero "exp" 1.));
  set result "expm1" (of_float (math_or_zero "expm1" 1.));
  set result "expm1Pf" (of_float (expm1_pf 1.));
  set result "log1p" (of_float (math_or_zero "log1p" 10.));
  set result "log1pPf" (of_float (log1p_pf 10.));
  set result "powPI" (of_float (pow_pi (-100.)));
  result
