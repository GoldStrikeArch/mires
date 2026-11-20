module Blake2b exposing (State, blake2b)

{-| noble's \_BLAKE2b with its G1b and G2b on 64-bit words as high and low halves.

noble's G1b and G2b read the four words they mix from its work vector BBUF and write them back;
here they take and return them as a record, and the work vector is a record of words.
BLAKE2b does not share its mix with BLAKE2s.

-}

import Array exposing (Array)
import Bitwise exposing (complement, or)
import Blake2 exposing (Blake2, Options)
import U64 exposing (Word, add, add3H, add3L, rotr32H, rotr32L, rotrBH, rotrBL, rotrSH, rotrSL)
import Words exposing (at)


{-| The hash words h0..h7.
-}
type alias State =
    List Word


{-| The SHA-512 IV as high and low halves (noble's B2B\_IV stores them low first).
-}
iv : Array Word
iv =
    Array.fromList
        [ Word 0x6A09E667 0xF3BCC908, Word 0xBB67AE85 0x84CAA73B, Word 0x3C6EF372 0xFE94F82B, Word 0xA54FF53A 0x5F1D36F1
        , Word 0x510E527F 0xADE682D1, Word 0x9B05688C 0x2B3E6C1F, Word 0x1F83D9AB 0xFB41BD6B, Word 0x5BE0CD19 0x137E2179
        ]


{-| The work vector of one compression.
-}
type alias V =
    { v0 : Word, v1 : Word, v2 : Word, v3 : Word, v4 : Word, v5 : Word, v6 : Word, v7 : Word
    , v8 : Word, v9 : Word, v10 : Word, v11 : Word, v12 : Word, v13 : Word, v14 : Word, v15 : Word
    }


{-| The four words a G call mixes.
-}
type alias Quad =
    { a : Word, b : Word, c : Word, d : Word }


xor : Word -> Word -> Word
xor x y =
    { h = Bitwise.xor x.h y.h, l = Bitwise.xor x.l y.l }


{-| G's first half: a = a + b + x, d = rotr(d ^ a, 32), c = c + d, b = rotr(b ^ c, 24).
-}
g1b : Word -> Quad -> Quad
g1b x { a, b, c, d } =
    let
        ll =
            add3L a.l b.l x.l

        a1 =
            { h = add3H ll a.h b.h x.h, l = or 0 ll }

        dx =
            xor d a1

        d1 =
            { h = rotr32H dx.h dx.l, l = rotr32L dx.h dx.l }

        c1 =
            add c.h c.l d1.h d1.l

        bx =
            xor b c1
    in
    { a = a1, b = { h = rotrSH bx.h bx.l 24, l = rotrSL bx.h bx.l 24 }, c = c1, d = d1 }


{-| G's second half: a = a + b + y, d = rotr(d ^ a, 16), c = c + d, b = rotr(b ^ c, 63).
-}
g2b : Word -> Quad -> Quad
g2b x { a, b, c, d } =
    let
        ll =
            add3L a.l b.l x.l

        a1 =
            { h = add3H ll a.h b.h x.h, l = or 0 ll }

        dx =
            xor d a1

        d1 =
            { h = rotrSH dx.h dx.l 16, l = rotrSL dx.h dx.l 16 }

        c1 =
            add c.h c.l d1.h d1.l

        bx =
            xor b c1
    in
    { a = a1, b = { h = rotrBH bx.h bx.l 63, l = rotrBL bx.h bx.l 63 }, c = c1, d = d1 }


{-| One round: G on the four columns, then on the four diagonals.
-}
round : Array Word -> Int -> V -> V
round m r v =
    let
        x i =
            Array.get (at (16 * r + i) Blake2.bsigma) m |> Maybe.withDefault (Word 0 0)

        col0 =
            g2b (x 1) (g1b (x 0) { a = v.v0, b = v.v4, c = v.v8, d = v.v12 })

        col1 =
            g2b (x 3) (g1b (x 2) { a = v.v1, b = v.v5, c = v.v9, d = v.v13 })

        col2 =
            g2b (x 5) (g1b (x 4) { a = v.v2, b = v.v6, c = v.v10, d = v.v14 })

        col3 =
            g2b (x 7) (g1b (x 6) { a = v.v3, b = v.v7, c = v.v11, d = v.v15 })

        dia0 =
            g2b (x 9) (g1b (x 8) { a = col0.a, b = col1.b, c = col2.c, d = col3.d })

        dia1 =
            g2b (x 11) (g1b (x 10) { a = col1.a, b = col2.b, c = col3.c, d = col0.d })

        dia2 =
            g2b (x 13) (g1b (x 12) { a = col2.a, b = col3.b, c = col0.c, d = col1.d })

        dia3 =
            g2b (x 15) (g1b (x 14) { a = col3.a, b = col0.b, c = col1.c, d = col2.d })
    in
    { v0 = dia0.a, v1 = dia1.a, v2 = dia2.a, v3 = dia3.a
    , v4 = dia3.b, v5 = dia0.b, v6 = dia1.b, v7 = dia2.b
    , v8 = dia2.c, v9 = dia3.c, v10 = dia0.c, v11 = dia1.c
    , v12 = dia1.d, v13 = dia2.d, v14 = dia3.d, v15 = dia0.d
    }


rounds : Array Word -> Int -> V -> V
rounds m r v =
    if r == 12 then
        v

    else
        rounds m (r + 1) (round m r v)


{-| 64-bit words from little-endian 32-bit words, low half first.
-}
toWords : List Int -> List Word
toWords halves =
    case halves of
        l :: h :: rest ->
            Word h l :: toWords rest

        _ ->
            []


compress : List Int -> Int -> Bool -> State -> State
compress block length isLast state =
    let
        hs =
            Array.fromList state

        h i =
            Array.get i hs |> Maybe.withDefault (Word 0 0)

        k i =
            Array.get i iv |> Maybe.withDefault (Word 0 0)

        -- The byte counter goes into v12; v14 is inverted on the last block.
        v12 =
            k 4

        v14 =
            k 6

        v =
            rounds (Array.fromList (toWords (Words.wordsLE block)))
                0
                { v0 = h 0, v1 = h 1, v2 = h 2, v3 = h 3, v4 = h 4, v5 = h 5, v6 = h 6, v7 = h 7
                , v8 = k 0, v9 = k 1, v10 = k 2, v11 = k 3
                , v12 = { h = Bitwise.xor v12.h (U64.fromNumH length), l = Bitwise.xor v12.l (U64.fromNumL length) }
                , v13 = k 5
                , v14 = if isLast then { h = complement v14.h, l = complement v14.l } else v14
                , v15 = k 7
                }
    in
    List.map3 (\hi vi vj -> xor hi (xor vi vj))
        state
        [ v.v0, v.v1, v.v2, v.v3, v.v4, v.v5, v.v6, v.v7 ]
        [ v.v8, v.v9, v.v10, v.v11, v.v12, v.v13, v.v14, v.v15 ]


blake2b : Options -> Result String (Blake2 State)
blake2b opts =
    let
        words =
            Maybe.map (Words.wordsLE >> toWords) >> Maybe.withDefault [ Word 0 0, Word 0 0 ]

        init outputLen =
            -- p[0] goes into the low half of h0, the salt into h4 and h5, the personalization
            -- into h6 and h7.
            List.map2 xor
                (Array.toList iv)
                (Word 0 (Blake2.paramWord outputLen opts) :: Word 0 0 :: Word 0 0 :: Word 0 0 :: words opts.salt ++ words opts.personalization)
    in
    Blake2.make { blockLen = 128, keyLen = 64, saltLen = 16, persLen = 16 } opts init compress (List.concatMap (\w -> [ w.l, w.h ]))
