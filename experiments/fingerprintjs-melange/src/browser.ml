open Host

let count_flags (items : bool array) : int =
  let n = Js.Array.length items in
  let rec loop i sum =
    if i >= n then sum
    else
      loop (i + 1)
        (sum + if Js.Array.unsafe_get items i then 1 else 0)
  in
  loop 0 0

let vendor_index (n : js) (needle : string) : int =
  let vendor = get n "vendor" in
  let s = if is_nullish vendor then "" else string_value vendor in
  string_index_of s needle

let is_trident () : bool =
  let w = window () in
  let n = navigator () in
  count_flags
    [|
      has w "MSCSSMatrix";
      has w "msSetImmediate";
      has w "msIndexedDB";
      has n "msMaxTouchPoints";
      has n "msPointerEnabled";
    |]
  >= 4

let is_edge_html () : bool =
  let w = window () in
  let n = navigator () in
  count_flags
    [|
      has w "msWriteProfilerMark";
      has w "MSStream";
      has n "msLaunchUri";
      has n "msSaveBlob";
    |]
  >= 3
  && not (is_trident ())

let is_chromium () : bool =
  let w = window () in
  let n = navigator () in
  count_flags
    [|
      has n "webkitPersistentStorage";
      has n "webkitTemporaryStorage";
      vendor_index n "Google" = 0;
      has w "webkitResolveLocalFileSystemURL";
      has w "BatteryManager";
      has w "webkitMediaStream";
      has w "webkitSpeechGrammar";
    |]
  >= 5

let is_webkit () : bool =
  let w = window () in
  let n = navigator () in
  count_flags
    [|
      has w "ApplePayError";
      has w "CSSPrimitiveValue";
      has w "Counter";
      vendor_index n "Apple" = 0;
      has w "RGBColor";
      has w "WebKitMediaKeys";
    |]
  >= 4

let is_desktop_webkit () : bool =
  let w = window () in
  let html_element = get w "HTMLElement" in
  let document_ctor = get w "Document" in
  count_flags
    [|
      has w "safari";
      not (has w "ongestureend");
      not (has w "TouchEvent");
      not (has w "orientation");
      truthy html_element
      && not (has (get html_element "prototype") "autocapitalize");
      truthy document_ctor && has (get document_ctor "prototype") "pointerLockElement";
    |]
  >= 4

let is_safari_webkit () : bool =
  let w = window () in
  Misc.is_function_native (get w "print")
  && string_value (get w "browser") = "[object WebPageNamespace]"

let gecko_style () : js =
  let el = get (document ()) "documentElement" in
  if is_nullish el then object_ ()
  else
    let style = get el "style" in
    if is_nullish style then object_ () else style

let is_gecko () : bool =
  let w = window () in
  count_flags
    [|
      has (navigator ()) "buildID";
      has (gecko_style ()) "MozAppearance";
      has w "onmozfullscreenchange";
      has w "mozInnerScreenX";
      has w "CSSMozDocumentRule";
      has w "CanvasCaptureMediaStream";
    |]
  >= 4

let css_supports (a : string) (b : string) : bool =
  let css = get (window ()) "CSS" in
  if is_nullish css then false
  else to_bool (invoke2 css "supports" (of_string a) (of_string b))

let css_supports1 (condition : string) : bool =
  let css = get (window ()) "CSS" in
  if is_nullish css then false
  else to_bool (invoke1 css "supports" (of_string condition))

let is_gecko_120_or_newer () : bool =
  let n = navigator () in
  count_flags
    [|
      has n "userActivation";
      css_supports "color" "light-dark(#000, #fff)";
      css_supports "height" "1lh";
      has n "globalPrivacyControl";
    |]
  >= 3

let is_gecko_143_or_newer () : bool =
  let proto = get (get (global_this ()) "CompositionEvent") "prototype" in
  count_flags
    [|
      css_supports1 "selector(::details-content)";
      css_supports1 "selector(::before::marker)";
      css_supports1 "selector(::after::marker)";
      not (has proto "locale");
    |]
  >= 3

