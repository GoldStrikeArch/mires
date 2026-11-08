open Host

let known_for_suspending = -1
let not_supported = -2
let timeout = -3
let known_for_antifingerprinting = -4

let does_browser_suspend_audio_context () =
  Browser.is_webkit () && (not (Browser.is_desktop_webkit ()))
  && not (Browser.is_webkit_606_or_newer ())

let does_browser_perform_antifingerprinting () =
  (Browser.is_webkit () && Browser.is_webkit_616_or_newer ()
  && Browser.is_safari_webkit ())
  || Browser.is_chromium () && Browser.is_samsung_internet ()
     && Browser.is_chromium_122_or_newer ()

let make_inner_error (name : string) : js =
  let error = new_error name in
  set error "name" (of_string name);
  error

let get_hash (signal : js) : float =
  let rec loop i hash =
    if i >= typed_length signal then hash
    else loop (i + 1) (hash +. math_abs (to_float (get_i signal i)))
  in
  loop 0 0.

let start_rendering_audio (context : js) : js * js =
  let finalize_ref = ref (source0 "" (fun _ -> undefined)) in
  let result =
    new_promise (fun resolve reject ->
        let is_finalized = ref false in
        let render_try_count = ref 0 in
        let started_running_at = ref 0. in
        set context "oncomplete" (source1 "" (fun event ->
            ignore (apply1 resolve (get event "renderedBuffer"));
            undefined));
        let start_running_timeout () =
          let delay =
            math_min 500.
              (!started_running_at +. 5000. -. date_now ())
          in
          ignore
            (set_timeout
               (source0 "" (fun _ ->
                    ignore (apply1 reject (make_inner_error "timeout"));
                    undefined))
               delay)
        in
        let rec try_render () =
          try_host
            (fun () ->
              let rendering = invoke0 context "startRendering" in
              if is_promise rendering then ignore (suppress rendering);
              let state = to_string (get context "state") in
              (match state with
              | "running" ->
                  started_running_at := date_now ();
                  if !is_finalized then start_running_timeout ()
              | "suspended" ->
                  if not (truthy (get (document ()) "hidden")) then
                    render_try_count := !render_try_count + 1;
                  if !is_finalized && !render_try_count >= 3 then
                    ignore (apply1 reject (make_inner_error "suspended"))
                  else
                    ignore
                      (set_timeout
                         (source0 "" (fun _ ->
                              try_render ();
                              undefined))
                         500.)
              | _ -> ());
              undefined)
            (fun error ->
              ignore (apply1 reject error);
              undefined)
          |> ignore
        in
        try_render ();
        finalize_ref :=
          source0 "" (fun _ ->
              if not !is_finalized then (
                is_finalized := true;
                if !started_running_at > 0. then start_running_timeout ());
              undefined);
        undefined)
  in
  (result, !finalize_ref)

let get_unstable_audio_fingerprint () : js =
  let w = window () in
  let ctor = get w "OfflineAudioContext" in
  let ctor = if truthy ctor then ctor else get w "webkitOfflineAudioContext" in
  if not (truthy ctor) then of_int not_supported
  else if does_browser_suspend_audio_context () then of_int known_for_suspending
  else
    let context = construct3 ctor (of_int 1) (of_int 5000) (of_float 44100.) in
    let oscillator = invoke0 context "createOscillator" in
    set oscillator "type" (of_string "triangle");
    set (get oscillator "frequency") "value" (of_float 10000.);
    let compressor = invoke0 context "createDynamicsCompressor" in
    set (get compressor "threshold") "value" (of_float (-50.));
    set (get compressor "knee") "value" (of_float 40.);
    set (get compressor "ratio") "value" (of_float 12.);
    set (get compressor "attack") "value" (of_float 0.);
    set (get compressor "release") "value" (of_float 0.25);
    ignore (invoke1 oscillator "connect" compressor);
    ignore (invoke1 compressor "connect" (get context "destination"));
    ignore (invoke1 oscillator "start" (of_float 0.));
    let render_promise, finish = start_rendering_audio context in
    let fingerprint =
      suppress
        (promise_then2 render_promise
           (fun buffer ->
             let data = invoke1 buffer "getChannelData" (of_int 0) in
             let sub = invoke1 data "subarray" (of_int 4500) in
             of_float (get_hash sub))
           (fun error ->
             let name = to_string (get error "name") in
             if name = "timeout" || name = "suspended" then of_int timeout
             else
               let throw : js -> js =
                 [%mel.raw {| function(e) { throw e; } |}]
               in
               throw error))
    in
    source0 "" (fun _ ->
        ignore (apply0 finish);
        fingerprint)

let get_audio_fingerprint (_ : js) : js =
  if does_browser_perform_antifingerprinting () then
    of_int known_for_antifingerprinting
  else get_unstable_audio_fingerprint ()
