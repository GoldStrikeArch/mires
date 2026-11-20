external number : string -> float = "Number"
external text : 'a -> string = "String"
external sqrt : float -> float = "sqrt" [@@mel.scope "Math"]
let root s = sqrt (number s)
