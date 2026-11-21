(* big.js 7.0.1 (big.mjs): the decimal type, its parser, [round], [stringify] and the methods the
   contract uses, function by function and loop by loop. Shared by the Melange and js_of_ocaml
   ports: big.ml and host.mli are byte-identical in ports/big-js/melange/src/ and
   ports/big-js/jsoo/src/; host.ml and the entry module are per compiler.

   A decimal is an immutable record. Its coefficient is an int array of digits, most significant
   first, which each operation builds afresh: where upstream mutates its own copy of an array in
   place (borrows, carries, the division remainder), this code mutates a fresh copy, and where
   upstream shifts, pops or truncates, it takes a sub-array. The exponent is a float because
   upstream's is a JS number: past int32 and 2^53 it rounds as a double does (so a run of
   decrements stays a run of decrements), and Infinity and NaN exponents keep upstream's
   comparison chains. *)

exception Big_error of string

type t = { s : int; e : float; c : int array }

let name = "[big.js] "
let invalid = name ^ "Invalid "
let invalid_dp = invalid ^ "decimal places"
let invalid_rm = invalid ^ "rounding mode"
let div_by_zero = name ^ "Division by zero"
let max_dp = 1e6
let max_power = 1e6
let ne = -7.
let pe = 21.
let fail message = raise (Big_error message)

(* Upstream's [n !== ~~n || n < lo || n > hi], negated: an integer in the range (all ranges lie
   within int32). *)
let is_int_in n lo hi = n >= lo && n <= hi && n = Float.floor n

