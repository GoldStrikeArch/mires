(* Convert between native JS strings and UTF-8 byte strings in the shared OCaml core. *)
type value
external box : 'a -> value = "%identity"
external cast : value -> 'a = "%identity"
external obj : unit -> value = "Object" [@@mel.new]
external null_object : value -> value = "create" [@@mel.scope "Object"]
external array : int -> value = "Array" [@@mel.new]
external get : value -> string -> value = "" [@@mel.get_index]
external set : value -> string -> value -> unit = "" [@@mel.set_index]
external at : value -> int -> value = "" [@@mel.get_index]
external put : value -> int -> value -> unit = "" [@@mel.set_index]
external length : value -> int = "length" [@@mel.get]
external keys : value -> value = "keys" [@@mel.scope "Object"]
external sorted : value -> value = "sort" [@@mel.send]
external is_array : value -> bool = "isArray" [@@mel.scope "Array"]
external js_lower : value -> value = "toLowerCase" [@@mel.send]
external json : value -> value = "stringify" [@@mel.scope "JSON"]
external pretty : value -> value -> int -> value = "stringify" [@@mel.scope "JSON"]
external code_point_at : value -> int -> int = "codePointAt" [@@mel.send]
external from_code_point : int -> value = "fromCodePoint" [@@mel.scope "String"]
external join : value -> string -> value = "join" [@@mel.send]
external push_part : value -> value -> unit = "push" [@@mel.send]
let join_parts a = join a ""
let ascii_read v = cast v
let ascii_str s = box s
external truth : value -> bool = "Boolean"
external typeof : value -> string = "#typeof"
let undef=box Js.undefined
let null=box Js.null
let absent v=Js.Nullable.isNullable (cast v)
let boolean b=box b
let number n=box n
let integer v : int=cast v
let float_value v : float=cast v
external delete_property : value -> string -> bool = "deleteProperty" [@@mel.scope "Reflect"]
let delete o k=ignore (delete_property o k)
let is_string v=typeof v="string"
let object_value v=typeof v="object" && not(absent v)
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
let lower s=read(js_lower(str s))
let key_get o k=get o (cast k)
let key_set o k v=set o(cast k)v

external to_well_formed : value -> value = "toWellFormed" [@@mel.send]
external byte_array : int -> value = "Uint8Array" [@@mel.new]
external decoder : unit -> value = "TextDecoder" [@@mel.new]
external decode_bytes : value -> value -> value = "decode" [@@mel.send]
let scalar_text s = read (to_well_formed (str s))
let decode_octets s =
  let bytes = byte_array (String.length s) in
  String.iteri (fun i c -> put bytes i (number (Char.code c))) s;
  read (decode_bytes (decoder ()) bytes)
