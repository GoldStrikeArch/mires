(* Decimal coefficients contain normalized base-ten digits, most significant first. *)
type decimal = {s:int; e:float; c:int array}
let dec s e c = {s;e;c}
let size = Array.length
let slen = String.length
let filled = Array.make
let cut a start stop = Array.sub a start (stop-start)
let extend a d = Array.append a [|d|]
let code s i = Char.code s.[i]
let letter n = String.make 1 (Char.chr n)
let strint = string_of_int
let cat = (^)
let error (msg : string) : unit = failwith msg
let machineRoot = Float_host.root
let floatText = Float_host.text
let finite = Float.is_finite
let asFloat = float_of_int
let asIndex = int_of_float
let maxExp = Float.max
let minExp = Float.min
let even e = mod_float e 2. = 0.
let halvedExponent e = floor (e /. 2.)
let parseExponent s start stop = float_of_string (String.sub s start (stop-start))
let grow a = Array.append a (Array.make (Array.length a) 0)


let normalizeFrom (sign : int) (exponent : float) (digits : int array) (offset : int) : decimal =
    let start = ref (offset) in
    while (((!start) < (size (digits))) && (digits.((!start)) = 0)) do
        start := (!start) + 1;
        ()
    done;
    let stop = ref ((size (digits))) in
    while (((!stop) > (!start)) && (digits.(((!stop) - 1)) = 0)) do
        stop := (!stop) - 1;
        ()
    done;
    let out = ref ((dec (sign) (0.0) ((filled (1) (0))))) in
    if ((!stop) > (!start)) then (
        out := (dec (sign) ((exponent -. (asFloat (((!start) - offset))))) ((cut (digits) ((!start)) ((!stop)))));
        ()
    );
    (!out)

let normalize (sign : int) (exponent : float) (digits : int array) : decimal =
    (normalizeFrom (sign) (exponent) (digits) (0))

let parse (text : string) : decimal =
    let length = (slen (text)) in
    let pos = ref (0) in
    let sign = ref (1) in
    if (((!pos) < length) && ((code (text) ((!pos))) = 45)) then (
        sign := (-1);
        pos := (!pos) + 1;
        ()
    );
    let digits = (filled (length) (0)) in
    let count = ref (0) in
    let places = ref (0) in
    while (((!pos) < length) && ((code (text) ((!pos))) >= 48) && ((code (text) ((!pos))) <= 57)) do
        digits.((!count)) <- ((code (text) ((!pos))) - 48);
        count := (!count) + 1;
        pos := (!pos) + 1;
        places := (!places) + 1;
        ()
    done;
    if (((!pos) < length) && ((code (text) ((!pos))) = 46)) then (
        pos := (!pos) + 1;
        while (((!pos) < length) && ((code (text) ((!pos))) >= 48) && ((code (text) ((!pos))) <= 57)) do
            digits.((!count)) <- ((code (text) ((!pos))) - 48);
            count := (!count) + 1;
            pos := (!pos) + 1;
            ()
        done;
        ()
    );
    if ((!count) = 0) then (
        (error ("[big.js] Invalid number"));
        ()
    );
    let scale = ref (0.0) in
    if (((!pos) < length) && (((code (text) ((!pos))) = 69) || ((code (text) ((!pos))) = 101))) then (
        pos := (!pos) + 1;
        let exponentStart = (!pos) in
        if (((!pos) < length) && (((code (text) ((!pos))) = 45) || ((code (text) ((!pos))) = 43))) then (
            pos := (!pos) + 1;
            ()
        );
        let start = (!pos) in
        while (((!pos) < length) && ((code (text) ((!pos))) >= 48) && ((code (text) ((!pos))) <= 57)) do
            pos := (!pos) + 1;
            ()
        done;
        if ((!pos) = start) then (
            (error ("[big.js] Invalid number"));
            ()
        );
        scale := (parseExponent (text) (exponentStart) ((!pos)));
        ()
    );
    if ((!pos) <> length) then (
        (error ("[big.js] Invalid number"));
        ()
    );
    let leading = ref (0) in
    while (((!leading) < (!count)) && (digits.((!leading)) = 0)) do
        leading := (!leading) + 1;
        ()
    done;
    (normalizeFrom ((!sign)) (((((asFloat ((!places))) +. (!scale)) -. (asFloat ((!leading)))) -. 1.0)) ((cut (digits) (0) ((!count)))) ((!leading)))

let coefficient (x : decimal) (idx : int) : int =
    (if ((idx >= 0) && (idx < (size (x.c)))) then x.c.(idx) else 0)

