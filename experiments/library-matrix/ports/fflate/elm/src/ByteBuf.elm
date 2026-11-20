module ByteBuf exposing
    ( ByteBuilder
    , Bytes
    , IntTree(..)
    , append
    , appendList
    , appendSlice
    , builderGetByte
    , builderLength
    , builderPushByte
    , builderPushRepeatByte
    , builderToBytes
    , empty
    , emptyBuilder
    , foldl
    , foldlRange
    , foldr
    , fromBytes
    , fromList
    , fromReversedList
    , getBits
    , getBits16
    , getByte
    , getU16Le
    , getU24Le
    , getU32Le
    , intTreeGet
    , intTreeInc
    , intTreeSet
    , intTreeSwap
    , length
    , shiftForSize
    , slice
    , toJsonValue
    , toList
    , zeros
    )

{-| Persistent integer radix trees (`IntTree`) and packed byte buffers (`Bytes`, `ByteBuilder`).

`IntTree` is an 8-way sparse persistent integer array with zero-allocation reads (`get` returns
`Int` directly, defaulting absent entries to `0`), single-traversal `swap` (for LZ77 `head`
updates), and single-traversal `inc` (for frequency tables).

`Bytes` packs 4 bytes little-endian per 32-bit `Int` (32 bytes per `Leaf8`) with an O(1)
`(offset, length)` slice view. `ByteBuilder` buffers up to 32 bytes in a flat tail before flushing
a complete `Leaf8` into the tree, giving O(1) amortized byte appends and O(1) reads of the most
recent 0..31 bytes (hot for RLE back-references during inflate).

-}

import Bitwise
import Json.Encode as Encode


{-| 8-way sparse persistent integer array.
-}
type IntTree
    = Empty
    | Leaf8 Int Int Int Int Int Int Int Int
    | Node8 IntTree IntTree IntTree IntTree IntTree IntTree IntTree IntTree


shiftForSize : Int -> Int
shiftForSize n =
    if n <= 8 then
        0

    else if n <= 64 then
        3

    else if n <= 512 then
        6

    else if n <= 4096 then
        9

    else if n <= 32768 then
        12

    else if n <= 262144 then
        15

    else if n <= 2097152 then
        18

    else if n <= 16777216 then
        21

    else
        24


intTreeGet : Int -> Int -> IntTree -> Int
intTreeGet shift idx tree =
    case tree of
        Empty ->
            0

        Leaf8 a0 a1 a2 a3 a4 a5 a6 a7 ->
            case Bitwise.and 7 idx of
                0 ->
                    a0

                1 ->
                    a1

                2 ->
                    a2

                3 ->
                    a3

                4 ->
                    a4

                5 ->
                    a5

                6 ->
                    a6

                _ ->
                    a7

        Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
            let
                nextShift =
                    shift - 3
            in
            case Bitwise.and 7 (Bitwise.shiftRightZfBy shift idx) of
                0 ->
                    intTreeGet nextShift idx n0

                1 ->
                    intTreeGet nextShift idx n1

                2 ->
                    intTreeGet nextShift idx n2

                3 ->
                    intTreeGet nextShift idx n3

                4 ->
                    intTreeGet nextShift idx n4

                5 ->
                    intTreeGet nextShift idx n5

                6 ->
                    intTreeGet nextShift idx n6

                _ ->
                    intTreeGet nextShift idx n7


