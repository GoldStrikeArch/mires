module Blake2 exposing (Blake2, Options, bsigma, digest, make, paramWord, update)

{-| noble's \_BLAKE2 base (blake2.ts) and \_blake.ts's BSIGMA, shared by BLAKE2b and BLAKE2s.

The final block is compressed with a flag, so a full buffer waits for more input. As in `Md`,
an update prepends the buffered bytes to the new ones and compresses every block of the result
except the last one; `compressed` counts the bytes compressed so far, where noble's `length` also
counts the buffered ones.

-}

import Array exposing (Array)
import Bitwise exposing (or, shiftLeftBy)
import Words


{-| Message word order of each round (noble's BSIGMA); BLAKE2b's rounds 10 and 11 repeat 0 and 1.
-}
bsigma : Array Int
bsigma =
    Array.fromList
        [ 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15
        , 14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3
        , 11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4
        , 7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8
        , 9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13
        , 2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9
        , 12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11
        , 13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10
        , 6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5
        , 10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0
        , 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15
        , 14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3
        ]


{-| noble's Blake2Opts: the options reference.mjs passes to every hash. reference.mjs passes
`dkLen` on whatever its type, and only BLAKE2's `anumber` requires a number, so a present `dkLen`
that is not one is kept as an error for BLAKE2 to report.
-}
type alias Options =
    { dkLen : Maybe (Result String Float)
    , key : Maybe (List Int)
    , salt : Maybe (List Int)
    , personalization : Maybe (List Int)
    }


{-| The base over a compression state `s`: `compress` takes a block, the byte counter and the
last-block flag; `get` lists the state as little-endian 32-bit words.
-}
type alias Blake2 s =
    { blockLen : Int
    , outputLen : Int
    , buffer : List Int
    , pos : Int
    , compressed : Int
    , state : s
    , compress : List Int -> Int -> Bool -> s -> s
    , get : s -> List Int
    }


{-| noble's constructors: `anumber` on the digest length, then checkBlake2Opts, where `keyLen`
bounds both the digest and the key length; then the parameter block is applied by `init`, and
the zero-padded key is absorbed as the first block.
-}
make :
    { blockLen : Int, keyLen : Int, saltLen : Int, persLen : Int }
    -> Options
    -> (Int -> s)
    -> (List Int -> Int -> Bool -> s -> s)
    -> (s -> List Int)
    -> Result String (Blake2 s)
make { blockLen, keyLen, saltLen, persLen } opts init compress get =
    case Maybe.withDefault (Ok (toFloat keyLen)) opts.dkLen of
        Err _ ->
            Err "\"outputLen\" expected number"

        Ok dkLen ->
            check { blockLen = blockLen, keyLen = keyLen, saltLen = saltLen, persLen = persLen } opts dkLen init compress get


check :
    { blockLen : Int, keyLen : Int, saltLen : Int, persLen : Int }
    -> Options
    -> Float
    -> (Int -> s)
    -> (List Int -> Int -> Bool -> s -> s)
    -> (s -> List Int)
    -> Result String (Blake2 s)
check { blockLen, keyLen, saltLen, persLen } opts dkLen init compress get =
    let
        outputLen =
            floor dkLen

        hasLength n =
            Maybe.map (\bytes -> List.length bytes /= n) >> Maybe.withDefault False

        keyLength =
            opts.key |> Maybe.map List.length
    in
    if not (dkLen >= 0 && dkLen <= 9007199254740991 && toFloat outputLen == dkLen) then
        Err "\"outputLen\" expected integer >= 0"

    else if outputLen <= 0 || outputLen > keyLen then
        Err "\"dkLen\" out of range"

    else if Maybe.map (\n -> n < 1 || n > keyLen) keyLength |> Maybe.withDefault False then
        Err "\"key\" expected to be undefined or of a valid length"

    else if hasLength saltLen opts.salt then
        Err "\"salt\" has the wrong length"

    else if hasLength persLen opts.personalization then
        Err "\"personalization\" has the wrong length"

    else
        let
            hash =
                { blockLen = blockLen
                , outputLen = outputLen
                , buffer = []
                , pos = 0
                , compressed = 0
                , state = init outputLen
                , compress = compress
                , get = get
                }
        in
        Ok
            (case opts.key of
                Just key ->
                    update (key ++ List.repeat (blockLen - List.length key) 0) hash

                Nothing ->
                    hash
            )


{-| The parameter block's first word, RFC 7693 §2.5: p[0] = 0x0101kknn.
-}
paramWord : Int -> Options -> Int
paramWord outputLen opts =
    let
        keyLength =
            opts.key |> Maybe.map List.length |> Maybe.withDefault 0
    in
    or outputLen (or (shiftLeftBy 8 keyLength) (or (shiftLeftBy 16 0x01) (shiftLeftBy 24 0x01)))


update : List Int -> Blake2 s -> Blake2 s
update data hash =
    absorb (hash.buffer ++ data) (hash.pos + List.length data) hash


{-| Compresses every block at the front of `bytes`, which holds `count` bytes, except the last,
and buffers the rest.
-}
absorb : List Int -> Int -> Blake2 s -> Blake2 s
absorb bytes count hash =
    if count > hash.blockLen then
        let
            counter =
                hash.compressed + hash.blockLen
        in
        absorb (List.drop hash.blockLen bytes)
            (count - hash.blockLen)
            { hash | state = hash.compress (List.take hash.blockLen bytes) counter False hash.state, compressed = counter }

    else
        { hash | buffer = bytes, pos = count }


digest : Blake2 s -> List Int
digest hash =
    let
        last =
            hash.buffer ++ List.repeat (hash.blockLen - hash.pos) 0
    in
    hash.get (hash.compress last (hash.compressed + hash.pos) True hash.state)
        |> List.concatMap Words.bytesLE
        |> List.take hash.outputLen