let cmp (x : decimal) (y : decimal) : int =
    let result = ref (0) in
    if (x.c.(0) = 0) then (
        result := (if (y.c.(0) = 0) then 0 else (-y.s));
        ()
    ) else (
        if ((y.c.(0) = 0) || (x.s <> y.s)) then (
            result := x.s;
            ()
        ) else (
            if (x.e <> y.e) then (
                result := (if (x.e > y.e) then x.s else (-x.s));
                ()
            ) else (
                let i = ref (0) in
                let limit = (min ((size (x.c))) ((size (y.c)))) in
                while (((!i) < limit) && (x.c.((!i)) = y.c.((!i)))) do
                    i := (!i) + 1;
                    ()
                done;
                if ((!i) < limit) then (
                    result := (if (x.c.((!i)) > y.c.((!i))) then x.s else (-x.s));
                    ()
                ) else (
                    if ((size (x.c)) <> (size (y.c))) then (
                        result := (if ((size (x.c)) > (size (y.c))) then x.s else (-x.s));
                        ()
                    );
                    ()
                );
                ()
            );
            ()
        );
        ()
    );
    (!result)

let add (x : decimal) (y : decimal) (minus : bool) : decimal =
    let sign = (if minus then (-y.s) else y.s) in
    let result = ref ((dec (sign) (y.e) (y.c))) in
    if (x.c.(0) = 0) then (
        if ((y.c.(0) = 0) && (sign <> x.s)) then (
            result := (dec (1) (0.0) ((filled (1) (0))));
            ()
        );
        ()
    ) else (
        if (y.c.(0) = 0) then (
            result := x;
            ()
        ) else (
            let same = (x.s = sign) in
            let reverse = ((not same) && (((cmp (x) ((dec (x.s) (y.e) (y.c)))) * x.s) < 0)) in
            let top = (maxExp (x.e) (y.e)) in
            let offsetX = (asIndex ((top -. x.e))) in
            let offsetY = (asIndex ((top -. y.e))) in
            let width = (max ((offsetX + (size (x.c)))) ((offsetY + (size (y.c))))) in
            let digits = (filled ((width + 1)) (0)) in
            let carry = ref (0) in
            let p = ref ((width - 1)) in
            while ((!p) >= 0) do
                let a = (coefficient (x) (((!p) - offsetX))) in
                let b = (coefficient (y) (((!p) - offsetY))) in
                let total = ((!carry) + (if same then (a + b) else (if reverse then (b - a) else (a - b)))) in
                carry := (if (total < 0) then (-1) else (total / 10));
                digits.(((!p) + 1)) <- (total - ((!carry) * 10));
                p := (!p) - 1;
                ()
            done;
            digits.(0) <- (!carry);
            result := (normalizeFrom ((if reverse then (-x.s) else x.s)) ((if ((!carry) <> 0) then (top +. 1.0) else top)) (digits) ((if ((!carry) <> 0) then 0 else 1)));
            if ((!result).c.(0) = 0) then (
                result := (dec (1) (0.0) ((!result).c));
                ()
            );
            ()
        );
        ()
    );
    (!result)

let times (x : decimal) (y : decimal) : decimal =
    let digits = (filled (((size (x.c)) + (size (y.c)))) (0)) in
    let i = ref (((size (x.c)) - 1)) in
    while ((!i) >= 0) do
        let carry = ref (0) in
        let j = ref (((size (y.c)) - 1)) in
        while ((!j) >= 0) do
            let k = (((!i) + (!j)) + 1) in
            let v = (((x.c.((!i)) * y.c.((!j))) + digits.(k)) + (!carry)) in
            digits.(k) <- (v mod 10);
            carry := (v / 10);
            j := (!j) - 1;
            ()
        done;
        digits.((!i)) <- (!carry);
        i := (!i) - 1;
        ()
    done;
    let exponent = (x.e +. y.e) in
    (normalizeFrom ((x.s * y.s)) ((if (digits.(0) = 0) then exponent else (exponent +. 1.0))) (digits) ((if (digits.(0) = 0) then 1 else 0)))

