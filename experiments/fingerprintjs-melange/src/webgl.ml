open Host

let status_no_gl_context = -1
let status_get_parameter_not_a_function = -2

let valid_context_parameters =
  [|
    10752; 2849; 2884; 2885; 2886; 2928; 2929; 2930; 2931; 2932; 2960; 2961;
    2962; 2963; 2964; 2965; 2966; 2967; 2968; 2978; 3024; 3042; 3088; 3089;
    3106; 3107; 32773; 32777; 32777; 32823; 32824; 32936; 32937; 32938; 32939;
    32968; 32969; 32970; 32971; 3317; 33170; 3333; 3379; 3386; 33901; 33902;
    34016; 34024; 34076; 3408; 3410; 3411; 3412; 3413; 3414; 3415; 34467; 34816;
    34817; 34818; 34819; 34877; 34921; 34930; 35660; 35661; 35724; 35738; 35739;
    36003; 36004; 36005; 36347; 36348; 36349; 37440; 37441; 37443; 7936; 7937;
    7938;
  |]

let valid_extension_params =
  [| 34047; 35723; 36063; 34852; 34853; 34854; 34229; 36392; 36795; 38449 |]

let shader_types = [| "FRAGMENT_SHADER"; "VERTEX_SHADER" |]

let precision_types =
  [|
    "LOW_FLOAT";
    "MEDIUM_FLOAT";
    "HIGH_FLOAT";
    "LOW_INT";
    "MEDIUM_INT";
    "HIGH_INT";
  |]

let renderer_info_extension_name = "WEBGL_debug_renderer_info"
let polygon_mode_extension_name = "WEBGL_polygon_mode"

let int_in (arr : int array) (code : int) : bool =
  let n = Js.Array.length arr in
  let rec loop i =
    if i >= n then false
    else if Js.Array.unsafe_get arr i = code then true
    else loop (i + 1)
  in
  loop 0

let should_avoid_debug_renderer_info () = Browser.is_gecko ()

let should_avoid_polygon_mode_extensions () =
  Browser.is_chromium () || Browser.is_webkit ()

let is_valid_parameter_getter (gl : js) = is_function (get gl "getParameter")

let param_to_string (value : js) : string =
  if is_nullish value then "" else string_value value

let get_webgl_context (cache : js) : js =
  let cache = if is_undefined cache || is_null cache then object_ () else cache in
  let existing = get cache "webgl" in
  if truthy existing then get existing "context"
  else
    let canvas = invoke1 (document ()) "createElement" (of_string "canvas") in
    let context = ref undefined in
    ignore
      (invoke2 canvas "addEventListener" (of_string "webglCreateContextError")
         (source0 "" (fun _ ->
              context := undefined;
              undefined)));
    let rec try_type i =
      if i >= 2 then ()
      else
        let type_ = if i = 0 then "webgl" else "experimental-webgl" in
        let _ =
          try_host
            (fun () ->
              context := invoke1 canvas "getContext" (of_string type_);
              undefined)
            (fun _ -> undefined)
        in
        if truthy !context then () else try_type (i + 1)
    in
    try_type 0;
    let entry = object_ () in
    set entry "context" !context;
    set cache "webgl" entry;
    !context

let is_constant_like (key : js) : bool =
  is_string key && not (re_test (new_re "[^A-Z0-9_x]" "") (to_string key))

let get_constants_from_prototype (obj : js) : js =
  let proto = object_get_prototype_of obj in
  let keys = object_keys proto in
  let out = array_ () in
  let rec loop i =
    if i >= array_length keys then ()
    else
      let key = get_i keys i in
      if is_constant_like key then ignore (push out key);
      loop (i + 1)
  in
  loop 0;
  out

let get_shader_precision (gl : js) (shader_type : string)
    (precision_type : string) : js =
  let fmt =
    invoke2 gl "getShaderPrecisionFormat" (get gl shader_type)
      (get gl precision_type)
  in
  if not (truthy fmt) then array_ ()
  else
    let out = array_ () in
    ignore (push out (get fmt "rangeMin"));
    ignore (push out (get fmt "rangeMax"));
    ignore (push out (get fmt "precision"));
    out

