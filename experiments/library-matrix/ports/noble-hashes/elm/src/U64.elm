module U64 exposing
    ( Word, add, add3H, add3L, add4H, add4L, add5H, add5L, fromNumH, fromNumL
    , rotr32H, rotr32L, rotrBH, rotrBL, rotrSH, rotrSL, shrSH, shrSL
    )

{-| noble's \_u64.ts: a 64-bit word is a high and a low 32-bit half, each an Int.

Elm's Int is a JavaScript number, as noble's are, so the sums are noble's to the letter: the low
halves add up exactly, and the carry into the high half is that sum divided by 2^32 with `//`,
which is noble's `(low / 2 ** 32) | 0`.

-}

import Bitwise exposing (or, shiftLeftBy, shiftRightZfBy)


{-| The halves of a 64-bit word, as noble's `add` returns them.
-}
type alias Word =
    { h : Int, l : Int }


{-| `n >>> 0`: the unsigned value of a 32-bit half.
-}
unsigned : Int -> Int
unsigned =
    shiftRightZfBy 0


{-| noble's fromNumH: the high half of an integer below 2^53.
-}
fromNumH : Int -> Int
fromNumH n =
    n // 4294967296


{-| noble's fromNumL (`n >>> 0`): the low half of an integer below 2^53.
-}
fromNumL : Int -> Int
fromNumL =
    unsigned


{-| High half of a 64-bit logical right shift by `s` in 0..31.
-}
shrSH : Int -> Int -> Int -> Int
shrSH h _ s =
    shiftRightZfBy s h


{-| Low half of a 64-bit logical right shift by `s` in 1..31.
-}
shrSL : Int -> Int -> Int -> Int
shrSL h l s =
    or (shiftLeftBy (32 - s) h) (shiftRightZfBy s l)


{-| High half of a 64-bit right rotation by `s` in 1..31.
-}
rotrSH : Int -> Int -> Int -> Int
rotrSH h l s =
    or (shiftRightZfBy s h) (shiftLeftBy (32 - s) l)


{-| Low half of a 64-bit right rotation by `s` in 1..31.
-}
rotrSL : Int -> Int -> Int -> Int
rotrSL h l s =
    or (shiftLeftBy (32 - s) h) (shiftRightZfBy s l)


{-| High half of a 64-bit right rotation by `s` in 33..63.
-}
rotrBH : Int -> Int -> Int -> Int
rotrBH h l s =
    or (shiftLeftBy (64 - s) h) (shiftRightZfBy (s - 32) l)


{-| Low half of a 64-bit right rotation by `s` in 33..63.
-}
rotrBL : Int -> Int -> Int -> Int
rotrBL h l s =
    or (shiftRightZfBy (s - 32) h) (shiftLeftBy (64 - s) l)


{-| High half of a 64-bit right rotation by 32: the low half.
-}
rotr32H : Int -> Int -> Int
rotr32H _ l =
    l


{-| Low half of a 64-bit right rotation by 32: the high half.
-}
rotr32L : Int -> Int -> Int
rotr32L h _ =
    h


{-| The sum of two 64-bit words.
-}
add : Int -> Int -> Int -> Int -> Word
add ah al bh bl =
    let
        l =
            unsigned al + unsigned bl
    in
    { h = or 0 (ah + bh + l // 4294967296), l = or 0 l }


{-| The unmasked sum of three low halves, for `add3H`.
-}
add3L : Int -> Int -> Int -> Int
add3L al bl cl =
    unsigned al + unsigned bl + unsigned cl


{-| The high half of a three-word sum; `low` is the unmasked `add3L`.
-}
add3H : Int -> Int -> Int -> Int -> Int
add3H low ah bh ch =
    or 0 (ah + bh + ch + low // 4294967296)


{-| The unmasked sum of four low halves, for `add4H`.
-}
add4L : Int -> Int -> Int -> Int -> Int
add4L al bl cl dl =
    unsigned al + unsigned bl + unsigned cl + unsigned dl


{-| The high half of a four-word sum; `low` is the unmasked `add4L`.
-}
add4H : Int -> Int -> Int -> Int -> Int -> Int
add4H low ah bh ch dh =
    or 0 (ah + bh + ch + dh + low // 4294967296)


{-| The unmasked sum of five low halves, for `add5H`.
-}
add5L : Int -> Int -> Int -> Int -> Int -> Int
add5L al bl cl dl el =
    unsigned al + unsigned bl + unsigned cl + unsigned dl + unsigned el


{-| The high half of a five-word sum; `low` is the unmasked `add5L`.
-}
add5H : Int -> Int -> Int -> Int -> Int -> Int -> Int
add5H low ah bh ch dh eh =
    or 0 (ah + bh + ch + dh + eh + low // 4294967296)
