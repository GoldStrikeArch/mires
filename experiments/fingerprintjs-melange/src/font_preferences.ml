open Host

let default_text = "mmMwWLliI0fiflO&1"

let apply_style (element : js) (style : js) : unit =
  if is_undefined style then ()
  else
    let names = object_keys style in
    let rec loop i =
      if i >= array_length names then ()
      else
        let name = to_string (get_i names i) in
        let value = get style name in
        if not (is_undefined value) then set (get element "style") name value;
        loop (i + 1)
    in
    loop 0

let style_font (font : string) : js =
  let s = object_ () in
  set s "font" (of_string font);
  s

let style_family (family : string) : js =
  let s = object_ () in
  set s "fontFamily" (of_string family);
  s

let style_size (size : string) : js =
  let s = object_ () in
  set s "fontSize" (of_string size);
  s

let presets () : js =
  let p = object_ () in
  set p "default" (let a = array_ () in a);
  set p "apple"
    (let a = array_ () in
     ignore (push a (style_font "-apple-system-body"));
     a);
  set p "serif"
    (let a = array_ () in
     ignore (push a (style_family "serif"));
     a);
  set p "sans"
    (let a = array_ () in
     ignore (push a (style_family "sans-serif"));
     a);
  set p "mono"
    (let a = array_ () in
     ignore (push a (style_family "monospace"));
     a);
  set p "min"
    (let a = array_ () in
     ignore (push a (style_size "1px"));
     a);
  set p "system"
    (let a = array_ () in
     ignore (push a (style_family "system-ui"));
     a);
  p

let normalize_font_measurement (value : float) : float =
  let decimals = if Browser.is_android () then 0. else 3. in
  let pow = math_pow 10. decimals in
  math_floor (value *. pow) /. pow

let with_natural_fonts (action : js -> js -> js -> js) : js =
  let container_width = 4000. in
  Dom.with_iframe
    (fn2 "naturalFonts" (fun _iframe iframe_window ->
         let iframe_document = get iframe_window "document" in
         let iframe_body = get iframe_document "body" in
         let body_style = get iframe_body "style" in
         set body_style "width" (of_string (number_to_string container_width ^ "px"));
         set body_style "webkitTextSizeAdjust" (of_string "none");
         set body_style "textSizeAdjust" (of_string "none");
         if Browser.is_chromium () then
           set body_style "zoom"
             (of_string
                (number_to_string
                   (1. /. to_float (get iframe_window "devicePixelRatio"))))
         else if Browser.is_webkit () then
           set body_style "zoom" (of_string "reset");
         let lines = invoke1 iframe_document "createElement" (of_string "div") in
         let count = int_of_float (container_width /. 20.) in
         (* `(containerWidthPx / 20) << 0` *)
         let words = array_ () in
         let rec fill i =
           if i >= count then ()
           else (
             ignore (push words (of_string "word"));
             fill (i + 1))
         in
         fill 0;
         set lines "textContent" (of_string (array_join words " "));
         ignore (invoke1 iframe_body "appendChild" lines);
         apply_curried3 (hide action) iframe_document iframe_body iframe_window))
    (of_string
       "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">")
    undefined

let get_font_preferences (_ : js) : js =
  with_natural_fonts (fun doc container iframe_window ->
      let preset_map = presets () in
      let elements = object_ () in
      let sizes = object_ () in
      let keys = object_keys preset_map in
      let rec create i =
        if i >= array_length keys then ()
        else
          let key = to_string (get_i keys i) in
          let preset = get preset_map key in
          let style =
            if array_length preset > 0 then get_i preset 0 else object_ ()
          in
          let text =
            if array_length preset > 1 then to_string (get_i preset 1)
            else default_text
          in
          let element = invoke1 doc "createElement" (of_string "span") in
          set element "textContent" (of_string text);
          set (get element "style") "whiteSpace" (of_string "nowrap");
          apply_style element style;
          set elements key element;
          ignore (invoke1 container "append" (invoke1 doc "createElement" (of_string "br")));
          ignore (invoke1 container "append" element);
          create (i + 1)
      in
      create 0;
      let should_normalize =
        Browser.is_chromium () && Browser.is_chromium_128_or_newer ()
      in
      let rec measure i =
        if i >= array_length keys then ()
        else
          let key = to_string (get_i keys i) in
          let raw_width =
            to_float (get (invoke0 (get elements key) "getBoundingClientRect") "width")
          in
          let size =
            if should_normalize then
              normalize_font_measurement
                (raw_width *. to_float (get iframe_window "devicePixelRatio"))
            else raw_width
          in
          set sizes key (of_float size);
          measure (i + 1)
      in
      measure 0;
      sizes)
