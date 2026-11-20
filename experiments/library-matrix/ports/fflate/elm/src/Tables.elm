module Tables exposing
    ( clim
    , crct
    , defaultPlvl
    , deoChain
    , deoNice
    , fd
    , fdeb
    , fdt
    , fl
    , fleb
    , flt
    , rev15
    , revfd
    , revfl
    )

{-| Immutable lookup tables and O(1) bit/code helpers matching `fflate@0.8.3` (`esm/browser.js`).

All top-level bindings are pure immutable Elm values computed once at module load. `rev15` (4
bitwise ops) and `revfd` (integer binary search) are computed in O(1) to avoid 32 KiB static
tables.

-}

import Bitwise
import ByteBuf exposing (IntTree(..), intTreeGet, intTreeSet)


fleb : Int -> Int
fleb i =
    if i < 8 then
        0

    else if i < 12 then
        1

    else if i < 16 then
        2

    else if i < 20 then
        3

    else if i < 24 then
        4

    else if i < 28 then
        5

    else
        0


fdeb : Int -> Int
fdeb i =
    if i < 4 || i >= 30 then
        0

    else
        Bitwise.shiftRightZfBy 1 (i - 2)


clim : Int -> Int
clim i =
    case i of
        0 ->
            16

        1 ->
            17

        2 ->
            18

        3 ->
            0

        4 ->
            8

        5 ->
            7

        6 ->
            9

        7 ->
            6

        8 ->
            10

        9 ->
            5

        10 ->
            11

        11 ->
            4

        12 ->
            12

        13 ->
            3

        14 ->
            13

        15 ->
            2

        16 ->
            14

        17 ->
            1

        18 ->
            15

        _ ->
            0


deoNice : Int -> Int
deoNice lvl =
    case lvl of
        1 ->
            8

        2 ->
            16

        3 ->
            16

        4 ->
            16

        5 ->
            32

        6 ->
            128

        7 ->
            128

        8 ->
            258

        9 ->
            258

        _ ->
            128


deoChain : Int -> Int
deoChain lvl =
    case lvl of
        1 ->
            4

        2 ->
            8

        3 ->
            16

        4 ->
            32

        5 ->
            32

        6 ->
            128

        7 ->
            256

        8 ->
            1024

        9 ->
            4096

        _ ->
            128


defaultPlvl : Int -> Int
defaultPlvl len =
    if len <= 2980 then
        12

    else if len <= 5806 then
        13

    else if len <= 11308 then
        14

    else if len <= 22026 then
        15

    else if len <= 42901 then
        16

    else if len <= 83561 then
        17

    else if len <= 162754 then
        18

    else if len <= 317003 then
        19

    else
        20


{-| 15-bit reversal matching `fflate`'s `rev` table (`rev[i]` for `0 <= i < 32768`; `0` for `>= 32768`).
-}
rev15 : Int -> Int
rev15 i =
    if i < 0 || i >= 32768 then
        0

    else
        let
            x1 =
                Bitwise.or
                    (Bitwise.shiftRightZfBy 1 (Bitwise.and 0xAAAA i))
                    (Bitwise.shiftLeftBy 1 (Bitwise.and 0x5555 i))

            x2 =
                Bitwise.or
                    (Bitwise.shiftRightZfBy 2 (Bitwise.and 0xCCCC x1))
                    (Bitwise.shiftLeftBy 2 (Bitwise.and 0x3333 x1))

            x3 =
                Bitwise.or
                    (Bitwise.shiftRightZfBy 4 (Bitwise.and 0xF0F0 x2))
                    (Bitwise.shiftLeftBy 4 (Bitwise.and 0x0F0F x2))
        in
        Bitwise.shiftRightZfBy 1
            (Bitwise.or
                (Bitwise.shiftRightZfBy 8 (Bitwise.and 0xFF00 x3))
                (Bitwise.shiftLeftBy 8 (Bitwise.and 0x00FF x3))
            )


