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
let makeHost k args : j = inject (Js.Unsafe.new_obj (globalValue k) args)
let boolean b : j = inject (Js.bool b)
let nullish x = missing x || not (Js.Opt.test (Obj.magic x))
let as_float (v:j) = Js.float_of_number (Js.Unsafe.coerce v)
let as_string (v:j) = Js.to_string (Js.Unsafe.coerce v)
let boxed_float f = inject (Js.number_of_float f)
let as_array (x:j) : j array = Js.to_array (Js.Unsafe.coerce x)
let ok v = call (globalValue "Promise") "resolve" [|v|]
let bad v = call (globalValue "Promise") "reject" [|v|]
let recoverPromise p f = call p "catch" [|callback f|]
let handled p s f = call p "then" [|callback s;callback f|]
let timer f ms : j = Js.Unsafe.fun_call (globalValue "setTimeout") [|f;ms|]
let stopTimer id = ignore (Js.Unsafe.fun_call (globalValue "clearTimeout") [|id|] : j)
let isNumeric x = truth (call (globalValue "Number") "isFinite" [|x|])
let header h k = call h "get" [|strVal k|]
let waitTime ms = promise (fun resolve _ -> ignore (timer resolve ms);undef ())
let lastOrIndex xs i = let a=as_array xs in a.(min (int_of_float (as_float i)) (Array.length a-1))
let smaller a b = as_float a < as_float b
let increment a = num (1+int_of_float (as_float a))
let lesser a b = if smaller a b then a else b
let retryTag a = strVal ("retry-" ^ string_of_int (int_of_float (as_float a)))
let exponential a = boxed_float (300. *. 2. ** as_float a)
let jsonType a =
  if nullish a then false else
    let raw=as_string a in
    let stop=Option.value (String.index_opt raw ';') ~default:(String.length raw) in
    let m=String.sub raw 0 stop |> String.trim |> String.lowercase_ascii in
    String.contains m '/' && List.exists (fun suffix -> String.ends_with ~suffix m) ["/json";"+json";"-json";".json"]
let parseAfter a =
  let s=String.trim (as_string a) in
  if s<>"" && String.for_all (fun c -> c>='0' && c<='9') s then boxed_float (1000. *. float_of_string s) else nil ()
let canonicalMethod m =
  match String.lowercase_ascii m with
  | "get" | "post" | "put" | "patch" | "head" | "delete" | "query" | "options" | "trace" as known ->
      String.uppercase_ascii known
  | _ -> m
let allowsMethod c m =
  let k=canonicalMethod (as_string m) and methods=get c "methods" in
  if missing methods then List.mem k ["GET";"PUT";"HEAD";"DELETE";"OPTIONS";"TRACE";"QUERY"]
  else Array.exists (fun v->canonicalMethod (as_string v)=k) (as_array methods)
let allowsStatus c status =
  let choices=get c "statusCodes" in
  if nullish status then false
  else if missing choices then List.mem (int_of_float (as_float status)) [408;413;429;500;502;503;504]
  else Array.exists (fun n->as_float n=as_float status) (as_array choices)