intTreeSet : Int -> Int -> Int -> IntTree -> IntTree
intTreeSet shift idx val tree =
    if shift == 0 then
        case tree of
            Leaf8 a0 a1 a2 a3 a4 a5 a6 a7 ->
                case Bitwise.and 7 idx of
                    0 ->
                        Leaf8 val a1 a2 a3 a4 a5 a6 a7

                    1 ->
                        Leaf8 a0 val a2 a3 a4 a5 a6 a7

                    2 ->
                        Leaf8 a0 a1 val a3 a4 a5 a6 a7

                    3 ->
                        Leaf8 a0 a1 a2 val a4 a5 a6 a7

                    4 ->
                        Leaf8 a0 a1 a2 a3 val a5 a6 a7

                    5 ->
                        Leaf8 a0 a1 a2 a3 a4 val a6 a7

                    6 ->
                        Leaf8 a0 a1 a2 a3 a4 a5 val a7

                    _ ->
                        Leaf8 a0 a1 a2 a3 a4 a5 a6 val

            _ ->
                case Bitwise.and 7 idx of
                    0 ->
                        Leaf8 val 0 0 0 0 0 0 0

                    1 ->
                        Leaf8 0 val 0 0 0 0 0 0

                    2 ->
                        Leaf8 0 0 val 0 0 0 0 0

                    3 ->
                        Leaf8 0 0 0 val 0 0 0 0

                    4 ->
                        Leaf8 0 0 0 0 val 0 0 0

                    5 ->
                        Leaf8 0 0 0 0 0 val 0 0

                    6 ->
                        Leaf8 0 0 0 0 0 0 val 0

                    _ ->
                        Leaf8 0 0 0 0 0 0 0 val

    else
        let
            nextShift =
                shift - 3
        in
        case tree of
            Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
                case Bitwise.and 7 (Bitwise.shiftRightZfBy shift idx) of
                    0 ->
                        Node8 (intTreeSet nextShift idx val n0) n1 n2 n3 n4 n5 n6 n7

                    1 ->
                        Node8 n0 (intTreeSet nextShift idx val n1) n2 n3 n4 n5 n6 n7

                    2 ->
                        Node8 n0 n1 (intTreeSet nextShift idx val n2) n3 n4 n5 n6 n7

                    3 ->
                        Node8 n0 n1 n2 (intTreeSet nextShift idx val n3) n4 n5 n6 n7

                    4 ->
                        Node8 n0 n1 n2 n3 (intTreeSet nextShift idx val n4) n5 n6 n7

                    5 ->
                        Node8 n0 n1 n2 n3 n4 (intTreeSet nextShift idx val n5) n6 n7

                    6 ->
                        Node8 n0 n1 n2 n3 n4 n5 (intTreeSet nextShift idx val n6) n7

                    _ ->
                        Node8 n0 n1 n2 n3 n4 n5 n6 (intTreeSet nextShift idx val n7)

            _ ->
                let
                    child =
                        intTreeSet nextShift idx val Empty
                in
                case Bitwise.and 7 (Bitwise.shiftRightZfBy shift idx) of
                    0 ->
                        Node8 child Empty Empty Empty Empty Empty Empty Empty

                    1 ->
                        Node8 Empty child Empty Empty Empty Empty Empty Empty

                    2 ->
                        Node8 Empty Empty child Empty Empty Empty Empty Empty

                    3 ->
                        Node8 Empty Empty Empty child Empty Empty Empty Empty

                    4 ->
                        Node8 Empty Empty Empty Empty child Empty Empty Empty

                    5 ->
                        Node8 Empty Empty Empty Empty Empty child Empty Empty

                    6 ->
                        Node8 Empty Empty Empty Empty Empty Empty child Empty

                    _ ->
                        Node8 Empty Empty Empty Empty Empty Empty Empty child


{-| Read old value and store new value in a single tree traversal.
-}
intTreeSwap : Int -> Int -> Int -> IntTree -> ( Int, IntTree )
intTreeSwap shift idx val tree =
    if shift == 0 then
        case tree of
            Leaf8 a0 a1 a2 a3 a4 a5 a6 a7 ->
                case Bitwise.and 7 idx of
                    0 ->
                        ( a0, Leaf8 val a1 a2 a3 a4 a5 a6 a7 )

                    1 ->
                        ( a1, Leaf8 a0 val a2 a3 a4 a5 a6 a7 )

                    2 ->
                        ( a2, Leaf8 a0 a1 val a3 a4 a5 a6 a7 )

                    3 ->
                        ( a3, Leaf8 a0 a1 a2 val a4 a5 a6 a7 )

                    4 ->
                        ( a4, Leaf8 a0 a1 a2 a3 val a5 a6 a7 )

                    5 ->
                        ( a5, Leaf8 a0 a1 a2 a3 a4 val a6 a7 )

                    6 ->
                        ( a6, Leaf8 a0 a1 a2 a3 a4 a5 val a7 )

                    _ ->
                        ( a7, Leaf8 a0 a1 a2 a3 a4 a5 a6 val )

            _ ->
                ( 0, intTreeSet 0 idx val Empty )

    else
        let
            nextShift =
                shift - 3
        in
        case tree of
            Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
                case Bitwise.and 7 (Bitwise.shiftRightZfBy shift idx) of
                    0 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n0
                        in
                        ( old, Node8 c n1 n2 n3 n4 n5 n6 n7 )

                    1 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n1
                        in
                        ( old, Node8 n0 c n2 n3 n4 n5 n6 n7 )

                    2 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n2
                        in
                        ( old, Node8 n0 n1 c n3 n4 n5 n6 n7 )

                    3 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n3
                        in
                        ( old, Node8 n0 n1 n2 c n4 n5 n6 n7 )

                    4 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n4
                        in
                        ( old, Node8 n0 n1 n2 n3 c n5 n6 n7 )

                    5 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n5
                        in
                        ( old, Node8 n0 n1 n2 n3 n4 c n6 n7 )

                    6 ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n6
                        in
                        ( old, Node8 n0 n1 n2 n3 n4 n5 c n7 )

                    _ ->
                        let
                            ( old, c ) =
                                intTreeSwap nextShift idx val n7
                        in
                        ( old, Node8 n0 n1 n2 n3 n4 n5 n6 c )

            _ ->
                ( 0, intTreeSet shift idx val Empty )


