open Geometry

type datum = { name : string; weight : float; parent_name : string; children : datum array }
type input = { op:string; source:string; data:datum option; rows:datum array; width:float; height:float;
               padding:float; sort:bool; node_size:bool; round:bool; tile:string }
type cell = Text of string | Num of float | Null

let node_of d =
  let n = circle 0. 0. 0. in n.id <- d.name; n.value <- d.weight; n

let walk nodes root post =
  let stack=Stack.create () and order=ref [] in
  Stack.push root stack;
  while not (Stack.is_empty stack) do
    let v=Stack.pop stack in order:=v :: !order;
    let kids=nodes.(v).kids in
    if post then Array.iter (fun c -> Stack.push c stack) kids
    else for i=Array.length kids-1 downto 0 do Stack.push kids.(i) stack done
  done;
  Array.of_list (if post then !order else List.rev !order)

let nested datum =
  let next=ref 0 and nodes=ref [] in
  let rec read parent d =
    let i= !next in incr next;
    let n=node_of d in n.parent<-parent; nodes:=n :: !nodes;
    n.kids <- Array.map (read i) d.children; i
  in ignore(read (-1) datum); Array.of_list (List.rev !nodes),0

let slash s i =
  if i<0 || i>=String.length s || s.[i]<>'/' then false else
  let k=ref i in while !k>0 && s.[!k-1]='\\' do decr k done;
  (i- !k) mod 2=0
let normalize s =
  let n=String.length s in
  let s=if slash s (n-1) && not(slash s (n-2)) then String.sub s 0 (n-1) else s in
  if s<>"" && s.[0]='/' then s else "/" ^ s
let parent_path s =
  let len=String.length s in if len<2 then "" else
  let i=ref (len-1) in while !i>1 && not(slash s !i) do decr i done;String.sub s 0 !i

let stratify rows paths =
  let count=Array.length rows in
  let by_id=Hashtbl.create count and parents=Queue.create () and entries=Queue.create () in
  Array.iteri (fun i d ->
    let n=node_of d in
    if paths then n.id <- normalize n.id;
    if n.id<>"" then Hashtbl.replace by_id n.id (if Hashtbl.mem by_id n.id then -1 else i);
    Queue.add (if paths then parent_path n.id else d.parent_name) parents;Queue.add n entries
  ) rows;
  let ready=Queue.create () in
  while not (Queue.is_empty parents) do
    let p=Queue.take parents in
    if paths && p<>"" && not(Hashtbl.mem by_id p) then begin
      Hashtbl.add by_id p (Queue.length entries);let n=circle 0. 0. 0. in n.id<-p;
      Queue.add n entries;Queue.add (parent_path p) parents
    end;
    Queue.add p ready
  done;
  let nodes=Array.init (Queue.length entries) (fun _ -> Queue.take entries) in
  let kids=Array.make (Array.length nodes) [] and root=ref (-1) in
  Array.iteri (fun i n ->
    let parent=Queue.take ready in
    if parent="" then (if !root>=0 then invalid_arg "multiple roots";root:=i)
    else (let p=Option.value ~default:(-1) (Hashtbl.find_opt by_id parent) in
      if p<0 then invalid_arg "missing or ambiguous parent";n.parent<-p;kids.(p)<-i::kids.(p))
  ) nodes;
  if !root<0 then invalid_arg "no root";
  Array.iteri (fun i n -> n.kids<-Array.of_list(List.rev kids.(i))) nodes;
  let expected=ref (Array.length nodes) in
  while paths && !root>=count && Array.length nodes.(!root).kids=1 do root:=nodes.(!root).kids.(0);decr expected done;
  nodes.(!root).parent <- -1;
  if Array.length (walk nodes !root false) <> !expected then invalid_arg "cycle";
  nodes,!root

let run req =
  let nodes,root = match req.data with Some data -> nested data | None -> stratify req.rows (req.source="path") in
  Array.iter (fun v ->
    let n=nodes.(v) in
    if n.value = 0. then n.value <- 0.;
    for j=Array.length n.kids-1 downto 0 do
      let c=nodes.(n.kids.(j)) in n.value<-n.value+.c.value;n.height<-max n.height (c.height+1)
    done;
    if req.sort then Array.stable_sort (fun a b -> Float.compare nodes.(b).value nodes.(a).value) n.kids
  ) (walk nodes root true);
  let pre=walk nodes root false and post=walk nodes root true in
  Array.iter (fun v -> let n=nodes.(v) in if n.parent>=0 then n.depth<-nodes.(n.parent).depth+1) pre;
  let op=req.op in
  (match op with
  | "cluster" -> cluster nodes pre post req.width req.height req.node_size
  | "tree" -> tidy nodes pre post req.width req.height req.node_size
  | "pack" -> pack nodes pre post req.width req.height req.padding
  | "partition"|"treemap" -> rectangles nodes pre req.width req.height req.padding (op="partition") req.tile req.round
  | "iterator"|"stratify"|"hierarchy" -> () | _ -> invalid_arg "operation");
  if op="iterator" then begin
    let j=ref 1 in pre.(0)<-root;
    for i=0 to Array.length pre-1 do
      Array.iter (fun c -> pre.(!j)<-c;incr j) nodes.(pre.(i)).kids
    done
  end;
  Array.map (fun v ->
    let n=nodes.(v) in
    let coords=match op with "tree"|"cluster"->[n.x;n.y] | "pack"->[n.x;n.y;n.r]
      | "treemap"|"partition"->[n.x0;n.y0;n.x1;n.y1] | _->[] in
    Array.of_list (Text n.id :: (if n.parent<0 then Null else Text nodes.(n.parent).id)
      :: Num(float_of_int n.depth) :: Num(float_of_int n.height) :: Num n.value :: List.map (fun x->Num x) coords)
  ) pre
