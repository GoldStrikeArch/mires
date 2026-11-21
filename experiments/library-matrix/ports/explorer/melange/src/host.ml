type value
external box : 'a -> value = "%identity"
external obj : unit -> value = "Object" [@@mel.new]
external array : unit -> value = "Array" [@@mel.new]
external get : value -> string -> value = "" [@@mel.get_index]
external set : value -> string -> value -> unit = "" [@@mel.set_index]
external at : value -> int -> value = "" [@@mel.get_index]
external push : value -> value -> unit = "push" [@@mel.send]
external length : value -> int = "length" [@@mel.get]
external json : value -> value = "stringify" [@@mel.scope "JSON"]
external parse : value -> value = "parse" [@@mel.scope "JSON"]
external join : value -> value -> value = "join" [@@mel.send]
external text : value -> string = "%identity"
let nil=box Js.null
let str(s:string)=box s
let int (n:int)=box n
let bool (b:bool)=box b