buildBaseTable : (Int -> Int) -> Int -> IntTree
buildBaseTable eb start =
    let
        go i acc tree =
            if i >= 31 then
                tree

            else
                let
                    step =
                        if i == 0 then
                            1

                        else
                            Bitwise.shiftLeftBy (eb (i - 1)) 1

                    next =
                        acc + step
                in
                go (i + 1) next (intTreeSet 3 i next tree)
    in
    go 0 start Empty


flRawTree : IntTree
flRawTree =
    buildBaseTable fleb 2


flTree : IntTree
flTree =
    intTreeSet 3 28 258 flRawTree


fl : Int -> Int
fl i =
    intTreeGet 3 i flTree


revflExactTree : IntTree
revflExactTree =
    let
        fillRange i j end base tree =
            if (j - end) >= 0 then
                tree

            else
                fillRange i (j + 1) end base (intTreeSet 6 j (Bitwise.or (Bitwise.shiftLeftBy 5 (j - base)) i) tree)

        go i tree =
            if i >= 30 then
                tree

            else
                let
                    b0 =
                        intTreeGet 3 i flRawTree

                    b1 =
                        intTreeGet 3 (i + 1) flRawTree
                in
                go (i + 1) (fillRange i b0 b1 b0 tree)
    in
    intTreeSet 6 258 28 (go 1 Empty)


revfl : Int -> Int
revfl l =
    intTreeGet 6 l revflExactTree


fdTree : IntTree
fdTree =
    buildBaseTable fdeb 0


fd : Int -> Int
fd i =
    if i < 0 || i >= 31 then
        0

    else
        intTreeGet 3 i fdTree


{-| O(1) exact match for `freb(fdeb, 0).r[d]` for `1 <= d <= 32768`.
-}
revfd : Int -> Int
revfd d =
    if d <= 4 then
        if d <= 1 then
            0

        else
            d - 1

    else
        let
            m =
                d - 1

            k =
                if m < 256 then
                    if m < 16 then
                        if m < 8 then
                            2

                        else
                            3

                    else if m < 64 then
                        if m < 32 then
                            4

                        else
                            5

                    else if m < 128 then
                        6

                    else
                        7

                else if m < 4096 then
                    if m < 1024 then
                        if m < 512 then
                            8

                        else
                            9

                    else if m < 2048 then
                        10

                    else
                        11

                else if m < 16384 then
                    if m < 8192 then
                        12

                    else
                        13

                else
                    14

            eb =
                k - 1

            half =
                Bitwise.and 1 (Bitwise.shiftRightZfBy eb m)

            code =
                k + k + half

            extra =
                Bitwise.and (Bitwise.shiftLeftBy eb 1 - 1) m
        in
        Bitwise.or (Bitwise.shiftLeftBy 5 extra) code


{-| Fixed literal/length code lengths (288 entries, shift = 6).
-}
flt : IntTree
flt =
    let
        go i tree =
            if i >= 288 then
                tree

            else
                let
                    v =
                        if i < 144 then
                            8

                        else if i < 256 then
                            9

                        else if i < 280 then
                            7

                        else
                            8
                in
                go (i + 1) (intTreeSet 6 i v tree)
    in
    go 0 Empty


{-| Fixed distance code lengths (32 entries, shift = 3).
-}
fdt : IntTree
fdt =
    let
        go i tree =
            if i >= 32 then
                tree

            else
                go (i + 1) (intTreeSet 3 i 5 tree)
    in
    go 0 Empty


{-| Reflected IEEE CRC-32 table (256 entries, shift = 6).
-}
crctTree : IntTree
crctTree =
    let
        round k c =
            if k == 0 then
                c

            else
                let
                    poly =
                        if Bitwise.and 1 c /= 0 then
                            -306674912

                        else
                            0
                in
                round (k - 1) (Bitwise.xor poly (Bitwise.shiftRightZfBy 1 c))

        go i tree =
            if i >= 256 then
                tree

            else
                go (i + 1) (intTreeSet 6 i (round 8 i) tree)
    in
    go 0 Empty


crct : Int -> Int
crct i =
    intTreeGet 6 i crctTree
