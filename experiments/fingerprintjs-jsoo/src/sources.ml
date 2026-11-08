open Js_of_ocaml
open Js_host
open Data
open Browser

let algorithm_version = "5.2.0"

let source_names =
  [|
    "userAgentData";
    "fonts";
    "domBlockers";
    "fontPreferences";
    "audio";
    "screenFrame";
    "canvas";
    "osCpu";
    "languages";
    "colorDepth";
    "deviceMemory";
    "screenResolution";
    "hardwareConcurrency";
    "timezone";
    "sessionStorage";
    "localStorage";
    "indexedDB";
    "openDatabase";
    "cpuClass";
    "platform";
    "plugins";
    "touchSupport";
    "vendor";
    "vendorFlavors";
    "cookiesEnabled";
    "colorGamut";
    "invertedColors";
    "forcedColors";
    "monochrome";
    "contrast";
    "reducedMotion";
    "reducedTransparency";
    "hdr";
    "math";
    "pdfViewerEnabled";
    "architecture";
    "applePay";
    "privateClickMeasurement";
    "audioBaseLatency";
    "dateTimeLocale";
    "webGlBasics";
    "webGlExtensions";
  |]

let source_function_names =
  [|
    "getUserAgentData";
    "getFonts";
    "getDomBlockers";
    "getFontPreferences";
    "getAudioFingerprint";
    "getScreenFrame";
    "getCanvasFingerprint";
    "getOsCpu";
    "getLanguages";
    "getColorDepth";
    "getDeviceMemory";
    "getScreenResolution";
    "getHardwareConcurrency";
    "getTimezone";
    "getSessionStorage";
    "getLocalStorage";
    "getIndexedDB";
    "getOpenDatabase";
    "getCpuClass";
    "getPlatform";
    "getPlugins";
    "getTouchSupport";
    "getVendor";
    "getVendorFlavors";
    "areCookiesEnabled";
    "getColorGamut";
    "areColorsInverted";
    "areColorsForced";
    "getMonochromeDepth";
    "getContrastPreference";
    "isMotionReduced";
    "isTransparencyReduced";
    "isHDR";
    "getMathFingerprint";
    "isPdfViewerEnabled";
    "getArchitecture";
    "getApplePayState";
    "getPrivateClickMeasurement";
    "getAudioContextBaseLatency";
    "getDateTimeLocale";
    "getWebGlBasics";
    "getWebGlExtensions";
  |]

let js_undefined_or_string v =
  if is_undefined v || is_null v then undefined else v

let js_undefined_or_number f =
  if classify_float f = FP_nan then undefined else js_float f

(* ---------- userAgentData ---------- *)

