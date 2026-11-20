open Host
open Differ
let canonical v =
  let rec visit v=
    if is_array v then begin let a=array(length v)in for i=0 to length v-1 do put a i(visit(at v i))done;a end
    else if object_value v then begin let o=null_object null and ks=sorted(keys v)in
      for i=0 to length ks-1 do let k=at ks i in key_set o k(visit(key_get v k))done;o end else v in
  visit v
let document op v=if op="diffJson" && not(is_string v)then read(pretty(canonical v)null 2)else read v
let write_change span v=
  let o=obj()in set o "count"(number span.count);set o "added"(boolean(span.kind=Add));set o "removed"(boolean(span.kind=Remove));set o "value" v;o
let run input=
  let op=read(get input "op")and a=get input "before"and b=get input "after"and arg=get input "patch"and opts=get input "options" in
  let flag key=not(absent opts)&&truth(get opts key)in
  let o={ignore_case=flag "ignoreCase";individual=flag "oneChangePerToken";strip_cr=flag "stripTrailingCr";ignore_ws=flag "ignoreWhitespace";newline_token=flag "newlineIsToken";ignore_eof=flag "ignoreNewlineAtEof"}in
  match op with
  |"diffArrays"->
    let values v=Patch.array_of_js(fun x->read(json x))v in
    let spans=Array.of_list(myers(values a)(values b)o.individual)in let old=ref 0 and newer=ref 0 in
    Patch.array_to_js(fun span->
      let value=array span.count in for i=0 to span.count-1 do put value i(if span.kind=Remove then at a(!old+i)else at b(!newer+i))done;
      if span.kind<>Add then old:= !old+span.count;if span.kind<>Remove then newer:= !newer+span.count;write_change span value)spans
  |"diffChars"|"diffWords"|"diffLines"|"diffJson"->
    let tokens v=tokenize op (document op v)o in Patch.array_to_js(fun c->write_change c.span(str c.value))(changes op(tokens a)(tokens b)o)
  |"structuredPatch"|"createTwoFilesPatch"->
    let ctx=if absent opts||absent(get opts "context")then 4 else integer(get opts "context")in
    let p=Patch.create input ctx in if op="structuredPatch"then Patch.write false p else str(Patch.format p)
  |"parsePatch"->Patch.array_to_js(Patch.write false)(Patch.parse(read arg))
  |"reversePatch"->if is_array arg then begin let n=length arg in let v=array n in for i=0 to n-1 do put v (n-i-1)(Patch.reverse(at arg i))done;v end else Patch.reverse arg
  |"applyPatch"->let p=if is_string arg then begin let ps=Patch.parse(read arg)in if Array.length ps<>1 then invalid_arg "multiple patches";ps.(0)end
    else if is_array arg then begin if length arg<>1 then invalid_arg "multiple patches";Patch.read_patch(at arg 0)end else Patch.read_patch arg in Patch.apply(read a)p
  |_->invalid_arg "unknown operation"
