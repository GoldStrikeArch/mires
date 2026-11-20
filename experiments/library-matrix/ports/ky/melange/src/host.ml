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
external makeRaw : j -> j array -> j = "construct" [@@mel.scope "Reflect"]
let makeHost k args = makeRaw (globalValue k) args
let boolean (b:bool) : j = cast b
let nullish x = x == nil () || missing x
let as_float (v:j) : float = cast v
let as_string (v:j) : string = cast v
let boxed_float (f:float) : j = cast f
let as_array (x:j) : j array = cast x
external ok : j -> j = "resolve" [@@mel.scope "Promise"]
external bad : j -> j = "reject" [@@mel.scope "Promise"]
external recoverPromise : j -> (j -> j) -> j = "catch" [@@mel.send]
external handled : j -> (j -> j) -> (j -> j) -> j = "then" [@@mel.send]
external timer : j -> j -> j = "setTimeout"
external stopTimer : j -> unit = "clearTimeout"
external numeric : j -> bool = "isFinite" [@@mel.scope "Number"]
let isNumeric = numeric
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
