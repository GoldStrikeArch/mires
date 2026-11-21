open Host

exception Error of string
let syntax()=raise(Error "syntax")
type token={kind:string;value:value}
type parser={tokens:token array;mutable pos:int}
let node t=let n=obj()in set n "type"(str t);n
let named t v=let n=node t in set n "name" v;n
let leaf t v=let n=node t in set n "value" v;n
let list xs=let a=array()in List.iter(fun v->push a v)xs;a
let children t xs=let n=node t in set n "children"(list xs);n
let is_alpha c=(c>=65&&c<=90)||(c>=97&&c<=122)||c=95
let digit c=c>=48&&c<=57
(* JSON.parse, whose SyntaxError category() files as "syntax"; anything else it raises propagates *)
let json_value v=try parse v with e when syntax_error e->syntax()
let lex source=
  let n=length source and i=ref 0 and tokens=ref []in
  let add k v=tokens:={kind=k;value=v}::!tokens in
  while !i<n do
    let start= !i and c=unit_at source !i in incr i;
    let next=if !i<n then unit_at source !i else 0 in
    if is_alpha c then begin
      while !i<n && (let ch=unit_at source !i in is_alpha ch||digit ch)do incr i done;
      add "Field"(slice source start !i)
    end else if c=45||digit c then begin
      while !i<n && (let ch=unit_at source !i in ch=45||digit ch)do incr i done;
      add "Number"(num(parse_int(slice source start !i)))
    end else if c=34||c=39||c=96 then begin
      while !i<n && unit_at source !i<>c do
        if unit_at source !i=92 && !i+1<n && (unit_at source(!i+1)=c||unit_at source(!i+1)=92) then incr i;
        incr i
      done;
      let raw=slice source(start+1)!i in incr i;
      if c=34 then add "Quoted"(json_value(slice source start !i))
      else if c=39 then add "Literal"(replace raw "\\'" "'")
      else begin
        let v=replace(trim_start raw)"\\`" "`"in
        let first=if length v=0 then 0 else unit_at v 0 in
        let quoted()=json_value(concat(concat(str "\"")v)(str "\""))in
        let x=if first=91||first=123||first=34||List.mem(text v)["true";"false";"null"]then json_value v
          else if digit first||first=45 then (try parse v with _->quoted())else quoted()in
        add "Literal" x
      end
    end else if c<>32 && c<>9 && c<>10 then begin
      let two=match c,next with 91,63->"Filter"|91,93->"Flatten"|38,38->"And"|124,124->"Or"|33,61->"NE"|61,61->"EQ"|60,61->"LTE"|62,61->"GTE"|_->""in
      let k=if two<>""then begin incr i;two end else match c with
        |60->"LT"|62->"GT"|33->"Not"|38->"Expref"|124->"Pipe"|91->"Lbracket"|93->"Rbracket"|123->"Lbrace"|125->"Rbrace"|40->"Lparen"|41->"Rparen"|42->"Star"|64->"Current"|44->"Comma"|46->"Dot"|58->"Colon"|_->syntax()in
      add k und
    end
  done;add "EOF" und;Array.of_list(List.rev !tokens)
(* jmespath's bindingPower table. It has no entry for Colon and Literal: `rbp < bindingPower[Colon]`
   is false, and so is `bindingPower[Literal] < 10` in _parseProjectionRHS, which therefore rejects a
   literal after a projection. *)
let power=function
  |"Colon"|"Literal"->None
  |"Pipe"->Some 1|"Or"->Some 2|"And"->Some 3|"EQ"|"NE"|"GT"|"LT"|"GTE"|"LTE"->Some 5|"Flatten"->Some 9|"Star"->Some 20
  |"Filter"->Some 21|"Dot"->Some 40|"Not"->Some 45|"Lbrace"->Some 50|"Lbracket"->Some 55|"Lparen"->Some 60|_->Some 0
let binds rbp k=match power k with Some p->rbp<p|None->false
let look p n=if p.pos+n<Array.length p.tokens then p.tokens.(p.pos+n).kind else "EOF"
let next p=if p.pos>=Array.length p.tokens then syntax();let t=p.tokens.(p.pos)in p.pos<-p.pos+1;t
let take p k=if (next p).kind<>k then syntax()
let rec expr p rbp=
  let lhs=ref(nud p(next p))in
  while binds rbp (look p 0)do let k=(next p).kind in lhs:=led p k !lhs done;!lhs
