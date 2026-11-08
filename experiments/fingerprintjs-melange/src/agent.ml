open Host

let escape_key (key : string) : string =
  let re = new_re "([:|\\\\])" "g" in
  string_replace key re "\\$1"

let components_to_canonical_string (components : js) : string =
  let keys = array_sort (object_keys components) in
  let rec loop i result =
    if i >= array_length keys then result
    else
      let component_key = to_string (get_i keys i) in
      let component = get components component_key in
      let value =
        if has component "error" then "error"
        else
          let serialized = json_stringify (get component "value") in
          if is_undefined serialized then "undefined" else to_string serialized
      in
      let part = escape_key component_key ^ ":" ^ value in
      loop (i + 1) (if result = "" then part else result ^ "|" ^ part)
  in
  loop 0 ""

let components_to_debug_string (components : js) : js =
  let replacer =
    source1 "" (fun value ->
        (* JSON replacer is (key, value); we wrap as 1-arg so we need 2-arg. *)
        value)
  in
  let _ = replacer in
  let replacer2 : js =
    [%mel.raw
      {|
      function() {
        return function(_key, value) {
          if (value instanceof Error) {
            var result = {};
            result.name = value.name;
            result.message = value.message;
            result.stack = value.stack === undefined || value.stack === null
              ? undefined
              : String(value.stack).split("\n");
            var keys = Object.keys(value);
            for (var i = 0; i < keys.length; i++) {
              result[keys[i]] = value[keys[i]];
            }
            return result;
          }
          return value;
        };
      }
    |}]
  in
  json_stringify_with components (apply0 replacer2) (of_int 2)

let hash_components (components : js) : js =
  of_string (Hashing.x64hash128 (components_to_canonical_string components))

let comment_template =
  "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"

let get_confidence (components : js) : js =
  let score =
    if Browser.is_android () then 0.4
    else if Browser.is_webkit () then
      if
        Browser.is_desktop_webkit ()
        && not (Browser.is_webkit_616_or_newer () && Browser.is_safari_webkit ())
      then 0.5
      else 0.3
    else
      let platform = get components "platform" in
      let platform =
        if has platform "value" then string_value (get platform "value") else ""
      in
      if re_test (new_re "^Win" "") platform then 0.6
      else if re_test (new_re "^Mac" "") platform then 0.5
      else 0.7
  in
  let pro = Data.round (0.99 +. (0.01 *. score)) 0.0001 in
  let comment =
    string_replace comment_template (new_re "\\$" "g") (number_to_string pro)
  in
  let result = object_ () in
  set result "score" (of_float score);
  set result "comment" (of_string comment);
  result

let make_lazy_get_result (components : js) : js =
  let make : js -> js -> js -> string -> js =
    [%mel.raw
      {|
      function(components, hashFn, confidence, version) {
        var cache;
        var result = {};
        var getter = function() {
          if (cache === undefined) cache = hashFn(components);
          return cache;
        };
        var setter = function(visitorId) { cache = visitorId; };
        Object.defineProperty(getter, "name", { value: "get visitorId", configurable: true });
        Object.defineProperty(setter, "name", { value: "set visitorId", configurable: true });
        Object.defineProperty(result, "visitorId", {
          get: getter,
          set: setter,
          enumerable: true,
          configurable: true
        });
        result.confidence = confidence;
        result.components = components;
        result.version = version;
        return result;
      }
    |}]
  in
  make components (hide hash_components) (get_confidence components) version

let make_agent (get_components : js) (debug : js) : js =
  let creation_time = date_now () in
  let agent = object_ () in
  let get_fn =
    source1 "get" (fun options ->
        let start_time = date_now () in
        promise_then (apply0 get_components) (fun components ->
            let result = make_lazy_get_result components in
            let opt_debug =
              (not (is_nullish options)) && truthy (get options "debug")
            in
            if truthy debug || opt_debug then
              console_log
                (of_string
                   ("Copy the text below to get the debug data:\n\n```\nversion: "
                   ^ version
                   ^ "\nuserAgent: "
                   ^ string_value (get (navigator ()) "userAgent")
                   ^ "\ntimeBetweenLoadAndGet: "
                   ^ number_to_string (start_time -. creation_time)
                   ^ "\nvisitorId: "
                   ^ string_value (get result "visitorId")
                   ^ "\ncomponents: "
                   ^ to_string (components_to_debug_string components)
                   ^ "\n```"));
            result))
  in
  set agent "get" get_fn;
  agent

let maybe_monitor (options : js) : unit =
  let f : js -> unit =
    [%mel.raw
      {|
      function(options) {
        var monitoring = options.monitoring;
        if (!(monitoring === undefined ? true : monitoring)) {
          return;
        }
        if (window.__fpjs_d_m || Math.random() >= 0.001) {
          return;
        }
        try {
          var request = new XMLHttpRequest();
          request.open("get", "https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring", true);
          request.send();
        } catch (error) {
          console.error(error);
        }
      }
    |}]
  in
  f options

let load (options : js) : js =
  let options = if is_undefined options then object_ () else options in
  let delay = get options "delayFallback" in
  let debug = get options "debug" in
  maybe_monitor options;
  promise_then (Fp_async.prepare_for_sources delay) (fun _ ->
      let source_options = object_ () in
      set source_options "cache" (object_ ());
      set source_options "debug" debug;
      let get_components = Sources.load_builtin_sources source_options in
      make_agent get_components debug)
