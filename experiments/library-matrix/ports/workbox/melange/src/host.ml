type j
external globals : j = "globalThis"
external get : j -> string -> j = "" [@@mel.get_index]
external setProp : j -> string -> j -> unit = "" [@@mel.set_index]
external at : j -> int -> j = "" [@@mel.get_index]
external cast : 'a -> 'b = "%identity"
let globalValue k = get globals k
let strVal (s:string) : j = cast s
let nil () : j = cast Js.Nullable.null
let undef () : j = cast None
let missing x = x == undef ()
external truth : j -> bool = "Boolean"
let equalText x (s:string) = x == strVal s
let equalNum x n = x == (cast (n:int) : j)
external objectValue : unit -> j = "Object" [@@mel.new]
let pack (a:j array) : j = cast a
external push : j -> j -> unit = "push" [@@mel.send]
external apply : j -> j -> j array -> j = "apply" [@@mel.scope "Reflect"]
let call x k args = apply (get x k) x args
let callback (f:j->j) : j = cast f
external chain : j -> (j -> j) -> j = "then" [@@mel.send]
external always : j -> (j -> j) -> j = "finally" [@@mel.send]
external mapValues : j -> (j -> j) -> j = "map" [@@mel.send]
external resolved : unit -> j = "resolve" [@@mel.scope "Promise"]
let sequence xs f = Array.fold_left (fun p x -> chain p (fun _ -> f x)) (resolved ()) (cast xs)
external makeRaw : j -> j array -> j = "construct" [@@mel.scope "Reflect"]
let makeHost k args = makeRaw (globalValue k) args
let boolean (b:bool) : j = cast b
let nullish x = x == nil () || missing x
let as_string (v:j) : string = cast v
external ok : j -> j = "resolve" [@@mel.scope "Promise"]
external bad : j -> j = "reject" [@@mel.scope "Promise"]
external recoverPromise : j -> (j -> j) -> j = "catch" [@@mel.send]
external collect : j -> j = "from" [@@mel.scope "Array"]
let badStatus v = (cast v : int) >= 400
external skipWaiting : unit -> j = "skipWaiting"
external fetchRequest : j -> j = "fetch"
let same x y = x == y
let suffix s k = strVal (as_string s ^ k)
let starts s prefix = String.starts_with ~prefix (as_string s)
let ends s suffix = String.ends_with ~suffix (as_string s)
let relative s base = let url=as_string s in let from=String.length (as_string base) in strVal (String.sub url from (String.length url-from))
let uniqueName () = strVal ("mires-workbox-" ^ as_string (call (globalValue "crypto") "randomUUID" [||]))
let defaulted x y = if missing x then y else x
