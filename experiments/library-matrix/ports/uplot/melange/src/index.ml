(* The boundary (Melange): the input and the result are JS values behind the abstract [v], read and
   built with Melange externals, and the canvas calls are externals too. The chart is
   [Chart.Make (Host)], shared with the js_of_ocaml port. *)

module Host = struct
  type v

  external field : v -> string -> v = "" [@@mel.get_index]
  external at : v -> int -> v = "" [@@mel.get_index]
  external len : v -> int = "length" [@@mel.get]
  external cast : 'a -> 'b = "%identity"

  let null : v = cast Js.null
  let is_null v = v == null
  let num v : float = cast v
  let number v : v = cast v
  let bool v : bool = cast v

  external array : unit -> v = "Array" [@@mel.new]
  external push : v -> v -> unit = "push" [@@mel.send]
  external round : float -> float = "round" [@@mel.scope "Math"]
  external log10 : float -> float = "log10" [@@mel.scope "Math"]
  external is_integer : float -> bool = "isInteger" [@@mel.scope "Number"]
  external to_string : float -> string = "String"
  external parse_float : string -> float = "Number"
  external window : v = "window"

  let pixel_ratio () = num (field window "devicePixelRatio")

  external doc : v = "document"
  external element : v -> string -> v = "createElement" [@@mel.send]
  external append : v -> v -> unit = "appendChild" [@@mel.send]

  let canvas () =
    let c = element doc "canvas" in
    append (field doc "body") c;
    c

  external remove : v -> unit = "remove" [@@mel.send]
  external get_context : v -> string -> v = "getContext" [@@mel.send]

  let context c = get_context c "2d"

  external set : v -> string -> 'a -> unit = "" [@@mel.set_index]

  let set_number o key (x : float) = set o key x
  let set_string o key (x : string) = set o key x

  external path : unit -> v = "Path2D" [@@mel.new]
  external line : v -> float -> float -> unit = "lineTo" [@@mel.send]
  external rect : v -> float -> float -> float -> float -> unit = "rect" [@@mel.send]
  external save : v -> unit = "save" [@@mel.send]
  external restore : v -> unit = "restore" [@@mel.send]
  external translate : v -> float -> float -> unit = "translate" [@@mel.send]
  external clip : v -> v -> unit = "clip" [@@mel.send]
  external stroke : v -> v -> unit = "stroke" [@@mel.send]
end

module Port = Chart.Make (Host)

let run = Port.run
