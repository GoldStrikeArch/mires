open Host
open Parser

let kind v = if strict v nil then "null" else if is_array v then "array" else
  match typeof v with
  | "object" when strict(get v "jmespathType")(str "Expref")->"expref"
  | t->t
let object_kind v=let t=kind v in t="object"||t="expref"
let falsy v=match kind v with
  |"null"->true|"boolean"->strict v(bool false)|"array"|"string"->length v=0|"object"|"expref"->length(keys v)=0|_->false
let every n f=let rec loop i=i=n||f i&&loop(i+1)in loop 0
let rec equal a b=
  if strict a b then true else let t=kind a in
  if t<>kind b then false else if t="array"then length a=length b && every(length a)(fun i->equal(at a i)(at b i))
  else if t="object"then let ks=keys a in length ks=length(keys b)&&every(length ks)(fun i->let k=at ks i in equal(prop a k)(prop b k))else false
let cmp a b=
  let primitive v=if typeof v="object"&&not(strict v nil)then as_string v else v in
  let x=primitive a and y=primitive b in
  if typeof x="string"&&typeof y="string"then
    let bound=min(length x)(length y)in let rec loop i=if i=bound then compare(length x)(length y)
      else let d=compare(unit_at x i)(unit_at y i)in if d=0 then loop(i+1)else d in Some(loop 0)
  else let x=number x and y=number y in if Float.is_nan x||Float.is_nan y then None else Some(compare x y)
let signature=function
  |"abs"|"floor"|"ceil"->["number"]|"avg"|"sum"->["numbers"]|"contains"->["array|string";"any"]
  |"starts_with"|"ends_with"->["string";"string"]|"length"->["object|string|array"]|"map"->["expref";"array"]
  |"max"|"min"|"sort"->["numbers|strings"]|"keys"|"values"|"merge"->["object"]|"max_by"|"min_by"|"sort_by"->["array";"expref"]
  |"join"->["string";"strings"]|"reverse"->["string|array"]|"to_array"|"to_number"|"to_string"|"type"|"not_null"->["any"]
  |_->raise(Error "unknown-function")
let valid v t=
  let subtype k=is_array v && every(length v)(fun i->kind(at v i)=k)in
  List.exists(function "any"->true|"strings"->subtype "string"|"numbers"->subtype "number"|tag->kind v=tag)(String.split_on_char '|' t)
let child n i=at(get n "children")i
let rec eval n v=
  if strict n nil || strict n und then syntax();
  let k=text(get n "type")in let arg i=eval(child n i)v in
  match k with
  |"Field"->if not(object_kind v)then nil else let x=prop v(get n "name")in if strict x und then nil else x
  |"Identity"|"Current"->v|"Literal"->get n "value"|"Pipe"|"IndexExpression"|"Subexpression"->eval(child n 1)(arg 0)
  |"Index"->if not(is_array v)then nil else let f=number(get n "value")in let p=if f<0. then f+.float_of_int(length v)else f in
    if p>=0.&&p<float_of_int(length v)then at v(int_of_float p)else nil
  |"Slice"->if not(is_array v)then nil else begin
    let p = child n 2 in
    let step = if strict p nil then 1. else number p in
    if step = 0. then raise (Error "invalid-value");
    let len = float_of_int (length v) in
    let low, high = if step < 0. then -1., len -. 1. else 0., len in
    let bound x default =
      if strict x nil then default else
      let raw = number x in
      max low (min high (if raw < 0. then raw +. len else raw)) in
    let i = ref (bound (child n 0) (if step < 0. then high else low)) in
    let stop = bound (child n 1) (if step < 0. then low else high) in
    let out = array () in
    while (if step < 0. then !i > stop else !i < stop) do
      push out (at v (int_of_float !i));
      i := !i +. step
    done;
    out
  end
  |"Projection"|"ValueProjection"|"FilterProjection"->let base=arg 0 in
    let src=if k<>"ValueProjection"then base else if object_kind base then values base else nil in
    if not(is_array src)then nil else let out=array()in
    for i=0 to length src-1 do let x=at src i in if k<>"FilterProjection"||not(falsy(eval(child n 2)x))then
      let y=eval(child n 1)x in if not(strict y nil)then push out y done;out
  |"Flatten"->let src=arg 0 in if not(is_array src)then nil else let out=array()in
    for i=0 to length src-1 do let x=at src i in if is_array x then (for j=0 to length x-1 do push out(at x j) done)else push out x done;out
  |"Comparator"->let a=arg 0 in let b=arg 1 in
    let op=text(get n "name")in bool(match op with "EQ"->equal a b|"NE"->not(equal a b)|_->match cmp a b with
      None->false|Some c->(match op with "LT"->c<0|"GT"->c>0|"LTE"->c<=0|_->c>=0))
  |"AndExpression"|"OrExpression"->let a=arg 0 in if falsy a=(k="OrExpression")then arg 1 else a
  |"NotExpression"->bool(falsy(arg 0))
  |"MultiSelectHash"|"MultiSelectList"->if strict v nil then nil else let hash=k="MultiSelectHash"in let out=if hash then obj()else array()in
    let cs=get n "children"in for i=0 to length cs-1 do let c=at cs i in if hash then put out(get c "name")(eval(get c "value")v)else push out(eval c v)done;out
  |"ExpressionReference"->let c=child n 0 in set c "jmespathType"(str "Expref");c
  |"Function"->let args=array()and cs=get n "children"in
    for i=0 to length cs-1 do push args(eval(at cs i)v)done;let name=get n "name"in if typeof name<>"string"then raise(Error "unknown-function");call (text name) args
  |_->syntax()
