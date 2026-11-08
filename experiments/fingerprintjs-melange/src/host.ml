(* JS-value host boundary.
   Every helper here is an external or [%mel.raw] so the emit stays on
   JavaScript values. OCaml records/variants are not used for anything that
   JSON.stringify or the public API can observe. *)

type js

external hide : 'a -> js = "%identity"
external show : js -> 'a = "%identity"
external of_string : string -> js = "%identity"
external to_string : js -> string = "%identity"
external of_int : int -> js = "%identity"
external to_int : js -> int = "%identity"
external of_float : float -> js = "%identity"
external to_float : js -> float = "%identity"
external of_bool : bool -> js = "%identity"
external to_bool : js -> bool = "%identity"

external undefined : js = "#undefined"
external null : js = "#null"
external js_typeof : 'a -> string = "#typeof"

let is_undefined (v : js) = js_typeof v = "undefined"
let is_null (v : js) = v == null
let is_nullish (v : js) = is_undefined v || is_null v
let is_function (v : js) = js_typeof v = "function"
let is_object (v : js) = js_typeof v = "object" && (not (is_null v))
let is_string (v : js) = js_typeof v = "string"
let is_number (v : js) = js_typeof v = "number"

external get : js -> string -> js = "" [@@mel.get_index]
external set : js -> string -> js -> unit = "" [@@mel.set_index]
external get_i : js -> int -> js = "" [@@mel.get_index]
external set_i : js -> int -> js -> unit = "" [@@mel.set_index]

let has : js -> string -> bool =
  [%mel.raw {| function(o, k) { return k in o; } |}]

let has_value : js -> js -> bool =
  [%mel.raw {| function(o, k) { return k in o; } |}]

let object_ () : js = [%mel.raw {| {} |}]

let array_ () : js = [%mel.raw {| [] |}]

external array_is_array : js -> bool = "isArray" [@@mel.scope "Array"]

external object_keys : js -> js = "keys" [@@mel.scope "Object"]
external object_define_property : js -> string -> js -> js = "defineProperty"
[@@mel.scope "Object"]
external object_get_prototype_of : js -> js = "getPrototypeOf"
[@@mel.scope "Object"]

external date_now : unit -> float = "now" [@@mel.scope "Date"]

external json_stringify : js -> js = "stringify" [@@mel.scope "JSON"]

let json_stringify_with : js -> js -> js -> js =
  [%mel.raw
    {| function(v, r, s) { return JSON.stringify(v, r, s); } |}]

external parse_int : js -> float = "parseInt"
external parse_float : js -> float = "parseFloat"
external is_nan : float -> bool = "isNaN"
external is_finite : float -> bool = "isFinite"

let number_value : js -> float =
  [%mel.raw {| function(v) { return +v; } |}]

let string_value : js -> string =
  [%mel.raw {| function(v) { return String(v); } |}]

let js_string : js -> js =
  [%mel.raw {| function(v) { return String(v); } |}]

external math_abs : float -> float = "abs" [@@mel.scope "Math"]
external math_min : float -> float -> float = "min" [@@mel.scope "Math"]
external math_max : float -> float -> float = "max" [@@mel.scope "Math"]
external math_round : float -> float = "round" [@@mel.scope "Math"]
external math_floor : float -> float = "floor" [@@mel.scope "Math"]
external math_pow : float -> float -> float = "pow" [@@mel.scope "Math"]
external math_random : unit -> float = "random" [@@mel.scope "Math"]
external math_pi : float = "PI" [@@mel.scope "Math"]
external math_log : float -> float = "log" [@@mel.scope "Math"]
external math_sqrt : float -> float = "sqrt" [@@mel.scope "Math"]
external math_exp : float -> float = "exp" [@@mel.scope "Math"]
external math_acos : float -> float = "acos" [@@mel.scope "Math"]
external math_acosh : float -> float = "acosh" [@@mel.scope "Math"]
external math_asin : float -> float = "asin" [@@mel.scope "Math"]
external math_asinh : float -> float = "asinh" [@@mel.scope "Math"]
external math_atan : float -> float = "atan" [@@mel.scope "Math"]
external math_atanh : float -> float = "atanh" [@@mel.scope "Math"]
external math_sin : float -> float = "sin" [@@mel.scope "Math"]
external math_sinh : float -> float = "sinh" [@@mel.scope "Math"]
external math_cos : float -> float = "cos" [@@mel.scope "Math"]
external math_cosh : float -> float = "cosh" [@@mel.scope "Math"]
external math_tan : float -> float = "tan" [@@mel.scope "Math"]
external math_tanh : float -> float = "tanh" [@@mel.scope "Math"]
external math_expm1 : float -> float = "expm1" [@@mel.scope "Math"]
external math_log1p : float -> float = "log1p" [@@mel.scope "Math"]

