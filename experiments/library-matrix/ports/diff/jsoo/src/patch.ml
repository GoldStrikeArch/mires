open Differ
open Host
type hunk={old_start:int;old_lines:int;new_start:int;new_lines:int;lines:string array}
type patch={meta:Host.value;hunks:hunk array}
let starts s prefix=String.starts_with ~prefix s
let ends s suffix=String.ends_with ~suffix s
let getn o k=integer(get o k)
let getstr o k=read(get o k)
let prop o k v=set o k (str v)
let length js=Host.length js
let array_of_js f v=Array.init(length v)(fun i->f(at v i))
let array_to_js f a=let v=array(Array.length a) in Array.iteri(fun i e->put v i (f e))a;v
let update_numbers h x reverse=
  let write k v=set x k(number v)in
  if reverse then begin write "oldLines" h.old_lines;write "oldStart" h.old_start;write "newLines" h.new_lines;write "newStart" h.new_start end
  else begin write "oldStart" h.old_start;write "oldLines" h.old_lines;write "newStart" h.new_start;write "newLines" h.new_lines end
let write reversed p=
  let hunk h=let x=obj() in update_numbers h x reversed;set x "lines"(array_to_js str h.lines);x in
  set p.meta "hunks"(array_to_js hunk p.hunks);p.meta
let read_patch v=
  let meta=obj()in let ks=keys v in for i=0 to length ks-1 do let k=at ks i in key_set meta k(key_get v k)done;
  let h x={old_start=getn x "oldStart";old_lines=getn x "oldLines";new_start=getn x "newStart";new_lines=getn x "newLines";lines=array_of_js read (get x "lines")} in
  {meta;hunks=array_of_js h (get v "hunks")}
let create input context =
  let old=line_tokens(getstr input "before") defaults and newer=line_tokens(getstr input "after")defaults in
  let spans=Array.of_list(myers old newer false)in
  let hunks=ref [] and old_start=ref 0 and new_start=ref 0 and old_lines=ref 0 and new_lines=ref 0 and output=ref [] in
  let emit kind src a n=
    let mark=match kind with Keep->" "|Add->"+"|Remove->"-" in
    for i=a to a+n-1 do let s=src.(i)in
      if ends s "\n" then output:=(mark^cut_r s 1)::!output
      else output:="\\ No newline at end of file"::(mark^s)::!output
    done;if kind<>Add then old_lines:= !old_lines+n;if kind<>Remove then new_lines:= !new_lines+n in
  let flush()=hunks:={old_start= !old_start;new_start= !new_start;old_lines= !old_lines;new_lines= !new_lines;lines=Array.of_list(List.rev !output)}::!hunks;
    old_start:=0;old_lines:=0;new_lines:=0;output:=[] in
  let a=ref 0 and b=ref 0 and kept=ref 0 in
  Array.iteri(fun i r->
    (if r.kind=Keep then begin
      if !old_start<>0 then begin let close=i+1=Array.length spans||r.count>context*2 in
        emit Keep newer !b (if close then min r.count context else r.count);if close then flush()end;
      kept:=r.count
    end else begin
      if !old_start=0 then begin let k=min context !kept in old_start:= !a+1-k;new_start:= !b+1-k;emit Keep newer (!b-k) k end;
      if r.kind=Add then emit Add newer !b r.count else emit Remove old !a r.count
    end);
    if r.kind<>Add then a:= !a+r.count;if r.kind<>Remove then b:= !b+r.count)spans;
  if !old_start<>0 then flush();
  let meta=obj()in List.iter(fun k->set meta k(get input k))["oldFileName";"newFileName";"oldHeader";"newHeader"];
  {meta;hunks=Array.of_list(List.rev !hunks)}

let quote s=
  let s = Host.scalar_text s in
  let needed c=let n=Char.code c in n<32||n>126||n=34||n=92 in
  if not(String.exists needed s)then s else
  let b=Buffer.create(len s+5)in Buffer.add_char b '"';String.iter(fun c->let n=Char.code c in
    if n>=7&&n<=13 then begin Buffer.add_char b '\\';Buffer.add_char b "abtnvfr".[n-7] end
    else if n=34||n=92 then begin Buffer.add_char b '\\';Buffer.add_char b c end
    else if n<32||n>126 then begin Buffer.add_char b '\\';List.iter(fun k->Buffer.add_char b(Char.chr(48+((n lsr k)land 7))))[6;3;0]end
    else Buffer.add_char b c)s;Buffer.add_char b '"';Buffer.contents b