let get_webgl_basics (options : js) : js =
  let cache = get options "cache" in
  let cache = if is_undefined cache then object_ () else cache in
  let gl = get_webgl_context cache in
  if not (truthy gl) then of_int status_no_gl_context
  else if not (is_valid_parameter_getter gl) then
    of_int status_get_parameter_not_a_function
  else
    let debug =
      if should_avoid_debug_renderer_info () then null
      else invoke1 gl "getExtension" (of_string renderer_info_extension_name)
    in
    let result = object_ () in
    set result "version"
      (of_string (param_to_string (invoke1 gl "getParameter" (get gl "VERSION"))));
    set result "vendor"
      (of_string (param_to_string (invoke1 gl "getParameter" (get gl "VENDOR"))));
    set result "vendorUnmasked"
      (of_string
         (if truthy debug then
            param_to_string
              (invoke1 gl "getParameter" (get debug "UNMASKED_VENDOR_WEBGL"))
          else ""));
    set result "renderer"
      (of_string
         (param_to_string (invoke1 gl "getParameter" (get gl "RENDERER"))));
    set result "rendererUnmasked"
      (of_string
         (if truthy debug then
            param_to_string
              (invoke1 gl "getParameter" (get debug "UNMASKED_RENDERER_WEBGL"))
          else ""));
    set result "shadingLanguageVersion"
      (of_string
         (param_to_string
            (invoke1 gl "getParameter" (get gl "SHADING_LANGUAGE_VERSION"))));
    result

let get_webgl_extensions (options : js) : js =
  let cache = get options "cache" in
  let cache = if is_undefined cache then object_ () else cache in
  let gl = get_webgl_context cache in
  if not (truthy gl) then of_int status_no_gl_context
  else if not (is_valid_parameter_getter gl) then
    of_int status_get_parameter_not_a_function
  else
    let extensions = invoke0 gl "getSupportedExtensions" in
    let context_attributes = invoke0 gl "getContextAttributes" in
    let unsupported = array_ () in
    let attributes = array_ () in
    let parameters = array_ () in
    let extension_parameters = array_ () in
    let shader_precisions = array_ () in
    (if truthy context_attributes then
       let names = object_keys context_attributes in
       let rec loop i =
         if i >= array_length names then ()
         else
           let name = to_string (get_i names i) in
           ignore
             (push attributes
                (of_string (name ^ "=" ^ string_value (get context_attributes name))));
           loop (i + 1)
       in
       loop 0);
    let constants = get_constants_from_prototype gl in
    let rec loop_const i =
      if i >= array_length constants then ()
      else
        let constant = to_string (get_i constants i) in
        let code = get gl constant in
        let extra =
          if is_number code && int_in valid_context_parameters (to_int code) then
            "=" ^ string_value (invoke1 gl "getParameter" code)
          else ""
        in
        ignore
          (push parameters
             (of_string (constant ^ "=" ^ string_value code ^ extra)));
        loop_const (i + 1)
    in
    loop_const 0;
    (if truthy extensions then
       let rec loop_ext i =
         if i >= array_length extensions then ()
         else
           let name = to_string (get_i extensions i) in
           if
             (name = renderer_info_extension_name
             && should_avoid_debug_renderer_info ())
             || name = polygon_mode_extension_name
                && should_avoid_polygon_mode_extensions ()
           then loop_ext (i + 1)
           else
             let extension = invoke1 gl "getExtension" (of_string name) in
             if not (truthy extension) then (
               ignore (push unsupported (of_string name));
               loop_ext (i + 1))
             else
               let ext_consts = get_constants_from_prototype extension in
               let rec loop_c j =
                 if j >= array_length ext_consts then ()
                 else
                   let constant = to_string (get_i ext_consts j) in
                   let code = get extension constant in
                   let extra =
                     if
                       is_number code
                       && int_in valid_extension_params (to_int code)
                     then "=" ^ string_value (invoke1 gl "getParameter" code)
                     else ""
                   in
                   ignore
                     (push extension_parameters
                        (of_string (constant ^ "=" ^ string_value code ^ extra)));
                   loop_c (j + 1)
               in
               loop_c 0;
               loop_ext (i + 1)
       in
       loop_ext 0);
    let rec loop_shader i =
      if i >= Js.Array.length shader_types then ()
      else
        let shader_type = Js.Array.unsafe_get shader_types i in
        let rec loop_prec j =
          if j >= Js.Array.length precision_types then ()
          else
            let precision_type = Js.Array.unsafe_get precision_types j in
            let precision = get_shader_precision gl shader_type precision_type in
            ignore
              (push shader_precisions
                 (of_string
                    (shader_type ^ "." ^ precision_type ^ "="
                   ^ array_join precision ",")));
            loop_prec (j + 1)
        in
        loop_prec 0;
        loop_shader (i + 1)
    in
    loop_shader 0;
    ignore (array_sort extension_parameters);
    ignore (array_sort parameters);
    let result = object_ () in
    set result "contextAttributes" attributes;
    set result "parameters" parameters;
    set result "shaderPrecisions" shader_precisions;
    set result "extensions" extensions;
    set result "extensionParameters" extension_parameters;
    set result "unsupportedExtensions" unsupported;
    result
