module Utf8 exposing (bytesFromUnits)

import Array exposing (Array)
import Bitwise


{-| Encode UTF-16 code units the way `TextEncoder` does: unpaired surrogates
become U+FFFD. Units come from the JS host (`charCodeAt`) because Elm's
`String` walker pairs every high surrogate with the next unit.
-}
bytesFromUnits : List Int -> Array Int
bytesFromUnits units =
    walk units Array.empty


walk : List Int -> Array Int -> Array Int
walk units acc =
    case units of
        [] ->
            acc

        unit :: rest ->
            if unit < 0xD800 || unit > 0xDFFF then
                walk rest (pushCode acc unit)

            else if unit <= 0xDBFF then
                case rest of
                    low :: rest2 ->
                        if low >= 0xDC00 && low <= 0xDFFF then
                            let
                                cp =
                                    0x00010000
                                        + Bitwise.shiftLeftBy 10 (unit - 0xD800)
                                        + (low - 0xDC00)
                            in
                            walk rest2 (pushCode acc cp)

                        else
                            walk rest (pushCode acc 0xFFFD)

                    [] ->
                        pushCode acc 0xFFFD

            else
                walk rest (pushCode acc 0xFFFD)


pushCode : Array Int -> Int -> Array Int
pushCode acc cp =
    List.foldl Array.push acc (encodeCodePoint cp)


encodeCodePoint : Int -> List Int
encodeCodePoint cp =
    if cp < 0x80 then
        [ cp ]

    else if cp < 0x800 then
        [ Bitwise.or 0xC0 (Bitwise.shiftRightZfBy 6 cp)
        , Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        ]

    else if cp < 0x00010000 then
        [ Bitwise.or 0xE0 (Bitwise.shiftRightZfBy 12 cp)
        , Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightZfBy 6 cp) 0x3F)
        , Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        ]

    else
        [ Bitwise.or 0xF0 (Bitwise.shiftRightZfBy 18 cp)
        , Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightZfBy 12 cp) 0x3F)
        , Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightZfBy 6 cp) 0x3F)
        , Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        ]
