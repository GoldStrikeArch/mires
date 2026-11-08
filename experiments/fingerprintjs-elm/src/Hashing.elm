module Hashing exposing (x64hash128, x64hash128FromUnits)

{-| MurmurHash3 x64 128, matching FingerprintJS 5.2.0 / murmurHash3.js.
-}

import Array exposing (Array)
import Bitwise
import Utf8


type alias X64 =
    { high : Int
    , low : Int
    }


x64 : Int -> Int -> X64
x64 high low =
    { high = high, low = low }


zero : X64
zero =
    x64 0 0


c1 : X64
c1 =
    x64 0x87C37B91 0x114253D5


c2 : X64
c2 =
    x64 0x4CF5AD43 0x2745937F


f1 : X64
f1 =
    x64 0xFF51AFD7 0xED558CCD


f2 : X64
f2 =
    x64 0xC4CEB9FE 0x1A85EC53


mConst : X64
mConst =
    x64 0 5


n1 : X64
n1 =
    x64 0 0x52DCE729


n2 : X64
n2 =
    x64 0 0x38495AB5


lsr : Int -> Int -> Int
lsr value bits =
    Bitwise.shiftRightZfBy bits value


lsl : Int -> Int -> Int
lsl value bits =
    Bitwise.shiftLeftBy bits value


land16 : Int -> Int
land16 value =
    Bitwise.and value 0xFFFF


add : X64 -> X64 -> X64
add left right =
    let
        m0 =
            lsr left.high 16

        m1 =
            land16 left.high

        m2 =
            lsr left.low 16

        m3 =
            land16 left.low

        r0 =
            lsr right.high 16

        r1 =
            land16 right.high

        r2 =
            lsr right.low 16

        r3 =
            land16 right.low

        o3 =
            m3 + r3

        o2a =
            lsr o3 16 + m2 + r2

        o1a =
            lsr o2a 16 + m1 + r1

        o0a =
            lsr o1a 16 + m0 + r0
    in
    { high = Bitwise.or (lsl (land16 o0a) 16) (land16 o1a)
    , low = Bitwise.or (lsl (land16 o2a) 16) (land16 o3)
    }


multiply : X64 -> X64 -> X64
multiply left right =
    let
        m0 =
            lsr left.high 16

        m1 =
            land16 left.high

        m2 =
            lsr left.low 16

        m3 =
            land16 left.low

        r0 =
            lsr right.high 16

        r1 =
            land16 right.high

        r2 =
            lsr right.low 16

        r3 =
            land16 right.low

        o3 =
            m3 * r3

        o2a =
            lsr o3 16 + m2 * r3

        o1 =
            lsr o2a 16

        o2b =
            land16 o2a + m3 * r2

        o1b =
            o1 + lsr o2b 16

        o2c =
            land16 o2b

        o1c =
            o1b + m1 * r3

        o0 =
            lsr o1c 16

        o1d =
            land16 o1c + m2 * r2

        o0b =
            o0 + lsr o1d 16

        o1e =
            land16 o1d + m3 * r1

        o0c =
            o0b + lsr o1e 16

        o1f =
            land16 o1e

        o0d =
            land16 (o0c + m0 * r3 + m1 * r2 + m2 * r1 + m3 * r0)
    in
    { high = Bitwise.or (lsl o0d 16) o1f
    , low = Bitwise.or (lsl o2c 16) (land16 o3)
    }


rotl : X64 -> Int -> X64
rotl value bits0 =
    let
        bits =
            modBy 64 bits0

        high =
            value.high

        low =
            value.low
    in
    if bits == 0 then
        value

    else if bits == 32 then
        { high = low, low = high }

    else if bits < 32 then
        { high = Bitwise.or (lsl high bits) (lsr low (32 - bits))
        , low = Bitwise.or (lsl low bits) (lsr high (32 - bits))
        }

    else
        let
            shifted =
                bits - 32
        in
        { high = Bitwise.or (lsl low shifted) (lsr high (32 - shifted))
        , low = Bitwise.or (lsl high shifted) (lsr low (32 - shifted))
        }


xor : X64 -> X64 -> X64
xor left right =
    { high = Bitwise.xor left.high right.high
    , low = Bitwise.xor left.low right.low
    }


xorByte : X64 -> Int -> Int -> X64
xorByte value byte shift =
    if shift < 32 then
        { value | low = Bitwise.xor value.low (lsl byte shift) }

    else
        { value | high = Bitwise.xor value.high (lsl byte (shift - 32)) }


finalMix : X64 -> X64
finalMix value =
    let
        step1 =
            xor value (x64 0 (lsr value.high 1))

        step2 =
            multiply step1 f1

        step3 =
            xor step2 (x64 0 (lsr step2.high 1))

        step4 =
            multiply step3 f2
    in
    xor step4 (x64 0 (lsr step4.high 1))


