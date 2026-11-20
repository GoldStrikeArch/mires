open Js_of_ocaml
type j = Js.Unsafe.any
let inject = Js.Unsafe.inject
let globalValue k : j = Js.Unsafe.get Js.Unsafe.global k
let get (x:j) k : j = Js.Unsafe.get x k
let setProp (x:j) k (v:j) = Js.Unsafe.set x k v
let at (x:j) i : j = Js.Unsafe.get x i
let strVal s : j = inject (Js.string s)
let num i : j = inject i
let nil () : j = inject Js.null
let undef () : j = inject Js.undefined
let missing x = not (Js.Optdef.test (Obj.magic x))
let truth x = Js.to_bool (Js.Unsafe.fun_call (globalValue "Boolean") [|x|])
let equalText x s = Js.to_string (Js.Unsafe.coerce x) = s
let equalNum x (n:int) = (Obj.magic x : int) = n
let objectValue () : j = inject (Js.Unsafe.obj [||])
let pack a : j = inject (Js.array a)
let call (x:j) k (args:j array) : j = Js.Unsafe.meth_call x k args
let push x v = ignore (call x "push" [|v|])
let callback f : j = inject (Js.wrap_callback f)
let promise f : j = Js.Unsafe.new_obj (globalValue "Promise") [|inject (Js.wrap_callback f)|]
let settle f v = ignore (Js.Unsafe.fun_call f [|v|] : j)
let chain p f = call p "then" [|callback f|]
let always p f = call p "finally" [|callback f|]
let mapValues a f = call a "map" [|callback f|]
let sequence xs f =
  let arr = Js.to_array (Js.Unsafe.coerce xs) in
  Array.fold_left (fun p x -> chain p (fun _ -> f x)) (call (globalValue "Promise") "resolve" [||]) arr
let abortError () : j = Js.Unsafe.new_obj (globalValue "DOMException") [|strVal "AbortError";strVal "AbortError"|]
let errorName x = strVal ("!" ^ Js.to_string (Js.Unsafe.coerce x))
let uniqueName () = strVal ("mires-matrix-idb-" ^ Js.to_string (Js.Unsafe.coerce (call (globalValue "crypto") "randomUUID" [||])))