let quoted s=
  if not(starts s "\"")then None else
  let b=Buffer.create(len s) in
  let oct c=c>='0'&&c<='7' in
  let rec octets i bytes =
    if i+1 < len s && s.[i]='\\' && oct s.[i+1] then
      if i+3 >= len s || not (oct s.[i+2] && oct s.[i+3]) then None else begin
        let d n = Char.code s.[i+n] - 48 in
        Buffer.add_char bytes (Char.chr (((d 1 lsl 6) lor (d 2 lsl 3) lor d 3) land 255));
        octets (i+4) bytes
      end
    else begin
      Buffer.add_string b (Host.decode_octets (Buffer.contents bytes));
      scan i
    end
  and scan i =
    if i >= len s then None else match s.[i] with
    | '"' -> Some (Buffer.contents b, i+1)
    | '\\' when i+1 < len s ->
      let c = s.[i+1] in
      if oct c then octets i (Buffer.create 8)
      else if c='"' || c='\\' then (Buffer.add_char b c; scan (i+2))
      else (match String.index_opt "abtnvfr" c with
        | None -> None
        | Some j -> Buffer.add_char b (Char.chr (j+7)); scan (i+2))
    | '\\' -> None
    | c -> Buffer.add_char b c; scan (i+1)
  in scan 1
let unquote s=match quoted s with None->s|Some(n,_)->n
let format p=
  let b=Buffer.create 256 in let push=Buffer.add_string b in let append k=push k;push "\n" in
  let old=get p.meta "oldFileName" and newer=get p.meta "newFileName" in
  if not(absent old||absent newer)&&read old=read newer then append("Index: "^read old);
  append "===================================================================";
  if not(absent old||absent newer)then List.iter(fun(tag,k,h)->let header=get p.meta h in append(tag^quote(getstr p.meta k)^(if truth header then "\t"^read header else "")))
    ["--- ","oldFileName","oldHeader";"+++ ","newFileName","newHeader"];
  Array.iter(fun h->
    let range start n=string_of_int(start-(if n=0 then 1 else 0))^","^string_of_int n in
    append("@@ -"^range h.old_start h.old_lines^" +"^range h.new_start h.new_lines^" @@");Array.iter append h.lines)p.hunks;
  Buffer.contents b
let git_names p s=
  let names a b=prop p "oldFileName" a;prop p "newFileName" b in
  match quoted s with Some(a,n)->let b=trim(cut_l s n)in if b<>"" then names a(unquote b)
  |None->match String.index_opt s '"' with Some j when j>0->(match quoted(cut_l s j)with Some(b,_)->names(trim(sub s 0 j))b|_->())
  |_->if starts s "a/"then let splits=ref []in for i=0 to len s-3 do if sub s i(i+3)=" b/"then splits:=i::!splits done;
    let a=Array.of_list(List.rev !splits)in if Array.length a>0 then let i=a.(Array.length a/2)in names(sub s 0 i)(cut_l s(i+1))
let extended p l=
  List.iter(fun(tag,name,flag,part)->if starts l tag then begin prop p name(part^unquote(cut_l l(len tag)));set p flag(boolean true)end)
    ["rename from ","oldFileName","isRename","a/";"rename to ","newFileName","isRename","b/";"copy from ","oldFileName","isCopy","a/";"copy to ","newFileName","isCopy","b/"];
  List.iter(fun(tag,flag,name)->if starts l tag then begin if flag<>"" then set p flag(boolean true);prop p name(cut_l l(len tag))end)
    ["new file mode ","isCreate","newMode";"deleted file mode ","isDelete","oldMode";"old mode ","","oldMode";"new mode ","","newMode"];
  if starts l "Binary files "then set p "isBinary"(boolean true)