(* JS truthiness of a number, for upstream's [if (a = xe - ye)]. *)
let truthy n = n <> 0. && not (Float.is_nan n)

(* ECMAScript ToInt32, for upstream's [| 0] and [&] on exponents. *)
let to_int32 v =
  if not (Float.is_finite v) then 0
  else
    let m = Float.rem (Float.trunc v) 4294967296. in
    int_of_float
      (if m >= 2147483648. then m -. 4294967296.
       else if m < -2147483648. then m +. 4294967296.
       else m)

(* [Array.prototype.slice(0, stop)], where a negative stop counts from the back. *)
let head c stop =
  let length = float_of_int (Array.length c) in
  Array.sub c 0
    (int_of_float (if stop < 0. then Float.max (length +. stop) 0. else Float.min stop length))

let same_digits a b = Array.length a = Array.length b && Array.for_all2 Int.equal a b
let digits c = String.init (Array.length c) (fun i -> Char.chr (48 + c.(i)))
let is_digit ch = '0' <= ch && ch <= '9'

(* The first index below n at which a and b differ, or -1. *)
let first_difference a b n =
  let i = ref 0 in
  while !i < n && a.(!i) = b.(!i) do incr i done;
  if !i < n then !i else -1

(* Upstream's NUMERIC, {|/^-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$/i|}: ASCII digits only,
   nothing trimmed, and $ is the end of the string. *)
let numeric n =
  let length = String.length n in
  let i = ref 0 in
  let at chars = !i < length && String.contains chars n.[!i] in
  let digits () =
    let start = !i in
    while !i < length && is_digit n.[!i] do incr i done;
    !i - start
  in
  if at "-" then incr i;
  let whole = digits () in
  let fraction = if at "." then (incr i; digits ()) else 0 in
  let exponent_ok =
    (not (at "eE"))
    || begin
         incr i;
         if at "+-" then incr i;
         digits () > 0
       end
  in
  (whole > 0 || fraction > 0) && exponent_ok && !i = length

(* Upstream [parse]: the decimal string n, validated against NUMERIC. *)
let parse source =
  if not (numeric source) then fail (invalid ^ "number");
  (* Determine sign. *)
  let s, n =
    if source.[0] = '-' then (-1, String.sub source 1 (String.length source - 1)) else (1, source)
  in
  (* Decimal point? *)
  let dot = String.index_opt n '.' in
  let n =
    match dot with
    | Some d -> String.sub n 0 d ^ String.sub n (d + 1) (String.length n - d - 1)
    | None -> n
  in
  (* Exponential form? *)
  let exponent_at =
    match (String.index_opt n 'e', String.index_opt n 'E') with
    | Some i, Some j -> Some (min i j)
    | i, None | None, i -> i
  in
  let e, n =
    match exponent_at with
    | Some i when i > 0 ->
        (* Determine exponent. *)
        let exponent = Host.number (String.sub n (i + 1) (String.length n - i - 1)) in
        (float_of_int (Option.value dot ~default:i) +. exponent, String.sub n 0 i)
    | _ ->
        (* Integer. *)
        (float_of_int (Option.value dot ~default:(String.length n)), n)
  in
  let nl = String.length n in
  (* Determine leading zeros. *)
  let lead = ref 0 in
  while !lead < nl && n.[!lead] = '0' do incr lead done;
  if !lead = nl then (* Zero. *) { s; e = 0.; c = [| 0 |] }
  else begin
    (* Determine trailing zeros. *)
    let last = ref (nl - 1) in
    while n.[!last] = '0' do decr last done;
    (* Convert string to array of digits without leading/trailing zeros. *)
    let c = Array.init (!last - !lead + 1) (fun k -> Char.code n.[!lead + k] - 48) in
    { s; e = e -. float_of_int !lead -. 1.; c }
  end

(* Upstream [round]: x rounded to sd significant digits using rounding mode rm. [more] says
   whether a division was truncated. *)
let round_to ?(more = false) x sd rm =
  if rm <> 0. && rm <> 1. && rm <> 2. && rm <> 3. then fail invalid_rm;
  let xc = x.c in
  let length = Array.length xc in
  if sd < 1. then begin
    let up =
      (rm = 3. && (more || xc.(0) <> 0))
      || sd = 0.
         && ((rm = 1. && xc.(0) >= 5)
            || (rm = 2. && (xc.(0) > 5 || (xc.(0) = 5 && (more || length > 1)))))
    in
    if up then (* 1, 0.1, 0.01, 0.001, 0.0001 etc. *) { x with e = x.e -. sd +. 1.; c = [| 1 |] }
    else (* Zero. *) { x with e = 0.; c = [| 0 |] }
  end
  else if sd < float_of_int length then begin
    let sd = int_of_float sd in
    (* xc.(sd) is the digit after the digit that may be rounded up. *)
    let up =
      (rm = 1. && xc.(sd) >= 5)
      || rm = 2.
         && (xc.(sd) > 5
            || (xc.(sd) = 5 && (more || sd + 1 < length || xc.(sd - 1) land 1 = 1)))
      || (rm = 3. && (more || xc.(0) <> 0))
    in
    (* Remove any digits after the required precision. *)
    let c = Array.sub xc 0 sd in
    (* Round up? Rounding up may mean the previous digit has to be rounded up. *)
    let c, e =
      if not up then (c, x.e)
      else begin
        let i = ref (sd - 1) in
        c.(!i) <- c.(!i) + 1;
        while c.(!i) > 9 && !i > 0 do
          c.(!i) <- 0;
          decr i;
          c.(!i) <- c.(!i) + 1
        done;
        if c.(0) > 9 then begin
          c.(0) <- 0;
          (Array.append [| 1 |] c, x.e +. 1.)
        end
        else (c, x.e)
      end
    in
    (* Remove trailing zeros. *)
    let last = ref (Array.length c - 1) in
    while c.(!last) = 0 do decr last done;
    { x with e; c = Array.sub c 0 (!last + 1) }
  end
  else x

(* Upstream [stringify]: x in normal or exponential notation. *)
let stringify x do_exponential is_nonzero =
  let e = x.e in
  let s = digits x.c in
  let n = String.length s in
  let s =
    if do_exponential then
      (* Exponential notation. *)
      String.sub s 0 1
      ^ (if n > 1 then "." ^ String.sub s 1 (n - 1) else "")
      ^ (if e < 0. then "e" else "e+")
      ^ Host.string_of_number e
    else if e < 0. then (* Normal notation. *) "0." ^ String.make (int_of_float (-.e -. 1.)) '0' ^ s
    else if e > 0. then begin
      let point = int_of_float e + 1 in
      if point > n then s ^ String.make (point - n) '0'
      else if point < n then String.sub s 0 point ^ "." ^ String.sub s point (n - point)
      else s
    end
    else if n > 1 then String.sub s 0 1 ^ "." ^ String.sub s 1 (n - 1)
    else s
  in
  if x.s < 0 && is_nonzero then "-" ^ s else s

(* 1, -1 or 0 as x is greater than, less than or equal to y. *)
let cmp x y =
  let xc = x.c and yc = y.c and i = x.s and j = y.s in
  (* Either zero? *)
  if xc.(0) = 0 || yc.(0) = 0 then (if xc.(0) = 0 then (if yc.(0) = 0 then 0 else -j) else i)
  (* Signs differ? *)
  else if i <> j then i
  else begin
    let isneg = i < 0 in
    let sign greater = if greater <> isneg then 1 else -1 in
    (* Compare exponents. *)
    if x.e <> y.e then sign (x.e > y.e)
    else begin
      (* Compare digit by digit, then lengths. *)
      let xl = Array.length xc and yl = Array.length yc in
      let k = first_difference xc yc (min xl yl) in
      if k >= 0 then sign (xc.(k) > yc.(k)) else if xl = yl then 0 else sign (xl > yl)
    end
  end

(* x divided by y, rounded if necessary to dp decimal places using rounding mode rm. *)
let div x y dp rm =
  let a = x.c (* dividend *) and b = y.c (* divisor *) in
  let s = if x.s = y.s then 1 else -1 in
  if not (is_int_in dp 0. max_dp) then fail invalid_dp;
  (* Divisor is zero? *)
  if b.(0) = 0 then fail div_by_zero;
  (* Dividend is 0? Return +-0. *)
  if a.(0) = 0 then { s; e = 0.; c = [| 0 |] }
  else begin
    let bl = Array.length b and al = Array.length a in
    let ai = ref bl in
    let bz = Array.append [| 0 |] b (* divisor with a leading zero *) in
    (* The remainder: the dividend's first digits, with zeros to make it as long as the divisor. *)
    let r = ref (Array.init bl (fun i -> if i < al then a.(i) else 0)) in
    let quotient = ref [] (* digits, last first *) and qi = ref 0 in
    let qe = x.e -. y.e in
    let p = dp +. qe +. 1. (* precision of the result *) in
    let k = ref (if p < 0. then 0. else p) in
    (* Compare divisor and remainder. *)
    let compare () =
      let rl = Array.length !r in
      if bl <> rl then if bl > rl then 1 else -1
      else
        let i = first_difference b !r bl in
        if i < 0 then 0 else if b.(i) > !r.(i) then 1 else -1
    in
    (* Subtract the divisor from the remainder. The remainder can't be more than 1 digit longer
       than the divisor: equalise lengths using the divisor with an extra leading zero. *)
    let subtract () =
      let r' = !r in
      let rl = Array.length r' in
      let bt = if rl = bl then b else bz in
      for j = rl - 1 downto 0 do
        if r'.(j) < bt.(j) then begin
          let i = ref (j - 1) in
          while !i > 0 && r'.(!i) = 0 do
            r'.(!i) <- 9;
            decr i
          done;
          r'.(!i) <- r'.(!i) - 1;
          r'.(j) <- r'.(j) + 10
        end;
        r'.(j) <- r'.(j) - bt.(j)
      done;
      let lead = ref 0 in
      while r'.(!lead) = 0 do incr lead done;
      r := Array.sub r' !lead (rl - !lead)
    in
    let going = ref true in
    while !going do
      (* n is how many times the divisor goes into current remainder (at most nine). *)
      let n = ref 0 and order = ref (compare ()) in
      (* If divisor < remainder, subtract divisor from remainder. *)
      while !order < 0 do
        subtract ();
        incr n;
        order := compare ()
      done;
      (* Add the digit n to the result array. *)
      quotient := (if !order <> 0 then !n else !n + 1) :: !quotient;
      incr qi;
      (* Update the remainder. Upstream's [undefined], the dividend's end, is the empty array. *)
      let next = if !ai < al then Some a.(!ai) else None in
      (r :=
         if Array.length !r > 0 && !r.(0) <> 0 && !order <> 0 then
           Array.append !r [| Option.value next ~default:0 |]
         else match next with Some digit -> [| digit |] | None -> [||]);
      let more = !ai < al || Array.length !r > 0 in
      incr ai;
      (* [k--] is falsy at 0 and at NaN, as [k > 0] is. *)
      going := more && !k > 0.;
      k := !k -. 1.
    done;
    let qc = Array.of_list (List.rev !quotient) in
    (* Leading zero? Do not remove if result is simply zero (qi == 1). There can't be more than
       one zero. *)
    let qc, qe, p =
      if qc.(0) = 0 && !qi <> 1 then (Array.sub qc 1 (!qi - 1), qe -. 1., p -. 1.) else (qc, qe, p)
    in
    let q = { s; e = qe; c = qc } in
    (* Round? *)
    if float_of_int !qi > p then round_to ~more:(Array.length !r > 0) q p rm else q
  end

let zeros n = Array.make (int_of_float n) 0

(* x minus y, and x plus y; each turns into the other when the signs differ. *)
let rec minus x y =
  let b = y.s in
  (* Signs differ? *)
  if x.s <> b then plus x { y with s = -b }
  else if x.c.(0) = 0 || y.c.(0) = 0 then
    (* Either zero? *)
    if y.c.(0) <> 0 then { y with s = -b } else if x.c.(0) <> 0 then x else { y with s = 1 }
  else begin
    let xe = x.e and ye = y.e in
    (* Determine which is the bigger number. Prepend zeros to equalise exponents. *)
    let a = xe -. ye in
    let xlty, a, xc, yc, ye =
      if truthy a then
        if a < 0. then (true, -.a, Array.append (zeros (-.a)) x.c, Array.copy y.c, ye)
        else (false, a, Array.copy x.c, Array.append (zeros a) y.c, xe)
      else begin
        (* Exponents equal. Check digit by digit. *)
        let xl = Array.length x.c and yl = Array.length y.c in
        let i = first_difference x.c y.c (min xl yl) in
        ((if i >= 0 then x.c.(i) < y.c.(i) else xl < yl), 0., Array.copy x.c, Array.copy y.c, ye)
      end
    in
    (* x < y? Point xc to the array of the bigger number. *)
    let xc, yc, sign = if xlty then (yc, xc, -y.s) else (xc, yc, y.s) in
    (* Append zeros to xc if shorter. No need to add zeros to yc if shorter as subtraction only
       needs to start at yc.length. *)
    let xc =
      let shortfall = Array.length yc - Array.length xc in
      if shortfall > 0 then Array.append xc (Array.make shortfall 0) else xc
    in
    (* Subtract yc from xc. *)
    let j = ref (Array.length yc) in
    while float_of_int !j > a do
      decr j;
      if xc.(!j) < yc.(!j) then begin
        let i = ref (!j - 1) in
        while !i > 0 && xc.(!i) = 0 do
          xc.(!i) <- 9;
          decr i
        done;
        xc.(!i) <- xc.(!i) - 1;
        xc.(!j) <- xc.(!j) + 10
      end;
      xc.(!j) <- xc.(!j) - yc.(!j)
    done;
    (* Remove trailing zeros. *)
    let last = ref (Array.length xc - 1) in
    while !last >= 0 && xc.(!last) = 0 do decr last done;
    (* Remove leading zeros and adjust exponent accordingly. *)
    let lead = ref 0 and ye = ref ye in
    while !lead <= !last && xc.(!lead) = 0 do
      incr lead;
      ye := !ye -. 1.
    done;
    if !lead > !last then (* n - n = +0 *) { s = 1; e = 0.; c = [| 0 |] }
    else { s = sign; e = !ye; c = Array.sub xc !lead (!last - !lead + 1) }
  end

and plus x y =
  (* Signs differ? *)
  if x.s <> y.s then minus x { y with s = -y.s }
  else if x.c.(0) = 0 || y.c.(0) = 0 then
    (* Either zero? *)
    if y.c.(0) <> 0 then y else if x.c.(0) <> 0 then x else { y with s = x.s }
  else begin
    (* Prepend zeros to equalise exponents. *)
    let e = x.e -. y.e in
    let xc, yc, ye =
      if not (truthy e) then (Array.copy x.c, Array.copy y.c, y.e)
      else if e > 0. then (Array.copy x.c, Array.append (zeros e) y.c, x.e)
      else (Array.append (zeros (-.e)) x.c, Array.copy y.c, y.e)
    in
    (* Point xc to the longer array. *)
    let xc, yc = if Array.length xc - Array.length yc < 0 then (yc, xc) else (xc, yc) in
    (* Only start adding at yc.length - 1 as the further digits of xc can be left as they are. *)
    let k = ref 0 in
    for i = Array.length yc - 1 downto 0 do
      let sum = xc.(i) + yc.(i) + !k in
      k := sum / 10;
      xc.(i) <- sum mod 10
    done;
    (* No need to check for zero, as +x + +y != 0 && -x + -y != 0 *)
    let xc, ye = if !k <> 0 then (Array.append [| !k |] xc, ye +. 1.) else (xc, ye) in
    (* Remove trailing zeros. *)
    let last = ref (Array.length xc - 1) in
    while xc.(!last) = 0 do decr last done;
    { y with e = ye; c = Array.sub xc 0 (!last + 1) }
  end

(* x times y. *)
let times x y =
  (* Determine sign of result. *)
  let s = if x.s = y.s then 1 else -1 in
  (* Return signed 0 if either 0. *)
  if x.c.(0) = 0 || y.c.(0) = 0 then { s; e = 0.; c = [| 0 |] }
  else begin
    (* If array xc has fewer digits than yc, swap xc and yc, and lengths. *)
    let xc, yc = if Array.length x.c < Array.length y.c then (y.c, x.c) else (x.c, y.c) in
    let a = Array.length xc and b = Array.length yc in
    (* Initialise coefficient array of result with zeros. *)
    let c = Array.make (a + b) 0 in
    (* Multiply. *)
    let carry = ref 0 in
    for i = b - 1 downto 0 do
      carry := 0;
      for j = a + i downto i + 1 do
        (* Current sum of products at this digit position, plus carry. *)
        carry := !carry + c.(j) + (yc.(i) * xc.(j - i - 1));
        c.(j) <- !carry mod 10;
        carry := !carry / 10
      done;
      c.(i) <- !carry
    done;
    (* Initialise exponent of result as x.e + y.e; increment it if there is a final carry,
       otherwise remove leading zero. *)
    let first, e = if !carry <> 0 then (0, x.e +. y.e +. 1.) else (1, x.e +. y.e) in
    (* Remove trailing zeros. *)
    let last = ref (a + b - 1) in
    while c.(!last) = 0 do decr last done;
    { s; e; c = Array.sub c first (!last - first + 1) }
  end

(* x raised to the integer power n; a negative power divides with dp and rm. *)
let pow x n dp rm =
  let one = { s = 1; e = 0.; c = [| 1 |] } in
  match n with
  | Some n when is_int_in n (-.max_power) max_power ->
      let isneg = n < 0. in
      let rec loop k x y =
        let y = if k land 1 <> 0 then times y x else y in
        let k = k asr 1 in
        if k = 0 then y else loop k (times x x) y
      in
      let y = loop (int_of_float (Float.abs n)) x one in
      if isneg then div one y dp rm else y
  | _ -> fail (invalid ^ "exponent")

(* x rounded to dp decimal places (0 when absent) using rounding mode rm. *)
let round x dp rm =
  (match dp with Some dp when not (is_int_in dp (-.max_dp) max_dp) -> fail invalid_dp | _ -> ());
  round_to x (Option.value dp ~default:0. +. x.e +. 1.) rm

(* JS [toExponential()] up to and including its [e]: the shortest digits of s in d.ddd form. *)
let mantissa s =
  let text = Host.to_exponential s in
  String.sub text 0 (String.index text 'e' + 1)

(* The square root of x, rounded to dp decimal places using rounding mode rm. *)
let sqrt x dp rm =
  let half = { s = 1; e = -1.; c = [| 5 |] } in
  (* Zero? *)
  if x.c.(0) = 0 then x (* Negative? *)
  else if x.s < 0 then fail (name ^ "No square root")
  else begin
    (* Estimate. *)
    let estimate = Float.sqrt (Host.number (stringify x true true)) in
    let e = x.e in
    let r =
      (* Math.sqrt underflow/overflow? Re-estimate: pass x coefficient to Math.sqrt as integer,
         then adjust the result exponent. *)
      if estimate = 0. || estimate = Float.infinity then begin
        let c = digits x.c in
        let c = if to_int32 (float_of_int (String.length c) +. e) land 1 = 0 then c ^ "0" else c in
        let estimate = Float.sqrt (Host.number c) in
        (* [| 0] gives an int32, but the subtraction is a double one: it can reach -2^31 - 1. *)
        let exponent =
          float_of_int (to_int32 ((e +. 1.) /. 2.))
          -. if e < 0. || to_int32 e land 1 <> 0 then 1. else 0.
        in
        let prefix = if estimate = Float.infinity then "5e" else mantissa estimate in
        parse (prefix ^ Host.string_of_number exponent)
      end
      else parse (Host.string_of_number estimate)
    in
    (* Big.DP += 4 for the iteration; div validates it. *)
    let places = dp +. 4. in
    let compared = r.e +. places in
    (* Newton-Raphson iteration. *)
    let rec iterate t =
      let r = times half (plus t (div x t places rm)) in
      if same_digits (head t.c compared) (head r.c compared) then r else iterate r
    in
    let r = iterate r in
    round_to r (dp +. r.e +. 1.) rm
  end

(* Exponential notation with dp fixed decimal places, rounded using rounding mode rm. *)
let to_exponential x dp rm =
  let n = x.c.(0) in
  let x =
    match dp with
    | None -> x
    | Some dp ->
        if not (is_int_in dp 0. max_dp) then fail invalid_dp;
        let x = round_to x (dp +. 1.) rm in
        let width = int_of_float dp + 1 - Array.length x.c in
        if width > 0 then { x with c = Array.append x.c (Array.make width 0) } else x
  in
  stringify x true (n <> 0)

(* Normal notation with dp fixed decimal places, rounded using rounding mode rm. *)
let to_fixed x dp rm =
  let n = x.c.(0) in
  let x =
    match dp with
    | None -> x
    | Some dp ->
        if not (is_int_in dp 0. max_dp) then fail invalid_dp;
        let x = round_to x (dp +. x.e +. 1.) rm in
        (* x.e may have changed if the value is rounded up. *)
        let width = dp +. x.e +. 1. -. float_of_int (Array.length x.c) in
        if width > 0. then { x with c = Array.append x.c (Array.make (int_of_float width) 0) }
        else x
  in
  stringify x false (n <> 0)

(* sd significant digits, rounded using rounding mode rm, in exponential notation if sd is less
   than the number of digits of the integer part. *)
let to_precision x sd rm =
  let n = x.c.(0) in
  let x =
    match sd with
    | None -> x
    | Some sd ->
        if not (is_int_in sd 1. max_dp) then fail (invalid ^ "precision");
        let x = round_to x sd rm in
        let width = int_of_float sd - Array.length x.c in
        if width > 0 then { x with c = Array.append x.c (Array.make width 0) } else x
  in
  let exponential = match sd with Some sd -> sd <= x.e | None -> false in
  stringify x (exponential || x.e <= ne || x.e >= pe) (n <> 0)

(* Exponential notation at exponents of NE and below or PE and above; no sign for zero. *)
let to_string x = stringify x (x.e <= ne || x.e >= pe) (x.c.(0) <> 0)

(* One operation of libraries/big-js/reference.mjs, with its operands already converted to text
   by the Big constructor's rule and its options read from the input. *)
let operate ~op ~x ~y ~n ~mode ~dp ~rm =
  let x = parse x in
  let mode = Option.value mode ~default:rm in
  match op with
  | "toString" -> to_string x
  | "sqrt" -> to_string (sqrt x dp rm)
  | "plus" -> to_string (plus x (parse y))
  | "minus" -> to_string (minus x (parse y))
  | "times" -> to_string (times x (parse y))
  | "div" -> to_string (div x (parse y) dp rm)
  | "cmp" -> string_of_int (cmp x (parse y))
  | "pow" -> to_string (pow x n dp rm)
  | "round" -> to_string (round x n mode)
  | "toFixed" -> to_fixed x n mode
  | "toExponential" -> to_exponential x n mode
  | "toPrecision" -> to_precision x n mode
  | _ -> fail "unknown decimal operation"
