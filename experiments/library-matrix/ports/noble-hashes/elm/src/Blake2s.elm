module Blake2s exposing (State, blake2s)

{-| noble's \_BLAKE2s, whose IV is SHA-256's, with \_blake.ts's G1s and G2s.

noble's \_compress keeps v0..v15 in locals and G1s/G2s return the four words they mix; here
they are records. BLAKE2s does not share its mix with BLAKE2b.

-}

import Array exposing (Array)
import Bitwise exposing (complement, or)
import Blake2 exposing (Blake2, Options)
import Sha2
import U64
import Words exposing (at, rotr)


{-| The hash words h0..h7.
-}
type alias State =
    List Int


{-| The work vector of one compression.
-}
type alias V =
    { v0 : Int, v1 : Int, v2 : Int, v3 : Int, v4 : Int, v5 : Int, v6 : Int, v7 : Int
    , v8 : Int, v9 : Int, v10 : Int, v11 : Int, v12 : Int, v13 : Int, v14 : Int, v15 : Int
    }


{-| The four words a G call mixes.
-}
type alias Quad =
    { a : Int, b : Int, c : Int, d : Int }


{-| G's first half: a = a + b + x, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 12).
-}
g1s : Int -> Quad -> Quad
g1s x { a, b, c, d } =
    let
        a1 =
            or 0 (a + b + x)

        d1 =
            rotr (Bitwise.xor d a1) 16

        c1 =
            or 0 (c + d1)
    in
    { a = a1, b = rotr (Bitwise.xor b c1) 12, c = c1, d = d1 }


{-| G's second half: a = a + b + y, d = rotr(d ^ a, 8), c = c + d, b = rotr(b ^ c, 7).
-}
g2s : Int -> Quad -> Quad
g2s x { a, b, c, d } =
    let
        a1 =
            or 0 (a + b + x)

        d1 =
            rotr (Bitwise.xor d a1) 8

        c1 =
            or 0 (c + d1)
    in
    { a = a1, b = rotr (Bitwise.xor b c1) 7, c = c1, d = d1 }


{-| One round: G on the four columns, then on the four diagonals.
-}
round : Array Int -> Int -> V -> V
round m r v =
    let
        x i =
            at (at (16 * r + i) Blake2.bsigma) m

        col0 =
            g2s (x 1) (g1s (x 0) { a = v.v0, b = v.v4, c = v.v8, d = v.v12 })

        col1 =
            g2s (x 3) (g1s (x 2) { a = v.v1, b = v.v5, c = v.v9, d = v.v13 })

        col2 =
            g2s (x 5) (g1s (x 4) { a = v.v2, b = v.v6, c = v.v10, d = v.v14 })

        col3 =
            g2s (x 7) (g1s (x 6) { a = v.v3, b = v.v7, c = v.v11, d = v.v15 })

        dia0 =
            g2s (x 9) (g1s (x 8) { a = col0.a, b = col1.b, c = col2.c, d = col3.d })

        dia1 =
            g2s (x 11) (g1s (x 10) { a = col1.a, b = col2.b, c = col3.c, d = col0.d })

        dia2 =
            g2s (x 13) (g1s (x 12) { a = col2.a, b = col3.b, c = col0.c, d = col1.d })

        dia3 =
            g2s (x 15) (g1s (x 14) { a = col3.a, b = col0.b, c = col1.c, d = col2.d })
    in
    { v0 = dia0.a, v1 = dia1.a, v2 = dia2.a, v3 = dia3.a
    , v4 = dia3.b, v5 = dia0.b, v6 = dia1.b, v7 = dia2.b
    , v8 = dia2.c, v9 = dia3.c, v10 = dia0.c, v11 = dia1.c
    , v12 = dia1.d, v13 = dia2.d, v14 = dia3.d, v15 = dia0.d
    }


rounds : Array Int -> Int -> V -> V
rounds m r v =
    if r == 10 then
        v

    else
        rounds m (r + 1) (round m r v)


ivWords : Array Int
ivWords =
    Array.fromList Sha2.sha256IV


compress : List Int -> Int -> Bool -> State -> State
compress block length isLast state =
    let
        words =
            Array.fromList state

        h i =
            at i words

        iv i =
            at i ivWords

        -- v8..v15 come from the IV, with the byte counter in v12 and v13 and v14 inverted last.
        v =
            rounds (Array.fromList (Words.wordsLE block))
                0
                { v0 = h 0, v1 = h 1, v2 = h 2, v3 = h 3, v4 = h 4, v5 = h 5, v6 = h 6, v7 = h 7
                , v8 = iv 0, v9 = iv 1, v10 = iv 2, v11 = iv 3
                , v12 = Bitwise.xor (U64.fromNumL length) (iv 4)
                , v13 = Bitwise.xor (U64.fromNumH length) (iv 5)
                , v14 = if isLast then complement (iv 6) else iv 6
                , v15 = iv 7
                }
    in
    List.map3 (\hi vi vj -> Bitwise.xor hi (Bitwise.xor vi vj))
        state
        [ v.v0, v.v1, v.v2, v.v3, v.v4, v.v5, v.v6, v.v7 ]
        [ v.v8, v.v9, v.v10, v.v11, v.v12, v.v13, v.v14, v.v15 ]


blake2s : Options -> Result String (Blake2 State)
blake2s opts =
    let
        words =
            Maybe.map Words.wordsLE >> Maybe.withDefault [ 0, 0 ]

        init outputLen =
            -- p[0] goes into h0, the salt into h4 and h5, the personalization into h6 and h7.
            List.map2 Bitwise.xor
                Sha2.sha256IV
                (Blake2.paramWord outputLen opts :: 0 :: 0 :: 0 :: words opts.salt ++ words opts.personalization)
    in
    Blake2.make { blockLen = 64, keyLen = 32, saltLen = 8, persLen = 8 } opts init compress identity
