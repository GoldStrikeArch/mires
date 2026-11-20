module Decimal exposing (operate)

-- big.js 7.0.1 on digit lists. Elm's Int is a JS number, so exponents past int32 and 2^53 round
-- as upstream's do; `truncate` is JS `| 0` and Bitwise is JS `&`, which upstream applies to
-- exponents. Elm compiles a comparison of two non-literal numbers to `_Utils_cmp`, which orders
-- NaN above everything, and `== 0` to a falsiness test, which NaN passes, so where a value can be
-- NaN the code tests for NaN first.

import Bitwise


type alias Decimal =
    { sign : Int, e : Int, c : List Int }


single sign e n =
    Decimal sign e [n]


first xs =
    Maybe.withDefault 0 (List.head xs)


trimLeft xs =
    case xs of
        0 :: rest -> trimLeft rest
        _ -> xs


trimRight xs =
    List.reverse (trimLeft (List.reverse xs))


-- Drop leading zeros one at a time, decrementing the exponent as upstream's `--ye` does (past 2^53
-- a run of decrements is not one subtraction), then trailing zeros.
normalize sign exponent digits =
    case digits of
        0 :: rest -> normalize sign (exponent - 1) rest
        [] -> single sign 0 0
        _ -> Decimal sign exponent (trimRight digits)


-- Upstream's `n !== ~~n || n < lo || n > hi`, negated: an integer in the range (within int32).
intIn lo hi n =
    if n >= lo && n <= hi && n == toFloat (floor n) then Just (floor n) else Nothing


-- JS `Number(text)` for the strings upstream passes it: NaN where it is not a number.
number text =
    String.toFloat text |> Maybe.withDefault (0/0)


validDigits s =
    String.all (\c -> c >= '0' && c <= '9') s


-- The exponent is one `Number()` of its signed digits, and the result's exponent is
-- ((dot + exp) - lead) - 1, as upstream computes it.
parse text =
    let
        neg = String.startsWith "-" text
        body = String.toLower (if neg then String.dropLeft 1 text else text)
        number_ sign expr mantissa =
            let
                parts = String.split "." mantissa
                all = String.concat parts
                digits = String.toList all |> List.map (\c -> Char.toCode c - 48)
                integral = List.head parts |> Maybe.withDefault ""
                lead = List.length digits - List.length (trimLeft digits)
            in
            if List.length parts > 2 || String.isEmpty all || not (validDigits all) then Err "[big.js] Invalid number"
            else if lead == List.length digits then Ok (single sign 0 0)
            else Ok (Decimal sign (String.length integral + expr - lead - 1) (trimRight (trimLeft digits)))
        signum = if neg then -1 else 1
    in
    case String.split "e" body of
        [m] -> number_ signum 0 m
        [m,e] ->
            let
                plain = if String.startsWith "-" e || String.startsWith "+" e then String.dropLeft 1 e else e
            in
            if String.isEmpty plain || not (validDigits plain) then Err "[big.js] Invalid number"
            else number_ signum (floor (number e)) m
        _ -> Err "[big.js] Invalid number"


cmp x y =
    if first x.c == 0 then
        if first y.c == 0 then 0 else -y.sign
    else if first y.c == 0 || x.sign /= y.sign then x.sign
    else if x.e /= y.e then
        if x.e > y.e then x.sign else -x.sign
    else case compare x.c y.c of
        LT -> -x.sign
        EQ -> 0
        GT -> x.sign


-- Carry or borrow from right to left, counting digits in integral decimal columns.
combine subtract a b =
    let
        step (u,v) (carry, digits) =
            let
                n = carry + (if subtract then u-v else u+v)
                next = if n < 0 then -1 else n // 10
            in
            (next, (n-next*10) :: digits)
        (last,values) = List.foldr step (0,[]) (List.map2 Tuple.pair a b)
    in
    last :: values


-- Upstream prepends zeros only when the exponent difference is truthy (neither 0 nor NaN, as for
-- Infinity - Infinity). The result's exponent is the larger one, plus one after a carry or minus
-- one per leading zero.
add x y subtract =
    let
        sign = if subtract then -y.sign else y.sign
    in
    if first x.c == 0 then {y | sign=if first y.c == 0 && sign /= x.sign then 1 else sign}
    else if first y.c == 0 then x
    else
        let
            same = x.sign == sign
            reverse = not same && cmp x {y | sign=x.sign} * x.sign < 0
            d = toFloat x.e - toFloat y.e
            (xLead,yLead,top) =
                if d /= 0 && not (isNaN d) then
                    if d > 0 then (0, x.e - y.e, x.e) else (y.e - x.e, 0, y.e)
                else (0, 0, y.e)
            width = max (xLead + List.length x.c) (yLead + List.length y.c)
            aligned lead a = List.repeat lead 0 ++ a.c ++ List.repeat (width - lead - List.length a.c) 0
            total = if reverse then combine True (aligned yLead y) (aligned xLead x) else combine (not same) (aligned xLead x) (aligned yLead y)
            resultSign = if reverse then -x.sign else x.sign
        in
        case total of
            carry :: rest -> if carry /= 0 then normalize resultSign (top+1) total else normalize resultSign top rest
            [] -> single 1 0 0