let file l=len l>3&&(starts l "---"||starts l "+++")&&(l.[3]=' '||l.[3]='\t')
let header l=starts l "diff --git "||starts l "Index: "||starts l "diff -r "
let line_mark l=if l="" then ' 'else l.[0]
let parse_file p l=
  let parts=String.split_on_char '\t'(trim(cut_l l 3))in let s=List.hd parts in
  let name=if starts s "\""then unquote s else let b=Buffer.create(len s)and i=ref 0 in
    while !i<len s do Buffer.add_char b s.[!i];if s.[!i]='\\' && !i+1<len s && s.[!i+1]='\\'then incr i;incr i done;Buffer.contents b in
  let side=if l.[0]='-'then "old"else "new" in prop p(side^"FileName")name;
  prop p(side^"Header")(match parts with _::h::_->trim h|_->"")
let parse_hunk body i=
  let l=body.(!i)in incr i;
  let range s=match String.split_on_char ','(cut_l s 1)with [a]->int_of_string a,1|[a;b]->let n=int_of_string b in int_of_string a+(if n=0 then 1 else 0),n|_->invalid_arg "hunk range" in
  let old_start,o,new_start,n=match String.split_on_char ' ' l with "@@"::a::b::"@@"::_ when starts a "-"&&starts b "+"->let x,y=range a and z,t=range b in x,y,z,t|_->invalid_arg "hunk header"in
  let old=ref 0 and newer=ref 0 and lines=ref [] in
  while !i<Array.length body&&(!old<o|| !newer<n||starts body.(!i)"\\")do let s=body.(!i)in
    if s=""&& !i+1=Array.length body then invalid_arg "line";
    (match line_mark s with '+'->incr newer|'-'->incr old|' '->incr old;incr newer|'\\'->()|_->invalid_arg "line");
    lines:=s::!lines;incr i done;
  let old_lines=if !old=0&&o=1 then 0 else o and new_lines=if !newer=0&&n=1 then 0 else n in
  if old_lines<> !old||new_lines<> !newer then invalid_arg "line count";
  if !i<Array.length body&&body.(!i)<>""&&String.contains "+ -" (line_mark body.(!i))&&not(file body.(!i))then invalid_arg "extra line";
  {old_start;old_lines;new_start;new_lines;lines=Array.of_list(List.rev !lines)}
let parse s=
  let body=Array.of_list(String.split_on_char '\n' s)and out=ref []and i=ref 0 in
  while !i<Array.length body do
    let meta=obj()and hunks=ref []and seen=ref false in set meta "hunks" undef;
    while !i<Array.length body && not(file body.(!i)||starts body.(!i)"@@ "|| !seen&&header body.(!i)) do
      let l=body.(!i)in
      (if header l then begin seen:=true;if starts l "diff --git "then begin set meta "isGit"(boolean true);git_names meta(cut_l l 11)end
        else if starts l "Index: "then prop meta "index"(trim(cut_l l 7))else prop meta "index"(cut_l l(1+String.rindex l ' '))end
      else if truth(get meta "isGit") then extended meta l);incr i
    done;
    for _=0 to 1 do if !i<Array.length body&&file body.(!i)then begin parse_file meta body.(!i);incr i end done;
    if absent(get meta "oldFileName")<>absent(get meta "newFileName")then invalid_arg "file header";
    while !i<Array.length body && not(header body.(!i)||file body.(!i)||starts body.(!i)"===================================================================")do
      if starts body.(!i)"@@ "then hunks:=parse_hunk body i::!hunks else incr i
    done;out:={meta;hunks=Array.of_list(List.rev !hunks)}::!out
  done;Array.of_list(List.rev !out)
