open Js_of_ocaml
module Host = struct
  type v = Js.Unsafe.any
  let inject=Js.Unsafe.inject
  let field a k=Js.Unsafe.get a k
  let at a i=Js.Unsafe.get a i
  let len a=Js.Unsafe.get a "length"
  let bool a=Js.to_bool (Js.Unsafe.coerce a)
  let number x=inject (Js.number_of_float x)
  let num x=Js.float_of_number (Js.Unsafe.coerce x)
  let null=inject Js.null
  let is_null v=Js.Unsafe.equals v Js.null
  let round x=Js.Unsafe.meth_call (field Js.Unsafe.global "Math") "round" [|number x|] |> num
  let new_ name=Js.Unsafe.new_obj (field Js.Unsafe.global name) [||]
  let array ()=new_ "Array"
  let push a x=ignore(Js.Unsafe.meth_call a "push" [|x|])
  let text s=inject(Js.string s)
  let canvas ()=
    let doc=field Js.Unsafe.global "document" in
    let c=Js.Unsafe.meth_call doc "createElement" [|text "canvas"|] in
    ignore(Js.Unsafe.meth_call (field doc "body") "appendChild" [|c|]);c
  let remove v=ignore(Js.Unsafe.meth_call v "remove" [||])
  let context c=Js.Unsafe.meth_call c "getContext" [|text "2d"|]
  let resize c ctx w h=Js.Unsafe.set c "width" (number w);Js.Unsafe.set c "height" (number h);Js.Unsafe.set ctx "strokeStyle" (text "#172839")
  let path ()=new_ "Path2D"
  let line p x y=ignore(Js.Unsafe.meth_call p "lineTo" [|number x;number y|])
  let rect p x y w h=ignore(Js.Unsafe.meth_call p "rect" [|number x;number y;number w;number h|])
  let pixel_ratio () = num (field Js.Unsafe.global "devicePixelRatio")
  let stroke_style ctx width =
    Js.Unsafe.set ctx "lineWidth" (number width);
    Js.Unsafe.set ctx "lineJoin" (text "round");
    Js.Unsafe.set ctx "lineCap" (text "butt")
  let translate ctx x y = ignore (Js.Unsafe.meth_call ctx "translate" [|number x; number y|])
  let save ctx=ignore(Js.Unsafe.meth_call ctx "save" [||])
  let restore ctx=ignore(Js.Unsafe.meth_call ctx "restore" [||])
  let stroke ctx p=ignore(Js.Unsafe.meth_call ctx "stroke" [|p|])
  let clip ctx p=ignore(Js.Unsafe.meth_call ctx "clip" [|p|])
end
module Port=Chart.Make(Host)
let ()=Js.export "run" (Js.wrap_callback Port.run)