u32le : Array Int -> Int -> Int
u32le bytes index =
    let
        b0 =
            Array.get index bytes |> Maybe.withDefault 0

        b1 =
            Array.get (index + 1) bytes |> Maybe.withDefault 0

        b2 =
            Array.get (index + 2) bytes |> Maybe.withDefault 0

        b3 =
            Array.get (index + 3) bytes |> Maybe.withDefault 0
    in
    b0
        |> Bitwise.or (lsl b1 8)
        |> Bitwise.or (lsl b2 16)
        |> Bitwise.or (lsl b3 24)


mixBlock : Array Int -> Int -> X64 -> X64 -> ( X64, X64 )
mixBlock bytes index h1 h2 =
    let
        k1a =
            x64 (u32le bytes (index + 4)) (u32le bytes index)

        k2a =
            x64 (u32le bytes (index + 12)) (u32le bytes (index + 8))

        k1b =
            multiply k1a c1

        k1c =
            rotl k1b 31

        k1d =
            multiply k1c c2

        h1a =
            xor h1 k1d

        h1b =
            rotl h1a 27

        h1c =
            add h1b h2

        h1d =
            multiply h1c mConst

        h1e =
            add h1d n1

        k2b =
            multiply k2a c2

        k2c =
            rotl k2b 33

        k2d =
            multiply k2c c1

        h2a =
            xor h2 k2d

        h2b =
            rotl h2a 31

        h2c =
            add h2b h1e

        h2d =
            multiply h2c mConst

        h2e =
            add h2d n2
    in
    ( h1e, h2e )


processBlocks : Int -> Int -> Array Int -> X64 -> X64 -> ( X64, X64 )
processBlocks index blockEnd bytes h1 h2 =
    if index < blockEnd then
        let
            ( nextH1, nextH2 ) =
                mixBlock bytes index h1 h2
        in
        processBlocks (index + 16) blockEnd bytes nextH1 nextH2

    else
        ( h1, h2 )


tailByte : Array Int -> Int -> Int
tailByte bytes index =
    Array.get index bytes |> Maybe.withDefault 0


processTail : Int -> Int -> Array Int -> X64 -> X64 -> ( X64, X64 )
processTail start remainder bytes h1 h2 =
    let
        takeK1 offset acc =
            if offset < min 8 remainder then
                xorByte acc (tailByte bytes (start + offset)) (offset * 8)

            else
                acc

        takeK2 offset acc =
            if offset < remainder - 8 then
                xorByte acc (tailByte bytes (start + 8 + offset)) (offset * 8)

            else
                acc

        k1 =
            takeK1 0 zero
                |> takeK1 1
                |> takeK1 2
                |> takeK1 3
                |> takeK1 4
                |> takeK1 5
                |> takeK1 6
                |> takeK1 7

        k2 =
            takeK2 0 zero
                |> takeK2 1
                |> takeK2 2
                |> takeK2 3
                |> takeK2 4
                |> takeK2 5
                |> takeK2 6

        h2Mixed =
            if remainder > 8 then
                let
                    mixed =
                        multiply k2 c2
                            |> (\value -> rotl value 33)
                            |> (\value -> multiply value c1)
                in
                xor h2 mixed

            else
                h2

        h1Mixed =
            if remainder > 0 then
                let
                    mixed =
                        multiply k1 c1
                            |> (\value -> rotl value 31)
                            |> (\value -> multiply value c2)
                in
                xor h1 mixed

            else
                h1
    in
    ( h1Mixed, h2Mixed )


hexDigit : Int -> String
hexDigit n =
    String.slice n (n + 1) "0123456789abcdef"


hex4 : Int -> String
hex4 n =
    hexDigit (Bitwise.and (lsr n 12) 0x0F)
        ++ hexDigit (Bitwise.and (lsr n 8) 0x0F)
        ++ hexDigit (Bitwise.and (lsr n 4) 0x0F)
        ++ hexDigit (Bitwise.and n 0x0F)


hex32 : Int -> String
hex32 n =
    hex4 (lsr n 16) ++ hex4 (land16 n)


x64hash128 : String -> Int -> String
x64hash128 input seed =
    x64hash128FromUnits (String.foldl (\char acc -> acc ++ [ Char.toCode char ]) [] input) seed


x64hash128FromUnits : List Int -> Int -> String
x64hash128FromUnits units seed =
    let
        key =
            Utf8.bytesFromUnits units

        length =
            Array.length key

        remainder =
            modBy 16 length

        blockEnd =
            length - remainder

        seed32 =
            seed

        ( h1Blocks, h2Blocks ) =
            processBlocks 0 blockEnd key (x64 0 seed32) (x64 0 seed32)

        ( h1Tail, h2Tail ) =
            processTail blockEnd remainder key h1Blocks h2Blocks

        lengthWord =
            x64 0 length

        h1a =
            xor h1Tail lengthWord

        h2a =
            xor h2Tail lengthWord

        h1b =
            add h1a h2a

        h2b =
            add h2a h1b

        h1c =
            finalMix h1b

        h2c =
            finalMix h2b

        h1d =
            add h1c h2c

        h2d =
            add h2c h1d
    in
    hex32 h1d.high ++ hex32 h1d.low ++ hex32 h2d.high ++ hex32 h2d.low
