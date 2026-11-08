open Js_of_ocaml
open Js_host
open Data

let count_in (pairs : (string * js) list) : int =
  count_truthy (List.map (fun (key, obj) -> js_in_s key obj) pairs)

let is_trident () =
  let w = window () in
  let n = navigator () in
  count_in
    [
      "MSCSSMatrix", w;
      "msSetImmediate", w;
      "msIndexedDB", w;
      "msMaxTouchPoints", n;
      "msPointerEnabled", n;
    ]
  >= 4

let is_edge_html () =
  let w = window () in
  let n = navigator () in
  count_in [ "msWriteProfilerMark", w; "MSStream", w; "msLaunchUri", n; "msSaveBlob", n ]
  >= 3
  && not (is_trident ())

let vendor_starts_google n =
  let vendor = get n "vendor" in
  if is_nullish vendor then false
  else
    let s = to_string vendor in
    let idx =
      try
        int_of_float
          (to_float (meth (js_string s) "indexOf" [| js_string "Google" |]))
      with _ -> -1
    in
    idx = 0

let is_chromium () =
  let w = window () in
  let n = navigator () in
  count_truthy
    [
      js_in_s "webkitPersistentStorage" n;
      js_in_s "webkitTemporaryStorage" n;
      vendor_starts_google n;
      js_in_s "webkitResolveLocalFileSystemURL" w;
      js_in_s "BatteryManager" w;
      js_in_s "webkitMediaStream" w;
      js_in_s "webkitSpeechGrammar" w;
    ]
  >= 5

let is_webkit () =
  let w = window () in
  let n = navigator () in
  let vendor = get n "vendor" in
  let apple =
    if is_nullish vendor then false
    else
      int_of_float (to_float (meth vendor "indexOf" [| js_string "Apple" |])) = 0
  in
  count_truthy
    [
      js_in_s "ApplePayError" w;
      js_in_s "CSSPrimitiveValue" w;
      js_in_s "Counter" w;
      apple;
      js_in_s "RGBColor" w;
      js_in_s "WebKitMediaKeys" w;
    ]
  >= 4

let is_desktop_webkit () =
  let w = window () in
  let html_element = get w "HTMLElement" in
  let document_ctor = get w "Document" in
  count_truthy
    [
      js_in_s "safari" w;
      not (js_in_s "ongestureend" w);
      not (js_in_s "TouchEvent" w);
      not (js_in_s "orientation" w);
      is_truthy html_element
      && not (js_in_s "autocapitalize" (get html_element "prototype"));
      is_truthy document_ctor && js_in_s "pointerLockElement" (get document_ctor "prototype");
    ]
  >= 4

let is_safari_webkit () =
  let w = window () in
  Misc.is_function_native (get w "print")
  && to_string (fun_call (host_object "String") [| get w "browser" |])
     = "[object WebPageNamespace]"

let is_gecko () =
  let w = window () in
  let style =
    let el = get (document ()) "documentElement" in
    if is_truthy el then get el "style" else obj ()
  in
  count_truthy
    [
      js_in_s "buildID" (navigator ());
      js_in_s "MozAppearance" style;
      js_in_s "onmozfullscreenchange" w;
      js_in_s "mozInnerScreenX" w;
      js_in_s "CSSMozDocumentRule" w;
      js_in_s "CanvasCaptureMediaStream" w;
    ]
  >= 4

let css () = get (window ()) "CSS"

let css_supports_1 query =
  is_truthy (meth (css ()) "supports" [| js_string query |])

let css_supports a b =
  is_truthy (meth (css ()) "supports" [| js_string a; js_string b |])

let is_gecko_120_or_newer () =
  let n = navigator () in
  count_truthy
    [
      js_in_s "userActivation" n;
      css_supports "color" "light-dark(#000, #fff)";
      css_supports "height" "1lh";
      js_in_s "globalPrivacyControl" n;
    ]
  >= 3

let is_gecko_143_or_newer () =
  let proto = get (get (window ()) "CompositionEvent") "prototype" in
  count_truthy
    [
      css_supports_1 "selector(::details-content)";
      css_supports_1 "selector(::before::marker)";
      css_supports_1 "selector(::after::marker)";
      not (js_in_s "locale" proto);
    ]
  >= 3

let is_chromium_86_or_newer () =
  let w = window () in
  count_truthy
    [
      not (js_in_s "MediaSettingsRange" w);
      js_in_s "RTCEncodedAudioFrame" w;
      to_string (fun_call (host_object "String") [| get w "Intl" |]) = "[object Intl]";
      to_string (fun_call (host_object "String") [| get w "Reflect" |]) = "[object Reflect]";
    ]
  >= 3

