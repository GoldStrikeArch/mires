module Codec exposing (strFromU8Utf16Le, strToU8, strToU8Latin1List)

{-| UTF-8 and Latin-1 string codecs matching `fflate@0.8.3`'s `strToU8` and `strFromU8`.

`strFromU8Utf16Le` implements the WHATWG Encoding Standard UTF-8 decoder (single leading UTF-8 BOM
stripped, maximal-subpart `U+FFFD` replacement for ill-formed byte sequences, surrogate pairs for
astral code points) and emits exact UTF-16LE bytes.

-}

import Bitwise
import ByteBuf exposing (ByteBuilder, Bytes, builderPushByte, builderToBytes, emptyBuilder)


strToU8 : String -> Bool -> Bytes
strToU8 str latin1 =
    if latin1 then
        builderToBytes (String.foldl stepLatin1 emptyBuilder str)

    else
        builderToBytes (String.foldl stepUtf8 emptyBuilder str)


strToU8Latin1List : String -> List Int
strToU8Latin1List str =
    let
        step char acc =
            let
                cp =
                    Char.toCode char
            in
            if cp < 0x00010000 then
                Bitwise.and 255 cp :: acc

            else
                let
                    u =
                        cp - 0x00010000

                    hi =
                        Bitwise.and 255 (Bitwise.or 0xD800 (Bitwise.shiftRightZfBy 10 u))

                    lo =
                        Bitwise.and 255 (Bitwise.or 0xDC00 (Bitwise.and 0x03FF u))
                in
                lo :: hi :: acc
    in
    List.reverse (String.foldl step [] str)


stepLatin1 : Char -> ByteBuilder -> ByteBuilder
stepLatin1 char builder =
    let
        cp =
            Char.toCode char
    in
    if cp < 0x00010000 then
        builderPushByte (Bitwise.and 255 cp) builder

    else
        let
            u =
                cp - 0x00010000

            hi =
                Bitwise.or 0xD800 (Bitwise.shiftRightZfBy 10 u)

            lo =
                Bitwise.or 0xDC00 (Bitwise.and 0x03FF u)
        in
        builder
            |> builderPushByte (Bitwise.and 255 hi)
            |> builderPushByte (Bitwise.and 255 lo)


stepUtf8 : Char -> ByteBuilder -> ByteBuilder
stepUtf8 char builder =
    let
        cp =
            Char.toCode char
    in
    if cp < 0x80 then
        builderPushByte cp builder

    else if cp < 0x0800 then
        builder
            |> builderPushByte (Bitwise.or 0xC0 (Bitwise.shiftRightZfBy 6 cp))
            |> builderPushByte (Bitwise.or 0x80 (Bitwise.and 0x3F cp))

    else if cp < 0x00010000 then
        builder
            |> builderPushByte (Bitwise.or 0xE0 (Bitwise.shiftRightZfBy 12 cp))
            |> builderPushByte (Bitwise.or 0x80 (Bitwise.and 0x3F (Bitwise.shiftRightZfBy 6 cp)))
            |> builderPushByte (Bitwise.or 0x80 (Bitwise.and 0x3F cp))

    else
        builder
            |> builderPushByte (Bitwise.or 0xF0 (Bitwise.shiftRightZfBy 18 cp))
            |> builderPushByte (Bitwise.or 0x80 (Bitwise.and 0x3F (Bitwise.shiftRightZfBy 12 cp)))
            |> builderPushByte (Bitwise.or 0x80 (Bitwise.and 0x3F (Bitwise.shiftRightZfBy 6 cp)))
            |> builderPushByte (Bitwise.or 0x80 (Bitwise.and 0x3F cp))


pushCodePointUtf16Le : Int -> ByteBuilder -> ByteBuilder
pushCodePointUtf16Le cp builder =
    if cp < 0x00010000 then
        builder
            |> builderPushByte (Bitwise.and 255 cp)
            |> builderPushByte (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 cp))

    else
        let
            u =
                cp - 0x00010000

            hi =
                Bitwise.or 0xD800 (Bitwise.shiftRightZfBy 10 u)

            lo =
                Bitwise.or 0xDC00 (Bitwise.and 0x03FF u)
        in
        builder
            |> builderPushByte (Bitwise.and 255 hi)
            |> builderPushByte (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 hi))
            |> builderPushByte (Bitwise.and 255 lo)
            |> builderPushByte (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 lo))


pushReplacement : ByteBuilder -> ByteBuilder
pushReplacement builder =
    builder
        |> builderPushByte 0xFD
        |> builderPushByte 0xFF


strFromU8Utf16Le : Bytes -> Bool -> Bytes
strFromU8Utf16Le bytes latin1 =
    if latin1 then
        builderToBytes
            (ByteBuf.foldl
                (\b acc -> acc |> builderPushByte b |> builderPushByte 0)
                emptyBuilder
                bytes
            )

    else
        let
            len =
                ByteBuf.length bytes

            startIdx =
                if
                    len
                        >= 3
                        && ByteBuf.getByte 0 bytes
                        == 0xEF
                        && ByteBuf.getByte 1 bytes
                        == 0xBB
                        && ByteBuf.getByte 2 bytes
                        == 0xBF
                then
                    3

                else
                    0

            decode idx cp seen needed lo hi builder =
                if (idx - len) >= 0 then
                    if needed /= 0 then
                        builderToBytes (pushReplacement builder)

                    else
                        builderToBytes builder

                else
                    let
                        b =
                            ByteBuf.getByte idx bytes
                    in
                    if needed == 0 then
                        if b < 0x80 then
                            decode (idx + 1) 0 0 0 0x80 0xBF (builder |> builderPushByte b |> builderPushByte 0)

                        else if b >= 0xC2 && b <= 0xDF then
                            decode (idx + 1) (Bitwise.and 0x1F b) 0 1 0x80 0xBF builder

                        else if b >= 0xE0 && b <= 0xEF then
                            let
                                nextLo =
                                    if b == 0xE0 then
                                        0xA0

                                    else
                                        0x80

                                nextHi =
                                    if b == 0xED then
                                        0x9F

                                    else
                                        0xBF
                            in
                            decode (idx + 1) (Bitwise.and 0x0F b) 0 2 nextLo nextHi builder

                        else if b >= 0xF0 && b <= 0xF4 then
                            let
                                nextLo =
                                    if b == 0xF0 then
                                        0x90

                                    else
                                        0x80

                                nextHi =
                                    if b == 0xF4 then
                                        0x8F

                                    else
                                        0xBF
                            in
                            decode (idx + 1) (Bitwise.and 0x07 b) 0 3 nextLo nextHi builder

                        else
                            decode (idx + 1) 0 0 0 0x80 0xBF (pushReplacement builder)

                    else if (b - lo) < 0 || (b - hi) > 0 then
                        -- Maximal subpart ended: emit U+FFFD and reprocess `b` at same `idx`.
                        decode idx 0 0 0 0x80 0xBF (pushReplacement builder)

                    else
                        let
                            nextCp =
                                Bitwise.or (Bitwise.shiftLeftBy 6 cp) (Bitwise.and 0x3F b)

                            nextSeen =
                                seen + 1
                        in
                        if (nextSeen - needed) == 0 then
                            decode (idx + 1) 0 0 0 0x80 0xBF (pushCodePointUtf16Le nextCp builder)

                        else
                            decode (idx + 1) nextCp nextSeen needed 0x80 0xBF builder
        in
        decode startIdx 0 0 0 0x80 0xBF emptyBuilder
