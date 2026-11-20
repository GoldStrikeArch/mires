module Words exposing (Word, add, bitAnd, bitOr, bitXor, clz, compareWords, ctz, divide, format, modulo, mul, neg, one, parse, rotl, shl, shr, sub, zero)

import Bitwise as B
import Char
import String

type alias Word =
    { lo : Int, hi : Int }

zero = Word 0 0
one = Word 1 0

unsigned : Int -> Int
unsigned = B.shiftRightZfBy 0

low16 : Int -> Int
low16 = B.and 65535

top16 : Int -> Int
top16 = B.shiftRightZfBy 16

pack : Int -> Int -> Int
pack hi lo = B.or (B.shiftLeftBy 16 hi) (low16 lo)

add a b =
    let
        low = unsigned a.lo + unsigned b.lo
        carry = if low >= 4294967296 then 1 else 0
    in
    Word (B.or 0 low) (B.or 0 (a.hi + b.hi + carry))

neg a =
    Word (B.or 0 -a.lo) (B.or 0 (-a.hi - (if a.lo == 0 then 0 else 1)))

sub a b = add a (neg b)
bitAnd a b = Word (B.and a.lo b.lo) (B.and a.hi b.hi)
bitOr a b = Word (B.or a.lo b.lo) (B.or a.hi b.hi)
bitXor a b = Word (B.xor a.lo b.lo) (B.xor a.hi b.hi)

mul a b =
    let
        a0 = low16 a.lo
        a1 = top16 a.lo
        a2 = low16 a.hi
        a3 = top16 a.hi
        b0 = low16 b.lo
        b1 = top16 b.lo
        b2 = low16 b.hi
        b3 = top16 b.hi
        c0 = a0*b0
        c1 = c0 // 65536 + a1*b0 + a0*b1
        c2 = c1 // 65536 + a2*b0 + a1*b1 + a0*b2
        c3 = c2 // 65536 + a3*b0 + a2*b1 + a1*b2 + a0*b3
    in
    Word (pack c1 c0) (pack c3 c2)

shl n a =
    let k = B.and n 63 in
    if k == 0 then a
    else if k >= 32 then Word 0 (B.shiftLeftBy (k-32) a.lo)
    else Word (B.shiftLeftBy k a.lo) (B.or (B.shiftLeftBy k a.hi) (B.shiftRightZfBy (32-k) a.lo))

shr arithmetic n a =
    let
        k = B.and n 63
        shift i = if arithmetic then B.shiftRightBy i a.hi else B.or 0 (B.shiftRightZfBy i a.hi)
    in
    if k == 0 then a
    else if k >= 32 then Word (shift (k-32)) (if arithmetic && a.hi < 0 then -1 else 0)
    else Word (B.or (B.shiftRightZfBy k a.lo) (B.shiftLeftBy (32-k) a.hi)) (shift k)

rotl n a = bitOr (shl n a) (shr False -n a)

clz32 x i =
    if x < 0 || i == 32 then i else clz32 (B.shiftLeftBy 1 x) (i+1)

ctz32 x i =
    if B.and x 1 /= 0 || i == 32 then i else ctz32 (B.shiftRightZfBy 1 x) (i+1)

clz a = if a.hi == 0 then 32 + clz32 a.lo 0 else clz32 a.hi 0
ctz a = if a.lo == 0 then 32 + ctz32 a.hi 0 else ctz32 a.lo 0

less a b = unsigned a.hi < unsigned b.hi || (a.hi == b.hi && unsigned a.lo < unsigned b.lo)

compareWords a b u v =
    let
        an = not u && a.hi < 0
        bn = not v && b.hi < 0
    in
    if a == b && (u == v || a.hi >= 0) then 0
    else if (if an /= bn then an else if u then less a b else (sub a b).hi < 0) then -1
    else 1

-- Short division in base 65536. Each quotient limb fits in 16 bits and
-- the running remainder times 65536 fits in JavaScript's exact integers.
smallDiv d a =
    let
        h = top16 a.hi
        i = remainderBy d h * 65536 + low16 a.hi
        j = remainderBy d i * 65536 + top16 a.lo
        k = remainderBy d j * 65536 + low16 a.lo
    in
    ( Word (pack (j // d) (k // d)) (pack (h // d) (i // d)), remainderBy d k )

wideDiv b bit q rest =
    if bit < 0 then (q,rest) else
    let shifted = shl bit b in
    if less rest shifted then wideDiv b (bit-1) q rest
    else wideDiv b (bit-1) (bitOr q (shl bit one)) (sub rest shifted)

divmod a b u =
    let
        an = not u && a.hi < 0
        bn = not u && b.hi < 0
        mag = if an then neg a else a
        divisor = if bn then neg b else b
        (q,r) =
            if divisor.hi == 0 then
                let (result,small) = smallDiv (unsigned divisor.lo) mag in (result,Word (B.or 0 small) 0)
            else
                wideDiv divisor (clz divisor - clz mag) zero mag
    in
    (if an /= bn then neg q else q, if an then neg r else r)

divide a b u =
    if not u && a.hi == -2147483648 && b.hi == -1 && b.lo == -1 then a
    else Tuple.first (divmod a b u)

modulo a b u = Tuple.second (divmod a b u)

radix n = if n == 0 then 10 else n

digitChar d = Char.fromCode (if d < 10 then d+48 else d+87)

smallString base x tail =
    let
        text = String.cons (digitChar (remainderBy base x)) tail
        q = x // base
    in
    if q == 0 then text else smallString base q text

format a u n =
    let
        r = radix n
        negative = not u && a.hi < 0
        power = r ^ 6
        groups mag tail =
            let
                (q,rem) = smallDiv power mag
                raw = smallString r rem ""
            in
            if q == zero then raw ++ tail else groups q (String.padLeft 6 '0' raw ++ tail)
    in
    if r < 2 || r > 36 then "!"
    else if negative then "-" ++ groups (neg a) ""
    else groups a ""

digitValue c =
    let k = Char.toCode (Char.toLower c) in
    if k >= 48 && k <= 57 then k-48 else if k >= 97 && k <= 122 then k-87 else 36

parseDigits chars r part =
    case chars of
        [] -> part
        c :: rest ->
            let d = digitValue c in
            if d >= r then part else parseDigits rest r (add (mul part (Word r 0)) (Word d 0))

parseGroups text r acc =
    if String.isEmpty text then acc else
    let
        group = String.left 8 text
        trim = String.trimLeft group
        plus = if String.startsWith "+" trim then String.dropLeft 1 trim else trim
        raw = if r == 16 && String.startsWith "0x" (String.toLower plus) then String.dropLeft 2 plus else plus
        scaled = List.foldl (\_ a -> mul a (Word r 0)) acc (String.toList group)
    in
    parseGroups (String.dropLeft 8 text) r (add scaled (parseDigits (String.toList raw) r zero))

parse text base =
    let r = radix base in
    if String.isEmpty text then Nothing
    else if List.member text ["NaN","Infinity","+Infinity","-Infinity"] then Just zero
    else if r < 2 || r > 36 then Nothing
    else if String.startsWith "-" text then Maybe.map neg (parse (String.dropLeft 1 text) r)
    else if String.contains "-" text then Nothing
    else Just (parseGroups text r zero)
