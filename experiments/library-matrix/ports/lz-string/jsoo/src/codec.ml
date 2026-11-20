(* Phrases are big-endian byte strings with exactly two bytes per UTF-16 unit. *)
let symbol n = String.init 2 (fun i -> Char.chr (if i = 0 then n lsr 8 else n land 255))
let unit_at s i = (Char.code s.[i] lsl 8) lor Char.code s.[i+1]
let text data = String.concat "" (Array.to_list (Array.map symbol data))
let units s = Array.init (String.length s / 2) (fun i -> unit_at s (2*i))
let ascii s = text (Array.init (String.length s) (fun i -> Char.code s.[i]))
let base64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/="
let uri = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-$"
let width = function
  | "raw" | "bytes" -> 16 | "utf16" -> 15 | "base64" | "uri" -> 6
  | _ -> invalid_arg "unknown format"
type width = { mutable bits : int; mutable left : int }
let advance w =
  w.left <- w.left - 1;
  if w.left = 0 then (w.left <- 1 lsl w.bits; w.bits <- w.bits + 1)

let compress s format =
  let dictionary = Hashtbl.create 256 and pending = Hashtbl.create 256 in
  let size = width format and codes = { bits = 2; left = 2 } in
  let word = ref 0 and used = ref 0 and next = ref 3 and words = ref [] in
  let write n value =
    for bit = 0 to n - 1 do
      word := (!word lsl 1) lor ((value lsr bit) land 1);
      incr used;
      if !used = size then (words := !word :: !words; used := 0; word := 0)
    done
  in
  let emit phrase =
    if Hashtbl.mem pending phrase then (
      Hashtbl.remove pending phrase;
      let n = unit_at phrase 0 in
      write codes.bits (if n < 256 then 0 else 1);
      write (if n < 256 then 8 else 16) n;
      advance codes
    ) else write codes.bits (Hashtbl.find dictionary phrase);
    advance codes
  in
  let prefix = ref "" and special = ascii "__proto__" in
  for i = 0 to String.length s / 2 - 1 do
    (* Melange String.sub converts the whole input to bytes before slicing. *)
    let c = symbol (unit_at s (2*i)) in
    if not (Hashtbl.mem dictionary c) then (
      Hashtbl.add dictionary c !next; incr next; Hashtbl.add pending c ());
    let phrase = !prefix ^ c in
    if Hashtbl.mem dictionary phrase then prefix := phrase else (
      emit !prefix;
      if phrase <> special then Hashtbl.add dictionary phrase !next;
      incr next;
      prefix := c)
  done;
  if !prefix <> "" then emit !prefix;
  write codes.bits 2;
  write (size - !used) 0;
  let words = Array.of_list (List.rev !words) in
  match format with
  | "raw" -> words
  | "bytes" -> Array.init (Array.length words * 2) (fun i ->
      if i mod 2 = 0 then words.(i/2) lsr 8 else words.(i/2) land 255)
  | "utf16" -> Array.append (Array.map ((+) 32) words) [|32|]
  | _ ->
    let alphabet = if format = "base64" then base64 else uri in
    let data = Array.map (fun n -> Char.code alphabet.[n]) words in
    let pad = if format = "base64" then (4 - Array.length words mod 4) mod 4 else 0 in
    Array.append data (Array.make pad 61)

let decompress data format =
  if Array.length data = 0 then None else
  let size = width format in
  let words = match format with
    | "raw" -> data
    | "utf16" -> Array.map (fun n -> n - 32) data
    | "bytes" ->
      if Array.length data mod 2 <> 0 then invalid_arg "odd byte length";
      Array.init (Array.length data / 2) (fun i -> data.(2*i)*256 + data.(2*i+1))
    | _ ->
      let alphabet = if format = "uri" then uri else base64 in
      Array.map (fun n ->
        let n = if format = "uri" && n = 32 then 43 else n in
        if n > 255 then 0 else Option.value ~default:0 (String.index_opt alphabet (Char.chr n))) data
  in
  let index = ref 0 and position = ref size in
  let read n =
    let v = ref 0 in
    for bit = 0 to n - 1 do
      let w = if !index < Array.length words then words.(!index) else 0 in
      decr position;
      v := !v lor (((w lsr !position) land 1) lsl bit);
      if !position = 0 then (position := size; incr index)
    done; !v
  in
  match read 2 with
  | 2 -> Some ""
  | head ->
    let initial = if head = 3 then "" else symbol (read (if head = 0 then 8 else 16)) in
    let dictionary = Hashtbl.create 256 and next = ref 4 in
    Hashtbl.add dictionary 3 initial;
    let result = Buffer.create 128 and codes = { bits = 3; left = 4 } in
    Buffer.add_string result initial;
    let rec loop previous =
      if !index >= Array.length words then Some "" else
      let code = read codes.bits in
      if code = 2 then Some (Buffer.contents result) else
      let code = if code <= 1 then (
        let literal = symbol (read (if code = 0 then 8 else 16)) in
        let k = !next in Hashtbl.add dictionary k literal; incr next; advance codes; k
      ) else code in
      let entry = if code = !next then previous ^ symbol (unit_at previous 0)
        else Option.value ~default:"" (Hashtbl.find_opt dictionary code) in
      if entry = "" then None else (
        Buffer.add_string result entry;
        let w = if previous = "" then ascii "undefined" else previous in
        Hashtbl.add dictionary !next (w ^ symbol (unit_at entry 0));
        incr next; advance codes; loop entry)
    in loop initial

let run op format data repeat =
  match data with
  | None -> Some [||]
  | Some data ->
    if op = "decompress" then Option.map units (decompress data format) else
    let chunk = text data in
    let s = String.concat "" (List.init repeat (fun _ -> chunk)) in
    let packed = compress s format in
    match op with
    | "compress" -> Some packed
    | "roundtrip" when decompress packed format = Some s ->
      Some [| Array.length packed; String.length s / 2 |]
    | _ -> invalid_arg "unknown operation or roundtrip mismatch"