{-| Increment entry at `idx` by 1 in a single traversal.
-}
intTreeInc : Int -> Int -> IntTree -> IntTree
intTreeInc shift idx tree =
    if shift == 0 then
        case tree of
            Leaf8 a0 a1 a2 a3 a4 a5 a6 a7 ->
                case Bitwise.and 7 idx of
                    0 ->
                        Leaf8 (a0 + 1) a1 a2 a3 a4 a5 a6 a7

                    1 ->
                        Leaf8 a0 (a1 + 1) a2 a3 a4 a5 a6 a7

                    2 ->
                        Leaf8 a0 a1 (a2 + 1) a3 a4 a5 a6 a7

                    3 ->
                        Leaf8 a0 a1 a2 (a3 + 1) a4 a5 a6 a7

                    4 ->
                        Leaf8 a0 a1 a2 a3 (a4 + 1) a5 a6 a7

                    5 ->
                        Leaf8 a0 a1 a2 a3 a4 (a5 + 1) a6 a7

                    6 ->
                        Leaf8 a0 a1 a2 a3 a4 a5 (a6 + 1) a7

                    _ ->
                        Leaf8 a0 a1 a2 a3 a4 a5 a6 (a7 + 1)

            _ ->
                intTreeSet 0 idx 1 Empty

    else
        let
            nextShift =
                shift - 3
        in
        case tree of
            Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
                case Bitwise.and 7 (Bitwise.shiftRightZfBy shift idx) of
                    0 ->
                        Node8 (intTreeInc nextShift idx n0) n1 n2 n3 n4 n5 n6 n7

                    1 ->
                        Node8 n0 (intTreeInc nextShift idx n1) n2 n3 n4 n5 n6 n7

                    2 ->
                        Node8 n0 n1 (intTreeInc nextShift idx n2) n3 n4 n5 n6 n7

                    3 ->
                        Node8 n0 n1 n2 (intTreeInc nextShift idx n3) n4 n5 n6 n7

                    4 ->
                        Node8 n0 n1 n2 n3 (intTreeInc nextShift idx n4) n5 n6 n7

                    5 ->
                        Node8 n0 n1 n2 n3 n4 (intTreeInc nextShift idx n5) n6 n7

                    6 ->
                        Node8 n0 n1 n2 n3 n4 n5 (intTreeInc nextShift idx n6) n7

                    _ ->
                        Node8 n0 n1 n2 n3 n4 n5 n6 (intTreeInc nextShift idx n7)

            _ ->
                intTreeSet shift idx 1 Empty


{-| Replace an entire 8-word leaf at `wordIdx` (must be a multiple of 8).
-}
setLeaf : Int -> Int -> IntTree -> IntTree -> IntTree
setLeaf shift wordIdx leaf tree =
    if shift == 0 then
        leaf

    else
        let
            nextShift =
                shift - 3
        in
        case tree of
            Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
                case Bitwise.and 7 (Bitwise.shiftRightZfBy shift wordIdx) of
                    0 ->
                        Node8 (setLeaf nextShift wordIdx leaf n0) n1 n2 n3 n4 n5 n6 n7

                    1 ->
                        Node8 n0 (setLeaf nextShift wordIdx leaf n1) n2 n3 n4 n5 n6 n7

                    2 ->
                        Node8 n0 n1 (setLeaf nextShift wordIdx leaf n2) n3 n4 n5 n6 n7

                    3 ->
                        Node8 n0 n1 n2 (setLeaf nextShift wordIdx leaf n3) n4 n5 n6 n7

                    4 ->
                        Node8 n0 n1 n2 n3 (setLeaf nextShift wordIdx leaf n4) n5 n6 n7

                    5 ->
                        Node8 n0 n1 n2 n3 n4 (setLeaf nextShift wordIdx leaf n5) n6 n7

                    6 ->
                        Node8 n0 n1 n2 n3 n4 n5 (setLeaf nextShift wordIdx leaf n6) n7

                    _ ->
                        Node8 n0 n1 n2 n3 n4 n5 n6 (setLeaf nextShift wordIdx leaf n7)

            _ ->
                let
                    child =
                        setLeaf nextShift wordIdx leaf Empty
                in
                case Bitwise.and 7 (Bitwise.shiftRightZfBy shift wordIdx) of
                    0 ->
                        Node8 child Empty Empty Empty Empty Empty Empty Empty

                    1 ->
                        Node8 Empty child Empty Empty Empty Empty Empty Empty

                    2 ->
                        Node8 Empty Empty child Empty Empty Empty Empty Empty

                    3 ->
                        Node8 Empty Empty Empty child Empty Empty Empty Empty

                    4 ->
                        Node8 Empty Empty Empty Empty child Empty Empty Empty

                    5 ->
                        Node8 Empty Empty Empty Empty Empty child Empty Empty

                    6 ->
                        Node8 Empty Empty Empty Empty Empty Empty child Empty

                    _ ->
                        Node8 Empty Empty Empty Empty Empty Empty Empty child



