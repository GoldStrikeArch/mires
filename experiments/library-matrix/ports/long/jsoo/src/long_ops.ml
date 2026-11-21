module I = Int64

let radix = function 0 -> 10 | r when r >= 2 && r <= 36 -> r | _ -> invalid_arg "radix"
let digits = "0123456789abcdefghijklmnopqrstuvwxyz"

let rec parse s base =
  if s = "" then invalid_arg "empty string";
  if List.mem s ["NaN";"Infinity";"+Infinity";"-Infinity"] then 0L else
  let r = radix base in
  if s.[0] = '-' then I.neg (parse (String.sub s 1 (String.length s - 1)) r) else begin
    if String.contains s '-' then invalid_arg "hyphen";
    let value = ref 0L and start = ref 0 in
    let b = I.of_int r in
    while !start < String.length s do
      let stop = min (!start + 8) (String.length s) in
      let i = ref !start in
      while !i < stop && String.contains " \t\r\n\012\011" s.[!i] do incr i done;
      if !i < stop && s.[!i] = '+' then incr i;
      if r = 16 && !i + 1 < stop && s.[!i] = '0' && Char.lowercase_ascii s.[!i+1] = 'x'
      then i := !i + 2;
      let chunk = ref 0L in
      while !i < stop do
        match String.index_opt digits (Char.lowercase_ascii s.[!i]) with
        | Some d when d < r -> chunk := I.add (I.mul !chunk b) (I.of_int d); incr i
        | _ -> i := stop
      done;
      for _j = !start to stop - 1 do value := I.mul !value b done;
      value := I.add !value !chunk;
      start := stop
    done;
    !value
  end

let format a unsigned base =
  let r = I.of_int (radix base) in
  let negative = not unsigned && a < 0L in
  let mag = if negative then I.neg a else a in
  let buf = Bytes.create 65 in
  let rec write v i =
    let q = I.unsigned_div v r in
    let d = I.to_int (I.sub v (I.mul q r)) in
    Bytes.set buf i digits.[d];
    if q <> 0L then write q (i-1) else i
  in
  let from = write mag 64 in
  let from = if negative then (Bytes.set buf (from-1) '-'; from-1) else from in
  Bytes.sub_string buf from (65-from)

let divide a b unsigned =
  if b = 0L then invalid_arg "division by zero";
  if unsigned then I.unsigned_div a b
  else if I.shift_right a 32 = -2147483648L && b = -1L then a
  else I.div a b

let compare a b u v =
  if a = b && (u = v || a >= 0L) then 0 else
  let an = not u && a < 0L and bn = not v && b < 0L in
  let less = if an <> bn then an else if u then I.unsigned_compare a b < 0 else I.sub a b < 0L in
  if less then -1 else 1

let rotl a n = I.logor (I.shift_left a (n land 63)) (I.shift_right_logical a ((-n) land 63))
let words w = I.logor (I.shift_left (I.of_int w.(1)) 32) (I.logand (I.of_int w.(0)) 0xffffffffL)

let count_zero a leading =
  let rec count v i =
    if i = 64 then i else
    if leading then (if v < 0L then i else count (I.shift_left v 1) (i+1))
    else if I.logand v 1L <> 0L then i else count (I.shift_right_logical v 1) (i+1)
  in count a 0

let command op left right u v n r s =
  let a = words left and b = words right in
  try match op with
  | "compare" -> string_of_int (compare a b u v)
  | "clz" -> string_of_int (count_zero a true)
  | "ctz" -> string_of_int (count_zero a false)
  | "mix" ->
    let acc = ref a and checksum = ref 0l and k = I.logor b 1L in
    if n > 0 then
      for i = 0 to n-1 do
        acc := I.logxor b (rotl (I.add (I.mul !acc k) (I.of_int (i+1))) 11);
        acc := I.add !acc (divide !acc (I.of_int (3+(i land 255))) u);
        if i land 7 = 0 then begin
          let text = format !acc u r in
          checksum := Int32.add (Int32.mul !checksum 33l) (Int32.of_int (String.length text + Char.code text.[0]))
        end
      done;
    format !acc u r ^ ":" ^ Int32.to_string !checksum
  | _ ->
    let value = match op with
      | "fromBits" | "toString" -> a
      | "fromString" -> parse s n
      | "add" -> I.add a b | "subtract" -> I.sub a b | "multiply" -> I.mul a b
      | "divide" -> divide a b u
      | "modulo" -> if b = 0L then invalid_arg "division by zero" else if u then I.unsigned_rem a b else I.rem a b
      | "and" -> I.logand a b | "or" -> I.logor a b | "xor" -> I.logxor a b
      | "shiftLeft" -> I.shift_left a (n land 63)
      | "shiftRight" -> I.shift_right a (n land 63)
      | "shiftRightUnsigned" -> I.shift_right_logical a (n land 63)
      | "rotl" -> rotl a n
      | _ -> invalid_arg "operation"
    in format value u r
  with Invalid_argument _ -> "!"
