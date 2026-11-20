open Js_of_ocaml
type j = Js.Unsafe.any
let inject = Js.Unsafe.inject
let globalValue k : j = Js.Unsafe.get Js.Unsafe.global k
let get (x:j) k : j = Js.Unsafe.get x k
let setProp (x:j) k (v:j) = Js.Unsafe.set x k v
let at (x:j) i : j = Js.Unsafe.get x i
let strVal s : j = inject (Js.string s)
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
let chain p f = call p "then" [|callback f|]
let always p f = call p "finally" [|callback f|]
let mapValues a f = call a "map" [|callback f|]
let sequence xs f =
  let arr = Js.to_array (Js.Unsafe.coerce xs) in
  Array.fold_left (fun p x -> chain p (fun _ -> f x)) (call (globalValue "Promise") "resolve" [||]) arr
let makeHost k args : j = inject (Js.Unsafe.new_obj (globalValue k) args)
let boolean b : j = inject (Js.bool b)
let nullish x = missing x || not (Js.Opt.test (Obj.magic x))
let as_string (v:j) = Js.to_string (Js.Unsafe.coerce v)
let ok v = call (globalValue "Promise") "resolve" [|v|]
let bad v = call (globalValue "Promise") "reject" [|v|]
let recoverPromise p f = call p "catch" [|callback f|]
let collect v = call (globalValue "Array") "from" [|v|]
let badStatus v = (Obj.magic v : int) >= 400
let skipWaiting () : j = Js.Unsafe.meth_call Js.Unsafe.global "skipWaiting" [||]
let fetchRequest req : j = Js.Unsafe.meth_call Js.Unsafe.global "fetch" [|req|]
let same x y = truth (call (globalValue "Object") "is" [|x;y|])
let suffix s k = strVal (as_string s ^ k)
let starts s prefix = String.starts_with ~prefix (as_string s)
let ends s suffix = String.ends_with ~suffix (as_string s)
let relative s base = let url=as_string s in let from=String.length (as_string base) in strVal (String.sub url from (String.length url-from))
let uniqueName () = strVal ("mires-workbox-" ^ as_string (call (globalValue "crypto") "randomUUID" [||]))
let defaulted x y = if missing x then y else x