let rounded (x : decimal) (precision : float) (mode : int) (more : bool) : decimal =
    if ((mode < 0) || (mode > 3)) then (
        (error ("[big.js] Invalid rounding mode"));
        ()
    );
    let result = ref (x) in
    if (precision < (asFloat ((size (x.c))))) then (
        let sd = (asIndex ((maxExp (precision) ((-1.0))))) in
        let guardDigit = (if (sd >= 0) then x.c.(sd) else 0) in
        let odd = ((sd > 0) && ((x.c.((sd - 1)) mod 2) = 1)) in
        let sticky = (((sd + 1) < (size (x.c))) || more) in
        let up = (((mode = 1) && (guardDigit >= 5)) || ((mode = 2) && ((guardDigit > 5) || ((guardDigit = 5) && (odd || sticky)))) || ((mode = 3) && ((x.c.(0) <> 0) || more))) in
        if (sd <= 0) then (
            result := (if up then (dec (x.s) (((x.e -. precision) +. 1.0)) ((filled (1) (1)))) else (dec (x.s) (0.0) ((filled (1) (0)))));
            ()
        ) else (
            let digits = (cut (x.c) (0) (sd)) in
            let pos = ref ((sd - 1)) in
            if up then (
                while (((!pos) >= 0) && (digits.((!pos)) = 9)) do
                    digits.((!pos)) <- 0;
                    pos := (!pos) - 1;
                    ()
                done;
                if ((!pos) >= 0) then (
                    digits.((!pos)) <- (digits.((!pos)) + 1);
                    ()
                );
                ()
            );
            result := (if ((!pos) < 0) then (dec (x.s) ((x.e +. 1.0)) ((filled (1) (1)))) else (normalize (x.s) (x.e) (digits)));
            ()
        );
        ()
    );
    (!result)

let natural (a : int array) (b : int array) : int =
    let result = ref (0) in
    if ((size (a)) <> (size (b))) then (
        result := (if ((size (a)) > (size (b))) then 1 else (-1));
        ()
    ) else (
        let i = ref (0) in
        while (((!i) < (size (a))) && (a.((!i)) = b.((!i)))) do
            i := (!i) + 1;
            ()
        done;
        if ((!i) < (size (a))) then (
            result := (if (a.((!i)) > b.((!i))) then 1 else (-1));
            ()
        );
        ()
    );
    (!result)

let divide (x : decimal) (y : decimal) (dp : int) (mode : int) : decimal =
    if ((dp < 0) || (dp > 1000000)) then (
        (error ("[big.js] Invalid decimal places"));
        ()
    );
    if (y.c.(0) = 0) then (
        (error ("[big.js] Division by zero"));
        ()
    );
    let sign = (x.s * y.s) in
    let result = ref ((dec (sign) (0.0) ((filled (1) (0))))) in
    if (x.c.(0) <> 0) then (
        let rest = ref ((filled ((size (y.c))) (0))) in
        let pos = ref (0) in
        while ((!pos) < (size (y.c))) do
            (!rest).((!pos)) <- (if ((!pos) < (size (x.c))) then x.c.((!pos)) else 0);
            pos := (!pos) + 1;
            ()
        done;
        let exponent = ref ((x.e -. y.e)) in
        let digits = ref ((filled (16) (0))) in
        let count = ref (0) in
        let complete = ref (false) in
        while (not (!complete)) do
            let quotient = ref (0) in
            while ((natural ((!rest)) (y.c)) >= 0) do
                let shift = ((size ((!rest))) - (size (y.c))) in
                let borrow = ref (0) in
                let k = ref (((size ((!rest))) - 1)) in
                while ((!k) >= 0) do
                    let dv = (if ((!k) >= shift) then y.c.(((!k) - shift)) else 0) in
                    let v = (((!rest).((!k)) - dv) - (!borrow)) in
                    borrow := (if (v < 0) then 1 else 0);
                    (!rest).((!k)) <- (v + ((!borrow) * 10));
                    k := (!k) - 1;
                    ()
                done;
                let start = ref (0) in
                while (((!start) < (size ((!rest)))) && ((!rest).((!start)) = 0)) do
                    start := (!start) + 1;
                    ()
                done;
                if ((!start) > 0) then (
                    rest := (cut ((!rest)) ((!start)) ((size ((!rest)))));
                    ()
                );
                quotient := (!quotient) + 1;
                ()
            done;
            if (((!quotient) = 0) && ((!count) = 0)) then (
                exponent := ((!exponent) -. 1.0);
                ()
            ) else (
                if ((!count) = (size ((!digits)))) then (
                    digits := (grow ((!digits)));
                    ()
                );
                (!digits).((!count)) <- (!quotient);
                count := (!count) + 1;
                ()
            );
            let more = (((size ((!rest))) > 0) || ((!pos) < (size (x.c)))) in
            complete := (((!count) > 0) && ((not more) || ((asFloat ((!count))) > (((asFloat (dp)) +. (!exponent)) +. 1.0))));
            if (!complete) then (
                result := (dec (sign) ((!exponent)) ((cut ((!digits)) (0) ((!count)))));
                if ((asFloat ((!count))) > (((asFloat (dp)) +. (!exponent)) +. 1.0)) then (
                    result := (rounded ((!result)) ((((asFloat (dp)) +. (!exponent)) +. 1.0)) (mode) (more));
                    ()
                );
                ()
            ) else (
                let nxt = (if ((!pos) < (size (x.c))) then x.c.((!pos)) else 0) in
                if (((size ((!rest))) > 0) || (nxt <> 0)) then (
                    rest := (extend ((!rest)) (nxt));
                    ()
                );
                pos := (!pos) + 1;
                ()
            );
            ()
        done;
        ()
    );
    (!result)

