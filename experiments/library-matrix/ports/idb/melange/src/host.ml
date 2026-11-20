type j
external globals : j = "globalThis"
external get : j -> string -> j = "" [@@mel.get_index]
external setProp : j -> string -> j -> unit = "" [@@mel.set_index]
external at : j -> int -> j = "" [@@mel.get_index]
external cast : 'a -> 'b = "%identity"
let globalValue k = get globals k
let strVal (s:string) : j = cast s
let num (i:int) : j = cast i
let nil () : j = cast Js.Nullable.null
let undef () : j = cast None
let missing x = x == undef ()
external truth : j -> bool = "Boolean"
let equalText x (s:string) = x == strVal s
let equalNum x n = x == num n
external objectValue : unit -> j = "Object" [@@mel.new]
let pack (a:j array) : j = cast a
external push : j -> j -> unit = "push" [@@mel.send]
external apply : j -> j -> j array -> j = "apply" [@@mel.scope "Reflect"]
let call x k args = apply (get x k) x args
let callback (f:j->j) : j = cast f
external newPromise : ((j -> j -> j) [@u]) -> j = "Promise" [@@mel.new]
let promise f = newPromise (fun [@u] a b -> f a b)
let settle f v = (cast f : j -> unit) v
external chain : j -> (j -> j) -> j = "then" [@@mel.send]
external always : j -> (j -> j) -> j = "finally" [@@mel.send]
external mapValues : j -> (j -> j) -> j = "map" [@@mel.send]
external resolved : unit -> j = "resolve" [@@mel.scope "Promise"]
let sequence xs f = Array.fold_left (fun p x -> chain p (fun _ -> f x)) (resolved ()) (cast xs)
external domException : string -> string -> j = "DOMException" [@@mel.new]
let abortError () = domException "AbortError" "AbortError"
let errorName x = strVal ("!" ^ (cast x : string))
let uniqueName () = strVal ("mires-matrix-idb-" ^ (cast (call (globalValue "crypto") "randomUUID" [||]) : string))