let math_fn : string -> js = [%mel.raw {| function(n) { return Math[n]; } |}]

external atob : string -> string = "atob"

let window () : js = [%mel.raw {| globalThis.window |}]
let document () : js = [%mel.raw {| globalThis.document |}]
let navigator () : js = [%mel.raw {| globalThis.navigator |}]
let screen () : js = [%mel.raw {| globalThis.screen |}]
let global_this () : js = [%mel.raw {| globalThis |}]

let match_media : string -> js =
  [%mel.raw {| function(q) { return matchMedia(q); } |}]

external set_timeout : js -> float -> float = "setTimeout"
external clear_timeout : float -> unit = "clearTimeout"

let new_error : string -> js =
  [%mel.raw {| function(m) { return new Error(m); } |}]

let new_type_error : string -> js =
  [%mel.raw {| function(m) { return new TypeError(m); } |}]

let instance_of : js -> js -> bool =
  [%mel.raw {| function(v, C) { return v instanceof C; } |}]

let is_error : js -> bool =
  [%mel.raw {| function(v) { return v instanceof Error; } |}]

let is_dom_exception : js -> bool =
  [%mel.raw {| function(v) { return typeof DOMException === "function" && v instanceof DOMException; } |}]

let construct0 : js -> js =
  [%mel.raw {| function(C) { return new C(); } |}]

let construct1 : js -> js -> js =
  [%mel.raw {| function(C, a) { return new C(a); } |}]

let construct2 : js -> js -> js -> js =
  [%mel.raw {| function(C, a, b) { return new C(a, b); } |}]

let construct3 : js -> js -> js -> js -> js =
  [%mel.raw {| function(C, a, b, c) { return new C(a, b, c); } |}]

let call0 : js -> js -> js =
  [%mel.raw {| function(f, t) { return f.call(t); } |}]

let call1 : js -> js -> js -> js =
  [%mel.raw {| function(f, t, a) { return f.call(t, a); } |}]

let call2 : js -> js -> js -> js -> js =
  [%mel.raw {| function(f, t, a, b) { return f.call(t, a, b); } |}]

let call3 : js -> js -> js -> js -> js -> js =
  [%mel.raw {| function(f, t, a, b, c) { return f.call(t, a, b, c); } |}]

let call4 : js -> js -> js -> js -> js -> js -> js =
  [%mel.raw {| function(f, t, a, b, c, d) { return f.call(t, a, b, c, d); } |}]

let call6 : js -> js -> js -> js -> js -> js -> js -> js -> js =
  [%mel.raw
    {| function(f, t, a, b, c, d, e, g) { return f.call(t, a, b, c, d, e, g); } |}]

let invoke0 (target : js) (method_ : string) : js =
  call0 (get target method_) target

let invoke1 (target : js) (method_ : string) (a : js) : js =
  call1 (get target method_) target a

let invoke2 (target : js) (method_ : string) (a : js) (b : js) : js =
  call2 (get target method_) target a b

let invoke3 (target : js) (method_ : string) (a : js) (b : js) (c : js) : js =
  call3 (get target method_) target a b c

let invoke4 (target : js) (method_ : string) (a : js) (b : js) (c : js) (d : js)
    : js =
  call4 (get target method_) target a b c d

let apply1 : js -> js -> js =
  [%mel.raw {| function(f, a) { return f(a); } |}]

let apply2 : js -> js -> js -> js =
  [%mel.raw {| function(f, a, b) { return f(a, b); } |}]

let apply0 : js -> js =
  [%mel.raw {| function(f) { return f(); } |}]

let apply_curried2 : js -> js -> js -> js =
  [%mel.raw
    {|
    function(f, a, b) {
      if (typeof f === "function" && f.length >= 2) return f(a, b);
      return f(a)(b);
    }
  |}]

let apply_curried3 : js -> js -> js -> js -> js =
  [%mel.raw
    {|
    function(f, a, b, c) {
      if (typeof f === "function" && f.length >= 3) return f(a, b, c);
      var r = f(a);
      if (typeof r === "function" && r.length >= 2) return r(b, c);
      r = r(b);
      return typeof r === "function" ? r(c) : r;
    }
  |}]

external push : js -> js -> int = "push" [@@mel.send]
external array_length : js -> int = "length" [@@mel.get]
external array_sort : js -> js = "sort" [@@mel.send]
external array_reverse : js -> js = "reverse" [@@mel.send]
external array_concat : js -> js -> js = "concat" [@@mel.send]
external array_slice : js -> int -> js = "slice" [@@mel.send]
external array_join : js -> string -> string = "join" [@@mel.send]
external array_filter : js -> js -> js = "filter" [@@mel.send]
external array_map : js -> js -> js = "map" [@@mel.send]
external array_some : js -> js -> bool = "some" [@@mel.send]

