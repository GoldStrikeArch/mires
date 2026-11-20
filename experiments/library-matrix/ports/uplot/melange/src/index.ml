module Host = struct
  type v
  external field : v -> string -> v = "" [@@mel.get_index]
  external at : v -> int -> v = "" [@@mel.get_index]
  external len : v -> int = "length" [@@mel.get]
  external cast : 'a -> 'b = "%identity"
  let null : v = cast Js.null
  let is_null v=v==null
  let num v : float=cast v
  let number v : v=cast v
  let bool v : bool=cast v
  external round : float -> float = "round" [@@mel.scope "Math"]
  external array : unit -> v = "Array" [@@mel.new]
  external push : v -> v -> unit = "push" [@@mel.send]
  external doc : v = "document"
  external element : v -> string -> v = "createElement" [@@mel.send]
  external append : v -> v -> unit = "appendChild" [@@mel.send]
  let canvas ()=let c=element doc "canvas" in append (field doc "body") c;c
  external remove : v -> unit = "remove" [@@mel.send]
  external get_context : v -> string -> v = "getContext" [@@mel.send]
  let context c=get_context c "2d"
  external set : v -> string -> 'a -> unit = "" [@@mel.set_index]
  let resize c ctx w h=set c "width" w;set c "height" h;set ctx "strokeStyle" "#172839"
  external path : unit -> v = "Path2D" [@@mel.new]
  external line : v -> float -> float -> unit = "lineTo" [@@mel.send]
  external rect : v -> float -> float -> float -> float -> unit = "rect" [@@mel.send]
  external window : v = "window"
  let pixel_ratio () = num (field window "devicePixelRatio")
  let stroke_style ctx width = set ctx "lineWidth" width; set ctx "lineJoin" "round"; set ctx "lineCap" "butt"
  external translate : v -> float -> float -> unit = "translate" [@@mel.send]
  external save : v -> unit = "save" [@@mel.send]
  external restore : v -> unit = "restore" [@@mel.send]
  external stroke : v -> v -> unit = "stroke" [@@mel.send]
  external clip : v -> v -> unit = "clip" [@@mel.send]
end
module Port=Chart.Make(Host)
let run=Port.run
