open Host

let image_unsupported = "unsupported"
let image_skipped = "skipped"
let image_unstable = "unstable"

let does_browser_perform_anti_fingerprinting () =
  (Browser.is_webkit () && Browser.is_webkit_616_or_newer ()
  && Browser.is_safari_webkit ())
  || Browser.is_gecko () && Browser.is_gecko_120_or_newer ()

let make_canvas_context () : js * js =
  let canvas = invoke1 (document ()) "createElement" (of_string "canvas") in
  set canvas "width" (of_int 1);
  set canvas "height" (of_int 1);
  (canvas, invoke1 canvas "getContext" (of_string "2d"))

let is_supported (canvas : js) (context : js) : bool =
  truthy context && truthy (get canvas "toDataURL")

let does_support_winding (context : js) : bool =
  ignore (invoke4 context "rect" (of_int 0) (of_int 0) (of_int 10) (of_int 10));
  ignore (invoke4 context "rect" (of_int 2) (of_int 2) (of_int 6) (of_int 6));
  not
    (to_bool
       (call3 (get context "isPointInPath") context (of_int 5) (of_int 5)
          (of_string "evenodd")))

let canvas_to_string (canvas : js) : string = to_string (invoke0 canvas "toDataURL")

let render_text_image (canvas : js) (context : js) : unit =
  set canvas "width" (of_int 240);
  set canvas "height" (of_int 60);
  set context "textBaseline" (of_string "alphabetic");
  set context "fillStyle" (of_string "#f60");
  ignore (invoke4 context "fillRect" (of_int 100) (of_int 1) (of_int 62) (of_int 20));
  set context "fillStyle" (of_string "#069");
  set context "font" (of_string "11pt \"Times New Roman\"");
  let printed = "Cwm fjordbank gly " ^ from_char_code2 55357 56835 in
  ignore (invoke3 context "fillText" (of_string printed) (of_int 2) (of_int 15));
  set context "fillStyle" (of_string "rgba(102, 204, 0, 0.2)");
  set context "font" (of_string "18pt Arial");
  ignore (invoke3 context "fillText" (of_string printed) (of_int 4) (of_int 45))

let render_geometry_image (canvas : js) (context : js) : unit =
  set canvas "width" (of_int 122);
  set canvas "height" (of_int 110);
  set context "globalCompositeOperation" (of_string "multiply");
  let draw color x y =
    set context "fillStyle" (of_string color);
    ignore (invoke0 context "beginPath");
    ignore
      (call6
         (get context "arc")
         context (of_float x) (of_float y) (of_int 40) (of_float 0.)
         (of_float (math_pi *. 2.)) (of_bool true));
    ignore (invoke0 context "closePath");
    ignore (invoke0 context "fill")
  in
  draw "#f2f" 40. 40.;
  draw "#2ff" 80. 40.;
  draw "#ff2" 60. 80.;
  set context "fillStyle" (of_string "#f9c");
  ignore
    (call6 (get context "arc") context (of_int 60) (of_int 60) (of_int 60)
       (of_float 0.) (of_float (math_pi *. 2.)) (of_bool true));
  ignore
    (call6 (get context "arc") context (of_int 60) (of_int 60) (of_int 20)
       (of_float 0.) (of_float (math_pi *. 2.)) (of_bool true));
  ignore (invoke1 context "fill" (of_string "evenodd"))

let render_images (canvas : js) (context : js) : string * string =
  render_text_image canvas context;
  let text1 = canvas_to_string canvas in
  let text2 = canvas_to_string canvas in
  if text1 <> text2 then (image_unstable, image_unstable)
  else (
    render_geometry_image canvas context;
    (canvas_to_string canvas, text1))

let get_unstable_canvas_fingerprint (skip_images : js) : js =
  let canvas, context = make_canvas_context () in
  let winding, geometry, text =
    if not (is_supported canvas context) then
      (false, image_unsupported, image_unsupported)
    else
      let winding = does_support_winding context in
      if truthy skip_images then (winding, image_skipped, image_skipped)
      else
        let geometry, text = render_images canvas context in
        (winding, geometry, text)
  in
  let result = object_ () in
  set result "winding" (of_bool winding);
  set result "geometry" (of_string geometry);
  set result "text" (of_string text);
  result

let get_canvas_fingerprint (_ : js) : js =
  get_unstable_canvas_fingerprint
    (of_bool (does_browser_perform_anti_fingerprinting ()))