let reverse v=
  let p=read_patch v in let copy=truth(get v "isCopy")in let meta=obj()in
  let ks=keys v in for i=0 to length ks-1 do let k=at ks i in let name=read k in
    if not(copy&&List.mem name ["isCreate";"isCopy";"isRename"])then key_set meta k(key_get v k)done;
  List.iter(fun(k,other)->let a=get v other in
    if String.ends_with ~suffix:"FileName" k && truth(get v "isGit")&&not(absent a)then let n=read a in
      prop meta k(if starts n "a/"then "b/"^cut_l n 2 else if starts n "b/"then "a/"^cut_l n 2 else n)
    else set meta k a)
    ["oldFileName","newFileName";"oldHeader","newHeader";"newFileName","oldFileName";"newHeader","oldHeader";"oldMode","newMode";"newMode","oldMode"];
  if not copy then set meta "isCreate"(get v "isDelete");set meta "isDelete"(if copy then boolean true else get v "isCreate");
  let hunks=if copy then begin prop meta "newFileName" "/dev/null";set meta "newHeader" undef;[||]end else Array.map(fun h->
    {old_lines=h.new_lines;old_start=h.new_start;new_lines=h.old_lines;new_start=h.old_start;lines=Array.map(fun l->
        if starts l "+"then "-"^cut_l l 1 else if starts l "-"then "+"^cut_l l 1 else l)h.lines})p.hunks in
  write true {meta;hunks}
let convert source hunks=
  let win=ref false and unix=ref false in String.iteri(fun i c->if c='\n' then if i>0&&source.[i-1]='\r'then win:=true else unix:=true)source;
  if !win<> !unix then begin let pw=ref false and pu=ref false in
    let marker h i=i+1<Array.length h.lines&&starts h.lines.(i+1)"\\" in
    Array.iter(fun h->Array.iteri(fun i l->if not(starts l "\\")then if ends l "\r"then pw:=true else if not(marker h i)then pu:=true)h.lines)hunks;
    Array.iter(fun h->Array.iteri(fun i l->
      if !win && not !pw && not(starts l "\\"||marker h i)then h.lines.(i)<-l^"\r"
      else if !unix && !pw && not !pu && ends l "\r"then h.lines.(i)<-cut_r l 1)h.lines)hunks
  end
let apply source p=
  let exception Mismatch in
  if Array.length p.hunks=0 then str source else try
    convert source p.hunks;
    let data=Array.of_list(String.split_on_char '\n' source)in
    let ae=ref false and re=ref false and prev=ref ' ' in
    Array.iter(fun l->if starts l "\\"then begin if !prev='-'then ae:=true;if !prev='+'then re:=true end;prev:=line_mark l)p.hunks.(Array.length p.hunks-1).lines;
    let eof=data.(Array.length data-1)=""in
    let data=match !re,!ae with true,true when eof->raise Mismatch|true,false->if not eof then raise Mismatch;Array.sub data 0(Array.length data-1)
      |false,true->if eof then raise Mismatch;Array.append data [|""|]|_->data in
    let min_line=ref 0 and offset=ref 0 and result=ref []in
    Array.iter(fun h->
      let max_line=Array.length data-h.old_lines in if max_line< !min_line then raise Mismatch;
      let first=max !min_line(min max_line(h.old_start+ !offset-1))in
      let fits at=let i=ref at and ok=ref true in Array.iter(fun l->
        if !ok&&(line_mark l='-'||line_mark l=' ')then begin
          if !i>=Array.length data||data.(!i)<>(if l=""then ""else cut_l l 1)then ok:=false;incr i end)h.lines; !ok in
      let d=ref 0 and pos=ref (-1)in
      while !pos<0 && !d<=max(first- !min_line)(max_line-first)do
        if first+ !d<=max_line&&fits(first+ !d)then pos:=first+ !d
        else if first- !d>= !min_line && !d>0 && fits(first- !d)then pos:=first- !d;incr d
      done;
      if !pos<0 then raise Mismatch;for i= !min_line to !pos-1 do result:=data.(i)::!result done;
      let cur=ref !pos and ctx=ref 0 in Array.iter(fun l->match line_mark l with
        |' '->result:=data.(!cur)::!result;incr ctx;incr cur|'-'->incr cur;ctx:=0|'+'->result:=cut_l l 1::!result;ctx:=0|_->())h.lines;
      for _=1 to !ctx do result:=List.tl !result done;min_line:= !cur- !ctx;offset:= !pos+1-h.old_start)p.hunks;
    for i= !min_line to Array.length data-1 do result:=data.(i)::!result done;
    str(String.concat "\n"(List.rev !result))
  with Mismatch->boolean false
