module Pilot exposing (analyse)

{-| Word frequencies, FNV-1a and a fixed-decimal summary, as specified in
libraries/pilot/CONTRACT.md. Elm strings fold over Chars, which are code points.
-}

import Bitwise
import Dict exposing (Dict)


type alias Entry =
    { word : String, count : Int, length : Int, first : Int }


type alias State =
    { entries : Dict String Entry
    , tokens : Int
    , codePoints : Int
    , word : List Char
    , length : Int
    }


isWordCode : Int -> Bool
isWordCode cp =
    cp >= 0x80 || (cp >= 0x30 && cp <= 0x39) || (cp >= 0x41 && cp <= 0x5A) || (cp >= 0x61 && cp <= 0x7A)


flush : State -> State
flush state =
    if state.length == 0 then
        state

    else
        let
            key =
                String.fromList (List.reverse state.word)

            entries =
                case Dict.get key state.entries of
                    Just entry ->
                        Dict.insert key { entry | count = entry.count + 1 } state.entries

                    Nothing ->
                        Dict.insert key (Entry key 1 state.length (Dict.size state.entries)) state.entries
        in
        { entries = entries
        , tokens = state.tokens + 1
        , codePoints = state.codePoints + state.length
        , word = []
        , length = 0
        }


step : Char -> State -> State
step char state =
    let
        cp =
            Char.toCode char
    in
    if isWordCode cp then
        -- ASCII-only case folding.
        { state
            | word =
                (if cp >= 0x41 && cp <= 0x5A then
                    Char.fromCode (cp + 0x20)

                 else
                    char
                )
                    :: state.word
            , length = state.length + 1
        }

    else
        flush state


{-| The UTF-8 bytes of one code point.
-}
utf8 : Int -> List Int
utf8 cp =
    if cp < 0x80 then
        [ cp ]

    else if cp < 0x0800 then
        [ Bitwise.or 0xC0 (Bitwise.shiftRightBy 6 cp), Bitwise.or 0x80 (Bitwise.and cp 0x3F) ]

    else if cp < 0x00010000 then
        [ Bitwise.or 0xE0 (Bitwise.shiftRightBy 12 cp)
        , Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightBy 6 cp) 0x3F)
        , Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        ]

    else
        [ Bitwise.or 0xF0 (Bitwise.shiftRightBy 18 cp)
        , Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightBy 12 cp) 0x3F)
        , Bitwise.or 0x80 (Bitwise.and (Bitwise.shiftRightBy 6 cp) 0x3F)
        , Bitwise.or 0x80 (Bitwise.and cp 0x3F)
        ]


{-| One FNV-1a 32 step. Elm Ints are JS numbers, so `hash * 16777619` could pass 2^53 and lose
bits; the prime is 2^24 + 403, so the product is a 32-bit shift plus an exact small multiply,
wrapped unsigned with a zero-fill shift.
-}
fnvByte : Int -> Int -> Int
fnvByte byte hash =
    let
        x =
            Bitwise.xor hash byte
    in
    Bitwise.shiftRightZfBy 0 (Bitwise.shiftLeftBy 24 x + x * 403)


fnv1a : String -> Int
fnv1a text =
    String.foldl (\char hash -> List.foldl fnvByte hash (utf8 (Char.toCode char))) 0x811C9DC5 text


hexDigit : Int -> Char
hexDigit d =
    if d < 10 then
        Char.fromCode (0x30 + d)

    else
        Char.fromCode (0x57 + d)


hex8 : Int -> String
hex8 value =
    List.range 0 7
        |> List.map (\i -> hexDigit (Bitwise.and (Bitwise.shiftRightZfBy (4 * (7 - i)) value) 0x0F))
        |> String.fromList


{-| Fixed three decimals of a non-negative number; `round` rounds ties up, like JS toFixed.
-}
fixed3 : Float -> String
fixed3 value =
    let
        scaled =
            round (value * 1000)
    in
    String.fromInt (scaled // 1000) ++ "." ++ String.padLeft 3 '0' (String.fromInt (modBy 1000 scaled))


compareEntries : Entry -> Entry -> Order
compareEntries a b =
    case compare b.count a.count of
        EQ ->
            case compare b.length a.length of
                EQ ->
                    compare a.first b.first

                order ->
                    order

        order ->
            order


analyse : String -> Int -> String
analyse text top =
    let
        state =
            String.foldl step { entries = Dict.empty, tokens = 0, codePoints = 0, word = [], length = 0 } text
                |> flush

        ranked =
            List.sortWith compareEntries (Dict.values state.entries)

        mean =
            if state.tokens == 0 then
                0

            else
                toFloat state.codePoints / toFloat state.tokens

        row entry =
            String.join " "
                [ entry.word
                , String.fromInt entry.count
                , hex8 (fnv1a entry.word)
                , fixed3 (toFloat entry.count / toFloat state.tokens)
                ]
    in
    String.join "\n"
        ([ "tokens " ++ String.fromInt state.tokens
         , "distinct " ++ String.fromInt (Dict.size state.entries)
         , "mean " ++ fixed3 mean
         , "fnv " ++ hex8 (fnv1a text)
         ]
            ++ List.map row (List.take top ranked)
        )