let is_grease_brand brand =
  let re = new%js Js.regExp_withFlags (Js.string "not") (Js.string "i") in
  Js.to_bool (re##test (Js.string brand))

let get_user_agent_data (_options : js) : js =
  let n = navigator () in
  let ua = get n "userAgentData" in
  if not (is_truthy ua) then undefined
  else
    let brands_raw = get ua "brands" in
    let filtered = array () in
    let n_brands = array_length brands_raw in
    for i = 0 to n_brands - 1 do
      let item = array_get brands_raw i in
      let brand = to_string (get item "brand") in
      if not (is_grease_brand brand) then array_push filtered (js_string brand)
    done;
    let brands =
      if array_length filtered > 1 then
        let kept = array () in
        for i = 0 to array_length filtered - 1 do
          let b = array_get filtered i in
          if to_string b <> "Chromium" then array_push kept b
        done;
        kept
      else filtered
    in
    let result =
      obj_fields
        [|
          "brands", brands;
          "mobile", get ua "mobile";
          "platform", get ua "platform";
        |]
    in
    let get_hev = get ua "getHighEntropyValues" in
    if not (is_truthy get_hev) then result
    else
      let hints =
        array_of_list
          (List.map js_string [ "architecture"; "bitness"; "model"; "platformVersion" ])
      in
      let p = call_this get_hev ua [| hints |] in
      then2 p
        (js_fun1 (fun high ->
             set result "architecture" (get high "architecture");
             set result "bitness" (get high "bitness");
             set result "model" (get high "model");
             set result "platformVersion" (get high "platformVersion");
             result))
        (js_fun1 (fun error ->
             if
               is_dom_exception error && to_string (get error "name") = "NotAllowedError"
             then (
               set result "highEntropyStatus" (js_string "not_allowed");
               result)
             else promise_reject error))

(* ---------- fonts ---------- *)

let font_test_string = "mmMwWLliI0O&1"
let font_text_size = "48px"
let base_fonts = [ "monospace"; "sans-serif"; "serif" ]

let font_list =
  [
    "sans-serif-thin";
    "ARNO PRO";
    "Agency FB";
    "Arabic Typesetting";
    "Arial Unicode MS";
    "AvantGarde Bk BT";
    "BankGothic Md BT";
    "Batang";
    "Bitstream Vera Sans Mono";
    "Calibri";
    "Century";
    "Century Gothic";
    "Clarendon";
    "EUROSTILE";
    "Franklin Gothic";
    "Futura Bk BT";
    "Futura Md BT";
    "GOTHAM";
    "Gill Sans";
    "HELV";
    "Haettenschweiler";
    "Helvetica Neue";
    "Humanst521 BT";
    "Leelawadee";
    "Letter Gothic";
    "Levenim MT";
    "Lucida Bright";
    "Lucida Sans";
    "Menlo";
    "MS Mincho";
    "MS Outlook";
    "MS Reference Specialty";
    "MS UI Gothic";
    "MT Extra";
    "MYRIAD PRO";
    "Marlett";
    "Meiryo UI";
    "Microsoft Uighur";
    "Minion Pro";
    "Monotype Corsiva";
    "PMingLiU";
    "Pristina";
    "SCRIPTINA";
    "Segoe UI Light";
    "Serifa";
    "SimHei";
    "Small Fonts";
    "Staccato222 BT";
    "TRAJAN PRO";
    "Univers CE 55 Medium";
    "Vrinda";
    "ZWAdobeF";
  ]

let get_fonts (_options : js) : js =
  Host_dom.with_iframe
    (fun _iframe iwindow ->
      let doc = get iwindow "document" in
      let holder = get doc "body" in
      set (get holder "style") "fontSize" (js_string font_text_size);
      let spans_container = meth doc "createElement" [| js_string "div" |] in
      ignore
        (meth (get spans_container "style") "setProperty"
           [| js_string "visibility"; js_string "hidden"; js_string "important" |]);
      let create_span font_family =
        let span = meth doc "createElement" [| js_string "span" |] in
        let style = get span "style" in
        set style "position" (js_string "absolute");
        set style "top" (js_string "0");
        set style "left" (js_string "0");
        set style "fontFamily" (js_string font_family);
        set span "textContent" (js_string font_test_string);
        ignore (meth spans_container "appendChild" [| span |]);
        span
      in
      let default_width = Hashtbl.create 4 in
      let default_height = Hashtbl.create 4 in
      let base_spans = List.map create_span base_fonts in
      let font_spans =
        List.map
          (fun font ->
            ( font,
              List.map
                (fun base -> create_span (Printf.sprintf "'%s',%s" font base))
                base_fonts ))
          font_list
      in
      ignore (meth holder "appendChild" [| spans_container |]);
      List.iter2
        (fun base span ->
          Hashtbl.add default_width base (to_float (get span "offsetWidth"));
          Hashtbl.add default_height base (to_float (get span "offsetHeight")))
        base_fonts base_spans;
      let available = array () in
      List.iter
        (fun (font, spans) ->
          let differs =
            List.exists2
              (fun base span ->
                to_float (get span "offsetWidth") <> Hashtbl.find default_width base
                || to_float (get span "offsetHeight") <> Hashtbl.find default_height base)
              base_fonts spans
          in
          if differs then array_push available (js_string font))
        font_spans;
      available)
    ()

(* ---------- font preferences ---------- *)

let font_pref_default_text = "mmMwWLliI0fiflO&1"

let font_presets =
  [
    "default", [];
    "apple", [ "font", "-apple-system-body" ];
    "serif", [ "fontFamily", "serif" ];
    "sans", [ "fontFamily", "sans-serif" ];
    "mono", [ "fontFamily", "monospace" ];
    "min", [ "fontSize", "1px" ];
    "system", [ "fontFamily", "system-ui" ];
  ]

let normalize_font_measurement value =
  let decimals = if is_android () then 0 else 3 in
  let pow = js_pow 10. (float_of_int decimals) in
  js_floor (value *. pow) /. pow

let get_font_preferences (_options : js) : js =
  let container_width = 4000. in
  let initial =
    js_string
      {|<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1">|}
  in
  Host_dom.with_iframe
    (fun _iframe iframe_window ->
      let iframe_document = get iframe_window "document" in
      let iframe_body = get iframe_document "body" in
      let body_style = get iframe_body "style" in
      set body_style "width" (js_string (Printf.sprintf "%gpx" container_width));
      set body_style "webkitTextSizeAdjust" (js_string "none");
      set body_style "textSizeAdjust" (js_string "none");
      if is_chromium () then
        set body_style "zoom"
          (js_string
             (Printf.sprintf "%g" (1. /. to_float (get iframe_window "devicePixelRatio"))))
      else if is_webkit () then set body_style "zoom" (js_string "reset");
      let words = int_of_float (container_width /. 20.) in
      let rec repeat n acc = if n <= 0 then acc else repeat (n - 1) ("word " ^ acc) in
      let lines = meth iframe_document "createElement" [| js_string "div" |] in
      set lines "textContent" (js_string (String.trim (repeat words "")));
      ignore (meth iframe_body "appendChild" [| lines |]);
      let elements = Hashtbl.create 16 in
      List.iter
        (fun (key, style) ->
          let element = meth iframe_document "createElement" [| js_string "span" |] in
          set element "textContent" (js_string font_pref_default_text);
          set (get element "style") "whiteSpace" (js_string "nowrap");
          List.iter
            (fun (name, value) -> set (get element "style") name (js_string value))
            style;
          Hashtbl.add elements key element;
          ignore (meth iframe_body "append" [| meth iframe_document "createElement" [| js_string "br" |]; element |]))
        font_presets;
      let should_normalize = is_chromium () && is_chromium_128_or_newer () in
      let sizes = obj () in
      List.iter
        (fun (key, _) ->
          let element = Hashtbl.find elements key in
          let raw = to_float (get (meth element "getBoundingClientRect" [||]) "width") in
          let size =
            if should_normalize then
              normalize_font_measurement (raw *. to_float (get iframe_window "devicePixelRatio"))
            else raw
          in
          set sizes key (js_float size))
        font_presets;
      sizes)
    ~initial_html:initial ()

(* ---------- dom blockers ---------- *)

let force_show element =
  let style = get element "style" in
  ignore (meth style "setProperty" [| js_string "visibility"; js_string "hidden"; js_string "important" |]);
  ignore (meth style "setProperty" [| js_string "display"; js_string "block"; js_string "important" |])

let get_blocked_selectors selectors =
  let d = document () in
  let root = create_element "div" in
  let elements = Array.make (List.length selectors) undefined in
  let blocked = Hashtbl.create 64 in
  force_show root;
  List.iteri
    (fun i selector ->
      let element = Host_dom.selector_to_element selector in
      if to_string (get element "tagName") = "DIALOG" then ignore (meth element "show" [||]);
      let holder = create_element "div" in
      force_show holder;
      ignore (meth holder "appendChild" [| element |]);
      ignore (meth root "appendChild" [| holder |]);
      elements.(i) <- element)
    selectors;
  let rec wait_body () =
    if is_truthy (get d "body") then Lwt.return_unit
    else Lwt.bind (Async.wait_lwt 50.) wait_body
  in
  let open Lwt.Infix in
  wait_body () >>= fun () ->
  ignore (meth (get d "body") "appendChild" [| root |]);
  (try
     List.iteri
       (fun i selector ->
         if not (is_truthy (get elements.(i) "offsetParent")) then Hashtbl.replace blocked selector true)
       selectors
   with _ -> ());
  let parent = get root "parentNode" in
  if is_truthy parent then ignore (meth parent "removeChild" [| root |]);
  Lwt.return blocked

let get_dom_blockers (options : js) : js =
  if not (is_webkit () || is_android ()) then undefined
  else
    let debug = (not (is_undefined options)) && is_truthy (get options "debug") in
    let all_selectors =
      List.concat (List.map (fun (_, sels) -> sels) Blockers.filters)
    in
    lwt_to_promise
      (let open Lwt.Infix in
       get_blocked_selectors all_selectors >>= fun blocked ->
       if debug then (
         let buf = Buffer.create 256 in
         Buffer.add_string buf "DOM blockers debug:\n```";
         List.iter
           (fun (name, sels) ->
             Buffer.add_string buf ("\n" ^ name ^ ":");
             List.iter
               (fun sel ->
                 let mark = if Hashtbl.mem blocked sel then "🚫" else "➡️" in
                 Buffer.add_string buf ("\n  " ^ mark ^ " " ^ sel))
               sels)
           Blockers.filters;
         Buffer.add_string buf "\n```";
         console_log (Buffer.contents buf));
       let active = array () in
       List.iter
         (fun (name, sels) ->
           let blocked_count =
             List.fold_left (fun acc sel -> if Hashtbl.mem blocked sel then acc + 1 else acc) 0 sels
           in
           if float_of_int blocked_count > float_of_int (List.length sels) *. 0.6 then
             array_push active (js_string name))
         Blockers.filters;
       ignore (meth active "sort" [||]);
       Lwt.return active)

(* ---------- audio ---------- *)

let audio_known_for_suspending = -1.
let audio_not_supported = -2.
let audio_timeout = -3.
let audio_known_for_antifingerprinting = -4.

let does_browser_suspend_audio () =
  is_webkit () && (not (is_desktop_webkit ())) && not (is_webkit_606_or_newer ())

let does_browser_audio_antifingerprint () =
  (is_webkit () && is_webkit_616_or_newer () && is_safari_webkit ())
  || (is_chromium () && is_samsung_internet () && is_chromium_122_or_newer ())

let start_rendering_audio context =
  let render_try_max = 3 in
  let render_retry_delay = 500. in
  let running_max_await = 500. in
  let running_sufficient = 5000. in
  let finalize = ref (fun () -> ()) in
  let result =
    new_promise (fun resolve reject ->
        let is_finalized = ref false in
        let render_try_count = ref 0 in
        let started_running_at = ref 0. in
        set context "oncomplete"
          (js_fun1 (fun event ->
               ignore (fun_call resolve [| get event "renderedBuffer" |]);
               undefined));
        let start_running_timeout () =
          let delay =
            js_min running_max_await (!started_running_at +. running_sufficient -. date_now ())
          in
          ignore
            (set_timeout
               (fun () ->
                 let err = new_obj (host_object "Error") [| js_string "timeout" |] in
                 set err "name" (js_string "timeout");
                 ignore (fun_call reject [| err |]))
               delay)
        in
        let rec try_render () =
          try
            let rendering = meth context "startRendering" [||] in
            if is_promise rendering then ignore (Async.suppress_unhandled_rejection rendering);
            let state = to_string (get context "state") in
            if state = "running" then (
              started_running_at := date_now ();
              if !is_finalized then start_running_timeout ())
            else if state = "suspended" then (
              if not (is_truthy (get (document ()) "hidden")) then incr render_try_count;
              if !is_finalized && !render_try_count >= render_try_max then (
                let err = new_obj (host_object "Error") [| js_string "suspended" |] in
                set err "name" (js_string "suspended");
                ignore (fun_call reject [| err |]))
              else ignore (set_timeout try_render render_retry_delay))
          with exn -> ignore (fun_call reject [| js_error_value exn |])
        in
        try_render ();
        finalize :=
          fun () ->
            if not !is_finalized then (
              is_finalized := true;
              if !started_running_at > 0. then start_running_timeout ()))
  in
  result, finalize

let audio_hash signal =
  let n = array_length signal in
  let hash = ref 0. in
  for i = 0 to n - 1 do
    hash := !hash +. js_abs (to_float (array_get signal i))
  done;
  !hash

let get_unstable_audio_fingerprint () : js =
  let w = window () in
  let audio_ctx =
    let a = get w "OfflineAudioContext" in
    if is_truthy a then a else get w "webkitOfflineAudioContext"
  in
  if not (is_truthy audio_ctx) then js_float audio_not_supported
  else if does_browser_suspend_audio () then js_float audio_known_for_suspending
  else
    let hash_from = 4500 in
    let hash_to = 5000 in
    let context = new_obj audio_ctx [| js_int 1; js_int hash_to; js_int 44100 |] in
    let oscillator = meth context "createOscillator" [||] in
    set oscillator "type" (js_string "triangle");
    set (get oscillator "frequency") "value" (js_float 10000.);
    let compressor = meth context "createDynamicsCompressor" [||] in
    set (get compressor "threshold") "value" (js_float (-50.));
    set (get compressor "knee") "value" (js_float 40.);
    set (get compressor "ratio") "value" (js_float 12.);
    set (get compressor "attack") "value" (js_float 0.);
    set (get compressor "release") "value" (js_float 0.25);
    ignore (meth oscillator "connect" [| compressor |]);
    ignore (meth compressor "connect" [| get context "destination" |]);
    ignore (meth oscillator "start" [| js_int 0 |]);
    let render_promise, finalize = start_rendering_audio context in
    let fingerprint =
      Async.suppress_unhandled_rejection
        (then2 render_promise
           (js_fun1 (fun buffer ->
                let data = meth buffer "getChannelData" [| js_int 0 |] in
                let sub = meth data "subarray" [| js_int hash_from |] in
                js_float (audio_hash sub)))
           (js_fun1 (fun error ->
                let name = to_string (get error "name") in
                if name = "timeout" || name = "suspended" then js_float audio_timeout
                else promise_reject error)))
    in
    js_fun0 (fun () ->
        !finalize ();
        fingerprint)

let get_audio_fingerprint (_options : js) : js =
  if does_browser_audio_antifingerprint () then js_float audio_known_for_antifingerprinting
  else get_unstable_audio_fingerprint ()

(* ---------- screen frame ---------- *)

let screen_frame_check_interval = 2500.
let rounding_precision = 10.
let screen_frame_backup : js option ref = ref None
let screen_frame_timeout : js option ref = ref None

let is_frame_size_null frame =
  let rec loop i =
    if i >= 4 then true
    else if is_truthy (array_get frame i) then false
    else loop (i + 1)
  in
  loop 0

let get_current_screen_frame () =
  let s = screen () in
  let avail_top = replace_nan (to_float (get s "availTop")) null in
  let avail_left_raw = replace_nan (to_float (get s "availLeft")) (js_float 0.) in
  let avail_top_num = replace_nan (to_float (get s "availTop")) (js_float 0.) in
  let right =
    replace_nan
      (to_float (get s "width")
      -. to_float (get s "availWidth")
      -. to_float avail_left_raw)
      null
  in
  let bottom =
    replace_nan
      (to_float (get s "height")
      -. to_float (get s "availHeight")
      -. to_float avail_top_num)
      null
  in
  let left = replace_nan (to_float (get s "availLeft")) null in
  array_of_list [ avail_top; right; bottom; left ]

let watch_screen_frame () =
  match !screen_frame_timeout with
  | Some _ -> ()
  | None ->
    let rec check () =
      let frame = get_current_screen_frame () in
      if is_frame_size_null frame then
        screen_frame_timeout := Some (set_timeout check screen_frame_check_interval)
      else (
        screen_frame_backup := Some frame;
        screen_frame_timeout := None)
    in
    check ()

let get_unstable_screen_frame () : js =
  watch_screen_frame ();
  js_fun0 (fun () ->
      lwt_to_promise
        (let open Lwt.Infix in
         let frame = get_current_screen_frame () in
         (if is_frame_size_null frame then
            match !screen_frame_backup with
            | Some backup -> Lwt.return (meth backup "slice" [||])
            | None ->
              if is_truthy (get_fullscreen_element ()) then
                let p = exit_fullscreen () in
                let waiter, wakener = Lwt.task () in
                if is_promise p then (
                  ignore
                    (then2 p
                       (js_fun0 (fun () ->
                            Lwt.wakeup_later wakener ();
                            undefined))
                       (js_fun0 (fun () ->
                            Lwt.wakeup_later wakener ();
                            undefined)));
                  waiter >>= fun () -> Lwt.return (get_current_screen_frame ()))
                else Lwt.return (get_current_screen_frame ())
              else Lwt.return frame
          else Lwt.return frame)
         >>= fun frame ->
         if not (is_frame_size_null frame) then screen_frame_backup := Some frame;
         Lwt.return frame))

let get_screen_frame (_options : js) : js =
  let skip =
    (is_webkit () && is_webkit_616_or_newer () && is_safari_webkit ())
    || (is_gecko () && is_gecko_143_or_newer ())
  in
  if skip then js_fun0 (fun () -> promise_resolve undefined)
  else
    let getter = get_unstable_screen_frame () in
    js_fun0 (fun () ->
        then1 (fun_call getter [||])
          (js_fun1 (fun frame ->
               let process side =
                 if js_equals side null then null
                 else js_float (round (to_float side) ~base:rounding_precision ())
               in
               array_of_list
                 [
                   process (array_get frame 0);
                   process (array_get frame 1);
                   process (array_get frame 2);
                   process (array_get frame 3);
                 ])))

(* ---------- canvas ---------- *)

let image_unsupported = "unsupported"
let image_skipped = "skipped"
let image_unstable = "unstable"

let does_browser_canvas_antifingerprint () =
  (is_webkit () && is_webkit_616_or_newer () && is_safari_webkit ())
  || (is_gecko () && is_gecko_120_or_newer ())

let make_canvas_context () =
  let canvas = create_element "canvas" in
  set canvas "width" (js_int 1);
  set canvas "height" (js_int 1);
  canvas, meth canvas "getContext" [| js_string "2d" |]

let canvas_supported canvas context =
  is_truthy context && is_truthy (get canvas "toDataURL")

let does_support_winding context =
  ignore (meth context "rect" [| js_int 0; js_int 0; js_int 10; js_int 10 |]);
  ignore (meth context "rect" [| js_int 2; js_int 2; js_int 6; js_int 6 |]);
  not (is_truthy (meth context "isPointInPath" [| js_int 5; js_int 5; js_string "evenodd" |]))

let canvas_to_string canvas = meth canvas "toDataURL" [||]

let render_text_image canvas context =
  set canvas "width" (js_int 240);
  set canvas "height" (js_int 60);
  set context "textBaseline" (js_string "alphabetic");
  set context "fillStyle" (js_string "#f60");
  ignore (meth context "fillRect" [| js_int 100; js_int 1; js_int 62; js_int 20 |]);
  set context "fillStyle" (js_string "#069");
  set context "font" (js_string {|11pt "Times New Roman"|});
  let printed = "Cwm fjordbank gly " ^ string_from_char_code [| 55357; 56835 |] in
  ignore (meth context "fillText" [| js_string printed; js_int 2; js_int 15 |]);
  set context "fillStyle" (js_string "rgba(102, 204, 0, 0.2)");
  set context "font" (js_string "18pt Arial");
  ignore (meth context "fillText" [| js_string printed; js_int 4; js_int 45 |])

let render_geometry_image canvas context =
  set canvas "width" (js_int 122);
  set canvas "height" (js_int 110);
  set context "globalCompositeOperation" (js_string "multiply");
  List.iter
    (fun (color, x, y) ->
      set context "fillStyle" (js_string color);
      ignore (meth context "beginPath" [||]);
      ignore
        (meth context "arc"
           [|
             js_float x;
             js_float y;
             js_float 40.;
             js_float 0.;
             js_float (to_float (get (math ()) "PI") *. 2.);
             js_bool true;
           |]);
      ignore (meth context "closePath" [||]);
      ignore (meth context "fill" [||]))
    [ "#f2f", 40., 40.; "#2ff", 80., 40.; "#ff2", 60., 80. ];
  set context "fillStyle" (js_string "#f9c");
  ignore
    (meth context "arc"
       [|
         js_float 60.;
         js_float 60.;
         js_float 60.;
         js_float 0.;
         js_float (to_float (get (math ()) "PI") *. 2.);
         js_bool true;
       |]);
  ignore
    (meth context "arc"
       [|
         js_float 60.;
         js_float 60.;
         js_float 20.;
         js_float 0.;
         js_float (to_float (get (math ()) "PI") *. 2.);
         js_bool true;
       |]);
  ignore (meth context "fill" [| js_string "evenodd" |])

let render_images canvas context =
  render_text_image canvas context;
  let text1 = canvas_to_string canvas in
  let text2 = canvas_to_string canvas in
  if not (js_equals text1 text2) then js_string image_unstable, js_string image_unstable
  else (
    render_geometry_image canvas context;
    canvas_to_string canvas, text1)

let get_unstable_canvas_fingerprint (skip_images : js) : js =
  let skip =
    if is_undefined skip_images then false else is_truthy skip_images
  in
  let canvas, context = make_canvas_context () in
  let winding, geometry, text =
    if not (canvas_supported canvas context) then
      false, js_string image_unsupported, js_string image_unsupported
    else
      let winding = does_support_winding context in
      if skip then winding, js_string image_skipped, js_string image_skipped
      else
        let geometry, text = render_images canvas context in
        winding, geometry, text
  in
  obj_fields [| "winding", js_bool winding; "geometry", geometry; "text", text |]

let get_canvas_fingerprint (_options : js) : js =
  get_unstable_canvas_fingerprint (js_bool (does_browser_canvas_antifingerprint ()))

(* ---------- webgl ---------- *)

let status_no_gl_context = -1.
let status_get_parameter_not_a_function = -2.

let valid_context_parameters =
  let s = new_named "Set" [||] in
  List.iter
    (fun n -> ignore (meth s "add" [| js_int n |]))
    [
      10752; 2849; 2884; 2885; 2886; 2928; 2929; 2930; 2931; 2932; 2960; 2961; 2962; 2963; 2964;
      2965; 2966; 2967; 2968; 2978; 3024; 3042; 3088; 3089; 3106; 3107; 32773; 32777; 32777; 32823;
      32824; 32936; 32937; 32938; 32939; 32968; 32969; 32970; 32971; 3317; 33170; 3333; 3379; 3386;
      33901; 33902; 34016; 34024; 34076; 3408; 3410; 3411; 3412; 3413; 3414; 3415; 34467; 34816;
      34817; 34818; 34819; 34877; 34921; 34930; 35660; 35661; 35724; 35738; 35739; 36003; 36004;
      36005; 36347; 36348; 36349; 37440; 37441; 37443; 7936; 7937; 7938;
    ];
  s

let valid_extension_params =
  let s = new_named "Set" [||] in
  List.iter
    (fun n -> ignore (meth s "add" [| js_int n |]))
    [ 34047; 35723; 36063; 34852; 34853; 34854; 34229; 36392; 36795; 38449 ];
  s

let shader_types = [ "FRAGMENT_SHADER"; "VERTEX_SHADER" ]
let precision_types = [ "LOW_FLOAT"; "MEDIUM_FLOAT"; "HIGH_FLOAT"; "LOW_INT"; "MEDIUM_INT"; "HIGH_INT" ]
let renderer_info_extension_name = "WEBGL_debug_renderer_info"
let polygon_mode_extension_name = "WEBGL_polygon_mode"

let should_avoid_debug_renderer_info () = is_gecko ()
let should_avoid_polygon_mode_extensions () = is_chromium () || is_webkit ()

let is_valid_parameter_getter gl = typeof (get gl "getParameter") = "function"

let get_webgl_context (cache : js) : js =
  let webgl = get cache "webgl" in
  if is_truthy webgl then get webgl "context"
  else
    let canvas = create_element "canvas" in
    let context = ref undefined in
    ignore
      (meth canvas "addEventListener"
         [|
           js_string "webglCreateContextError";
           js_fun0 (fun () ->
               context := undefined;
               undefined);
         |]);
    List.iter
      (fun ty ->
        if not (is_truthy !context) then
          try
            let c = meth canvas "getContext" [| js_string ty |] in
            if is_truthy c then context := c
          with _ -> ())
      [ "webgl"; "experimental-webgl" ];
    set cache "webgl" (obj_fields [| "context", !context |]);
    !context

let get_webgl_context_export (args : js array) : js =
  let cache = if is_undefined (arg args 0) then obj () else arg args 0 in
  get_webgl_context cache

let param_to_string v =
  if is_nullish v then ""
  else
    try
      let s = meth v "toString" [||] in
      if is_nullish s then "" else to_string s
    with _ -> ""

let is_constant_like key =
  typeof key = "string"
  &&
  let re = new%js Js.regExp (Js.string {|[^A-Z0-9_x]|}) in
  not (Js.to_bool (re##test (Js.Unsafe.coerce key)))

let get_constants_from_prototype obj =
  let proto = get obj "__proto__" in
  let keys = object_keys proto in
  let out = array () in
  for i = 0 to array_length keys - 1 do
    let key = array_get keys i in
    if is_constant_like key then array_push out key
  done;
  out

let get_shader_precision gl shader_type precision_type =
  let fmt =
    meth gl "getShaderPrecisionFormat" [| get gl shader_type; get gl precision_type |]
  in
  if not (is_truthy fmt) then array ()
  else array_of_list [ get fmt "rangeMin"; get fmt "rangeMax"; get fmt "precision" ]

let get_webgl_basics (options : js) : js =
  let cache = if is_undefined options then obj () else get options "cache" in
  let cache = if is_undefined cache || is_null cache then obj () else cache in
  let gl = get_webgl_context cache in
  if not (is_truthy gl) then js_float status_no_gl_context
  else if not (is_valid_parameter_getter gl) then js_float status_get_parameter_not_a_function
  else
    let debug_ext =
      if should_avoid_debug_renderer_info () then null
      else meth gl "getExtension" [| js_string renderer_info_extension_name |]
    in
    let version = param_to_string (meth gl "getParameter" [| get gl "VERSION" |]) in
    let vendor = param_to_string (meth gl "getParameter" [| get gl "VENDOR" |]) in
    let vendor_unmasked =
      if is_truthy debug_ext then
        param_to_string (meth gl "getParameter" [| get debug_ext "UNMASKED_VENDOR_WEBGL" |])
      else ""
    in
    let renderer = param_to_string (meth gl "getParameter" [| get gl "RENDERER" |]) in
    let renderer_unmasked =
      if is_truthy debug_ext then
        param_to_string (meth gl "getParameter" [| get debug_ext "UNMASKED_RENDERER_WEBGL" |])
      else ""
    in
    let shading =
      param_to_string (meth gl "getParameter" [| get gl "SHADING_LANGUAGE_VERSION" |])
    in
    obj_fields
      [|
        "version", js_string version;
        "vendor", js_string vendor;
        "vendorUnmasked", js_string vendor_unmasked;
        "renderer", js_string renderer;
        "rendererUnmasked", js_string renderer_unmasked;
        "shadingLanguageVersion", js_string shading;
      |]

let get_webgl_extensions (options : js) : js =
  let cache = if is_undefined options then obj () else get options "cache" in
  let cache = if is_undefined cache || is_null cache then obj () else cache in
  let gl = get_webgl_context cache in
  if not (is_truthy gl) then js_float status_no_gl_context
  else if not (is_valid_parameter_getter gl) then js_float status_get_parameter_not_a_function
  else
    let extensions = meth gl "getSupportedExtensions" [||] in
    let context_attributes = meth gl "getContextAttributes" [||] in
    let unsupported = array () in
    let attributes = array () in
    let parameters = array () in
    let extension_parameters = array () in
    let shader_precisions = array () in
    if is_truthy context_attributes then (
      let keys = object_keys context_attributes in
      for i = 0 to array_length keys - 1 do
        let name = array_get keys i in
        let value = get_key context_attributes name in
        array_push attributes
          (js_string (to_string name ^ "=" ^ to_string (fun_call (host_object "String") [| value |])))
      done);
    let constants = get_constants_from_prototype gl in
    for i = 0 to array_length constants - 1 do
      let constant = array_get constants i in
      let code = get_key gl constant in
      let extra =
        if is_truthy (meth valid_context_parameters "has" [| code |]) then
          "=" ^ to_string (fun_call (host_object "String") [| meth gl "getParameter" [| code |] |])
        else ""
      in
      array_push parameters
        (js_string (to_string constant ^ "=" ^ to_string (fun_call (host_object "String") [| code |]) ^ extra))
    done;
    if is_truthy extensions then
      for i = 0 to array_length extensions - 1 do
        let name = to_string (array_get extensions i) in
        if
          (name = renderer_info_extension_name && should_avoid_debug_renderer_info ())
          || (name = polygon_mode_extension_name && should_avoid_polygon_mode_extensions ())
        then ()
        else
          let extension = meth gl "getExtension" [| js_string name |] in
          if not (is_truthy extension) then array_push unsupported (js_string name)
          else
            let ext_constants = get_constants_from_prototype extension in
            for j = 0 to array_length ext_constants - 1 do
              let constant = array_get ext_constants j in
              let code = get_key extension constant in
              let extra =
                if is_truthy (meth valid_extension_params "has" [| code |]) then
                  "="
                  ^ to_string
                      (fun_call (host_object "String") [| meth gl "getParameter" [| code |] |])
                else ""
              in
              array_push extension_parameters
                (js_string
                   (to_string constant ^ "="
                   ^ to_string (fun_call (host_object "String") [| code |])
                   ^ extra))
            done
      done;
    List.iter
      (fun shader_type ->
        List.iter
          (fun precision_type ->
            let precision = get_shader_precision gl shader_type precision_type in
            let joined = meth precision "join" [| js_string "," |] in
            array_push shader_precisions
              (js_string (shader_type ^ "." ^ precision_type ^ "=" ^ to_string joined)))
          precision_types)
      shader_types;
    ignore (meth extension_parameters "sort" [||]);
    ignore (meth parameters "sort" [||]);
    obj_fields
      [|
        "contextAttributes", attributes;
        "parameters", parameters;
        "shaderPrecisions", shader_precisions;
        "extensions", extensions;
        "extensionParameters", extension_parameters;
        "unsupportedExtensions", unsupported;
      |]

(* ---------- simple sources ---------- *)

let get_os_cpu _ = js_undefined_or_string (get (navigator ()) "oscpu")

let get_languages _ =
  let n = navigator () in
  let result = array () in
  let language =
    let a = get n "language" in
    if is_truthy a then a
    else
      let b = get n "userLanguage" in
      if is_truthy b then b
      else
        let c = get n "browserLanguage" in
        if is_truthy c then c else get n "systemLanguage"
  in
  if not (is_undefined language) then array_push result (array_of_list [ language ]);
  let languages = get n "languages" in
  if is_truthy (get (host_object "Array") "isArray") && is_truthy (fun_call (get (host_object "Array") "isArray") [| languages |])
  then (
    if not (is_chromium () && is_chromium_86_or_newer ()) then array_push result languages)
  else if typeof languages = "string" then
    if is_truthy languages then array_push result (meth languages "split" [| js_string "," |]);
  result

let get_color_depth _ = get (screen ()) "colorDepth"

let get_device_memory _ =
  replace_nan (to_float (get (navigator ()) "deviceMemory")) undefined

let get_unstable_screen_resolution () =
  let s = screen () in
  let parse v = replace_nan (to_int v) null in
  let dimensions = array_of_list [ parse (get s "width"); parse (get s "height") ] in
  ignore (meth (meth dimensions "sort" [||]) "reverse" [||]);
  dimensions

let get_screen_resolution _ =
  if is_webkit () && is_webkit_616_or_newer () && is_safari_webkit () then undefined
  else get_unstable_screen_resolution ()

let get_unstable_hardware_concurrency () =
  replace_nan (to_int (get (navigator ()) "hardwareConcurrency")) undefined

let get_hardware_concurrency _ =
  let value = get_unstable_hardware_concurrency () in
  if (not (is_undefined value)) && is_gecko () && is_gecko_143_or_newer () then
    js_float (if to_float value >= 8. then 8. else 4.)
  else value

let get_timezone_offset () =
  let year = to_float (meth (new_named "Date" [||]) "getFullYear" [||]) in
  let jan = new_named "Date" [| js_float year; js_int 0; js_int 1 |] in
  let jul = new_named "Date" [| js_float year; js_int 6; js_int 1 |] in
  js_max (to_float (meth jan "getTimezoneOffset" [||])) (to_float (meth jul "getTimezoneOffset" [||]))

let get_timezone _ =
  let intl = get (window ()) "Intl" in
  let dtf = if is_truthy intl then get intl "DateTimeFormat" else undefined in
  if is_truthy dtf then (
    let timezone = get (meth (new_obj dtf [||]) "resolvedOptions" [||]) "timeZone" in
    if is_truthy timezone then timezone
    else
      let offset = -.get_timezone_offset () in
      js_string (Printf.sprintf "UTC%s%g" (if offset >= 0. then "+" else "") offset))
  else
    let offset = -.get_timezone_offset () in
    js_string (Printf.sprintf "UTC%s%g" (if offset >= 0. then "+" else "") offset)

let get_session_storage _ =
  try js_bool (is_truthy (get (window ()) "sessionStorage")) with _ -> js_bool true

let get_local_storage _ =
  try js_bool (is_truthy (get (window ()) "localStorage")) with _ -> js_bool true

let get_indexed_db _ =
  if is_trident () || is_edge_html () then undefined
  else try js_bool (is_truthy (get (window ()) "indexedDB")) with _ -> js_bool true

let get_open_database _ = js_bool (is_truthy (get (window ()) "openDatabase"))

let get_cpu_class _ = js_undefined_or_string (get (navigator ()) "cpuClass")

let get_platform _ =
  let platform = get (navigator ()) "platform" in
  if to_string platform = "MacIntel" then
    if is_webkit () && not (is_desktop_webkit ()) then
      js_string (if is_ipad () then "iPad" else "iPhone")
    else platform
  else platform

let get_plugins _ =
  let raw = get (navigator ()) "plugins" in
  if not (is_truthy raw) then undefined
  else
    let plugins = array () in
    for i = 0 to array_length raw - 1 do
      let plugin = array_get raw i in
      if is_truthy plugin then (
        let mime_types = array () in
        for j = 0 to array_length plugin - 1 do
          let mime = array_get plugin j in
          array_push mime_types
            (obj_fields [| "type", get mime "type"; "suffixes", get mime "suffixes" |])
        done;
        array_push plugins
          (obj_fields
             [|
               "name", get plugin "name";
               "description", get plugin "description";
               "mimeTypes", mime_types;
             |]))
    done;
    plugins

let get_touch_support _ =
  let n = navigator () in
  let max_touch =
    if not (is_undefined (get n "maxTouchPoints")) then to_int (get n "maxTouchPoints")
    else if not (is_undefined (get n "msMaxTouchPoints")) then to_float (get n "msMaxTouchPoints")
    else 0.
  in
  let touch_event =
    try
      ignore (meth (document ()) "createEvent" [| js_string "TouchEvent" |]);
      true
    with _ -> false
  in
  obj_fields
    [|
      "maxTouchPoints", js_float max_touch;
      "touchEvent", js_bool touch_event;
      "touchStart", js_bool (js_in_s "ontouchstart" (window ()));
    |]

let get_vendor _ =
  let v = get (navigator ()) "vendor" in
  if is_truthy v then v else js_string ""

let vendor_flavor_keys =
  [
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
  ]

let get_vendor_flavors _ =
  let w = window () in
  let flavors = array () in
  List.iter
    (fun key ->
      let value = get w key in
      if is_truthy value && typeof value = "object" then array_push flavors (js_string key))
    vendor_flavor_keys;
  ignore (meth flavors "sort" [||]);
  flavors

let are_cookies_enabled _ =
  let d = document () in
  try
    set d "cookie" (js_string "cookietest=1; SameSite=Strict;");
    let result =
      int_of_float (to_float (meth (get d "cookie") "indexOf" [| js_string "cookietest=" |])) <> -1
    in
    set d "cookie" (js_string "cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT");
    js_bool result
  with _ -> js_bool false

let get_color_gamut _ =
  let rec loop = function
    | [] -> undefined
    | g :: rest ->
      if match_media_matches (Printf.sprintf "(color-gamut: %s)" g) then js_string g else loop rest
  in
  loop [ "rec2020"; "p3"; "srgb" ]

let media_bool pos neg query_fmt =
  if match_media_matches (Printf.sprintf query_fmt pos) then js_bool true
  else if match_media_matches (Printf.sprintf query_fmt neg) then js_bool false
  else undefined

let are_colors_inverted _ = media_bool "inverted" "none" "(inverted-colors: %s)"
let are_colors_forced _ = media_bool "active" "none" "(forced-colors: %s)"

let get_monochrome_depth _ =
  if not (match_media_matches "(min-monochrome: 0)") then undefined
  else
    let rec loop i =
      if i > 100 then failwith "Too high value"
      else if match_media_matches (Printf.sprintf "(max-monochrome: %d)" i) then js_int i
      else loop (i + 1)
    in
    loop 0

let get_contrast_preference _ =
  if match_media_matches "(prefers-contrast: no-preference)" then js_int 0
  else if
    match_media_matches "(prefers-contrast: high)" || match_media_matches "(prefers-contrast: more)"
  then js_int 1
  else if
    match_media_matches "(prefers-contrast: low)" || match_media_matches "(prefers-contrast: less)"
  then js_int (-1)
  else if match_media_matches "(prefers-contrast: forced)" then js_int 10
  else undefined

let is_motion_reduced _ = media_bool "reduce" "no-preference" "(prefers-reduced-motion: %s)"
let is_transparency_reduced _ = media_bool "reduce" "no-preference" "(prefers-reduced-transparency: %s)"
let is_hdr _ = media_bool "high" "standard" "(dynamic-range: %s)"

let math_or_fallback name =
  let fn = get (math ()) name in
  if typeof fn = "function" then fun x -> to_float (fun_call fn [| js_float x |]) else fun _ -> 0.

let get_math_fingerprint _ =
  let acos = math_or_fallback "acos" in
  let acosh = math_or_fallback "acosh" in
  let asin = math_or_fallback "asin" in
  let asinh = math_or_fallback "asinh" in
  let atanh = math_or_fallback "atanh" in
  let atan = math_or_fallback "atan" in
  let sin = math_or_fallback "sin" in
  let sinh = math_or_fallback "sinh" in
  let cos = math_or_fallback "cos" in
  let cosh = math_or_fallback "cosh" in
  let tan = math_or_fallback "tan" in
  let tanh = math_or_fallback "tanh" in
  let exp = math_or_fallback "exp" in
  let expm1 = math_or_fallback "expm1" in
  let log1p = math_or_fallback "log1p" in
  let log x = to_float (math_call "log" [| js_float x |]) in
  let sqrt x = to_float (math_call "sqrt" [| js_float x |]) in
  let pi = to_float (get (math ()) "PI") in
  let pow_pi value = js_pow pi value in
  let acosh_pf value = log (value +. sqrt ((value *. value) -. 1.)) in
  let asinh_pf value = log (value +. sqrt ((value *. value) +. 1.)) in
  let atanh_pf value = log ((1. +. value) /. (1. -. value)) /. 2. in
  let sinh_pf value = exp value -. 1. /. exp value /. 2. in
  let cosh_pf value = (exp value +. 1. /. exp value) /. 2. in
  let expm1_pf value = exp value -. 1. in
  let tanh_pf value = (exp (2. *. value) -. 1.) /. (exp (2. *. value) +. 1.) in
  let log1p_pf value = log (1. +. value) in
  obj_fields
    [|
      "acos", js_float (acos 0.123124234234234242);
      "acosh", js_float (acosh 1e308);
      "acoshPf", js_float (acosh_pf 1e154);
      "asin", js_float (asin 0.123124234234234242);
      "asinh", js_float (asinh 1.);
      "asinhPf", js_float (asinh_pf 1.);
      "atanh", js_float (atanh 0.5);
      "atanhPf", js_float (atanh_pf 0.5);
      "atan", js_float (atan 0.5);
      "sin", js_float (sin (-1e300));
      "sinh", js_float (sinh 1.);
      "sinhPf", js_float (sinh_pf 1.);
      "cos", js_float (cos 10.000000000123);
      "cosh", js_float (cosh 1.);
      "coshPf", js_float (cosh_pf 1.);
      "tan", js_float (tan (-1e300));
      "tanh", js_float (tanh 1.);
      "tanhPf", js_float (tanh_pf 1.);
      "exp", js_float (exp 1.);
      "expm1", js_float (expm1 1.);
      "expm1Pf", js_float (expm1_pf 1.);
      "log1p", js_float (log1p 10.);
      "log1pPf", js_float (log1p_pf 10.);
      "powPI", js_float (pow_pi (-100.));
    |]

let is_pdf_viewer_enabled _ = get (navigator ()) "pdfViewerEnabled"

let get_architecture _ =
  let f = new_named "Float32Array" [| js_int 1 |] in
  let u8 = new_named "Uint8Array" [| get f "buffer" |] in
  array_set f 0 (js_float infinity);
  let cur = array_get f 0 in
  array_set f 0 (js_float (to_float cur -. to_float cur));
  array_get u8 3

let get_apple_pay_state _ =
  let session = get (window ()) "ApplePaySession" in
  if is_nullish session || typeof (get session "canMakePayments") <> "function" then js_int (-1)
  else if Host_dom.is_any_parent_cross_origin () then js_int (-3)
  else
    try
      if is_truthy (meth session "canMakePayments" [||]) then js_int 1 else js_int 0
    with exn ->
      let error = js_error_value exn in
      let re = new%js Js.regExp_withFlags (Js.string {|\bfrom\b.*\binsecure\b|}) (Js.string "i") in
      if
        is_error error
        && to_string (get error "name") = "InvalidAccessError"
        && Js.to_bool (re##test (Js.Unsafe.coerce (get error "message")))
      then js_int (-2)
      else raise exn

let get_private_click_measurement _ =
  let link = create_element "a" in
  let source_id =
    let a = get link "attributionSourceId" in
    if is_undefined a then get link "attributionsourceid" else a
  in
  if is_undefined source_id then undefined
  else fun_call (host_object "String") [| source_id |]

let get_audio_base_latency _ =
  if not (is_android () || is_webkit ()) then js_int (-2)
  else if not (is_truthy (get (window ()) "AudioContext")) then js_int (-1)
  else
    let latency = get (new_named "AudioContext" [||]) "baseLatency" in
    if is_nullish latency then js_int (-1)
    else if not (is_truthy (fun_call (get (global ()) "isFinite") [| latency |])) then js_int (-3)
    else latency

let get_date_time_locale _ =
  let intl = get (window ()) "Intl" in
  if not (is_truthy intl) then js_int (-1)
  else
    let dtf = get intl "DateTimeFormat" in
    if not (is_truthy dtf) then js_int (-2)
    else
      let locale = get (meth (new_obj dtf [||]) "resolvedOptions" [||]) "locale" in
      if (not (is_truthy locale)) && not (js_equals locale (js_string "")) then js_int (-3)
      else locale

(* ---------- registry ---------- *)

let named_source ?(arity = 0) name f =
  js_fun ~name ~arity (fun args -> f (arg args 0))

let make_sources () : js =
  let sources = obj () in
  let table =
    [|
      named_source source_function_names.(0) get_user_agent_data;
      named_source source_function_names.(1) get_fonts;
      named_source source_function_names.(2) get_dom_blockers;
      named_source source_function_names.(3) get_font_preferences;
      named_source source_function_names.(4) get_audio_fingerprint;
      named_source source_function_names.(5) get_screen_frame;
      named_source source_function_names.(6) get_canvas_fingerprint;
      named_source source_function_names.(7) get_os_cpu;
      named_source source_function_names.(8) get_languages;
      named_source source_function_names.(9) get_color_depth;
      named_source source_function_names.(10) get_device_memory;
      named_source source_function_names.(11) get_screen_resolution;
      named_source source_function_names.(12) get_hardware_concurrency;
      named_source source_function_names.(13) get_timezone;
      named_source source_function_names.(14) get_session_storage;
      named_source source_function_names.(15) get_local_storage;
      named_source source_function_names.(16) get_indexed_db;
      named_source source_function_names.(17) get_open_database;
      named_source source_function_names.(18) get_cpu_class;
      named_source source_function_names.(19) get_platform;
      named_source source_function_names.(20) get_plugins;
      named_source source_function_names.(21) get_touch_support;
      named_source source_function_names.(22) get_vendor;
      named_source source_function_names.(23) get_vendor_flavors;
      named_source source_function_names.(24) are_cookies_enabled;
      named_source source_function_names.(25) get_color_gamut;
      named_source source_function_names.(26) are_colors_inverted;
      named_source source_function_names.(27) are_colors_forced;
      named_source source_function_names.(28) get_monochrome_depth;
      named_source source_function_names.(29) get_contrast_preference;
      named_source source_function_names.(30) is_motion_reduced;
      named_source source_function_names.(31) is_transparency_reduced;
      named_source source_function_names.(32) is_hdr;
      named_source source_function_names.(33) get_math_fingerprint;
      named_source source_function_names.(34) is_pdf_viewer_enabled;
      named_source source_function_names.(35) get_architecture;
      named_source source_function_names.(36) get_apple_pay_state;
      named_source source_function_names.(37) get_private_click_measurement;
      named_source source_function_names.(38) get_audio_base_latency;
      named_source source_function_names.(39) get_date_time_locale;
      named_source ~arity:1 source_function_names.(40) get_webgl_basics;
      named_source ~arity:1 source_function_names.(41) get_webgl_extensions;
    |]
  in
  for i = 0 to Array.length source_names - 1 do
    set sources source_names.(i) table.(i)
  done;
  sources
