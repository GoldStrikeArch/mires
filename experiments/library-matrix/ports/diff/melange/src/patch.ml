(* Unified patches after jsdiff 9.0.0 patch/create.js, parse.js, apply.js and
   reverse.js. Text is a WTF-8 byte string (Host.read); hunk ranges are
   JavaScript numbers, so parsed headers keep values past int32 and NaN. *)
open Differ
open Host

type hunk = {
  old_start : float;
  old_lines : float;
  new_start : float;
  new_lines : float;
  lines : string array;
}

(* [meta] is the host object being built, so its keys keep the order in
   which jsdiff first assigns them. *)
type patch = { meta : Host.value; hunks : hunk array }

let starts s prefix = String.starts_with ~prefix s
let ends s suffix = String.ends_with ~suffix s
let drop s n = String.sub s n (String.length s - n)
let array_of_js f v = Array.init (length v) (fun i -> f (at v i))

let array_to_js f a =
  let v = array (Array.length a) in
  Array.iteri (fun i e -> put v i (f e)) a;
  v

(* Whether [s] has [prefix] at [i]. *)
let starts_at s i prefix =
  let m = String.length prefix in
  let rec matches j = j = m || (s.[i + j] = prefix.[j] && matches (j + 1)) in
  i >= 0 && i + m <= String.length s && matches 0

(* The first index of [sub] in [s] at or after [from]. *)
let find s sub from =
  let rec go i = if i + String.length sub > String.length s then None else if starts_at s i sub then Some i else go (i + 1) in
  go from

let replace_all s pattern replacement =
  let b = Buffer.create (String.length s) in
  let rec go i =
    match find s pattern i with
    | Some j ->
        Buffer.add_string b (String.sub s i (j - i));
        Buffer.add_string b replacement;
        go (j + String.length pattern)
    | None -> Buffer.add_string b (drop s i)
  in
  go 0;
  Buffer.contents b

(* ---- structuredPatch and formatPatch ---- *)

let create input context =
  let before = line_tokens (read (get input "before")) defaults
  and after = line_tokens (read (get input "after")) defaults in
  let spans = Array.of_list (myers before after false) in
  let hunks = ref [] and output = ref [] and active = ref false in
  let old_start = ref 0 and new_start = ref 0 and old_lines = ref 0 and new_lines = ref 0 in
  let emit kind source first count =
    let mark = match kind with Keep -> " " | Add -> "+" | Remove -> "-" in
    for i = first to first + count - 1 do
      let s = source.(i) in
      if ends s "\n" then output := (mark ^ String.sub s 0 (String.length s - 1)) :: !output
      else output := "\\ No newline at end of file" :: (mark ^ s) :: !output
    done;
    if kind <> Add then old_lines := !old_lines + count;
    if kind <> Remove then new_lines := !new_lines + count
  in
  let close () =
    let hunk =
      {
        old_start = float_of_int !old_start;
        old_lines = float_of_int !old_lines;
        new_start = float_of_int !new_start;
        new_lines = float_of_int !new_lines;
        lines = Array.of_list (List.rev !output);
      }
    in
    hunks := hunk :: !hunks;
    active := false;
    old_lines := 0;
    new_lines := 0;
    output := []
  in
  let a = ref 0 and b = ref 0 and kept = ref 0 in
  Array.iteri
    (fun i span ->
      if span.kind = Keep then begin
        if !active then begin
          let last = i + 1 = Array.length spans || span.count > context * 2 in
          emit Keep after !b (if last then min span.count context else span.count);
          if last then close ()
        end;
        kept := span.count
      end
      else begin
        if not !active then begin
          let k = min context !kept in
          old_start := !a + 1 - k;
          new_start := !b + 1 - k;
          emit Keep after (!b - k) k;
          active := true
        end;
        if span.kind = Add then emit Add after !b span.count else emit Remove before !a span.count
      end;
      if span.kind <> Add then a := !a + span.count;
      if span.kind <> Remove then b := !b + span.count)
    spans;
  if !active then close ();
  let meta = obj () in
  List.iter (fun k -> set meta k (get input k)) [ "oldFileName"; "newFileName"; "oldHeader"; "newHeader" ];
  { meta; hunks = Array.of_list (List.rev !hunks) }