-- PACKED BYTE BUFFER


type Bytes
    = Bytes IntTree Int Int Int


empty : Bytes
empty =
    Bytes Empty 0 0 0


zeros : Int -> Bytes
zeros n =
    if n <= 0 then
        empty

    else
        let
            words =
                Bitwise.shiftRightZfBy 2 (n + 3)
        in
        Bytes Empty (shiftForSize words) 0 n


length : Bytes -> Int
length (Bytes _ _ _ len) =
    len


{-| Slice `[start .. end)` clamped to `[0 .. length]`, matching `slc` / `subarray` semantics
(with non-negative clamped indices; callers handle negative offsets before calling).
-}
slice : Int -> Int -> Bytes -> Bytes
slice start end (Bytes tree shift offset len) =
    let
        s =
            if start < 0 then
                0

            else if (start - len) > 0 then
                len

            else
                start

        e =
            if end < 0 then
                0

            else if (end - len) > 0 then
                len

            else
                end
    in
    if (s - e) >= 0 then
        empty

    else
        Bytes tree shift (offset + s) (e - s)


{-| Read a single byte at `idx`; returns `0` if out of bounds (matching JS `Uint8Array[oob] | 0`).
-}
getByte : Int -> Bytes -> Int
getByte idx (Bytes tree shift offset len) =
    if idx < 0 || (idx - len) >= 0 then
        0

    else
        let
            abs =
                offset + idx

            w =
                intTreeGet shift (Bitwise.shiftRightZfBy 2 abs) tree
        in
        Bitwise.and 255 (Bitwise.shiftRightZfBy (Bitwise.shiftLeftBy 3 (Bitwise.and 3 abs)) w)


getU16Le : Int -> Bytes -> Int
getU16Le idx bytes =
    Bitwise.or (getByte idx bytes) (Bitwise.shiftLeftBy 8 (getByte (idx + 1) bytes))


{-| Read 3 bytes little-endian starting at `idx` (`b0 | (b1 << 8) | (b2 << 16)`),
treating out-of-bounds reads as 0. Reads 1 word (when `(offset + idx) & 3 <= 1`) or 2 words.
-}
getU24Le : Int -> Bytes -> Int
getU24Le idx (Bytes tree shift offset len) =
    if idx < 0 || (idx - len) >= 0 then
        0

    else if (idx + 2 - len) < 0 then
        let
            abs =
                offset + idx

            wi =
                Bitwise.shiftRightZfBy 2 abs

            bo =
                Bitwise.and 3 abs

            w0 =
                intTreeGet shift wi tree
        in
        case bo of
            0 ->
                Bitwise.and 0x00FFFFFF w0

            1 ->
                Bitwise.shiftRightZfBy 8 w0

            2 ->
                let
                    w1 =
                        intTreeGet shift (wi + 1) tree
                in
                Bitwise.or (Bitwise.shiftRightZfBy 16 w0) (Bitwise.shiftLeftBy 16 (Bitwise.and 255 w1))

            _ ->
                let
                    w1 =
                        intTreeGet shift (wi + 1) tree
                in
                Bitwise.or (Bitwise.shiftRightZfBy 24 w0) (Bitwise.shiftLeftBy 8 (Bitwise.and 65535 w1))

    else
        let
            b0 =
                getByte idx (Bytes tree shift offset len)

            b1 =
                if (idx + 1 - len) < 0 then
                    getByte (idx + 1) (Bytes tree shift offset len)

                else
                    0
        in
        Bitwise.or b0 (Bitwise.shiftLeftBy 8 b1)


getU32Le : Int -> Bytes -> Int
getU32Le idx bytes =
    Bitwise.shiftRightZfBy 0
        (Bitwise.or
            (Bitwise.or (getByte idx bytes) (Bitwise.shiftLeftBy 8 (getByte (idx + 1) bytes)))
            (Bitwise.or (Bitwise.shiftLeftBy 16 (getByte (idx + 2) bytes)) (Bitwise.shiftLeftBy 24 (getByte (idx + 3) bytes)))
        )


