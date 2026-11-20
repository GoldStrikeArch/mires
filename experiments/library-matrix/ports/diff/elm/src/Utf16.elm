module Utf16 exposing (toList)

-- Elm's core string folds assume every surrogate has a partner. Split at
-- code-unit boundaries and combine only actual high/low pairs.
toList : String -> List Char
toList s =
    let
        unit n =
            String.fromChar (Char.fromCode n)

        charOf part =
            String.uncons part |> Maybe.map Tuple.first |> Maybe.withDefault ' '

        -- Pair the high unit with a known low unit to recover its code.
        loneHigh part =
            let
                cp =
                    Char.toCode (charOf (part ++ unit 0xDC00))
            in
            Char.fromCode (0xD800 + (cp - 0x00010000) // 1024)

        loop units rev =
            case units of
                [] ->
                    List.reverse rev

                c :: rest ->
                    if c >= unit 0xD800 && c <= unit 0xDBFF then
                        case rest of
                            d :: tail ->
                                if d >= unit 0xDC00 && d <= unit 0xDFFF then
                                    loop tail (charOf (c ++ d) :: rev)

                                else
                                    loop rest (loneHigh c :: rev)

                            [] ->
                                List.reverse (loneHigh c :: rev)

                    else
                        loop rest (charOf c :: rev)
    in
    loop (String.split "" s) []
