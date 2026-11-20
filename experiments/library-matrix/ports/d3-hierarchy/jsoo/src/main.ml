open Js_of_ocaml
let field v k = Js.Unsafe.get v k
let rec datum v : Hierarchy.datum =
  let str k = Option.fold ~none:"" ~some:Js.to_string (Js.Optdef.to_option (field v k)) in
  {name=str "id";parent_name=str "parentId";weight=Option.value ~default:0. (Js.Optdef.to_option(field v "value"));
   children=Option.fold ~none:[||] ~some:(fun a -> Array.map datum (Js.to_array a)) (Js.Optdef.to_option(field v "children"))}
let run input =
  let f k=field input k in let str k=Js.to_string(f k) and flag k=Js.to_bool(f k) in
  let source=str "source" in let nested=source<>"id" && source<>"path" in
  let req : Hierarchy.input={op=str "op";source;data=(if nested then Some(datum(f "data")) else None);
    rows=(if nested then [||] else Array.map datum (Js.to_array(f "rows")));width=f "width";height=f "height";
    padding=f "padding";node_size=flag "nodeSize";sort=flag "sort";round=flag "round";tile=str "tile"} in
  let cell=function Hierarchy.Text s->Js.Unsafe.inject(Js.string s) | Num x->Js.Unsafe.inject(Js.number_of_float x) | Null->Js.Unsafe.inject Js.null in
  Hierarchy.run req |> Array.map (fun row->Js.array(Array.map cell row)) |> Js.array
let () = Js.export "run" (Js.wrap_callback run)
