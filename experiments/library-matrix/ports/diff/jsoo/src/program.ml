(* The matrix boundary: one operation per input, as libraries/diff/reference.mjs runs it. *)
open Host
open Differ

(* jsdiff's canonicalize: arrays in order, object keys sorted and assigned to
   a plain object (so a "__proto__" key sets the prototype and is dropped). An
   object with a truthy toJSON member has it called, and JSON cannot make it
   callable, so that throws, as in npm. *)
let rec canonical v =
  if is_array v then begin
    let a = array (length v) in
    for i = 0 to length v - 1 do
      put a i (canonical (at v i))
    done;
    a
  end
  else if object_value v then begin
    if truth (get v "toJSON") then invalid_arg "obj.toJSON is not a function";
    let o = obj () and ks = sorted (keys v) in
    for i = 0 to length ks - 1 do
      let k = at ks i in
      key_set o k (canonical (key_get v k))
    done;
    o
  end
  else v

let document op v = if op = "diffJson" && not (is_string v) then read (pretty (canonical v) null 2) else read v

let write_change span value =
  let o = obj () in
  set o "count" (number span.count);
  set o "added" (boolean (span.kind = Add));
  set o "removed" (boolean (span.kind = Remove));
  set o "value" value;
  o

(* Elements compare by their JSON text, which is === for JSON scalars. With
   ignoreCase, strings also compare lowercased (the JSON text of a lowercased
   string is the lowercased JSON text). *)
let array_diff a b o =
  let key x = if o.ignore_case then lower (read (json x)) else read (json x) in
  let values v = Patch.array_of_js key v in
  let spans = Array.of_list (myers (values a) (values b) o.individual) in
  let old = ref 0 and newer = ref 0 in
  Patch.array_to_js
    (fun span ->
      let value = array span.count in
      for i = 0 to span.count - 1 do
        put value i (if span.kind = Remove then at a (!old + i) else at b (!newer + i))
      done;
      if span.kind <> Add then old := !old + span.count;
      if span.kind <> Remove then newer := !newer + span.count;
      write_change span value)
    spans

(* applyPatch takes exactly one file patch. *)
let single patches = if Array.length patches <> 1 then invalid_arg "applyPatch only works with a single input" else patches.(0)

let run input =
  let op = read (get input "op") and a = get input "before" and b = get input "after" in
  let patch = get input "patch" and opts = get input "options" in
  let flag key = (not (absent opts)) && truth (get opts key) in
  let o =
    {
      ignore_case = flag "ignoreCase";
      individual = flag "oneChangePerToken";
      strip_cr = flag "stripTrailingCr";
      ignore_ws = flag "ignoreWhitespace";
      newline_token = flag "newlineIsToken";
      ignore_eof = flag "ignoreNewlineAtEof";
    }
  in
  match op with
  | "diffArrays" -> array_diff a b o
  | "diffChars" | "diffWords" | "diffLines" | "diffJson" ->
      let tokens v = tokenize op (document op v) o in
      Patch.array_to_js (fun c -> write_change c.span (str c.value)) (changes op (tokens a) (tokens b) o)
  | "structuredPatch" | "createTwoFilesPatch" ->
      let context = if absent opts || absent (get opts "context") then 4. else float_value (get opts "context") in
      let p = Patch.create input context in
      if op = "structuredPatch" then Patch.write p else str (Patch.format p)
  | "parsePatch" -> Patch.array_to_js Patch.write (Patch.parse (read patch))
  | "reversePatch" ->
      if is_array patch then begin
        let n = length patch in
        let v = array n in
        for i = 0 to n - 1 do
          put v (n - i - 1) (Patch.reverse (at patch i))
        done;
        v
      end
      else Patch.reverse patch
  | "applyPatch" -> (
      let p =
        if is_string patch then single (Patch.parse (read patch))
        else if is_array patch then Patch.read_patch (single (Patch.array_of_js Fun.id patch))
        else Patch.read_patch patch
      in
      match Patch.apply (read a) p with Some text -> str text | None -> boolean false)
  | _ -> invalid_arg "unknown diff operation"