let is_chromium_122_or_newer () =
  let w = window () in
  let url_pattern = get w "URLPattern" in
  count_truthy
    [
      js_in_s "union" (get (get (global ()) "Set") "prototype");
      js_in_s "Iterator" w;
      is_truthy url_pattern && js_in_s "hasRegExpGroups" (get url_pattern "prototype");
      js_in_s "RGB8" (get (get w "WebGLRenderingContext") "prototype");
    ]
  >= 3

let is_chromium_128_or_newer () =
  let w = window () in
  let d = document () in
  let promise = get w "Promise" in
  let audio_ctx = get w "AudioContext" in
  count_truthy
    [
      is_truthy promise && js_in_s "try" promise;
      js_in_s "caretPositionFromPoint" d;
      is_truthy audio_ctx && js_in_s "onerror" (get audio_ctx "prototype");
      css_supports "ruby-align" "space-around";
    ]
  >= 3

let is_webkit_606_or_newer () =
  let w = window () in
  count_truthy
    [
      js_in_s "DOMRectList" w;
      js_in_s "RTCPeerConnectionIceEvent" w;
      js_in_s "SVGGeometryElement" w;
      js_in_s "ontransitioncancel" w;
    ]
  >= 3

let is_webkit_616_or_newer () =
  let w = window () in
  let n = navigator () in
  let html_button = get w "HTMLButtonElement" in
  count_truthy
    [
      not (js_in_s "getStorageUpdates" n);
      is_truthy html_button && js_in_s "popover" (get html_button "prototype");
      js_in_s "CSSCounterStyleRule" w;
      css_supports_1 "font-size-adjust: ex-height 0.5";
      css_supports_1 "text-transform: full-width";
    ]
  >= 4

let is_ipad () =
  let n = navigator () in
  if to_string (get n "platform") = "iPad" then true
  else
    let s = screen () in
    let width = to_float (get s "width") in
    let height = to_float (get s "height") in
    let ratio = width /. height in
    let element_proto = get (host_object "Element") "prototype" in
    count_truthy
      [
        js_in_s "MediaSource" (window ());
        is_truthy (get element_proto "webkitRequestFullscreen");
        ratio > 0.65 && ratio < 1.53;
      ]
    >= 2

let get_fullscreen_element () : js =
  let d = document () in
  let candidates =
    [
      get d "fullscreenElement";
      get d "msFullscreenElement";
      get d "mozFullScreenElement";
      get d "webkitFullscreenElement";
    ]
  in
  match List.find_opt (fun v -> not (is_nullish v) && is_truthy v) candidates with
  | Some v -> v
  | None ->
    (* TS: `d.fullscreenElement || ... || null` — falsy becomes next, last is null *)
    let rec or_list = function
      | [] -> null
      | v :: rest -> if is_truthy v then v else or_list rest
    in
    or_list candidates

let exit_fullscreen () : js =
  let d = document () in
  let fn =
    let candidates =
      [
        get d "exitFullscreen";
        get d "msExitFullscreen";
        get d "mozCancelFullScreen";
        get d "webkitExitFullscreen";
      ]
    in
    List.find is_truthy candidates
  in
  call_this fn d [||]

let is_android () =
  let is_it_chromium = is_chromium () in
  let is_it_gecko = is_gecko () in
  let w = window () in
  let n = navigator () in
  if is_it_chromium then
    let connection = get n "connection" in
    count_truthy
      [
        not (js_in_s "SharedWorker" w);
        is_truthy connection && js_in_s "ontypechange" connection;
        not (js_in_s "sinkId" (new_obj (get w "Audio") [||]));
      ]
    >= 2
  else if is_it_gecko then
    let app_version = to_string (get n "appVersion") in
    let re = new%js Js.regExp_withFlags (Js.string "android") (Js.string "i") in
    count_truthy
      [
        js_in_s "onorientationchange" w;
        js_in_s "orientation" w;
        Js.to_bool (re##test (Js.string app_version));
      ]
    >= 2
  else false

let is_samsung_internet () =
  let n = navigator () in
  let w = window () in
  let audio_proto = get (get w "Audio") "prototype" in
  let visual_viewport = get w "visualViewport" in
  count_truthy
    [
      js_in_s "srLatency" audio_proto;
      js_in_s "srChannelCount" audio_proto;
      js_in_s "devicePosture" n;
      is_truthy visual_viewport && js_in_s "segments" visual_viewport;
      js_in_s "getTextInformation" (get (host_object "Image") "prototype");
    ]
  >= 3

let export_bool name f =
  js_fun0 ~name (fun () -> js_bool (f ()))
