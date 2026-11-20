type value
external field : value -> string -> 'a = "" [@@mel.get_index]
external cast : 'a -> value = "%identity"
let rec datum v : Hierarchy.datum =
  let opt k = Js.Nullable.toOption (field v k) in
  { name=Option.value ~default:"" (opt "id"); weight=Option.value ~default:0. (opt "value");
    parent_name=Option.value ~default:"" (opt "parentId");children=Array.map datum (Option.value ~default:[||] (opt "children")) }
let run input =
  let f k = field input k in
  let source=f "source" in
  let nested=source<>"id" && source<>"path" in
  let req : Hierarchy.input = { op=f "op";source;data=(if nested then Some(datum (f "data")) else None);
    rows=(if nested then [||] else Array.map datum (f "rows"));width=f "width";height=f "height";
    padding=f "padding";node_size=f "nodeSize";sort=f "sort";round=f "round";tile=f "tile"} in
  Hierarchy.run req |> Array.map (Array.map (function Hierarchy.Text s->cast s | Num x->cast x | Null->cast Js.null))