(* quoteFileNameIfNeeded, over TextEncoder's bytes. *)
let quote s =
  let s = scalar_text s in
  let needed c =
    let n = Char.code c in
    n < 32 || n > 126 || n = 34 || n = 92
  in
  if not (String.exists needed s) then s
  else begin
    let b = Buffer.create (String.length s + 8) in
    Buffer.add_char b '"';
    String.iter
      (fun c ->
        let n = Char.code c in
        if n >= 7 && n <= 13 then begin
          Buffer.add_char b '\\';
          Buffer.add_char b "abtnvfr".[n - 7]
        end
        else if n = 34 || n = 92 then begin
          Buffer.add_char b '\\';
          Buffer.add_char b c
        end
        else if n < 32 || n > 126 then begin
          Buffer.add_char b '\\';
          List.iter (fun shift -> Buffer.add_char b (Char.chr (48 + ((n lsr shift) land 7)))) [ 6; 3; 0 ]
        end
        else Buffer.add_char b c)
      s;
    Buffer.add_char b '"';
    Buffer.contents b
  end

let format p =
  let b = Buffer.create 256 in
  let line s =
    Buffer.add_string b s;
    Buffer.add_char b '\n'
  in
  let old_name = get p.meta "oldFileName" and new_name = get p.meta "newFileName" in
  let named = not (absent old_name || absent new_name) in
  if named && read old_name = read new_name then line ("Index: " ^ read old_name);
  line "===================================================================";
  if named then
    List.iter
      (fun (tag, name, header) ->
        let header = get p.meta header in
        line (tag ^ quote (read (get p.meta name)) ^ (if truth header then "\t" ^ read header else "")))
      [ ("--- ", "oldFileName", "oldHeader"); ("+++ ", "newFileName", "newHeader") ];
  let range start count =
    let start = if count = 0. then start -. 1. else start in
    string_of_int (int_of_float start) ^ "," ^ string_of_int (int_of_float count)
  in
  Array.iter
    (fun h ->
      line ("@@ -" ^ range h.old_start h.old_lines ^ " +" ^ range h.new_start h.new_lines ^ " @@");
      Array.iter line h.lines)
    p.hunks;
  Buffer.contents b

(* ---- parsePatch ---- *)

(* parse.js classifies lines with anchored regexes; [space] is the
   ECMAScript \s set and \w is ASCII. *)
let underline = "==================================================================="
let space_at s i = i < String.length s && space (fst (cp s i))
let space_after line prefix = starts line prefix && space_at line (String.length prefix)

(* /^diff --git / *)
let is_git_header line = starts line "diff --git "

(* /^(---|\+\+\+)\s/ *)
let is_file_header line = space_after line "---" || space_after line "+++"

(* /^@@\s/ *)
let is_hunk_header line = space_after line "@@"
let word_at s i = i < String.length s && match s.[i] with 'a' .. 'z' | 'A' .. 'Z' | '0' .. '9' | '_' -> true | _ -> false

(* The end of diff(?: -r \w+)+ in a match of /^diff(?: -r \w+)+\s/. The
   repetition is greedy; a last group that is not followed by whitespace is
   given back, and the space that starts it matches \s. *)
let hg_prefix line =
  let rec word j = if word_at line j then word (j + 1) else j in
  let rec groups at ends =
    if starts_at line at " -r " && word_at line (at + 4) then
      let stop = word (at + 4) in
      groups stop (stop :: ends)
    else ends
  in
  if not (starts line "diff") then None
  else
    match groups 4 [] with
    | last :: _ when space_at line last -> Some last
    | _ :: previous :: _ -> Some previous
    | _ -> None

(* The length of the match of /^(?:Index:|diff(?: -r \w+)+)\s+/. *)
let index_header line =
  let start = if starts line "Index:" then Some 6 else hg_prefix line in
  let rec spaces i = if space_at line i then spaces (i + snd (cp line i)) else i in
  match start with
  | Some start when space_at line start -> Some (spaces start)
  | _ -> None

let is_diff_header line = is_git_header line || index_header line <> None

(* The capture of the extended-header regexes' trailing group: `.` stops at
   \r, U+2028 and U+2029. *)
let rest_of_line s =
  let n = String.length s in
  let rec stop i =
    if i >= n || s.[i] = '\r' then i
    else if i + 2 < n && s.[i] = '\xe2' && s.[i + 1] = '\x80' && (s.[i + 2] = '\xa8' || s.[i + 2] = '\xa9')
    then i
    else stop (i + 1)
  in
  String.sub s 0 (stop 0)

let digit_at s i = i < String.length s && s.[i] >= '0' && s.[i] <= '9'

(* The capture of (\d+), ASCII digits only. *)
let mode s =
  let rec stop i = if digit_at s i then stop (i + 1) else i in
  match stop 0 with 0 -> None | n -> Some (String.sub s 0 n)

let octal_at s i = i < String.length s && s.[i] >= '0' && s.[i] <= '7'

let escape = function
  | 'a' -> Some '\007'
  | 'b' -> Some '\b'
  | 'f' -> Some '\012'
  | 'n' -> Some '\n'
  | 'r' -> Some '\r'
  | 't' -> Some '\t'
  | 'v' -> Some '\011'
  | ('\\' | '"') as c -> Some c
  | _ -> None

(* parseQuotedFileName: the unescaped name and the length of the quoted text,
   or None for an unterminated quote or an unknown escape. A run of octal
   escapes holds raw bytes (Uint8Array keeps the low byte) that the host
   TextDecoder decodes together. Both loops are self tail calls. *)
let quoted s =
  let n = String.length s in
  let name = Buffer.create n and bytes = Buffer.create 8 in
  let byte i = Char.chr ((((Char.code s.[i] - 48) lsl 6) lor ((Char.code s.[i + 1] - 48) lsl 3) lor (Char.code s.[i + 2] - 48)) land 255) in
  let octal_escape j = j + 3 < n && s.[j] = '\\' && octal_at s (j + 1) && octal_at s (j + 2) && octal_at s (j + 3) in
  (* The index after a run of three-digit octal escapes starting at [j], or
     None when an escape in the run is cut short. *)
  let rec octets j =
    if not (j + 1 < n && s.[j] = '\\' && octal_at s (j + 1)) then Some j
    else if octal_escape j then begin
      Buffer.add_char bytes (byte (j + 1));
      octets (j + 4)
    end
    else None
  in
  let rec scan j =
    if j >= n then None
    else
      match s.[j] with
      | '"' -> Some (Buffer.contents name, j + 1)
      | '\\' when j + 1 < n -> (
          match escape s.[j + 1] with
          | Some c ->
              Buffer.add_char name c;
              scan (j + 2)
          | None when octal_escape j -> (
              Buffer.clear bytes;
              match octets j with
              | Some next ->
                  Buffer.add_string name (decode_octets (Buffer.contents bytes));
                  scan next
              | None -> None)
          | None -> None)
      | c ->
          Buffer.add_char name c;
          scan (j + 1)
  in
  if starts s "\"" then scan 1 else None

(* unquoteIfQuoted *)
let unquote s = match quoted s with Some (name, _) -> name | None -> s

(* A lone UTF-16 surrogate in WTF-8. *)
let surrogate u =
  String.init 3 (fun k -> Char.chr (match k with 0 -> 0xed | 1 -> 0x80 lor ((u lsr 6) land 0x3f) | _ -> 0x80 lor (u land 0x3f)))

(* s.substring(1): dropping the first unit of an astral character leaves its
   low surrogate. *)
let drop_first_unit s =
  if s = "" then ""
  else
    let c, width = cp s 0 in
    let rest = drop s width in
    if c > 0xffff then surrogate (0xdc00 lor ((c - 0x10000) land 0x3ff)) ^ rest else rest

(* s.substring(0, s.length - 1): dropping the last unit of an astral
   character leaves its high surrogate. *)
let drop_last_unit s =
  let rec last i start = if i >= String.length s then start else last (i + snd (cp s i)) i in
  if s = "" then ""
  else
    let start = last 0 0 in
    let head = String.sub s 0 start in
    let c, _ = cp s start in
    if c > 0xffff then head ^ surrogate (0xd800 lor ((c - 0x10000) lsr 10)) else head

(* parseGitDiffHeader: the names after "diff --git ", when they can be split.
   Both quoted forms skip one UTF-16 unit, assumed to be a space. *)
let git_names rest =
  match String.index_opt rest '"' with
  | Some 0 -> (
      match quoted rest with
      | None -> None
      | Some (old_name, length) ->
          let after = drop_first_unit (drop rest length) in
          if starts after "\"" then Option.map (fun (name, _) -> (old_name, name)) (quoted after)
          else Some (old_name, after))
  | Some at -> Option.map (fun (name, _) -> (drop_last_unit (String.sub rest 0 at), name)) (quoted (drop rest at))
  | None when starts rest "a/" -> (
      (* Split on the middle " b/", which is right when both names agree. *)
      let rec splits from found =
        match find rest " b/" from with Some at -> splits (at + 1) (at :: found) | None -> List.rev found
      in
      match splits 1 [] with
      | [] -> None
      | found ->
          let mid = List.nth found (List.length found / 2) in
          Some (String.sub rest 0 mid, drop rest (mid + 1)))
  | None -> None

(* One Git extended header line; at most one of parse.js's regexes matches. *)
let extended_header meta line =
  let after tag = drop line (String.length tag) in
  let flag key = set meta key (boolean true) in
  let name key prefix tag = set meta key (str (prefix ^ unquote (rest_of_line (after tag)))) in
  let set_mode key tag k = Option.iter (fun m -> k (); set meta key (str m)) (mode (after tag)) in
  if starts line "rename from " then begin
    name "oldFileName" "a/" "rename from ";
    flag "isRename"
  end
  else if starts line "rename to " then begin
    name "newFileName" "b/" "rename to ";
    flag "isRename"
  end
  else if starts line "copy from " then begin
    name "oldFileName" "a/" "copy from ";
    flag "isCopy"
  end
  else if starts line "copy to " then begin
    name "newFileName" "b/" "copy to ";
    flag "isCopy"
  end
  else if starts line "new file mode " then set_mode "newMode" "new file mode " (fun () -> flag "isCreate")
  else if starts line "deleted file mode " then set_mode "oldMode" "deleted file mode " (fun () -> flag "isDelete")
  else if starts line "old mode " then set_mode "oldMode" "old mode " ignore
  else if starts line "new mode " then set_mode "newMode" "new mode " ignore
  else if starts line "Binary files " then flag "isBinary"

(* parseFileHeader: a --- or +++ line, if there is one. *)
let parse_file_header lines i meta =
  if !i < Array.length lines && is_file_header lines.(!i) then begin
    let line = lines.(!i) in
    let data = String.split_on_char '\t' (trim (drop line 3)) in
    let file_name = List.hd data in
    let header = match data with _ :: h :: _ -> trim h | _ -> "" in
    let file_name = if starts file_name "\"" then unquote file_name else replace_all file_name "\\\\" "\\" in
    let side = if line.[0] = '-' then "old" else "new" in
    set meta (side ^ "FileName") (str file_name);
    set meta (side ^ "Header") (str header);
    incr i
  end

(* A run of ASCII digits as a JavaScript number; a huge run rounds to infinity. *)
let digits s i =
  let rec stop j = if digit_at s j then stop (j + 1) else j in
  let j = stop i in
  if j > i then Some (float_of_string (String.sub s i (j - i)), j) else None

(* The captures of /@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/ from [at],
   just after its leading "@@ -". *)
let range_at line at =
  let ( let* ) = Option.bind in
  let count i =
    match if i < String.length line && line.[i] = ',' then digits line (i + 1) else None with
    | Some (n, j) -> (Some n, j)
    | None -> (None, i)
  in
  let* old_start, i = digits line at in
  let old_lines, i = count i in
  let* new_start, i = if starts_at line i " +" then digits line (i + 2) else None in
  let new_lines, i = count i in
  if starts_at line i " @@" then Some (old_start, old_lines, new_start, new_lines) else None

(* The leftmost match, which String.prototype.split finds anywhere in the line. *)
let hunk_range line =
  let rec search from =
    match find line "@@ -" from with
    | None -> None
    | Some at -> ( match range_at line (at + 4) with Some range -> Some range | None -> search (at + 1))
  in
  search 0

(* parseHunk: an unmatched header gives NaN starts and one line per side. *)
let parse_hunk lines i =
  let n = Array.length lines in
  let old_start, old_lines, new_start, new_lines =
    match hunk_range lines.(!i) with
    | Some (a, b, c, d) -> (a, Option.value b ~default:1., c, Option.value d ~default:1.)
    | None -> (nan, 1., nan, 1.)
  in
  incr i;
  let old_start = if old_lines = 0. then old_start +. 1. else old_start in
  let new_start = if new_lines = 0. then new_start +. 1. else new_start in
  let body = ref [] and adds = ref 0 and removes = ref 0 in
  while
    !i < n && (float_of_int !removes < old_lines || float_of_int !adds < new_lines || starts lines.(!i) "\\")
  do
    let line = lines.(!i) in
    (* An empty line is context, except the last one: ''[0] is undefined. *)
    let operation = if line <> "" then Some line.[0] else if !i <> n - 1 then Some ' ' else None in
    (match operation with
    | Some '+' -> incr adds
    | Some '-' -> incr removes
    | Some ' ' ->
        incr adds;
        incr removes
    | Some '\\' -> ()
    | _ -> invalid_arg "hunk contained an invalid line");
    body := line :: !body;
    incr i
  done;
  let new_lines = if !adds = 0 && new_lines = 1. then 0. else new_lines in
  let old_lines = if !removes = 0 && old_lines = 1. then 0. else old_lines in
  if float_of_int !adds <> new_lines then invalid_arg "added line count did not match";
  if float_of_int !removes <> old_lines then invalid_arg "removed line count did not match";
  if !i < n && lines.(!i) <> "" && String.contains "+ -" lines.(!i).[0] && not (is_file_header lines.(!i)) then
    invalid_arg "hunk has more lines than expected";
  { old_start; old_lines; new_start; new_lines; lines = Array.of_list (List.rev !body) }

(* parseIndex: one file's metadata, file headers and hunks. *)
let parse_index lines i =
  let n = Array.length lines in
  let meta = obj () in
  set meta "hunks" undef;
  (* The metadata loop; false when a second diff header ends the file early. *)
  let rec metadata seen =
    if !i >= n then true
    else
      let line = lines.(!i) in
      if is_file_header line || is_hunk_header line then true
      else if is_git_header line then
        if seen then false
        else begin
          set meta "isGit" (boolean true);
          Option.iter
            (fun (old_name, new_name) ->
              set meta "oldFileName" (str old_name);
              set meta "newFileName" (str new_name))
            (git_names (drop line 11));
          incr i;
          while !i < n && not (is_file_header lines.(!i) || is_hunk_header lines.(!i) || is_diff_header lines.(!i)) do
            extended_header meta lines.(!i);
            incr i
          done;
          metadata true
        end
      else
        match index_header line with
        | Some _ when seen -> false
        | Some length ->
            set meta "index" (str (trim (drop line length)));
            incr i;
            metadata true
        | None ->
            incr i;
            metadata seen
  in
  let hunks = ref [] in
  if metadata false then begin
    parse_file_header lines i meta;
    parse_file_header lines i meta;
    if absent (get meta "oldFileName") <> absent (get meta "newFileName") then invalid_arg "missing file header";
    let rec body () =
      if !i < n then
        let line = lines.(!i) in
        if is_diff_header line || is_file_header line || starts line underline then ()
        else if is_hunk_header line then begin
          hunks := parse_hunk lines i :: !hunks;
          body ()
        end
        else begin
          incr i;
          body ()
        end
    in
    body ()
  end;
  { meta; hunks = Array.of_list (List.rev !hunks) }

let parse text =
  let lines = Array.of_list (String.split_on_char '\n' text) in
  let i = ref 0 in
  let rec files found = if !i < Array.length lines then files (parse_index lines i :: found) else List.rev found in
  Array.of_list (files [])

let write p =
  let hunk h =
    let x = obj () in
    set x "oldStart" (number h.old_start);
    set x "oldLines" (number h.old_lines);
    set x "newStart" (number h.new_start);
    set x "newLines" (number h.new_lines);
    set x "lines" (array_to_js str h.lines);
    x
  in
  set p.meta "hunks" (array_to_js hunk p.hunks);
  p.meta

(* ---- reversePatch, over the host values as Object.assign copies them ---- *)

let swap_prefix v =
  if not (is_string v) then v
  else
    let name = read v in
    if starts name "a/" then str ("b/" ^ drop name 2) else if starts name "b/" then str ("a/" ^ drop name 2) else v

let reverse_line l =
  let s = read l in
  if starts s "-" then str ("+" ^ drop s 1) else if starts s "+" then str ("-" ^ drop s 1) else l

let reverse_hunk h =
  let x = obj () in
  set x "oldLines" (get h "newLines");
  set x "oldStart" (get h "newStart");
  set x "newLines" (get h "oldLines");
  set x "newStart" (get h "oldStart");
  set x "lines" (array_to_js reverse_line (array_of_js Fun.id (get h "lines")));
  x

let reverse v =
  let git = truth (get v "isGit") in
  let name k = if git then swap_prefix (get v k) else get v k in
  let o = obj () in
  let ks = keys v in
  for i = 0 to length ks - 1 do
    let k = at ks i in
    key_set o k (key_get v k)
  done;
  set o "oldFileName" (name "newFileName");
  set o "oldHeader" (get v "newHeader");
  set o "newFileName" (name "oldFileName");
  set o "newHeader" (get v "oldHeader");
  set o "oldMode" (get v "newMode");
  set o "newMode" (get v "oldMode");
  set o "isCreate" (get v "isDelete");
  set o "isDelete" (get v "isCreate");
  set o "hunks" (array_to_js reverse_hunk (array_of_js Fun.id (get v "hunks")));
  if truth (get v "isCopy") then begin
    (* Reversing a copy deletes the copy destination. *)
    set o "newFileName" (str "/dev/null");
    set o "newHeader" undef;
    set o "isDelete" (boolean true);
    List.iter (delete o) [ "isCreate"; "isCopy"; "isRename" ];
    set o "hunks" (array 0)
  end;
  o

(* ---- applyPatch ---- *)

let read_patch v =
  let hunk x =
    {
      old_start = float_value (get x "oldStart");
      old_lines = float_value (get x "oldLines");
      new_start = float_value (get x "newStart");
      new_lines = float_value (get x "newLines");
      lines = array_of_js read (get x "lines");
    }
  in
  { meta = v; hunks = array_of_js hunk (get v "hunks") }

let contains s sub = find s sub 0 <> None

(* hasOnlyWinLineEndings, hasOnlyUnixLineEndings *)
let only_win_line_endings s =
  let bare_lf = ref false in
  String.iteri (fun i c -> if c = '\n' && i > 0 && s.[i - 1] <> '\r' then bare_lf := true) s;
  contains s "\r\n" && (not (starts s "\n")) && not !bare_lf

let only_unix_line_endings s = (not (contains s "\r\n")) && String.contains s '\n'
let before_marker lines i = i + 1 < Array.length lines && starts lines.(i + 1) "\\"
let some_line f p = Array.exists (fun h -> Array.exists f h.lines) p.hunks
let is_unix p = not (some_line (fun l -> (not (starts l "\\")) && ends l "\r") p)

let is_win p =
  some_line (fun l -> ends l "\r") p
  && Array.for_all
       (fun h ->
         let ok = ref true in
         Array.iteri (fun i l -> if not (starts l "\\" || ends l "\r" || before_marker h.lines i) then ok := false) h.lines;
         !ok)
       p.hunks

let map_lines f p = { p with hunks = Array.map (fun h -> { h with lines = Array.mapi (f h.lines) h.lines }) p.hunks }

let unix_to_win =
  map_lines (fun lines i l -> if starts l "\\" || ends l "\r" || before_marker lines i then l else l ^ "\r")

let win_to_unix = map_lines (fun _ _ l -> if ends l "\r" then String.sub l 0 (String.length l - 1) else l)

(* distanceIterator: positions after [start], alternating forward and
   backward by increasing distance, within [min_line, max_line]. *)
let distance_iterator start min_line max_line =
  let want_forward = ref true and backward_exhausted = ref false and forward_exhausted = ref false in
  let offset = ref 1 in
  let rec next () =
    let forward =
      if !want_forward && not !forward_exhausted then begin
        if !backward_exhausted then incr offset else want_forward := false;
        if start + !offset <= max_line then Some (start + !offset)
        else begin
          forward_exhausted := true;
          None
        end
      end
      else None
    in
    match forward with
    | Some _ -> forward
    | None when not !backward_exhausted ->
        if not !forward_exhausted then want_forward := true;
        if min_line <= start - !offset then begin
          let position = start - !offset in
          incr offset;
          Some position
        end
        else begin
          backward_exhausted := true;
          next ()
        end
    | None -> None
  in
  next

type fitted = { patched : string list; last_old_line : int }

let rec drop_first n l = match l with _ :: rest when n > 0 -> drop_first (n - 1) rest | _ -> l

(* applyHunk with no errors allowed: every removed and context line equals
   the source line at its position. Trailing context is given back. *)
let fit lines hunk to_pos =
  let source i = if i >= 0 && i < Array.length lines then Some lines.(i) else None in
  let rec go k to_pos patched context =
    if k = Array.length hunk then
      Some { patched = List.rev (drop_first context patched); last_old_line = to_pos - context - 1 }
    else
      let line = hunk.(k) in
      let operation = if line = "" then ' ' else line.[0] in
      let content = if line = "" then "" else drop line 1 in
      match operation with
      | '-' -> if source to_pos = Some content then go (k + 1) (to_pos + 1) patched 0 else None
      | '+' -> go (k + 1) to_pos (content :: patched) 0
      | ' ' -> if source to_pos = Some content then go (k + 1) (to_pos + 1) (content :: patched) (context + 1) else None
      | _ -> go (k + 1) to_pos patched context
  in
  go 0 to_pos [] 0

let apply source p =
  let p =
    if only_win_line_endings source && is_unix p then unix_to_win p
    else if only_unix_line_endings source && is_win p then win_to_unix p
    else p
  in
  let lines = Array.of_list (String.split_on_char '\n' source) in
  let count = Array.length p.hunks in
  if count = 0 then Some source
  else
    let remove_eofnl = ref false and add_eofnl = ref false and previous = ref "" in
    Array.iter
      (fun line ->
        if starts line "\\" then
          if starts !previous "+" then remove_eofnl := true else if starts !previous "-" then add_eofnl := true;
        previous := line)
      p.hunks.(count - 1).lines;
    let eof = lines.(Array.length lines - 1) = "" in
    let lines =
      match (!remove_eofnl, !add_eofnl) with
      | true, true -> if eof then None else Some lines
      | true, false -> if eof then Some (Array.sub lines 0 (Array.length lines - 1)) else None
      | false, true -> if eof then None else Some (Array.append lines [| "" |])
      | false, false -> Some lines
    in
    match lines with
    | None -> None
    | Some lines ->
        let line_at i = if i >= 0 && i < Array.length lines then lines.(i) else "" in
        let rec copy i stop result = if i >= stop then result else copy (i + 1) stop (line_at i :: result) in
        let rec hunks k min_line offset result =
          if k = count then Some (String.concat "\n" (List.rev (copy min_line (Array.length lines) result)))
          else
            let h = p.hunks.(k) in
            let old_start = int_of_float h.old_start in
            let max_line = Array.length lines - int_of_float h.old_lines in
            (* The declared position first, then the distance iterator around it. *)
            let to_pos = old_start + offset - 1 in
            let next = distance_iterator to_pos min_line max_line in
            let rec search to_pos =
              match fit lines h.lines to_pos with
              | Some fitted -> Some (to_pos, fitted)
              | None -> ( match next () with Some position -> search position | None -> None)
            in
            match search to_pos with
            | None -> None
            | Some (to_pos, fitted) ->
                let result = List.rev_append fitted.patched (copy min_line to_pos result) in
                hunks (k + 1) (fitted.last_old_line + 1) (to_pos + 1 - old_start) result
        in
        hunks 0 0 0 []