times x y =
    let
        byDigit d (offset,acc) =
            let
                step n (carry,ds) =
                    let v = d*n+carry in (v//10, modBy 10 v :: ds)
                (high,digits) = List.foldr step (0,[]) x.c
                row = high :: digits ++ List.repeat offset 0
                aligned = List.repeat (List.length acc - List.length row) 0 ++ row
            in
            (offset+1, List.drop 1 (combine False acc aligned))
        count = List.length x.c + List.length y.c
        (_,product) = List.foldr byDigit (0,List.repeat count 0) y.c
        sign = x.sign*y.sign
    in
    -- The exponent is x.e + y.e, incremented after a final carry; otherwise the leading zero goes.
    if first x.c == 0 || first y.c == 0 then single sign 0 0
    else case product of
        0 :: rest -> Decimal sign (x.e+y.e) (trimRight rest)
        _ -> Decimal sign (x.e+y.e+1) (trimRight product)


rounded x sd mode more =
    let
        len = List.length x.c
        discarded = if sd<0 then 0 else first (List.drop sd x.c)
        odd = sd>0 && modBy 2 (first (List.drop (sd-1) x.c))==1
        sticky = sd+1<len || more
        up = (mode==1 && discarded>=5) || (mode==2 && (discarded>5 || (discarded==5 && (odd || sticky)))) || (mode==3 && (first x.c/=0 || more))
        bump n (carry,ds) =
            let v = n+carry in (v//10,modBy 10 v :: ds)
        retained = List.take sd x.c
        (leading,tail) = List.foldr bump (if up then 1 else 0,[]) retained
    in
    if mode/=0 && mode/=1 && mode/=2 && mode/=3 then Err "[big.js] Invalid rounding mode"
    else if isNaN (toFloat sd) || sd>=len then Ok x
    else if sd<=0 then Ok (if up then single x.sign (x.e-sd+1) 1 else single x.sign 0 0)
    else if leading/=0 then Ok (Decimal x.sign (x.e+1) (trimRight (leading::tail)))
    else Ok (Decimal x.sign x.e (trimRight tail))


divide x y dp mode =
    let
        sign = x.sign*y.sign
        divisorLen = List.length y.c
        subtract rest count =
            let length = List.length rest in
            if length < divisorLen || (length==divisorLen && rest < y.c) then (rest,count)
            else
                subtract (trimLeft (combine True rest (List.repeat (length-divisorLen) 0 ++ y.c))) (count+1)
        loop places rest remaining exponent rev count =
            let
                (left,q) = subtract rest 0
                leading = q==0 && count==0
                e = if leading then exponent-1 else exponent
                digits = if leading then rev else q::rev
                n = if leading then count else count+1
                more = not (List.isEmpty left && List.isEmpty remaining)
                precision = places+e+1
            in
            if n>0 && (not more || n>precision) then
                let answer = Decimal sign e (List.reverse digits) in
                if n>precision then rounded answer precision mode more else Ok answer
            else
                let
                    d = first remaining
                    appended = if d==0 && List.isEmpty left then [] else left++[d]
                in
                loop places appended (List.drop 1 remaining) e digits n
        start = List.take divisorLen (x.c ++ List.repeat (divisorLen - List.length x.c) 0)
    in
    case intIn 0 1000000 dp of
        Nothing -> Err "[big.js] Invalid decimal places"
        Just places ->
            if first y.c==0 then Err "[big.js] Division by zero"
            else if first x.c==0 then Ok (single sign 0 0)
            -- A NaN precision (an Infinity - Infinity exponent): upstream's `k--` stops after the
            -- first digit, which it keeps even when it is zero, and it never rounds.
            else if isNaN (toFloat (places + (x.e-y.e) + 1)) then Ok (Decimal sign (x.e-y.e) [Tuple.second (subtract start 0)])
            else loop places start (List.drop divisorLen x.c) (x.e-y.e) [] 0


power x n dp mode =
    let
        one = single 1 0 1
        loop i base acc =
            if i==0 then acc
            else
                let next = if modBy 2 i == 1 then times acc base else acc in
                if i<2 then next else loop (i//2) (times base base) next
    in
    case Maybe.andThen (intIn -1000000 1000000) n of
        Nothing -> Err "[big.js] Invalid exponent"
        Just k ->
            let positive=loop (abs k) x one in
            if k<0 then divide one positive dp mode else Ok positive


joinDigits digits = String.concat (List.map String.fromInt digits)


-- Upstream's chain of tests: a NaN exponent fails both `e < 0` and `e > 0`.
stringify x exponential nonzero =
    let
        s = joinDigits x.c
        e = x.e
        n = String.length s
        rendered =
            if exponential then
                String.left 1 s ++ (if n>1 then "."++String.dropLeft 1 s else "") ++ (if e<0 then "e" else "e+") ++ String.fromInt e
            else if e<0 then "0."++String.repeat (-e-1) "0"++s
            else if e>0 then
                if e+1>n then s ++ String.repeat (e+1-n) "0"
                else if e+1<n then String.left (e+1) s ++ "." ++ String.dropLeft (e+1) s
                else s
            else if n>1 then String.left 1 s ++ "." ++ String.dropLeft 1 s
            else s
    in
    (if x.sign<0 && nonzero then "-" else "") ++ rendered


render x op n mode =
    let
        formatted v =
            let exp = op=="toExponential" || (op/="toFixed" && (v.e <= -7 || v.e>=21 || (op=="toPrecision" && Maybe.withDefault False (Maybe.map (\k -> k<=toFloat v.e) n)))) in
            stringify v exp (first x.c/=0)
        valid requested = if op=="toPrecision" then intIn 1 1000000 requested else intIn 0 1000000 requested
    in
    case Maybe.map valid n of
        Nothing -> Ok (formatted x)
        Just Nothing -> Err (if op=="toPrecision" then "[big.js] Invalid precision" else "[big.js] Invalid decimal places")
        Just (Just k) ->
            let
                sd = if op=="toFixed" then k+x.e+1 else if op=="toPrecision" then k else k+1
                -- Upstream pads while length < width, so a NaN width pads nothing.
                pad v =
                    let
                        width = if op=="toFixed" then k+v.e+1 else sd
                        missing = width - List.length v.c
                    in
                    formatted {v | c=v.c ++ List.repeat (if missing > 0 then missing else 0) 0}
            in
            Result.map pad (rounded x sd mode False)


-- `Array.prototype.slice(0, stop)`, where a negative stop counts from the back.
head stop digits =
    let n = toFloat (List.length digits) in
    List.take (round (if stop < 0 then max (n + stop) 0 else min stop n)) digits


-- Upstream P.sqrt: the host's binary64 estimate, Newton-Raphson iteration at DP + 4 places (div
-- validates them), and the stop test on the first r.e + DP + 4 digits of successive estimates.
root x dp mode =
    let
        half = single 1 -1 5
        places = dp + 4
        iterate compared t =
            case divide x t places mode of
                Err reason -> Err reason
                Ok ratio ->
                    let r = times half (add t ratio False) in
                    if head compared t.c == head compared r.c then rounded r (round dp + r.e + 1) mode False
                    else iterate compared r
        -- Elm compiles `estimate == 0` to `!estimate`, which NaN also passes: test NaN first.
        initial e estimate =
            if not (isNaN estimate) && (estimate == 0 || estimate == 1/0) then
                -- Math.sqrt underflow/overflow? Re-estimate from the coefficient as an integer,
                -- then set upstream's exponent ((e + 1) / 2 | 0) - (e < 0 || e & 1). Upstream
                -- parses toExponential()'s digits before its `e`: the shortest digits, the ones
                -- String() prints, placed at that exponent.
                let
                    digits = joinDigits x.c
                    padded = if Bitwise.and 1 (String.length digits + e) == 0 then digits ++ "0" else digits
                    fallback = sqrt (number padded)
                    exponent = truncate ((toFloat e + 1) / 2) - (if e < 0 || Bitwise.and 1 e /= 0 then 1 else 0)
                in
                if fallback == 1/0 then Ok (single 1 exponent 5)
                else parse (String.fromFloat fallback) |> Result.map (\r -> {r | e=exponent})
            -- A NaN estimate prints as "NaN", which upstream rejects as an invalid number.
            else parse (String.fromFloat estimate)
    in
    if first x.c==0 then Ok x
    else if x.sign<0 then Err "[big.js] No square root"
    else initial x.e (sqrt (number (stringify x True True))) |> Result.andThen (\r -> iterate (toFloat r.e + places) r)


operate op xText yText n dp rm mode =
    let
        decimalResult x =
            case op of
                "plus" -> parse yText |> Result.map (\y -> add x y False)
                "minus" -> parse yText |> Result.map (\y -> add x y True)
                "times" -> parse yText |> Result.map (times x)
                "div" -> parse yText |> Result.andThen (\y -> divide x y dp rm)
                "sqrt" -> root x dp rm
                "pow" -> power x n dp rm
                "round" ->
                    case intIn -1000000 1000000 (Maybe.withDefault 0 n) of
                        Nothing -> Err "[big.js] Invalid decimal places"
                        Just places -> rounded x (places+x.e+1) mode False
                _ -> Err "unknown decimal operation"
        evaluate x =
            case op of
                "cmp" -> parse yText |> Result.map (cmp x >> String.fromInt)
                "toString" -> render x op Nothing mode
                "toFixed" -> render x op n mode
                "toExponential" -> render x op n mode
                "toPrecision" -> render x op n mode
                _ -> decimalResult x |> Result.andThen (\v -> render v "toString" Nothing mode)
    in
    case Result.andThen evaluate (parse xText) of
        Ok value -> value
        Err reason -> reason
