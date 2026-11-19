(* Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
   libraries/pilot/CONTRACT.md. Melange strings are JavaScript strings (UTF-16), so code points
   are read with Js.String.codePointAt; counting uses a JS Map, sorting OCaml's stable sort. *)

type entry = { word : string; mutable count : int; length : int }

let is_word_code_point cp =
  cp >= 0x80
  || (cp >= 0x30 && cp <= 0x39)
  || (cp >= 0x41 && cp <= 0x5a)
  || (cp >= 0x61 && cp <= 0x7a)

let code_point_at text index =
  match Js.String.codePointAt ~index text with Some cp -> cp | None -> 0

(* FNV-1a 32 over the UTF-8 encoding of [text]; Melange ints are signed 32-bit. *)
let fnv1a text =
  let hash = ref 0x811c9dc5 in
  let add byte = hash := (!hash lxor byte) * 0x01000193 in
  let length = Js.String.length text in
  let index = ref 0 in
  while !index < length do
    let cp = code_point_at text !index in
    if cp < 0x80 then add cp
    else if cp < 0x800 then (
      add (0xc0 lor (cp lsr 6));
      add (0x80 lor (cp land 0x3f)))
    else if cp < 0x10000 then (
      add (0xe0 lor (cp lsr 12));
      add (0x80 lor ((cp lsr 6) land 0x3f));
      add (0x80 lor (cp land 0x3f)))
    else (
      add (0xf0 lor (cp lsr 18));
      add (0x80 lor ((cp lsr 12) land 0x3f));
      add (0x80 lor ((cp lsr 6) land 0x3f));
      add (0x80 lor (cp land 0x3f)));
    index := !index + if cp > 0xffff then 2 else 1
  done;
  !hash

(* The unsigned (>>> 0) reading of a signed 32-bit hash as 8 hex digits, through a float because
   an int cannot hold it. *)
let hex8 hash =
  let unsigned = if hash < 0 then float_of_int hash +. 4294967296. else float_of_int hash in
  let digits = Js.Float.toString ~radix:16 unsigned in
  String.make (8 - String.length digits) '0' ^ digits

let fixed3 value = Js.Float.toFixed ~digits:3 value

let analyse text top =
  let entries : (string, entry) Js.Map.t = Js.Map.make () in
  let order = ref [] in
  let tokens = ref 0 in
  let code_points = ref 0 in
  let word = ref [||] in
  let flush () =
    let length = Array.length !word in
    if length > 0 then (
      incr tokens;
      code_points := !code_points + length;
      let key = Js.String.fromCodePointMany !word in
      (match Js.Map.get ~key entries with
      | Some entry -> entry.count <- entry.count + 1
      | None ->
          let entry = { word = key; count = 1; length } in
          ignore (Js.Map.set ~key ~value:entry entries);
          order := entry :: !order);
      word := [||])
  in
  let length = Js.String.length text in
  let index = ref 0 in
  while !index < length do
    let cp = code_point_at text !index in
    if is_word_code_point cp then
      (* ASCII-only case folding. *)
      ignore (Js.Array.push ~value:(if cp >= 0x41 && cp <= 0x5a then cp + 0x20 else cp) !word)
    else flush ();
    index := !index + if cp > 0xffff then 2 else 1
  done;
  flush ();
  (* List.stable_sort keeps first-occurrence order among equal keys. *)
  let ranked =
    List.stable_sort
      (fun a b -> if a.count <> b.count then compare b.count a.count else compare b.length a.length)
      (List.rev !order)
  in
  let mean = if !tokens = 0 then 0. else float_of_int !code_points /. float_of_int !tokens in
  let header =
    [
      "tokens " ^ string_of_int !tokens;
      "distinct " ^ string_of_int (Js.Map.size entries);
      "mean " ^ fixed3 mean;
      "fnv " ^ hex8 (fnv1a text);
    ]
  in
  let rows =
    List.filteri (fun i _ -> i < top) ranked
    |> List.map (fun entry ->
           let share = float_of_int entry.count /. float_of_int !tokens in
           String.concat " "
             [ entry.word; string_of_int entry.count; hex8 (fnv1a entry.word); fixed3 share ])
  in
  String.concat "\n" (header @ rows)
