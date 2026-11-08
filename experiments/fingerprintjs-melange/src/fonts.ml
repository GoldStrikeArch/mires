open Host

let test_string = "mmMwWLliI0O&1"
let text_size = "48px"
let base_fonts = [| "monospace"; "sans-serif"; "serif" |]

let font_list =
  [|
    "sans-serif-thin";
    "ARNO PRO";
    "Agency FB";
    "Arabic Typesetting";
    "Arial Unicode MS";
    "AvantGarde Bk BT";
    "BankGothic Md BT";
    "Batang";
    "Bitstream Vera Sans Mono";
    "Calibri";
    "Century";
    "Century Gothic";
    "Clarendon";
    "EUROSTILE";
    "Franklin Gothic";
    "Futura Bk BT";
    "Futura Md BT";
    "GOTHAM";
    "Gill Sans";
    "HELV";
    "Haettenschweiler";
    "Helvetica Neue";
    "Humanst521 BT";
    "Leelawadee";
    "Letter Gothic";
    "Levenim MT";
    "Lucida Bright";
    "Lucida Sans";
    "Menlo";
    "MS Mincho";
    "MS Outlook";
    "MS Reference Specialty";
    "MS UI Gothic";
    "MT Extra";
    "MYRIAD PRO";
    "Marlett";
    "Meiryo UI";
    "Microsoft Uighur";
    "Minion Pro";
    "Monotype Corsiva";
    "PMingLiU";
    "Pristina";
    "SCRIPTINA";
    "Segoe UI Light";
    "Serifa";
    "SimHei";
    "Small Fonts";
    "Staccato222 BT";
    "TRAJAN PRO";
    "Univers CE 55 Medium";
    "Vrinda";
    "ZWAdobeF";
  |]

let get_fonts (_ : js) : js =
  Dom.with_iframe
    (fn2 "fontsAction" (fun _iframe iframe_window ->
         let doc = get iframe_window "document" in
         let holder = get doc "body" in
         set (get holder "style") "fontSize" (of_string text_size);
         let spans_container = invoke1 doc "createElement" (of_string "div") in
         ignore
           (invoke3 (get spans_container "style") "setProperty"
              (of_string "visibility") (of_string "hidden") (of_string "important"));
         let default_width = object_ () in
         let default_height = object_ () in
         let create_span font_family =
           let span = invoke1 doc "createElement" (of_string "span") in
           let style = get span "style" in
           set style "position" (of_string "absolute");
           set style "top" (of_string "0");
           set style "left" (of_string "0");
           set style "fontFamily" (of_string font_family);
           set span "textContent" (of_string test_string);
           ignore (invoke1 spans_container "appendChild" span);
           span
         in
         let create_span_with font_to_detect base_font =
           create_span ("'" ^ font_to_detect ^ "'," ^ base_font)
         in
         let base_spans = array_ () in
         let rec init_base i =
           if i >= Js.Array.length base_fonts then ()
           else (
             ignore
               (push base_spans
                  (create_span (Js.Array.unsafe_get base_fonts i)));
             init_base (i + 1))
         in
         init_base 0;
         let fonts_spans = object_ () in
         let rec init_fonts i =
           if i >= Js.Array.length font_list then ()
           else
             let font = Js.Array.unsafe_get font_list i in
             let spans = array_ () in
             let rec bases j =
               if j >= Js.Array.length base_fonts then ()
               else (
                 ignore
                   (push spans
                      (create_span_with font (Js.Array.unsafe_get base_fonts j)));
                 bases (j + 1))
             in
             bases 0;
             set fonts_spans font spans;
             init_fonts (i + 1)
         in
         init_fonts 0;
         ignore (invoke1 holder "appendChild" spans_container);
         let rec measure i =
           if i >= Js.Array.length base_fonts then ()
           else
             let base = Js.Array.unsafe_get base_fonts i in
             let span = get_i base_spans i in
             set default_width base (get span "offsetWidth");
             set default_height base (get span "offsetHeight");
             measure (i + 1)
         in
         measure 0;
         let available = array_ () in
         let rec check i =
           if i >= Js.Array.length font_list then ()
           else
             let font = Js.Array.unsafe_get font_list i in
             let font_spans = get fonts_spans font in
             let rec any j =
               if j >= Js.Array.length base_fonts then false
               else
                 let base = Js.Array.unsafe_get base_fonts j in
                 let span = get_i font_spans j in
                 if
                   not
                     (strict_eq (get span "offsetWidth") (get default_width base)
                     && strict_eq (get span "offsetHeight")
                          (get default_height base))
                 then true
                 else any (j + 1)
             in
             if any 0 then ignore (push available (of_string font));
             check (i + 1)
         in
         check 0;
         available))
    undefined undefined
