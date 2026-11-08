open Host

let componentsToDebugString =
  fn1 "componentsToDebugString" Agent.components_to_debug_string

let getFullscreenElement =
  fn0 "getFullscreenElement" (fun _ -> Browser.get_fullscreen_element ())

let getUnstableAudioFingerprint =
  fn0 "getUnstableAudioFingerprint" (fun _ -> Audio.get_unstable_audio_fingerprint ())

let getUnstableCanvasFingerprint =
  fn1 "getUnstableCanvasFingerprint" Canvas.get_unstable_canvas_fingerprint

let getUnstableHardwareConcurrency =
  fn0 "getUnstableHardwareConcurrency" (fun _ ->
      Sources_simple.get_unstable_hardware_concurrency ())

let getUnstableScreenFrame =
  fn0 "getUnstableScreenFrame" (fun _ -> Screen_frame.get_unstable_screen_frame ())

let getUnstableScreenResolution =
  fn0 "getUnstableScreenResolution" (fun _ ->
      Sources_simple.get_unstable_screen_resolution ())

let getWebGLContext = fn1 "getWebGLContext" Webgl.get_webgl_context

let hashComponents = fn1 "hashComponents" Agent.hash_components

let isAndroid = fn0 "isAndroid" (fun _ -> of_bool (Browser.is_android ()))
let isChromium = fn0 "isChromium" (fun _ -> of_bool (Browser.is_chromium ()))

let isDesktopWebKit =
  fn0 "isDesktopWebKit" (fun _ -> of_bool (Browser.is_desktop_webkit ()))

let isEdgeHTML = fn0 "isEdgeHTML" (fun _ -> of_bool (Browser.is_edge_html ()))
let isGecko = fn0 "isGecko" (fun _ -> of_bool (Browser.is_gecko ()))

let isSamsungInternet =
  fn0 "isSamsungInternet" (fun _ -> of_bool (Browser.is_samsung_internet ()))

let isTrident = fn0 "isTrident" (fun _ -> of_bool (Browser.is_trident ()))
let isWebKit = fn0 "isWebKit" (fun _ -> of_bool (Browser.is_webkit ()))

let load = fn_load Agent.load

let loadSources =
  fn_load_sources (fun sources options exclude interval ->
      Entropy.load_sources sources options exclude interval)

let murmurX64Hash128 = fn_x64hash128 Hashing.murmur

let prepareForSources = fn_prepare Fp_async.prepare_for_sources

let sources = Sources.make_sources ()

let transformSource =
  fn2 "transformSource" (fun source transform ->
      Entropy.transform_source source transform)

let withIframe = fn_with_iframe Dom.with_iframe

let default =
  let o = object_ () in
  set o "load" load;
  set o "hashComponents" hashComponents;
  set o "componentsToDebugString" componentsToDebugString;
  o