{-| Equivalent to `fflate`'s `bits(d, p, m)`: reads up to 16 bits starting at bit position `p`,
treating bytes past `length` as 0.
-}
getBits : Bytes -> Int -> Int -> Int
getBits (Bytes tree shift offset len) p m =
    let
        o =
            Bitwise.shiftRightZfBy 3 p
    in
    if (o + 1 - len) < 0 then
        let
            abs =
                offset + o

            wi =
                Bitwise.shiftRightZfBy 2 abs

            bo =
                Bitwise.and 3 abs

            w0 =
                intTreeGet shift wi tree

            raw16 =
                case bo of
                    0 ->
                        Bitwise.and 65535 w0

                    1 ->
                        Bitwise.and 65535 (Bitwise.shiftRightZfBy 8 w0)

                    2 ->
                        Bitwise.shiftRightZfBy 16 w0

                    _ ->
                        let
                            w1 =
                                intTreeGet shift (wi + 1) tree
                        in
                        Bitwise.or (Bitwise.shiftRightZfBy 24 w0) (Bitwise.shiftLeftBy 8 (Bitwise.and 255 w1))
        in
        Bitwise.and m (Bitwise.shiftRightZfBy (Bitwise.and 7 p) raw16)

    else if (o - len) < 0 then
        let
            abs =
                offset + o

            w0 =
                intTreeGet shift (Bitwise.shiftRightZfBy 2 abs) tree

            b0 =
                Bitwise.and 255 (Bitwise.shiftRightZfBy (Bitwise.shiftLeftBy 3 (Bitwise.and 3 abs)) w0)
        in
        Bitwise.and m (Bitwise.shiftRightZfBy (Bitwise.and 7 p) b0)

    else
        0


{-| Equivalent to `fflate`'s `bits16(d, p)`: reads at least 16 bits starting at bit position `p`,
treating bytes past `length` as 0.
-}
getBits16 : Bytes -> Int -> Int
getBits16 (Bytes tree shift offset len) p =
    let
        o =
            Bitwise.shiftRightZfBy 3 p
    in
    if (o + 2 - len) < 0 then
        let
            abs =
                offset + o

            wi =
                Bitwise.shiftRightZfBy 2 abs

            bo =
                Bitwise.and 3 abs

            w0 =
                intTreeGet shift wi tree

            raw24 =
                case bo of
                    0 ->
                        Bitwise.and 0x00FFFFFF w0

                    1 ->
                        Bitwise.shiftRightZfBy 8 w0

                    2 ->
                        let
                            w1 =
                                intTreeGet shift (wi + 1) tree
                        in
                        Bitwise.or (Bitwise.shiftRightZfBy 16 w0) (Bitwise.shiftLeftBy 16 (Bitwise.and 255 w1))

                    _ ->
                        let
                            w1 =
                                intTreeGet shift (wi + 1) tree
                        in
                        Bitwise.or (Bitwise.shiftRightZfBy 24 w0) (Bitwise.shiftLeftBy 8 (Bitwise.and 65535 w1))
        in
        Bitwise.shiftRightZfBy (Bitwise.and 7 p) raw24

    else if (o - len) < 0 then
        let
            b0 =
                getByte o (Bytes tree shift offset len)

            b1 =
                if (o + 1 - len) < 0 then
                    getByte (o + 1) (Bytes tree shift offset len)

                else
                    0
        in
        Bitwise.shiftRightZfBy (Bitwise.and 7 p) (Bitwise.or b0 (Bitwise.shiftLeftBy 8 b1))

    else
        0



-- BYTE BUILDER (32-BYTE TAIL BUFFERED, POSITIONAL TO AVOID _Utils_update)


type Word4
    = Word4 Int Int Int Int


emptyWord4 : Word4
emptyWord4 =
    Word4 0 0 0 0


type ByteBuilder
    = ByteBuilder IntTree Int Int Int Int Word4 Word4


emptyBuilder : ByteBuilder
emptyBuilder =
    ByteBuilder Empty 0 8 0 0 emptyWord4 emptyWord4


builderLength : ByteBuilder -> Int
builderLength (ByteBuilder _ _ _ flushedBytes tLen _ _) =
    flushedBytes + tLen


growForWord : Int -> Int -> Int -> IntTree -> ( Int, Int, IntTree )
growForWord wordIdx shift capWords tree =
    if (wordIdx - capWords) < 0 then
        ( shift, capWords, tree )

    else
        growForWord wordIdx (shift + 3) (capWords * 8) (Node8 tree Empty Empty Empty Empty Empty Empty Empty)


