(* JavaScript export root. No browser globals are read at import time. *)

open Js_host
open Browser

let sources = Sources.make_sources ()

let exports =
  let named =
    [
      ( "componentsToDebugString",
        js_fun ~name:"componentsToDebugString" ~arity:1 Agent.debug_export );
      "getFullscreenElement", js_fun0 ~name:"getFullscreenElement" get_fullscreen_element;
      ( "getUnstableAudioFingerprint",
        js_fun0 ~name:"getUnstableAudioFingerprint" Sources.get_unstable_audio_fingerprint );
      ( "getUnstableCanvasFingerprint",
        js_fun ~name:"getUnstableCanvasFingerprint" ~arity:1 (fun args ->
            Sources.get_unstable_canvas_fingerprint (arg args 0)) );
      ( "getUnstableHardwareConcurrency",
        js_fun0 ~name:"getUnstableHardwareConcurrency" Sources.get_unstable_hardware_concurrency );
      ( "getUnstableScreenFrame",
        js_fun0 ~name:"getUnstableScreenFrame" Sources.get_unstable_screen_frame );
      ( "getUnstableScreenResolution",
        js_fun0 ~name:"getUnstableScreenResolution" Sources.get_unstable_screen_resolution );
      "getWebGLContext", js_fun ~name:"getWebGLContext" ~arity:1 Sources.get_webgl_context_export;
      "hashComponents", js_fun ~name:"hashComponents" ~arity:1 Agent.hash_export;
      "isAndroid", export_bool "isAndroid" is_android;
      "isChromium", export_bool "isChromium" is_chromium;
      "isDesktopWebKit", export_bool "isDesktopWebKit" is_desktop_webkit;
      "isEdgeHTML", export_bool "isEdgeHTML" is_edge_html;
      "isGecko", export_bool "isGecko" is_gecko;
      "isSamsungInternet", export_bool "isSamsungInternet" is_samsung_internet;
      "isTrident", export_bool "isTrident" is_trident;
      "isWebKit", export_bool "isWebKit" is_webkit;
      "load", js_fun ~name:"load" ~arity:0 Agent.load_export;
      "loadSources", js_fun ~name:"loadSources" ~arity:4 Entropy.load_sources_export;
      "murmurX64Hash128", js_fun ~name:"x64hash128" ~arity:2 Hashing.murmur_x64_hash128;
      "prepareForSources", js_fun ~name:"prepareForSources" ~arity:0 Agent.prepare_export;
      "sources", sources;
      "transformSource", js_fun ~name:"transformSource" ~arity:2 Entropy.transform_source_export;
      "withIframe", js_fun ~name:"withIframe" ~arity:2 Host_dom.with_iframe_export;
    ]
  in
  let obj = obj () in
  List.iter (fun (name, value) -> set obj name value) named;
  let load = get obj "load" in
  let hash = get obj "hashComponents" in
  let debug = get obj "componentsToDebugString" in
  set obj "default"
    (obj_fields [| "load", load; "hashComponents", hash; "componentsToDebugString", debug |]);
  obj

let () = set (global ()) "__fp_jsoo_exports" exports
