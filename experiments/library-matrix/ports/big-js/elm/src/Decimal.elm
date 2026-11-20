module Decimal exposing (operate)



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


normalize sign exponent digits =
    let
        trimmed = trimLeft digits
        shift = List.length digits - List.length trimmed
    in
    if List.isEmpty trimmed then single sign 0 0
    else Decimal sign (exponent - shift) (trimLeft (List.reverse trimmed) |> List.reverse)


validDigits s =
    String.all (\c -> c >= '0' && c <= '9') s


parse text =
    let
        neg = String.startsWith "-" text
        body = String.toLower (if neg then String.dropLeft 1 text else text)
        number sign expr mantissa =
            let
                parts = String.split "." mantissa
                all = String.concat parts
                digits = String.toList all |> List.map (\c -> Char.toCode c - 48)
                integral = List.head parts |> Maybe.withDefault ""
            in
            if List.length parts > 2 || String.isEmpty all || not (validDigits all) then Err "[big.js] Invalid number"
            else Ok (normalize sign (expr + String.length integral - 1) digits)
        signum = if neg then -1 else 1
    in
    case String.split "e" body of
        [m] -> number signum 0 m
        [m,e] ->
            let
                plain = if String.startsWith "-" e || String.startsWith "+" e then String.dropLeft 1 e else e
            in
            if String.isEmpty plain || not (validDigits plain) then Err "[big.js] Invalid number"
            else case String.toInt e of
                Just exponent -> number signum exponent m
                Nothing -> Err "[big.js] Invalid number"
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


add x y subtract =
    let
        sign = if subtract then -y.sign else y.sign
        same = x.sign == sign
        reverse = not same && cmp x {y | sign=x.sign} * x.sign < 0
        top = max x.e y.e
        bottom = min (x.e - List.length x.c + 1) (y.e - List.length y.c + 1)
        aligned a = List.repeat (top-a.e) 0 ++ a.c ++ List.repeat (a.e - List.length a.c + 1 - bottom) 0
        aDigits = aligned x
        bDigits = aligned y
        total = if reverse then combine True bDigits aDigits else combine (not same) aDigits bDigits
    in
    if first x.c == 0 then {y | sign=if first y.c == 0 && sign /= x.sign then 1 else sign}
    else if first y.c == 0 then x
    else normalize (if reverse then -x.sign else x.sign) (top+1) total


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
    in
    normalize (x.sign*y.sign) (x.e+y.e+1) product


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
    if mode<0 || mode>3 then Err "[big.js] Invalid rounding mode"
    else if sd>=len then Ok x
    else if sd<=0 then Ok (if up then single x.sign (x.e-sd+1) 1 else single x.sign 0 0)
    else Ok (normalize x.sign (x.e+1) (leading::tail))


divide x y dp mode =
    let
        sign = x.sign*y.sign
        divisorLen = List.length y.c
        subtract rest count =
            let length = List.length rest in
            if length < divisorLen || (length==divisorLen && rest < y.c) then (rest,count)
            else
                subtract (trimLeft (combine True rest (List.repeat (length-divisorLen) 0 ++ y.c))) (count+1)
        loop rest remaining exponent rev count =
            let
                (left,q) = subtract rest 0
                leading = q==0 && count==0
                e = if leading then exponent-1 else exponent
                digits = if leading then rev else q::rev
                n = if leading then count else count+1
                more = not (List.isEmpty left && List.isEmpty remaining)
                precision = dp+e+1
            in
            if n>0 && (not more || n>precision) then
                let answer = Decimal sign e (List.reverse digits) in
                if n>precision then rounded answer precision mode more else Ok answer
            else
                let
                    d = first remaining
                    appended = if d==0 && List.isEmpty left then [] else left++[d]
                in
                loop appended (List.drop 1 remaining) e digits n
        start = List.take divisorLen (x.c ++ List.repeat (divisorLen - List.length x.c) 0)
    in
    if dp<0 || dp>1000000 then Err "[big.js] Invalid decimal places"
    else if first y.c==0 then Err "[big.js] Division by zero"
    else if first x.c==0 then Ok (single sign 0 0)
    else loop start (List.drop divisorLen x.c) (x.e-y.e) [] 0