builderPushByte : Int -> ByteBuilder -> ByteBuilder
builderPushByte rawByte (ByteBuilder tree shift capWords flushedBytes tLen lo4 hi4) =
    let
        byte =
            Bitwise.and 255 rawByte

        bShift =
            Bitwise.shiftLeftBy 3 (Bitwise.and 3 tLen)

        shifted =
            Bitwise.shiftLeftBy bShift byte
    in
    if tLen < 16 then
        let
            (Word4 w0 w1 w2 w3) =
                lo4

            nextLo4 =
                case Bitwise.shiftRightZfBy 2 tLen of
                    0 ->
                        Word4 (Bitwise.or w0 shifted) w1 w2 w3

                    1 ->
                        Word4 w0 (Bitwise.or w1 shifted) w2 w3

                    2 ->
                        Word4 w0 w1 (Bitwise.or w2 shifted) w3

                    _ ->
                        Word4 w0 w1 w2 (Bitwise.or w3 shifted)
        in
        ByteBuilder tree shift capWords flushedBytes (tLen + 1) nextLo4 hi4

    else if tLen < 31 then
        let
            (Word4 w4 w5 w6 w7) =
                hi4

            nextHi4 =
                case Bitwise.shiftRightZfBy 2 (tLen - 16) of
                    0 ->
                        Word4 (Bitwise.or w4 shifted) w5 w6 w7

                    1 ->
                        Word4 w4 (Bitwise.or w5 shifted) w6 w7

                    2 ->
                        Word4 w4 w5 (Bitwise.or w6 shifted) w7

                    _ ->
                        Word4 w4 w5 w6 (Bitwise.or w7 shifted)
        in
        ByteBuilder tree shift capWords flushedBytes (tLen + 1) lo4 nextHi4

    else
        let
            (Word4 w0 w1 w2 w3) =
                lo4

            (Word4 w4 w5 w6 w7) =
                hi4

            finalW7 =
                Bitwise.or w7 shifted

            leaf =
                Leaf8 w0 w1 w2 w3 w4 w5 w6 finalW7

            wordIdx =
                Bitwise.shiftRightZfBy 2 flushedBytes

            ( newShift, newCapWords, grownTree ) =
                growForWord wordIdx shift capWords tree

            newTree =
                setLeaf newShift wordIdx leaf grownTree
        in
        ByteBuilder newTree newShift newCapWords (flushedBytes + 32) 0 emptyWord4 emptyWord4


builderPushRepeatByte : Int -> Int -> ByteBuilder -> ByteBuilder
builderPushRepeatByte byte count builder =
    if count <= 0 then
        builder

    else
        builderPushRepeatByte byte (count - 1) (builderPushByte byte builder)


builderGetByte : Int -> ByteBuilder -> Int
builderGetByte idx (ByteBuilder tree shift _ flushedBytes tLen lo4 hi4) =
    if idx < 0 then
        0

    else if (idx - flushedBytes) < 0 then
        let
            w =
                intTreeGet shift (Bitwise.shiftRightZfBy 2 idx) tree
        in
        Bitwise.and 255 (Bitwise.shiftRightZfBy (Bitwise.shiftLeftBy 3 (Bitwise.and 3 idx)) w)

    else
        let
            rel =
                idx - flushedBytes
        in
        if (rel - tLen) >= 0 then
            0

        else
            let
                w =
                    if rel < 16 then
                        let
                            (Word4 w0 w1 w2 w3) =
                                lo4
                        in
                        case Bitwise.shiftRightZfBy 2 rel of
                            0 ->
                                w0

                            1 ->
                                w1

                            2 ->
                                w2

                            _ ->
                                w3

                    else
                        let
                            (Word4 w4 w5 w6 w7) =
                                hi4
                        in
                        case Bitwise.shiftRightZfBy 2 (rel - 16) of
                            0 ->
                                w4

                            1 ->
                                w5

                            2 ->
                                w6

                            _ ->
                                w7
            in
            Bitwise.and 255 (Bitwise.shiftRightZfBy (Bitwise.shiftLeftBy 3 (Bitwise.and 3 rel)) w)


builderToBytes : ByteBuilder -> Bytes
builderToBytes (ByteBuilder tree shift capWords flushedBytes tLen lo4 hi4) =
    let
        totalLen =
            flushedBytes + tLen
    in
    if totalLen == 0 then
        empty

    else if tLen == 0 then
        Bytes tree shift 0 totalLen

    else
        let
            (Word4 w0 w1 w2 w3) =
                lo4

            (Word4 w4 w5 w6 w7) =
                hi4

            leaf =
                Leaf8 w0 w1 w2 w3 w4 w5 w6 w7

            wordIdx =
                Bitwise.shiftRightZfBy 2 flushedBytes

            ( newShift, _, grownTree ) =
                growForWord wordIdx shift capWords tree

            newTree =
                setLeaf newShift wordIdx leaf grownTree
        in
        Bytes newTree newShift 0 totalLen


fromBytes : Bytes -> ByteBuilder
fromBytes bytes =
    foldl builderPushByte emptyBuilder bytes


