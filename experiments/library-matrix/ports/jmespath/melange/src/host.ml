(* Host boundary for native JSON values and JS UTF-16 string operations. *)
type value
external box : 'a -> value = "%identity"
external text : value -> string = "%identity"
external float_value : value -> float = "%identity"
external obj : unit -> value = "Object" [@@mel.new]
external array : unit -> value = "Array" [@@mel.new]
external get : value -> string -> value = "" [@@mel.get_index]
external prop : value -> value -> value = "" [@@mel.get_index]
external set : value -> string -> value -> unit = "" [@@mel.set_index]
external put : value -> value -> value -> unit = "" [@@mel.set_index]
external at : value -> int -> value = "" [@@mel.get_index]
external push : value -> value -> unit = "push" [@@mel.send]
external length : value -> int = "length" [@@mel.get]
external keys : value -> value = "keys" [@@mel.scope "Object"]
external values : value -> value = "values" [@@mel.scope "Object"]
external is_array : value -> bool = "isArray" [@@mel.scope "Array"]
external json : value -> value = "stringify" [@@mel.scope "JSON"]
external parse : value -> value = "parse" [@@mel.scope "JSON"]
external number : value -> float = "Number"
external as_string : value -> value = "String"
external parse_int : value -> float = "parseInt"
external concat : value -> value -> value = "concat" [@@mel.send]
external unit_at : value -> int -> int = "charCodeAt" [@@mel.send]
external slice : value -> int -> int -> value = "slice" [@@mel.send]
external replace : value -> string -> string -> value = "replace" [@@mel.send]
external trim_start : value -> value = "trimStart" [@@mel.send]
external index : value -> value -> int = "indexOf" [@@mel.send]
external index_from : value -> value -> int -> int = "indexOf" [@@mel.send]
external last_index : value -> value -> int = "lastIndexOf" [@@mel.send]
external locale_cmp : value -> value -> int = "localeCompare" [@@mel.send]
external join : value -> value -> value = "join" [@@mel.send]
external sorted : value -> value = "sort" [@@mel.send]
external reversed : value -> value = "reverse" [@@mel.send]
external typeof : value -> string = "#typeof"
external math_max : float -> float -> float = "max" [@@mel.scope "Math"]
external math_min : float -> float -> float = "min" [@@mel.scope "Math"]
external sort_by_function : value -> (value -> value -> float [@u]) -> value = "sort" [@@mel.send]
(* Array.prototype.sort with an OCaml comparator, in place *)
let sort_with a compare = ignore (sort_by_function a (fun [@u] x y -> compare x y))
let strict (x:value)y=x==y
let nil=box Js.null
let und=box Js.undefined
let str(s:string)=box s
let num(n:float)=box n
let bool(b:bool)=box b
(* JSON.parse's SyntaxError, which Melange hands over as Js.Exn.Error *)
let syntax_error=function Js.Exn.Error e->Js.Exn.name e=Some "SyntaxError"|_->false
