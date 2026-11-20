module Words exposing (at, bytesBE, bytesLE, rotr, wordsBE, wordsLE)

{-| Byte and word conversions, standing in for noble's DataView and Uint32Array views. Bytes are
Ints in 0..255; words are 32-bit Ints.
-}

import Array exposing (Array)
import Bitwise exposing (and, or, shiftLeftBy, shiftRightZfBy)


{-| noble's rotr for 32-bit words.
-}
rotr : Int -> Int -> Int
rotr word shift =
    or (shiftLeftBy (32 - shift) word) (shiftRightZfBy shift word)


{-| The entry at `index` of a table whose bounds the caller knows.
-}
at : Int -> Array Int -> Int
at index table =
    Array.get index table |> Maybe.withDefault 0


{-| The big-endian 32-bit words of `bytes`, whose length is a multiple of 4 (DataView.getUint32).
-}
wordsBE : List Int -> List Int
wordsBE bytes =
    wordsHelp (\b0 b1 b2 b3 -> or (shiftLeftBy 24 b0) (or (shiftLeftBy 16 b1) (or (shiftLeftBy 8 b2) b3))) bytes []


{-| The little-endian 32-bit words of `bytes`, as noble's Uint32Array views read them.
-}
wordsLE : List Int -> List Int
wordsLE bytes =
    wordsHelp (\b0 b1 b2 b3 -> or b0 (or (shiftLeftBy 8 b1) (or (shiftLeftBy 16 b2) (shiftLeftBy 24 b3)))) bytes []


wordsHelp : (Int -> Int -> Int -> Int -> Int) -> List Int -> List Int -> List Int
wordsHelp word bytes acc =
    case bytes of
        b0 :: b1 :: b2 :: b3 :: rest ->
            wordsHelp word rest (word b0 b1 b2 b3 :: acc)

        _ ->
            List.reverse acc


{-| The big-endian bytes of a word (DataView.setUint32).
-}
bytesBE : Int -> List Int
bytesBE word =
    [ byte 24 word, byte 16 word, byte 8 word, byte 0 word ]


{-| The little-endian bytes of a word.
-}
bytesLE : Int -> List Int
bytesLE word =
    [ byte 0 word, byte 8 word, byte 16 word, byte 24 word ]


byte : Int -> Int -> Int
byte shift word =
    and 255 (shiftRightZfBy shift word)
