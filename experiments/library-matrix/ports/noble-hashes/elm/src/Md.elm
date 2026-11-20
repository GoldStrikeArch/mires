module Md exposing (Md, chi, digest, maj, make, update)

{-| noble's \_md.ts: the Merkle–Damgård base of SHA-256 and SHA-512, and its Chi and Maj.

noble's HashMD fills a byte buffer and compresses each full block in place. Here the update
prepends the buffered bytes to the new ones and compresses every whole block of the result in
order; the bytes left over are the new buffer. The blocks and their order are the same.

-}

import Bitwise exposing (and, complement)
import U64
import Words


{-| Bits of `b` where `a` is set, else of `c`.
-}
chi : Int -> Int -> Int -> Int
chi a b c =
    Bitwise.xor (and a b) (and (complement a) c)


{-| The majority of each bit of `a`, `b` and `c`.
-}
maj : Int -> Int -> Int -> Int
maj a b c =
    Bitwise.xor (Bitwise.xor (and a b) (and a c)) (and b c)


{-| noble's HashMD over a compression state `s`: `process` compresses one block of `blockLen`
bytes into it, `get` lists its words, the high half before the low one for 64-bit words.
`length` counts bytes, a JS number as in noble.
-}
type alias Md s =
    { blockLen : Int
    , outputLen : Int
    , padOffset : Int
    , buffer : List Int
    , pos : Int
    , length : Int
    , state : s
    , process : List Int -> s -> s
    , get : s -> List Int
    }


make : Int -> Int -> Int -> s -> (List Int -> s -> s) -> (s -> List Int) -> Md s
make blockLen outputLen padOffset state process get =
    { blockLen = blockLen
    , outputLen = outputLen
    , padOffset = padOffset
    , buffer = []
    , pos = 0
    , length = 0
    , state = state
    , process = process
    , get = get
    }


update : List Int -> Md s -> Md s
update data md =
    let
        len =
            List.length data
    in
    absorb (md.buffer ++ data) (md.pos + len) { md | length = md.length + len }


{-| Compresses the whole blocks at the front of `bytes`, which holds `count` bytes, and buffers
the rest.
-}
absorb : List Int -> Int -> Md s -> Md s
absorb bytes count md =
    if count >= md.blockLen then
        absorb (List.drop md.blockLen bytes)
            (count - md.blockLen)
            { md | state = md.process (List.take md.blockLen bytes) md.state }

    else
        { md | buffer = bytes, pos = count }


digest : Md s -> List Int
digest md =
    let
        -- Append the bit '1' to the message, then zero-pad the rest of the block.
        pos =
            md.pos + 1

        padded =
            md.buffer ++ [ 0x80 ]

        -- No room for the length field: pad this block and put the length in the next.
        ( state, head, headLength ) =
            if md.padOffset > md.blockLen - pos then
                ( md.process (padded ++ List.repeat (md.blockLen - pos) 0) md.state, [], 0 )

            else
                ( md.state, padded, pos )

        -- noble's setU64FromNum: only the low 64 bits of the length field; the rest stays zero.
        bits =
            md.length * 8

        last =
            head
                ++ List.repeat (md.blockLen - 8 - headLength) 0
                ++ Words.bytesBE (U64.fromNumH bits)
                ++ Words.bytesBE (U64.fromNumL bits)
    in
    md.get (md.process last state)
        |> List.take (md.outputLen // 4)
        |> List.concatMap Words.bytesBE