and call name args=
  let spec=signature name in let arity=List.length spec in
  if length args<arity || ((name<>"merge"&&name<>"not_null")&&length args<>arity) then raise(Error "invalid-arity");
  List.iteri(fun i t->if not(valid(at args i)t)then raise(Error "invalid-type"))spec;
  let a=at args 0 and b=at args 1 in
  match name with
  |"abs"->num(abs_float(number a))|"floor"->num(floor(number a))|"ceil"->num(ceil(number a))
  |"avg"|"sum"->let total=ref 0. in for i=0 to length a-1 do total:= !total+.number(at a i)done;
    num(if name="avg"then !total/.float_of_int(length a)else !total)
  |"contains"->bool(index a b>=0)|"starts_with"->bool(last_index a b=0)|"ends_with"->bool(index_from a b(length a-length b)>=0)
  |"length"->num(float_of_int(if kind a="object"then length(keys a)else length a))
  |"join"->join b a|"type"->str(kind a)|"keys"->keys a|"values"->values a|"to_array"->if is_array a then a else list[a]
  |"to_number"->if typeof a="number"then a else if typeof a="string"&&not(Float.is_nan(number a))then num(number a)else nil
  |"to_string"->if typeof a="string"then a else json a
  |"not_null"->let rec loop i=if i=length args then nil else let x=at args i in if strict x nil then loop(i+1)else x in loop 0
  |"reverse"->if typeof a="string"then let x=ref(str "")in for i=length a-1 downto 0 do x:=concat !x(slice a i(i+1))done;!x
    else reversed(slice a 0(length a))
  |"sort"->sorted(slice a 0(length a))
  |"merge"->let out=obj()in for i=0 to length args-1 do let x=at args i in
    if not(strict x nil||strict x und)then let ks=keys x in for j=0 to length ks-1 do let k=at ks j in put out k(prop x k)done done;out
  |"max"|"min"->if length a=0 then nil else let best=ref(at a 0)in
    for i=1 to length a-1 do let x=at a i in let d=if typeof x="string"then locale_cmp x !best else Option.value ~default:0(cmp x !best)in
      if (if name="min"then d<0 else d>0)then best:=x done;!best
  |"map"->let out=array()in for i=0 to length b-1 do push out(eval a(at b i))done;out
  |"min_by"|"max_by"->let max_=name="max_by"in let extreme=ref(num(if max_ then neg_infinity else infinity)) and out=ref und in
    for i=0 to length a-1 do let x=at a i in let key=eval b x in
      if kind key<>"number"&&kind key<>"string"then raise(Error "invalid-type");
      (match cmp key !extreme with Some d when (if max_ then d>0 else d<0)->extreme:=key;out:=x|_->())done;!out
  |"sort_by"->let out=array()in if length a>0 then begin
    let ty=kind(eval b(at a 0))in if ty<>"number"&&ty<>"string"then raise(Error "invalid-type");
    let pairs=Array.init(length a)(fun i->let item=at a i in let key=eval b item in
      if kind key<>ty then raise(Error "invalid-type");key,item)in
    Array.stable_sort(fun (x,_) (y,_)->Option.value ~default:0(cmp x y))pairs;
    Array.iter(fun (_,item)->push out item)pairs
  end;out
  |_->raise(Error "unknown-function")
