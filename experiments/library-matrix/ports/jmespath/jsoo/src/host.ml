open Js_of_ocaml
type value=Js.Unsafe.any
let box=Js.Unsafe.inject
let nil=box Js.null
let und=box Js.undefined
let str s=box(Js.string s)
let text x=Js.to_string(Js.Unsafe.coerce x)
let num n=box(Js.number_of_float n)
let bool b=box(Js.bool b)
let strict a b=Js.strict_equals a b
let obj()=box(Js.Unsafe.obj [||])
let array()=box(Js.array [||])
let get v k=Js.Unsafe.get v k
let prop v (k:value)=Js.Unsafe.get v k
let set v k x=Js.Unsafe.set v k x
let put v (k:value)x=Js.Unsafe.set v k x
let at v i=Js.Unsafe.get v i
let length v:int=Js.Unsafe.get v "length"
let meth v k args=Js.Unsafe.meth_call v k args
let scoped scope name args=meth(Js.Unsafe.get Js.Unsafe.global scope)name args
let push v x=ignore(meth v "push" [|x|])
let keys v=scoped "Object" "keys" [|v|]
let values v=scoped "Object" "values" [|v|]
let is_array v=Js.to_bool(scoped "Array" "isArray" [|v|])
let json v=scoped "JSON" "stringify" [|v|]
let parse v=scoped "JSON" "parse" [|v|]
let global name v=Js.Unsafe.fun_call(Js.Unsafe.get Js.Unsafe.global name)[|v|]
let number v=Js.float_of_number(global "Number" v)
let as_string v=global "String" v
let parse_int v=Js.float_of_number(global "parseInt" v)
let concat a b=meth a "concat" [|b|]
let unit_at v i:int=meth v "charCodeAt" [|box i|]
let slice v i j=meth v "slice" [|box i;box j|]
let replace v a b=meth v "replace" [|str a;str b|]
let trim_start v=meth v "trimStart" [||]
let index v x:int=meth v "indexOf" [|x|]
let index_from v x i:int=meth v "indexOf" [|x;box i|]
let last_index v x:int=meth v "lastIndexOf" [|x|]
let locale_cmp a b:int=meth a "localeCompare" [|b|]
let join v x=meth v "join" [|x|]
let sorted v=meth v "sort" [||]
let reversed v=meth v "reverse" [||]
let typeof v=Js.to_string(Js.typeof v)
let math_max (a:float) (b:float):float=Js.float_of_number(Js.Unsafe.coerce(scoped "Math" "max" [|num a;num b|]))
let math_min (a:float) (b:float):float=Js.float_of_number(Js.Unsafe.coerce(scoped "Math" "min" [|num a;num b|]))
(* Array.prototype.sort with an OCaml comparator, in place *)
let sort_with a compare=ignore(meth a "sort" [|box(Js.wrap_callback(fun x y->Js.number_of_float(compare x y)))|])
(* Stack exhaustion, which the reference adapter lets propagate rather than file as "syntax".
   js_of_ocaml raises Stack_overflow for the engine's RangeError "Maximum call stack size exceeded"
   (V8, JavaScriptCore) and InternalError "too much recursion" (SpiderMonkey). *)
let stack_exhausted=function Stack_overflow->true|_->false