external string_length : string -> int = "length" [@@mel.get]
external char_code_at : string -> int -> int = "charCodeAt" [@@mel.send]
external string_slice : string -> int -> string = "slice" [@@mel.send]
external string_slice2 : string -> int -> int -> string = "slice"
[@@mel.send]
external string_index_of : string -> string -> int = "indexOf" [@@mel.send]
external string_split : string -> string -> js = "split" [@@mel.send]
external string_replace : string -> js -> string -> string = "replace"
[@@mel.send]
external from_char_code2 : int -> int -> string = "fromCharCode"
[@@mel.scope "String"]

external to_string_radix : int -> int -> string = "toString" [@@mel.send]
external number_to_string : float -> string = "toString" [@@mel.send]

let unsigned (n : int) : int = n lsr 0

let hex8 (n : int) : string =
  let s = "00000000" ^ to_string_radix (unsigned n) 16 in
  string_slice s (-8)

let new_uint8_array : int -> js =
  [%mel.raw {| function(n) { return new Uint8Array(n); } |}]

let new_float32_array : int -> js =
  [%mel.raw {| function(n) { return new Float32Array(n); } |}]

let new_uint8_from_buffer : js -> js =
  [%mel.raw {| function(b) { return new Uint8Array(b); } |}]

let text_encode : string -> js =
  [%mel.raw {| function(s) { return new TextEncoder().encode(s); } |}]

let typed_length (a : js) : int = to_int (get a "length")

let set_name (fn : js) (name : string) : js =
  let descriptor = object_ () in
  set descriptor "value" (of_string name);
  set descriptor "configurable" (of_bool true);
  ignore (object_define_property fn "name" descriptor);
  fn

let source0 (name : string) (impl : js -> js) : js =
  let wrap : string -> (js -> js) -> js =
    [%mel.raw
      {|
      function(name, impl) {
        var fn = function() { return impl(arguments[0]); };
        Object.defineProperty(fn, "name", { value: name, configurable: true });
        return fn;
      }
    |}]
  in
  wrap name impl

let source1 (name : string) (impl : js -> js) : js =
  let wrap : string -> (js -> js) -> js =
    [%mel.raw
      {|
      function(name, impl) {
        var fn = function(options) { return impl(options); };
        Object.defineProperty(fn, "name", { value: name, configurable: true });
        return fn;
      }
    |}]
  in
  wrap name impl

let fn0 (name : string) (impl : js -> js) : js = source0 name impl

let fn1 (name : string) (impl : js -> js) : js = source1 name impl

let call_ocaml : js -> js array -> js =
  [%mel.raw
    {|
    function(impl, args) {
      var current = impl;
      for (var i = 0; i < args.length; i++) {
        if (typeof current === "function" && current.length > 1 && i + current.length <= args.length) {
          current = current.apply(undefined, args.slice(i, i + current.length));
          i += current === impl ? 0 : current.length - 1;
        } else if (typeof current === "function") {
          current = current(args[i]);
        } else {
          return current;
        }
      }
      return current;
    }
  |}]

let fn2 (name : string) (impl : js -> js -> js) : js =
  let wrap : string -> (js -> js -> js) -> js =
    [%mel.raw
      {|
      function(name, impl) {
        var fn = function(a, b) {
          if (impl.length >= 2) return impl(a, b);
          var r = impl(a);
          return typeof r === "function" ? r(b) : r;
        };
        Object.defineProperty(fn, "name", { value: name, configurable: true });
        return fn;
      }
    |}]
  in
  wrap name impl

let fn_load (impl : js -> js) : js =
  let wrap : (js -> js) -> js =
    [%mel.raw
      {|
      function(impl) {
        var fn = function(options) { return impl(options === undefined ? undefined : options); };
        Object.defineProperty(fn, "name", { value: "load", configurable: true });
        // match `async function load(options = {})` — length 0
        Object.defineProperty(fn, "length", { value: 0, configurable: true });
        return fn;
      }
    |}]
  in
  wrap impl

let fn_prepare (impl : js -> js) : js =
  let wrap : (js -> js) -> js =
    [%mel.raw
      {|
      function(impl) {
        var fn = function(delayFallback) { return impl(delayFallback); };
        Object.defineProperty(fn, "name", { value: "prepareForSources", configurable: true });
        Object.defineProperty(fn, "length", { value: 0, configurable: true });
        return fn;
      }
    |}]
  in
  wrap impl

let fn_with_iframe (impl : js -> js -> js -> js) : js =
  let wrap : (js -> js -> js -> js) -> js =
    [%mel.raw
      {|
      function(impl) {
        var fn = function(action, initialHtml, pollInterval) {
          if (impl.length >= 3) return impl(action, initialHtml, pollInterval);
          var r = impl(action);
          if (typeof r === "function") r = r(initialHtml);
          if (typeof r === "function") r = r(pollInterval);
          return r;
        };
        Object.defineProperty(fn, "name", { value: "withIframe", configurable: true });
        Object.defineProperty(fn, "length", { value: 2, configurable: true });
        return fn;
      }
    |}]
  in
  wrap impl