let power (x : decimal) (n : int) (dp : int) (mode : int) : decimal =
    if ((n < (-1000000)) || (n > 1000000)) then (
        (error ("[big.js] Invalid exponent"));
        ()
    );
    let one = (dec (1) (0.0) ((filled (1) (1)))) in
    let result = ref (one) in
    let base = ref (x) in
    let i = ref ((if (n < 0) then (-n) else n)) in
    while ((!i) > 0) do
        if (((!i) mod 2) = 1) then (
            result := (times ((!result)) ((!base)));
            ()
        );
        i := ((!i) / 2);
        if ((!i) > 0) then (
            base := (times ((!base)) ((!base)));
            ()
        );
        ()
    done;
    if (n < 0) then (
        result := (divide (one) ((!result)) (dp) (mode));
        ()
    );
    (!result)

let render (x : decimal) (op : string) (n : int) (mode : int) : string =
    let v = ref (x) in
    if (n <> (-2000002)) then (
        let sd = ref ((asFloat (n))) in
        if (op = "toPrecision") then (
            if ((n < 1) || (n > 1000000)) then (
                (error ("[big.js] Invalid precision"));
                ()
            );
            ()
        ) else (
            if ((n < 0) || (n > 1000000)) then (
                (error ("[big.js] Invalid decimal places"));
                ()
            );
            sd := (if (op = "toFixed") then (((asFloat (n)) +. (!v).e) +. 1.0) else (asFloat ((n + 1))));
            ()
        );
        v := (rounded ((!v)) ((!sd)) (mode) (false));
        let width = (if (op = "toFixed") then (((asFloat (n)) +. (!v).e) +. 1.0) else (!sd)) in
        if (width > (asFloat ((size ((!v).c))))) then (
            let padded = (filled ((asIndex (width))) (0)) in
            let i = ref (0) in
            while ((!i) < (size ((!v).c))) do
                padded.((!i)) <- (!v).c.((!i));
                i := (!i) + 1;
                ()
            done;
            v := (dec ((!v).s) ((!v).e) (padded));
            ()
        );
        ()
    );
    let exponential = ((op = "toExponential") || ((op <> "toFixed") && (((!v).e <= (-7.0)) || ((!v).e >= 21.0) || ((op = "toPrecision") && (n <> (-2000002)) && ((asFloat (n)) <= (!v).e))))) in
    let text = ref ((if ((x.s < 0) && (x.c.(0) <> 0)) then "-" else "")) in
    if exponential then (
        text := (cat ((!text)) ((letter ((48 + (!v).c.(0))))));
        if ((size ((!v).c)) > 1) then (
            text := (cat ((!text)) ("."));
            let j = ref (1) in
            while ((!j) < (size ((!v).c))) do
                text := (cat ((!text)) ((letter ((48 + (!v).c.((!j)))))));
                j := (!j) + 1;
                ()
            done;
            ()
        );
        text := (cat ((!text)) ((if ((!v).e >= 0.0) then "e+" else "e")));
        text := (cat ((!text)) ((floatText ((!v).e))));
        ()
    ) else (
        let point = (asIndex ((!v).e)) in
        let p = ref ((max (0) (point))) in
        while ((!p) >= 0) do
            text := (cat ((!text)) ((letter ((48 + (coefficient ((!v)) ((point - (!p)))))))));
            p := (!p) - 1;
            ()
        done;
        let last = ((point - (size ((!v).c))) + 1) in
        if (last < 0) then (
            text := (cat ((!text)) ("."));
            p := (-1);
            while ((!p) >= last) do
                text := (cat ((!text)) ((letter ((48 + (coefficient ((!v)) ((point - (!p)))))))));
                p := (!p) - 1;
                ()
            done;
            ()
        );
        ()
    );
    (!text)

