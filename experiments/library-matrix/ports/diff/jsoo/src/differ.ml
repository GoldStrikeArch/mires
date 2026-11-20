(* Myers' furthest-reaching paths and the five jsdiff token policies. *)
type kind = Keep | Remove | Add
type span = { kind : kind; count : int }
type options = { ignore_case : bool; individual : bool; strip_cr : bool; ignore_ws : bool; newline_token : bool; ignore_eof : bool }
let defaults = {ignore_case=false; individual=false; strip_cr=false; ignore_ws=false; newline_token=false; ignore_eof=false}
type path = { old : int; spans : span list }

(* base.js: Myers' search over the furthest-reaching path on each diagonal, with jsdiff's choice
   of predecessor and its pruning at the edges of the edit graph. [best] holds the path on each
   diagonal, offset by [base]. *)
let myers old newer individual =
  let limit = Array.length old + Array.length newer in
  let base = limit + 1 in
  let best = Array.make ((base * 2) + 1) None in
  let add k n previous =
    if n = 0 then previous
    else if individual then begin
      let spans = ref previous in
      for _ = 1 to n do
        spans := { kind = k; count = 1 } :: !spans
      done;
      !spans
    end
    else
      match previous with
      | h :: t when h.kind = k -> { kind = k; count = h.count + n } :: t
      | _ -> { kind = k; count = n } :: previous
  in
  let common k p =
    let i = ref p.old and j = ref (p.old - k) in
    while !i + 1 < Array.length old && !j + 1 < Array.length newer && old.(!i + 1) = newer.(!j + 1) do
      incr i;
      incr j
    done;
    ({ old = !i; spans = add Keep (!i - p.old) p.spans }, !j)
  in
  let finished p j = p.old + 1 = Array.length old && j + 1 = Array.length newer in
  let first, j = common 0 { old = -1; spans = [] } in
  if finished first j then List.rev first.spans
  else begin
    best.(base) <- Some first;
    (* Once a path reaches the right or bottom edge, the diagonals beyond it are pointless. *)
    let low = ref (-limit) and high = ref limit in
    (* One edit length: the finished history, if a path reaches the end. *)
    let rec diagonals k d =
      if k > min !high d then None
      else begin
        let i = base + k in
        let remove = best.(i - 1) and insert = best.(i + 1) in
        best.(i - 1) <- None;
        let can_remove = match remove with Some r -> r.old + 1 < Array.length old | None -> false in
        let can_add = match insert with Some a -> a.old - k >= 0 && a.old - k < Array.length newer | None -> false in
        let next =
          match (insert, remove) with
          | Some a, Some r when can_add && ((not can_remove) || r.old < a.old) -> Some { a with spans = add Add 1 a.spans }
          | Some a, _ when can_add && not can_remove -> Some { a with spans = add Add 1 a.spans }
          | _, Some r when can_remove -> Some { old = r.old + 1; spans = add Remove 1 r.spans }
          | _ -> None
        in
        match next with
        | None ->
            best.(i) <- None;
            diagonals (k + 2) d
        | Some p ->
            let p, j = common k p in
            if finished p j then Some (List.rev p.spans)
            else begin
              best.(i) <- Some p;
              if p.old + 1 = Array.length old then high := min !high (k - 1);
              if j + 1 = Array.length newer then low := max !low (k + 1);
              diagonals (k + 2) d
            end
      end
    in
    let rec rounds d =
      if d > limit then []
      else match diagonals (max !low (-d)) d with Some spans -> spans | None -> rounds (d + 1)
    in
    rounds 1
  end

let sub s i j=String.sub s i (j-i)
let len=String.length
let cp = Host.cp
let space c = (9<=c&&c<=13)||c=32||c=160||c=0x1680||(0x2000<=c&&c<=0x200a)||c=0x2028||c=0x2029||c=0x202f||c=0x205f||c=0x3000||c=0xfeff
let word c = (65<=c&&c<=90)||(97<=c&&c<=122)||(48<=c&&c<=57)||c=95||c=0xad||(0xc0<=c&&c<=0xd6)||(0xd8<=c&&c<=0xf6)||(0xf8<=c&&c<=0x2c6)||(0x2c8<=c&&c<=0x2d7)||(0x2de<=c&&c<=0x2ff)||(0x1e00<=c&&c<=0x1eff)
let left_ws s =
  let rec loop i=if i=len s then i else let c,n=cp s i in if space c then loop(i+n) else i in loop 0
let right_ws s =
  let last=ref 0 and i=ref 0 in while !i<len s do let c,n=cp s !i in i:= !i+n;if not(space c)then last:= !i done;
  len s- !last
let cut_l s n=sub s n (len s)
let cut_r s n=sub s 0 (len s-n)
let left s=sub s 0 (left_ws s)
let right s=cut_l s (len s-right_ws s)
let trim s=let s=cut_l s (left_ws s) in cut_r s(right_ws s)
let words s =
  let parts=ref [] and start=ref 0 and prior=ref (-1) and i=ref 0 in
  while !i<len s do let c,n=cp s !i in let cls=if space c then 0 else if word c then 1 else 2 in
    if cls<> !prior || cls=2 then begin if !i> !start then parts:=sub s !start !i :: !parts;start:= !i end;
    prior:=cls;i:= !i+n
  done;
  if !start<len s then parts:=cut_l s !start :: !parts;
  let out=ref [] and prev=ref "" in
  List.iter(fun part -> let c,_=cp part 0 in if space c then begin
    (match !out with []->out:=[part]|h::t->out:=(h^part)::t);prev:=part
  end else begin
    (match !out with [h] when !prev<>"" && h= !prev -> out:=[h^part]|_ -> out:=(!prev^part)::!out);prev:=""
  end)(List.rev !parts); Array.of_list(List.rev !out)
let line_tokens s o =
  let out=ref [] and start=ref 0 in
  for i=0 to len s-1 do if s.[i]='\n' then begin let e=if i> !start&&s.[i-1]='\r' then i-1 else i in
    if o.newline_token then begin if e> !start then out:=sub s !start e::!out;out:=(if o.strip_cr then "\n" else sub s e (i+1))::!out end
    else out:=(if o.strip_cr && e<i then sub s !start e^"\n" else sub s !start (i+1))::!out;
    start:=i+1
  end done;if !start<len s then out:=cut_l s !start::!out;Array.of_list(List.rev !out)
let tokenize op s o=
  if op="diffWords" then words s else if op="diffLines" || op="diffJson" then line_tokens s o
  else let out=ref [] and i=ref 0 in while !i<len s do let _,n=cp s !i in out:=sub s !i (!i+n)::!out;i:= !i+n done;Array.of_list(List.rev !out)
let key op o raw =
  let s=if op="diffWords" || op="diffLines"&&o.ignore_ws&&(not o.newline_token||not(String.contains raw '\n')) then trim raw
  else if op="diffLines"&&o.ignore_eof&&not o.newline_token&&String.ends_with ~suffix:"\n" raw then cut_r raw 1 else raw in
  let s=if op="diffJson" then begin let b=Buffer.create(len s) in for i=0 to len s-1 do
    if s.[i]<>','||i+1=len s||(s.[i+1]<>'\n'&&s.[i+1]<>'\r') then Buffer.add_char b s.[i] done; Buffer.contents b end else s in
  if o.ignore_case then Host.lower s else s
type change = {span:span; mutable value:string}
let common_len a b suffix =
  let i=ref 0 and end_=min(len a)(len b) in
  while !i<end_ && (if suffix then a.[len a-1- !i]=b.[len b-1- !i] else a.[!i]=b.[!i]) do incr i done;
  if suffix then begin while !i>0 && !i<len a && Char.code a.[len a- !i] land 192=128 do decr i done end
  else begin while !i>0 && !i<len a && Char.code a.[!i] land 192=128 do decr i done end; !i
let overlap a b=
  let rec loop i=if i=len a then 0 else if String.starts_with ~prefix:(cut_l a i) b then len a-i else let _,n=cp a i in loop(i+n) in loop 0
let clean changes =
  let start=ref None and del=ref None and ins=ref None in
  let adjust ending= match !del,!ins with
  |Some d,Some i->
    let op=left d.value and np=left i.value and os=right d.value and ns=right i.value in
    Option.iter(fun s->let n=common_len op np false in s.value<-cut_r s.value(len np)^sub np 0 n;d.value<-cut_l d.value n;i.value<-cut_l i.value n)!start;
    Option.iter(fun e->let n=common_len os ns true in e.value<-cut_l ns(len ns-n)^cut_l e.value(len ns);d.value<-cut_r d.value n;i.value<-cut_r i.value n)ending
  |None,Some i->if Option.is_some !start then i.value<-cut_l i.value(left_ws i.value);Option.iter(fun e->e.value<-cut_l e.value(left_ws e.value))ending
  |Some d,None-> (match !start,ending with
    |Some s,Some e->let w=left e.value in let a=common_len w (left d.value) false in let b=common_len(cut_l w a)(right d.value)true in
      d.value<-cut_r(cut_l d.value a)b;e.value<-cut_l e.value(len w-b);s.value<-cut_r s.value b
    |Some s,None->d.value<-cut_l d.value(overlap(right s.value)(left d.value))
    |None,Some e->d.value<-cut_r d.value(overlap(right d.value)(left e.value))|_->())
  |_->() in
  Array.iter(fun c->match c.span.kind with Add->ins:=Some c|Remove->del:=Some c|Keep->adjust(Some c);start:=Some c;del:=None;ins:=None)changes;adjust None
(* JavaScript's length: the UTF-16 units of a WTF-8 string. *)
let units s =
  let n = ref 0 in
  String.iter (fun c -> let b = Char.code c in if b land 0xc0 <> 0x80 then incr n; if b >= 0xf0 then incr n) s;
  !n
let changes op old newer o =
  let spans=myers(Array.map(key op o)old)(Array.map(key op o)newer)o.individual in
  let a=ref 0 and b=ref 0 in
  let out=Array.of_list(List.map(fun span->
    let buf=Buffer.create 24 in for j=0 to span.count-1 do
      let s=if span.kind=Remove || op="diffJson" && span.kind=Keep && units old.(!a+j)>units newer.(!b+j) then old.(!a+j) else newer.(!b+j) in
      Buffer.add_string buf (if op="diffWords"&&j>0 then cut_l s(left_ws s)else s)
    done;
    if span.kind<>Add then a:= !a+span.count;if span.kind<>Remove then b:= !b+span.count;
    {span;value=Buffer.contents buf})spans) in
  if op="diffWords" && not o.individual then clean out;out
