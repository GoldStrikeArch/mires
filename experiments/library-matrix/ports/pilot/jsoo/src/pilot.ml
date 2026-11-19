(* Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
   libraries/pilot/CONTRACT.md. js_of_ocaml strings are OCaml byte strings: the boundary hands
   over UTF-8, so this module works on bytes. An ASCII byte never occurs inside a multi-byte UTF-8
   sequence, so a token is a maximal run of ASCII letters, ASCII digits and bytes >= 0x80, and its
   length in code points is its number of non-continuation bytes. *)

type entry = { word : string; mutable count : int; length : int }

let is_word_byte c =
  match c with 'a' .. 'z' | 'A' .. 'Z' | '0' .. '9' -> true | c -> Char.code c >= 0x80

(* FNV-1a 32 over the bytes of [s]. Int32 keeps the arithmetic 32-bit on every OCaml target
   (native ints are 63-bit, js_of_ocaml ints 32-bit). *)
let fnv1a s =
  let hash = ref 0x811c9dc5l in
  String.iter
    (fun c -> hash := Int32.mul (Int32.logxor !hash (Int32.of_int (Char.code c))) 0x01000193l)
    s;
  !hash

let hex8 hash = Printf.sprintf "%08lx" hash

let analyse text top =
  let index : (string, entry) Hashtbl.t = Hashtbl.create 64 in
  let order = ref [] in
  let tokens = ref 0 in
  let code_points = ref 0 in
  let word = Buffer.create 16 in
  let length = ref 0 in
  let flush () =
    if !length > 0 then (
      incr tokens;
      code_points := !code_points + !length;
      let key = Buffer.contents word in
      (match Hashtbl.find_opt index key with
      | Some entry -> entry.count <- entry.count + 1
      | None ->
          let entry = { word = key; count = 1; length = !length } in
          Hashtbl.add index key entry;
          order := entry :: !order);
      Buffer.clear word;
      length := 0)
  in
  String.iter
    (fun c ->
      if is_word_byte c then (
        (* ASCII-only case folding. *)
        Buffer.add_char word (Char.lowercase_ascii c);
        if Char.code c land 0xc0 <> 0x80 then incr length)
      else flush ())
    text;
  flush ();
  (* List.stable_sort keeps first-occurrence order among equal keys. *)
  let ranked =
    List.stable_sort
      (fun a b -> if a.count <> b.count then compare b.count a.count else compare b.length a.length)
      (List.rev !order)
  in
  let mean = if !tokens = 0 then 0. else float_of_int !code_points /. float_of_int !tokens in
  let out = Buffer.create 256 in
  Printf.bprintf out "tokens %d\ndistinct %d\nmean %.3f\nfnv %s" !tokens (Hashtbl.length index) mean
    (hex8 (fnv1a text));
  List.iteri
    (fun i entry ->
      if i < top then
        Printf.bprintf out "\n%s %d %s %.3f" entry.word entry.count (hex8 (fnv1a entry.word))
          (float_of_int entry.count /. float_of_int !tokens))
    ranked;
  Buffer.contents out