let root (x : decimal) (dp : int) (mode : int) : decimal =
    let result = ref (x) in
    if ((x.s > 0) && (dp >= 0) && (dp <= 999996) && (mode >= 0) && (mode <= 3) && (x.e < (asFloat ((((-2) * dp) - 2))))) then (
        result := (if (mode = 3) then (dec (1) ((asFloat ((-dp)))) ((filled (1) (1)))) else (dec (1) (0.0) ((filled (1) (0)))));
        ()
    ) else (
        if (x.c.(0) <> 0) then (
            if (x.s < 0) then (
                (error ("[big.js] No square root"));
                ()
            );
            let estimate = (machineRoot ((render (x) ("toExponential") ((-2000002)) (mode)))) in
            if ((estimate > 0.0) && (finite (estimate))) then (
                result := (parse ((floatText (estimate))));
                ()
            ) else (
                let coefficient = ref ("") in
                let i = ref (0) in
                while ((!i) < (size (x.c))) do
                    coefficient := (cat ((!coefficient)) ((letter ((48 + x.c.((!i)))))));
                    i := (!i) + 1;
                    ()
                done;
                if (even (((asFloat ((size (x.c)))) +. x.e))) then (
                    coefficient := (cat ((!coefficient)) ("0"));
                    ()
                );
                let fallback = (machineRoot ((!coefficient))) in
                let leading = (if (finite (fallback)) then (parse ((floatText (fallback)))) else (dec (1) (0.0) ((filled (1) (5))))) in
                result := (dec (1) ((halvedExponent (x.e))) (leading.c));
                ()
            );
            let half = (dec (1) ((-1.0)) ((filled (1) (5)))) in
            let work = (asIndex ((minExp (1000001.0) ((maxExp ((asFloat ((dp + 4)))) ((5.0 -. (!result).e))))))) in
            let count = (maxExp (1.0) (((!result).e +. (asFloat (work))))) in
            let matched = ref (false) in
            while (not (!matched)) do
                let nxt = (times (half) ((add ((!result)) ((divide (x) ((!result)) (work) (mode))) (false)))) in
                matched := ((!result).e = nxt.e);
                let length = (asIndex ((minExp (count) ((asFloat ((max ((size ((!result).c))) ((size (nxt.c)))))))))) in
                let j = ref (0) in
                while ((!matched) && ((!j) < length)) do
                    matched := (((!j) < (size ((!result).c))) && ((!j) < (size (nxt.c))) && ((!result).c.((!j)) = nxt.c.((!j))));
                    j := (!j) + 1;
                    ()
                done;
                result := nxt;
                ()
            done;
            result := (rounded ((!result)) ((((asFloat (dp)) +. (!result).e) +. 1.0)) (mode) (false));
            ()
        );
        ()
    );
    (!result)

let operate (op : string) (xText : string) (yText : string) (n : int) (dp : int) (rm : int) (mode : int) : string =
    let x = (parse (xText)) in
    let result = ref (x) in
    let out = ref ("") in
    if ((op = "plus") || (op = "minus") || (op = "times") || (op = "div") || (op = "cmp")) then (
        let y = (parse (yText)) in
        if (op = "cmp") then (
            out := (strint ((cmp (x) (y))));
            ()
        ) else (
            if (op = "times") then (
                result := (times (x) (y));
                ()
            ) else (
                if (op = "div") then (
                    result := (divide (x) (y) (dp) (rm));
                    ()
                ) else (
                    result := (add (x) (y) ((op = "minus")));
                    ()
                );
                ()
            );
            ()
        );
        ()
    ) else (
        if (op = "round") then (
            let places = (if (n = (-2000002)) then 0 else n) in
            if ((places < (-1000000)) || (places > 1000000)) then (
                (error ("[big.js] Invalid decimal places"));
                ()
            );
            result := (rounded (x) ((((asFloat (places)) +. x.e) +. 1.0)) (mode) (false));
            ()
        ) else (
            if (op = "pow") then (
                result := (power (x) (n) (dp) (rm));
                ()
            ) else (
                if (op = "sqrt") then (
                    result := (root (x) (dp) (rm));
                    ()
                ) else (
                    if ((op = "toString") || (op = "toExponential") || (op = "toFixed") || (op = "toPrecision")) then (
                        out := (render (x) (op) ((if (op = "toString") then (-2000002) else n)) (mode));
                        ()
                    ) else (
                        (error ("unknown decimal operation"));
                        ()
                    );
                    ()
                );
                ()
            );
            ()
        );
        ()
    );
    if ((!out) = "") then (
        out := (render ((!result)) ("toString") ((-2000002)) (mode));
        ()
    );
    (!out)