fromList : List Int -> Bytes
fromList list =
    builderToBytes (List.foldl builderPushByte emptyBuilder list)


fromReversedList : List Int -> Bytes
fromReversedList revList =
    fromList (List.reverse revList)


appendList : List Int -> ByteBuilder -> ByteBuilder
appendList list builder =
    List.foldl builderPushByte builder list


appendSlice : Int -> Int -> Bytes -> ByteBuilder -> ByteBuilder
appendSlice start end bytes builder =
    foldlRange start end builderPushByte builder bytes


append : Bytes -> Bytes -> Bytes
append a b =
    if length a == 0 then
        b

    else if length b == 0 then
        a

    else
        builderToBytes (foldl builderPushByte (fromBytes a) b)



-- FAST TREE FOLDS (ZERO RANDOM-ACCESS OVERHEAD)


foldl : (Int -> a -> a) -> a -> Bytes -> a
foldl fn acc (Bytes tree shift offset len) =
    if len <= 0 then
        acc

    else
        foldlTree fn shift 0 offset (offset + len) tree acc


foldlRange : Int -> Int -> (Int -> a -> a) -> a -> Bytes -> a
foldlRange start end fn acc (Bytes tree shift offset len) =
    let
        s =
            if start < 0 then
                0

            else if (start - len) > 0 then
                len

            else
                start

        e =
            if end < 0 then
                0

            else if (end - len) > 0 then
                len

            else
                end
    in
    if (s - e) >= 0 then
        acc

    else
        foldlTree fn shift 0 (offset + s) (offset + e) tree acc


foldlZeros : (Int -> a -> a) -> Int -> a -> a
foldlZeros fn count acc =
    if count <= 0 then
        acc

    else
        foldlZeros fn (count - 1) (fn 0 acc)


foldlWordFull : (Int -> a -> a) -> Int -> a -> a
foldlWordFull fn w acc =
    acc
        |> fn (Bitwise.and 255 w)
        |> fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 w))
        |> fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 16 w))
        |> fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 24 w))


foldlWordPartial : (Int -> a -> a) -> Int -> Int -> Int -> Int -> a -> a
foldlWordPartial fn base startByte endByte w acc =
    let
        a0 =
            if (base - startByte) >= 0 && (base - endByte) < 0 then
                fn (Bitwise.and 255 w) acc

            else
                acc

        a1 =
            if (base + 1 - startByte) >= 0 && (base + 1 - endByte) < 0 then
                fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 w)) a0

            else
                a0

        a2 =
            if (base + 2 - startByte) >= 0 && (base + 2 - endByte) < 0 then
                fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 16 w)) a1

            else
                a1
    in
    if (base + 3 - startByte) >= 0 && (base + 3 - endByte) < 0 then
        fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 24 w)) a2

    else
        a2


foldlTree : (Int -> a -> a) -> Int -> Int -> Int -> Int -> IntTree -> a -> a
foldlTree fn shift base startByte endByte tree acc =
    let
        nodeBytes =
            Bitwise.shiftLeftBy (shift + 5) 1

        nodeEnd =
            base + nodeBytes
    in
    if (endByte - base) <= 0 || (startByte - nodeEnd) >= 0 then
        acc

    else
        case tree of
            Empty ->
                let
                    lo =
                        if (startByte - base) > 0 then
                            startByte

                        else
                            base

                    hi =
                        if (endByte - nodeEnd) < 0 then
                            endByte

                        else
                            nodeEnd
                in
                foldlZeros fn (hi - lo) acc

            Leaf8 w0 w1 w2 w3 w4 w5 w6 w7 ->
                if (startByte - base) <= 0 && (endByte - nodeEnd) >= 0 then
                    acc
                        |> foldlWordFull fn w0
                        |> foldlWordFull fn w1
                        |> foldlWordFull fn w2
                        |> foldlWordFull fn w3
                        |> foldlWordFull fn w4
                        |> foldlWordFull fn w5
                        |> foldlWordFull fn w6
                        |> foldlWordFull fn w7

                else
                    acc
                        |> foldlWordPartial fn base startByte endByte w0
                        |> foldlWordPartial fn (base + 4) startByte endByte w1
                        |> foldlWordPartial fn (base + 8) startByte endByte w2
                        |> foldlWordPartial fn (base + 12) startByte endByte w3
                        |> foldlWordPartial fn (base + 16) startByte endByte w4
                        |> foldlWordPartial fn (base + 20) startByte endByte w5
                        |> foldlWordPartial fn (base + 24) startByte endByte w6
                        |> foldlWordPartial fn (base + 28) startByte endByte w7

            Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
                let
                    nextShift =
                        shift - 3

                    step =
                        Bitwise.shiftRightZfBy 3 nodeBytes
                in
                acc
                    |> foldlTree fn nextShift base startByte endByte n0
                    |> foldlTree fn nextShift (base + step) startByte endByte n1
                    |> foldlTree fn nextShift (base + 2 * step) startByte endByte n2
                    |> foldlTree fn nextShift (base + 3 * step) startByte endByte n3
                    |> foldlTree fn nextShift (base + 4 * step) startByte endByte n4
                    |> foldlTree fn nextShift (base + 5 * step) startByte endByte n5
                    |> foldlTree fn nextShift (base + 6 * step) startByte endByte n6
                    |> foldlTree fn nextShift (base + 7 * step) startByte endByte n7