and projection p rbp=
  let k=look p 0 in if (match power k with Some b->b<10|None->false)then node "Identity"else match k with "Lbracket"|"Filter"->expr p rbp|"Dot"->ignore(next p);dot p rbp|_->syntax()
and dot p rbp=match look p 0 with "Field"|"Quoted"|"Star"->expr p rbp|"Lbracket"->ignore(next p);multi p|"Lbrace"->ignore(next p);hash p|_->und
and multi p=
  let acc=ref []in while look p 0<>"Rbracket"do acc:=expr p 0::!acc;
    if look p 0="Comma"then begin ignore(next p);if look p 0="Rbracket"then syntax()end
  done;take p "Rbracket";children "MultiSelectList"(List.rev !acc)
and hash p=
  let rec loop acc=let n=next p in
    if n.kind<>"Field"&&n.kind<>"Quoted"then syntax();take p "Colon";
    let pair=named "KeyValuePair" n.value in set pair "value"(expr p 0);
    if look p 0="Comma"then begin ignore(next p);loop(pair::acc)end
    else if look p 0="Rbrace"then begin ignore(next p);children "MultiSelectHash"(List.rev(pair::acc))end
    else loop(pair::acc)
  in loop []
and bracket p lhs=
  if look p 0="Colon"||look p 1="Colon"then begin let parts=Array.make 3 nil and i=ref 0 in
    while look p 0<>"Rbracket" && !i<3 do let t=next p in
      if t.kind="Colon" then incr i else if t.kind="Number"then parts.(!i)<-t.value else syntax()
    done;take p "Rbracket";let ix=children "IndexExpression"[lhs;children "Slice"(Array.to_list parts)]in
    children "Projection"[ix;projection p 20]
  end else let t=next p in take p "Rbracket";children "IndexExpression"[lhs;leaf "Index" t.value]
and args p commas=
  let acc=ref []in while look p 0<>"Rparen"do
    let v=if look p 0="Current"then begin ignore(next p);node "Current"end else expr p 0 in
    acc:=v::!acc;if commas && look p 0="Comma"then ignore(next p)
  done;take p "Rparen";List.rev !acc
and nud p t=match t.kind with
  |"Literal"->leaf "Literal" t.value|"Quoted" when look p 0="Lparen"->syntax()|"Field"|"Quoted"->named "Field" t.value
  |"Not"->children "NotExpression"[expr p 45]|"Current"->node "Current"|"Expref"->children "ExpressionReference"[expr p 0]
  |"Star"->children "ValueProjection"[node "Identity";projection p 20]|"Filter"|"Flatten"->led p t.kind(node "Identity")
  |"Lbrace"->hash p|"Lbracket"->if look p 0="Colon"||look p 0="Number"then bracket p(node "Identity")
    else if look p 0="Star"&&look p 1="Rbracket"then begin p.pos<-p.pos+2;children "Projection"[node "Identity";projection p 20]end else multi p
  |"Lparen"->(match args p false with h::_->h|[]->und)|_->syntax()
and led p k lhs=match k with
  |"Dot" when look p 0="Star"->ignore(next p);children "ValueProjection"[lhs;projection p 40]
  |"Dot"->children "Subexpression"[lhs;dot p 40]
  |"Or"->children "OrExpression"[lhs;expr p 2]|"And"->children "AndExpression"[lhs;expr p 3]|"Pipe"->children "Pipe"[lhs;expr p 1]
  |"Lparen"->if strict lhs und then syntax();let n=named "Function"(get lhs "name")in set n "children"(list(args p true));n
  |"Filter"->let condition=expr p 0 in take p "Rbracket";let right=if look p 0="Flatten"then node "Identity"else projection p 21 in children "FilterProjection"[lhs;right;condition]
  |"Flatten"->children "Projection"[children "Flatten"[lhs];projection p 9]
  |"EQ"|"NE"|"LT"|"GT"|"LTE"|"GTE"->let n=named "Comparator"(str k)in let rhs=expr p 5 in set n "children"(list[lhs;rhs]);n
  |"Lbracket"->if look p 0="Colon"||look p 0="Number"then bracket p lhs else begin take p "Star";take p "Rbracket";children "Projection"[lhs;projection p 20]end
  |_->syntax()
let compile s=let p={tokens=lex s;pos=0}in let v=expr p 0 in if look p 0<>"EOF"then syntax();v
