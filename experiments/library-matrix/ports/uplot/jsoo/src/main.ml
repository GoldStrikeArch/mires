(* The boundary (js_of_ocaml): the input and the result are JS values behind [Js.Unsafe.any], read
   and built with Js.Unsafe, and the canvas calls are Js.Unsafe method calls. The chart is
   [Chart.Make (Host)], shared with the Melange port. *)

open Js_of_ocaml

module Host = struct
  type v = Js.Unsafe.any

  let inject = Js.Unsafe.inject
  let field a k = Js.Unsafe.get a k
  let at a i = Js.Unsafe.get a i
  let len a = Js.Unsafe.get a "length"
  let is_null v = Js.Unsafe.equals v Js.null
  let bool a = Js.to_bool (Js.Unsafe.coerce a)
  let num x = Js.float_of_number (Js.Unsafe.coerce x)
  let number x = inject (Js.number_of_float x)
  let null = inject Js.null
  let new_ name = Js.Unsafe.new_obj (field Js.Unsafe.global name) [||]
  let array () = new_ "Array"
  let push a x = ignore (Js.Unsafe.meth_call a "push" [| x |])
  let round x = num (Js.Unsafe.meth_call (field Js.Unsafe.global "Math") "round" [| number x |])
  let log10 x = num (Js.Unsafe.meth_call (field Js.Unsafe.global "Math") "log10" [| number x |])
  let is_integer x = Float.is_finite x && Float.is_integer x
  let to_string x = Js.to_string (Js.number_of_float x)##toString
  let parse_float s = num (Js.Unsafe.fun_call (field Js.Unsafe.global "Number") [| inject (Js.string s) |])
  let pixel_ratio () = num (field Js.Unsafe.global "devicePixelRatio")
  let text s = inject (Js.string s)

  let canvas () =
    let doc = field Js.Unsafe.global "document" in
    let c = Js.Unsafe.meth_call doc "createElement" [| text "canvas" |] in
    ignore (Js.Unsafe.meth_call (field doc "body") "appendChild" [| c |]);
    c

  let remove v = ignore (Js.Unsafe.meth_call v "remove" [||])
  let context c = Js.Unsafe.meth_call c "getContext" [| text "2d" |]
  let set_number o key x = Js.Unsafe.set o key (number x)
  let set_string o key x = Js.Unsafe.set o key (text x)
  let path () = new_ "Path2D"
  let line p x y = ignore (Js.Unsafe.meth_call p "lineTo" [| number x; number y |])
  let rect p x y w h = ignore (Js.Unsafe.meth_call p "rect" [| number x; number y; number w; number h |])
  let save ctx = ignore (Js.Unsafe.meth_call ctx "save" [||])
  let restore ctx = ignore (Js.Unsafe.meth_call ctx "restore" [||])
  let translate ctx x y = ignore (Js.Unsafe.meth_call ctx "translate" [| number x; number y |])
  let clip ctx p = ignore (Js.Unsafe.meth_call ctx "clip" [| p |])
  let stroke ctx p = ignore (Js.Unsafe.meth_call ctx "stroke" [| p |])
end

module Port = Chart.Make (Host)

let () = Js.export "run" (Js.wrap_callback Port.run)