foldr : (Int -> a -> a) -> a -> Bytes -> a
foldr fn acc (Bytes tree shift offset len) =
    if len <= 0 then
        acc

    else
        foldrTree fn shift 0 offset (offset + len) tree acc


foldrWordFull : (Int -> a -> a) -> Int -> a -> a
foldrWordFull fn w acc =
    acc
        |> fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 24 w))
        |> fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 16 w))
        |> fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 w))
        |> fn (Bitwise.and 255 w)


foldrWordPartial : (Int -> a -> a) -> Int -> Int -> Int -> Int -> a -> a
foldrWordPartial fn base startByte endByte w acc =
    let
        a3 =
            if (base + 3 - startByte) >= 0 && (base + 3 - endByte) < 0 then
                fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 24 w)) acc

            else
                acc

        a2 =
            if (base + 2 - startByte) >= 0 && (base + 2 - endByte) < 0 then
                fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 16 w)) a3

            else
                a3

        a1 =
            if (base + 1 - startByte) >= 0 && (base + 1 - endByte) < 0 then
                fn (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 w)) a2

            else
                a2
    in
    if (base - startByte) >= 0 && (base - endByte) < 0 then
        fn (Bitwise.and 255 w) a1

    else
        a1


foldrTree : (Int -> a -> a) -> Int -> Int -> Int -> Int -> IntTree -> a -> a
foldrTree fn shift base startByte endByte tree acc =
    let
        nodeBytes =
            Bitwise.shiftLeftBy (shift + 5) 1

        nodeEnd =
            base + nodeBytes
    in
    if (endByte - base) <= 0 || (startByte - nodeEnd) >= 0 then
        acc

    else
        case tree of
            Empty ->
                let
                    lo =
                        if (startByte - base) > 0 then
                            startByte

                        else
                            base

                    hi =
                        if (endByte - nodeEnd) < 0 then
                            endByte

                        else
                            nodeEnd
                in
                foldlZeros fn (hi - lo) acc

            Leaf8 w0 w1 w2 w3 w4 w5 w6 w7 ->
                if (startByte - base) <= 0 && (endByte - nodeEnd) >= 0 then
                    acc
                        |> foldrWordFull fn w7
                        |> foldrWordFull fn w6
                        |> foldrWordFull fn w5
                        |> foldrWordFull fn w4
                        |> foldrWordFull fn w3
                        |> foldrWordFull fn w2
                        |> foldrWordFull fn w1
                        |> foldrWordFull fn w0

                else
                    acc
                        |> foldrWordPartial fn (base + 28) startByte endByte w7
                        |> foldrWordPartial fn (base + 24) startByte endByte w6
                        |> foldrWordPartial fn (base + 20) startByte endByte w5
                        |> foldrWordPartial fn (base + 16) startByte endByte w4
                        |> foldrWordPartial fn (base + 12) startByte endByte w3
                        |> foldrWordPartial fn (base + 8) startByte endByte w2
                        |> foldrWordPartial fn (base + 4) startByte endByte w1
                        |> foldrWordPartial fn base startByte endByte w0

            Node8 n0 n1 n2 n3 n4 n5 n6 n7 ->
                let
                    nextShift =
                        shift - 3

                    step =
                        Bitwise.shiftRightZfBy 3 nodeBytes
                in
                acc
                    |> foldrTree fn nextShift (base + 7 * step) startByte endByte n7
                    |> foldrTree fn nextShift (base + 6 * step) startByte endByte n6
                    |> foldrTree fn nextShift (base + 5 * step) startByte endByte n5
                    |> foldrTree fn nextShift (base + 4 * step) startByte endByte n4
                    |> foldrTree fn nextShift (base + 3 * step) startByte endByte n3
                    |> foldrTree fn nextShift (base + 2 * step) startByte endByte n2
                    |> foldrTree fn nextShift (base + step) startByte endByte n1
                    |> foldrTree fn nextShift base startByte endByte n0


toList : Bytes -> List Int
toList bytes =
    foldr (::) [] bytes


toJsonValue : Bytes -> Encode.Value
toJsonValue bytes =
    Encode.list Encode.int (toList bytes)