power x n dp mode =
    let
        one = single 1 0 1
        loop i base acc =
            if i==0 then acc
            else
                let next = if modBy 2 i == 1 then times acc base else acc in
                if i<2 then next else loop (i//2) (times base base) next
    in
    if abs n > 1000000 then Err "[big.js] Invalid exponent"
    else
        let positive=loop (abs n) x one in
        if n<0 then divide one positive dp mode else Ok positive


joinDigits digits = String.concat (List.map String.fromInt digits)


stringify x exponential nonzero =
    let
        s = joinDigits x.c
        e = x.e
        rendered =
            if exponential then
                String.left 1 s ++ (if String.length s>1 then "."++String.dropLeft 1 s else "") ++ (if e>=0 then "e+" else "e") ++ String.fromInt e
            else if e<0 then "0."++String.repeat (-e-1) "0"++s
            else if e+1<String.length s then String.left (e+1) s ++ "." ++ String.dropLeft (e+1) s
            else s ++ String.repeat (e+1-String.length s) "0"
    in
    (if x.sign<0 && nonzero then "-" else "") ++ rendered


render x op n mode =
    let
        formatted v =
            let exp = op=="toExponential" || (op/="toFixed" && (v.e <= -7 || v.e>=21 || (op=="toPrecision" && Maybe.withDefault False (Maybe.map (\k -> k<=v.e) n)))) in
            stringify v exp (first x.c/=0)
    in
    case n of
        Nothing -> Ok (formatted x)
        Just k ->
            if op=="toPrecision" && (k<1 || k>1000000) then Err "[big.js] Invalid precision"
            else if k<0 || k>1000000 then Err "[big.js] Invalid decimal places"
            else
                let
                    sd = if op=="toFixed" then k+x.e+1 else if op=="toPrecision" then k else k+1
                    pad v =
                        let width = if op=="toFixed" then k+v.e+1 else sd in
                        formatted {v | c=v.c ++ List.repeat (width-List.length v.c) 0}
                in
                Result.map pad (rounded x sd mode False)


root x dp mode =
    let
        machine text = String.toFloat text |> Maybe.withDefault (1/0) |> sqrt
        estimate = machine (stringify x True True)
        initial =
            if estimate>0 && not (isInfinite estimate) then parse (String.fromFloat estimate)
            else
                let
                    padded = joinDigits x.c ++ (if modBy 2 (List.length x.c + x.e)==0 then "0" else "")
                    fallback = machine padded
                    base = if isInfinite fallback then Ok (single 1 0 5) else parse (String.fromFloat fallback)
                in
                Result.map (\r -> {r | e=floor (toFloat x.e / 2)}) base
        refine start =
            let
                work = max (dp+4) (5-start.e)
                count = max 1 (start.e+work)
                loop current =
                    case divide x current work mode of
                        Err reason -> Err reason
                        Ok ratio ->
                            let next = times (single 1 -1 5) (add current ratio False) in
                            if current.e==next.e && List.take count current.c==List.take count next.c then rounded next (dp+next.e+1) mode False
                            else loop next
            in
            loop start
    in
    if first x.c==0 then Ok x
    else if x.sign<0 then Err "[big.js] No square root"
    else if dp >= 0 && dp <= 999996 && mode >= 0 && mode <= 3 && x.e < -2 * dp - 2 then
        Ok (if mode == 3 then single 1 -dp 1 else single 1 0 0)
    else Result.andThen refine initial


operate op xText yText n dp rm mode =
    let
        decimalResult x =
            case op of
                "plus" -> parse yText |> Result.map (\y -> add x y False)
                "minus" -> parse yText |> Result.map (\y -> add x y True)
                "times" -> parse yText |> Result.map (times x)
                "div" -> parse yText |> Result.andThen (\y -> divide x y dp rm)
                "sqrt" -> root x dp rm
                "pow" -> power x (Maybe.withDefault -2000002 n) dp rm
                "round" ->
                    let places=Maybe.withDefault 0 n in
                    if abs places>1000000 then Err "[big.js] Invalid decimal places"
                    else rounded x (places+x.e+1) mode False
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