let fn_load_sources (impl : js -> js -> js -> js -> js) : js =
  let wrap : (js -> js -> js -> js -> js) -> js =
    [%mel.raw
      {|
      function(impl) {
        var fn = function(sources, sourceOptions, excludeSources, loopReleaseInterval) {
          if (impl.length >= 4) return impl(sources, sourceOptions, excludeSources, loopReleaseInterval);
          var r = impl(sources);
          if (typeof r === "function") r = r(sourceOptions);
          if (typeof r === "function") r = r(excludeSources);
          if (typeof r === "function") r = r(loopReleaseInterval);
          return r;
        };
        Object.defineProperty(fn, "name", { value: "loadSources", configurable: true });
        return fn;
      }
    |}]
  in
  wrap impl

let fn_x64hash128 (impl : js -> js -> js) : js =
  let wrap : (js -> js -> js) -> js =
    [%mel.raw
      {|
      function(impl) {
        var fn = function(input, seed) {
          if (impl.length >= 2) return impl(input, seed);
          var r = impl(input);
          return typeof r === "function" ? r(seed) : r;
        };
        Object.defineProperty(fn, "name", { value: "x64hash128", configurable: true });
        return fn;
      }
    |}]
  in
  wrap impl

let try_host : (unit -> js) -> (js -> js) -> js =
  [%mel.raw
    {|
    function(action, recover) {
      try { return action(); }
      catch (e) { return recover(e); }
    }
  |}]

let try_bool : (unit -> bool) -> bool -> bool =
  [%mel.raw
    {|
    function(action, fallback) {
      try { return action(); }
      catch (e) { return fallback; }
    }
  |}]

let promise_resolve : js -> js =
  [%mel.raw {| function(v) { return Promise.resolve(v); } |}]

let promise_reject : js -> js =
  [%mel.raw {| function(v) { return Promise.reject(v); } |}]

let promise_then : js -> (js -> js) -> js =
  [%mel.raw
    {| function(p, f) { return p.then(function(v) { return f(v); }); } |}]

let promise_then2 : js -> (js -> js) -> (js -> js) -> js =
  [%mel.raw
    {|
    function(p, ok, err) {
      return p.then(function(v) { return ok(v); }, function(e) { return err(e); });
    }
  |}]

let promise_all : js -> js =
  [%mel.raw {| function(a) { return Promise.all(a); } |}]

let new_promise : (js -> js -> js) -> js =
  [%mel.raw
    {|
    function(ex) {
      return new Promise(function(resolve, reject) {
        if (typeof ex === "function" && ex.length >= 2) return ex(resolve, reject);
        return ex(resolve)(reject);
      });
    }
  |}]

let is_promise (v : js) : bool =
  (not (is_nullish v)) && is_function (get v "then")

let suppress (p : js) : js =
  let wrap : js -> js =
    [%mel.raw
      {|
      function(p) {
        p.then(undefined, function() { return undefined; });
        return p;
      }
    |}]
  in
  wrap p

let new_re : string -> string -> js =
  [%mel.raw {| function(p, f) { return new RegExp(p, f); } |}]

let re_test (re : js) (s : string) : bool =
  to_bool (invoke1 re "test" (of_string s))

let re_exec (re : js) (s : string) : js = invoke1 re "exec" (of_string s)

let truthy (v : js) : bool =
  let f : js -> bool = [%mel.raw {| function(v) { return !!v; } |}] in
  f v

let not_op (v : js) : bool =
  let f : js -> bool = [%mel.raw {| function(v) { return !v; } |}] in
  f v

let strict_eq (a : js) (b : js) : bool = a == b

let console_log (v : js) : unit =
  let f : js -> unit = [%mel.raw {| function(v) { console.log(v); } |}] in
  f v

let console_error (v : js) : unit =
  let f : js -> unit = [%mel.raw {| function(v) { console.error(v); } |}] in
  f v

let version = "5.2.0"

let release_event_loop () : js =
  new_promise (fun resolve _reject ->
      let channel = construct0 (get (global_this ()) "MessageChannel") in
      set (get channel "port1") "onmessage" (source0 "" (fun _ ->
          ignore (apply1 resolve undefined);
          undefined));
      ignore (invoke1 (get channel "port2") "postMessage" null);
      undefined)

let wait (duration_ms : float) : js =
  new_promise (fun resolve _reject ->
      let cb = source0 "" (fun _ ->
          ignore (apply1 resolve undefined);
          undefined)
      in
      ignore (set_timeout cb duration_ms);
      undefined)
