open Js_of_ocaml
type value=Js.Unsafe.any
let box=Js.Unsafe.inject
let undef=box Js.undefined
let null=box Js.null
let obj ()=box(Js.Unsafe.obj [||])
let global k=Js.Unsafe.get Js.Unsafe.global k
let call v k a=Js.Unsafe.meth_call v k a
let null_object proto=call(global "Object")"create"[|proto|]
let array n=box(Js.Unsafe.new_obj(global "Array")[|box n|])
let get o k=Js.Unsafe.get o k
let set o k v=Js.Unsafe.set o k v
let at o i=Js.Unsafe.get o i
let put o i v=Js.Unsafe.set o i v
let length o :int=Js.Unsafe.get o "length"
let keys o=call(global "Object")"keys"[|o|]
let sorted a=call a "sort"[||]
let is_array o=Js.to_bool(call(global "Array")"isArray"[|o|])
let truth o=Js.to_bool(Js.Unsafe.fun_call(global "Boolean")[|o|])
let json v=call(global "JSON")"stringify"[|v|]
let pretty v r n=call(global "JSON")"stringify"[|v;r;box n|]
let code_point_at v i : int = call v "codePointAt" [|box i|]
let from_code_point cp = call (global "String") "fromCodePoint" [|box cp|]
let join_parts a = call a "join" [|box (Js.string "")|]
let push_part a v : unit = ignore (call a "push" [|v|])
let ascii_read v = Js.to_string (Js.Unsafe.coerce v)
let ascii_str s = box (Js.string s)
let cp s i =
  let byte n=Char.code s.[i+n] in
  let a=byte 0 in
  if a < 128 then a,1 else if a<224 then ((a land 31) lsl 6) lor (byte 1 land 63),2
  else if a<240 then ((a land 15) lsl 12) lor ((byte 1 land 63) lsl 6) lor (byte 2 land 63),3
  else ((a land 7) lsl 18) lor ((byte 1 land 63) lsl 12) lor ((byte 2 land 63) lsl 6) lor (byte 3 land 63),4
(* WTF-8 differs from UTF-8 only on isolated UTF-16 surrogates. *)
let read v =
  let n = length v in
  let rec ascii i = i = n || code_point_at v i < 128 && ascii (i + 1) in
  if ascii 0 then ascii_read v else
  let bytes = Buffer.create n in
  let byte c = Buffer.add_char bytes (Char.chr c) in
  let cont c = byte (128 lor (c land 63)) in
  let i = ref 0 in
  while !i < n do
    let c = code_point_at v !i in
    if c < 128 then byte c
    else if c < 0x800 then (byte (192 lor (c lsr 6)); cont c)
    else if c < 0x10000 then (byte (224 lor (c lsr 12)); cont (c lsr 6); cont c)
    else (byte (240 lor (c lsr 18)); cont (c lsr 12); cont (c lsr 6); cont c);
    i := !i + (if c > 0xffff then 2 else 1)
  done;
  Buffer.contents bytes

let str s =
  let rec ascii i = i = String.length s || Char.code s.[i] < 128 && ascii (i + 1) in
  if ascii 0 then ascii_str s else
  let parts = array 0 and i = ref 0 in
  while !i < String.length s do
    let c, n = cp s !i in
    push_part parts (from_code_point c);
    i := !i + n
  done;
  join_parts parts
let lower s=read(call(str s)"toLowerCase"[||])
let absent v=Js.Unsafe.equals v Js.null
let boolean b=box(Js.bool b)
let number n=box n
let integer (v:value) :int=Js.Unsafe.coerce v |> Obj.magic
let float_value (v:value) :float=Js.float_of_number (Js.Unsafe.coerce v)
let delete o (k:string)=ignore (call(global "Reflect")"deleteProperty"[|o;box(Js.string k)|])
let is_string v=Js.to_string(Js.typeof v)="string"
let object_value v=Js.to_string(Js.typeof v)="object"&&not(absent v)
let key_get o k=Js.Unsafe.get o k
let key_set o k v=Js.Unsafe.set o k v

let scalar_text s = read (call (str s) "toWellFormed" [||])

let slice s i j = String.sub s i (j - i)
let split_lines s = Array.of_list (String.split_on_char '\n' s)
let decode_octets s =
  let bytes = box (Js.Unsafe.new_obj (global "Uint8Array") [|box (String.length s)|]) in
  String.iteri (fun i c -> put bytes i (box (Char.code c))) s;
  let decoder = box (Js.Unsafe.new_obj (global "TextDecoder") [||]) in
  read (call decoder "decode" [|bytes|])