let is_chromium_86_or_newer () : bool =
  let w = window () in
  count_flags
    [|
      not (has w "MediaSettingsRange");
      has w "RTCEncodedAudioFrame";
      string_value (get w "Intl") = "[object Intl]";
      string_value (get w "Reflect") = "[object Reflect]";
    |]
  >= 3

let is_chromium_122_or_newer () : bool =
  let w = window () in
  let url_pattern = get w "URLPattern" in
  let set_proto = get (get (global_this ()) "Set") "prototype" in
  let gl_proto = get (get (global_this ()) "WebGLRenderingContext") "prototype" in
  count_flags
    [|
      has set_proto "union";
      has w "Iterator";
      truthy url_pattern && has (get url_pattern "prototype") "hasRegExpGroups";
      has gl_proto "RGB8";
    |]
  >= 3

let is_chromium_128_or_newer () : bool =
  let w = window () in
  let d = document () in
  let promise = get w "Promise" in
  let audio = get w "AudioContext" in
  count_flags
    [|
      truthy promise && has promise "try";
      has d "caretPositionFromPoint";
      truthy audio && has (get audio "prototype") "onerror";
      css_supports "ruby-align" "space-around";
    |]
  >= 3

let is_webkit_606_or_newer () : bool =
  let w = window () in
  count_flags
    [|
      has w "DOMRectList";
      has w "RTCPeerConnectionIceEvent";
      has w "SVGGeometryElement";
      has w "ontransitioncancel";
    |]
  >= 3

let is_webkit_616_or_newer () : bool =
  let w = window () in
  let n = navigator () in
  let button = get w "HTMLButtonElement" in
  count_flags
    [|
      not (has n "getStorageUpdates");
      truthy button && has (get button "prototype") "popover";
      has w "CSSCounterStyleRule";
      css_supports1 "font-size-adjust: ex-height 0.5";
      css_supports1 "text-transform: full-width";
    |]
  >= 4

let is_ipad () : bool =
  if to_string (get (navigator ()) "platform") = "iPad" then true
  else
    let s = screen () in
    let ratio = to_float (get s "width") /. to_float (get s "height") in
    let proto = get (get (global_this ()) "Element") "prototype" in
    count_flags
      [|
        has (window ()) "MediaSource";
        truthy (get proto "webkitRequestFullscreen");
        ratio > 0.65 && ratio < 1.53;
      |]
    >= 2

let get_fullscreen_element () : js =
  let d = document () in
  let v = get d "fullscreenElement" in
  if truthy v then v
  else
    let v = get d "msFullscreenElement" in
    if truthy v then v
    else
      let v = get d "mozFullScreenElement" in
      if truthy v then v
      else
        let v = get d "webkitFullscreenElement" in
        if truthy v then v else null

let exit_fullscreen () : js =
  let d = document () in
  let fn =
    let a = get d "exitFullscreen" in
    if truthy a then a
    else
      let b = get d "msExitFullscreen" in
      if truthy b then b
      else
        let c = get d "mozCancelFullScreen" in
        if truthy c then c else get d "webkitExitFullscreen"
  in
  call0 fn d

let is_android () : bool =
  let chromium = is_chromium () in
  let gecko = is_gecko () in
  let w = window () in
  let n = navigator () in
  if chromium then
    let connection = get n "connection" in
    let audio_ctor = get w "Audio" in
    let audio = construct0 audio_ctor in
    count_flags
      [|
        not (has w "SharedWorker");
        truthy connection && has connection "ontypechange";
        not (has audio "sinkId");
      |]
    >= 2
  else if gecko then
    count_flags
      [|
        has w "onorientationchange";
        has w "orientation";
        re_test (new_re "android" "i") (string_value (get n "appVersion"));
      |]
    >= 2
  else false

let is_samsung_internet () : bool =
  let n = navigator () in
  let w = window () in
  let audio_proto = get (get w "Audio") "prototype" in
  let visual = get w "visualViewport" in
  let image_proto = get (get (global_this ()) "Image") "prototype" in
  count_flags
    [|
      has audio_proto "srLatency";
      has audio_proto "srChannelCount";
      has n "devicePosture";
      truthy visual && has visual "segments";
      has image_proto "getTextInformation";
    |]
  >= 3
