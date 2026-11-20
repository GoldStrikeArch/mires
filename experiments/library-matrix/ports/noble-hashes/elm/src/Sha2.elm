module Sha2 exposing (digest)

import Array exposing (Array)
import Bitwise as B


iv256 : Array Int
iv256 =
    Array.fromList
        [ 0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19
        ]


iv512 : Array Int
iv512 =
    Array.fromList
        [ 0x6a09e667, 0xf3bcc908, 0xbb67ae85, 0x84caa73b, 0x3c6ef372, 0xfe94f82b, 0xa54ff53a, 0x5f1d36f1
        , 0x510e527f, 0xade682d1, 0x9b05688c, 0x2b3e6c1f, 0x1f83d9ab, 0xfb41bd6b, 0x5be0cd19, 0x137e2179
        ]


k256 : Array Int
k256 =
    Array.fromList
        [ 0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5
        , 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174
        , 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da
        , 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967
        , 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85
        , 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070
        , 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3
        , 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
        ]


k512h : Array Int
k512h =
    Array.fromList
        [ 0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5
        , 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174
        , 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da
        , 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967
        , 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85
        , 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070
        , 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3
        , 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
        , 0xca273ece, 0xd186b8c7, 0xeada7dd6, 0xf57d4f7f, 0x06f067aa, 0x0a637dc5, 0x113f9804, 0x1b710b35
        , 0x28db77f5, 0x32caab7b, 0x3c9ebe0a, 0x431d67c4, 0x4cc5d4be, 0x597f299c, 0x5fcb6fab, 0x6c44198c
        ]


k512l : Array Int
k512l =
    Array.fromList
        [ 0xd728ae22, 0x23ef65cd, 0xec4d3b2f, 0x8189dbbc, 0xf348b538, 0xb605d019, 0xaf194f9b, 0xda6d8118
        , 0xa3030242, 0x45706fbe, 0x4ee4b28c, 0xd5ffb4e2, 0xf27b896f, 0x3b1696b1, 0x25c71235, 0xcf692694
        , 0x9ef14ad2, 0x384f25e3, 0x8b8cd5b5, 0x77ac9c65, 0x592b0275, 0x6ea6e483, 0xbd41fbd4, 0x831153b5
        , 0xee66dfab, 0x2db43210, 0x98fb213f, 0xbeef0ee4, 0x3da88fc2, 0x930aa725, 0xe003826f, 0x0a0e6e70
        , 0x46d22ffc, 0x5c26c926, 0x5ac42aed, 0x9d95b3df, 0x8baf63de, 0x3c77b2a8, 0x47edaee6, 0x1482353b
        , 0x4cf10364, 0xbc423001, 0xd0f89791, 0x0654be30, 0xd6ef5218, 0x5565a910, 0x5771202a, 0x32bbd1b8
        , 0xb8d2d0c8, 0x5141ab53, 0xdf8eeb99, 0xe19b48a8, 0xc5c95a63, 0xe3418acb, 0x7763e373, 0xd6b2b8a3
        , 0x5defb2fc, 0x43172f60, 0xa1f0ab72, 0x1a6439ec, 0x23631e28, 0xde82bde9, 0xb2c67915, 0xe372532b
        , 0xea26619c, 0x21c0c207, 0xcde0eb1e, 0xee6ed178, 0x72176fba, 0xa2c898a6, 0xbef90dae, 0x131c471b
        , 0x23047d84, 0x40c72493, 0x15c9bebc, 0x9c100d4c, 0xcb3e42b6, 0xfc657e2a, 0x3ad6faec, 0x4a475817
        ]


type alias Word64 =
    ( Int, Int )


type alias Registers word =
    { a : word, b : word, c : word, d : word, e : word, f : word, g : word, h : word }


get zero i arr =
    Array.get i arr |> Maybe.withDefault zero


registers at =
    Registers (at 0) (at 1) (at 2) (at 3) (at 4) (at 5) (at 6) (at 7)


words r =
    [ r.a, r.b, r.c, r.d, r.e, r.f, r.g, r.h ]


add32 a b =
    B.or 0 (a + b)


add64 ( ah, al ) ( bh, bl ) =
    let
        lo =
            B.shiftRightZfBy 0 al + B.shiftRightZfBy 0 bl

        carry =
            if lo >= 4294967296 then
                1

            else
                0
    in
    ( B.or 0 (ah + bh + carry), B.or 0 lo )


xor3 a b c =
    B.xor a (B.xor b c)


choose a b c =
    B.xor (B.and a b) (B.and (B.complement a) c)


majority a b c =
    B.or (B.and a b) (B.and (B.or a b) c)


ror32 n a =
    B.or (B.shiftRightZfBy n a) (B.shiftLeftBy (32 - n) a)


ror64 n ( hi, lo ) =
    let
        rotate count a b =
            B.or (B.shiftRightZfBy count a) (B.shiftLeftBy (32 - count) b)
    in
    if n < 32 then
        ( rotate n hi lo, rotate n lo hi )

    else
        ( rotate (n - 32) lo hi, rotate (n - 32) hi lo )


shr64 n ( hi, lo ) =
    ( B.shiftRightZfBy n hi, B.or (B.shiftRightZfBy n lo) (B.shiftLeftBy (32 - n) hi) )


lanes op ( ah, al ) ( bh, bl ) ( ch, cl ) =
    ( op ah bh ch, op al bl cl )


word bytes i =
    let
        at offset =
            get 0 (i * 4 + offset) bytes
    in
    B.or (B.or (B.shiftLeftBy 24 (at 0)) (B.shiftLeftBy 16 (at 1))) (B.or (B.shiftLeftBy 8 (at 2)) (at 3))


schedule zero limit small0 small1 add first =
    let
        extend i w =
            if i == limit then
                w

            else
                let
                    back n =
                        get zero (i - n) w

                    next =
                        add (add (small0 (back 15)) (back 16)) (add (small1 (back 2)) (back 7))
                in
                extend (i + 1) (Array.push next w)
    in
    extend 16 (Array.initialize 16 first)


rounds zero count big0 big1 ch maj add constant w initial =
    let
        step i r =
            if i == count then
                r

            else
                let
                    t1 =
                        add (add r.h (big1 r.e)) (add (ch r.e r.f r.g) (add (constant i) (get zero i w)))

                    a =
                        add t1 (add (big0 r.a) (maj r.a r.b r.c))
                in
                step (i + 1) (Registers a r.a r.b r.c (add r.d t1) r.e r.f r.g)
    in
    List.map2 add (words initial) (words (step 0 initial))


compress256 bytes state =
    let
        small0 x =
            xor3 (ror32 7 x) (ror32 18 x) (B.shiftRightZfBy 3 x)

        small1 x =
            xor3 (ror32 17 x) (ror32 19 x) (B.shiftRightZfBy 10 x)

        big0 x =
            xor3 (ror32 2 x) (ror32 13 x) (ror32 22 x)

        big1 x =
            xor3 (ror32 6 x) (ror32 11 x) (ror32 25 x)

        w =
            schedule 0 64 small0 small1 add32 (word bytes)
    in
    rounds 0 64 big0 big1 choose majority add32 (\i -> get 0 i k256) w (registers (\i -> get 0 i state))
        |> Array.fromList


compress512 bytes state =
    let
        small0 x =
            lanes xor3 (ror64 1 x) (ror64 8 x) (shr64 7 x)

        small1 x =
            lanes xor3 (ror64 19 x) (ror64 61 x) (shr64 6 x)

        big0 x =
            lanes xor3 (ror64 28 x) (ror64 34 x) (ror64 39 x)

        big1 x =
            lanes xor3 (ror64 14 x) (ror64 18 x) (ror64 41 x)

        first i =
            ( word bytes (i * 2), word bytes (i * 2 + 1) )

        constant i =
            ( get 0 i k512h, get 0 i k512l )

        initial =
            registers (\i -> ( get 0 (i * 2) state, get 0 (i * 2 + 1) state ))

        w =
            schedule ( 0, 0 ) 80 small0 small1 add64 first
    in
    rounds ( 0, 0 ) 80 big0 big1 (lanes choose) (lanes majority) add64 constant w initial
        |> List.concatMap (\( h, l ) -> [ h, l ])
        |> Array.fromList


hash wide chunks =
    let
        ( width, initial, compress ) =
            if wide then
                ( 128, iv512, compress512 )

            else
                ( 64, iv256, compress256 )

        absorb bytes used pending state =
            case bytes of
                [] ->
                    ( used, pending, state )

                b :: rest ->
                    if used + 1 == width then
                        absorb rest 0 [] (compress (Array.fromList (List.reverse (b :: pending))) state)

                    else
                        absorb rest (used + 1) (b :: pending) state

        update bytes ( used, pending, state ) =
            absorb bytes used pending state

        ( pos, tail, prefix ) =
            List.foldl update ( 0, [], initial ) chunks

        lengthBytes n bits acc =
            if n == 0 then
                acc

            else
                lengthBytes (n - 1) (floor (toFloat bits / 256)) (B.and 255 bits :: acc)

        length =
            List.foldl (\chunk count -> count + List.length chunk) 0 chunks

        zeros =
            modBy width (width - width // 8 - pos - 1)

        padding =
            128 :: (List.repeat zeros 0 ++ lengthBytes (width // 8) (length * 8) [])

        ( _, _, result ) =
            update padding ( pos, tail, prefix )

        bytesOf x =
            List.map (\shift -> B.and 255 (B.shiftRightZfBy shift x)) [ 24, 16, 8, 0 ]
    in
    Array.toList result |> List.concatMap bytesOf


digest : Int -> List (List Int) -> List Int -> List Int
digest mode chunks key =
    if mode == 2 then
        let
            k =
                if List.length key > 64 then
                    hash False [ key ]

                else
                    key

            pad mask =
                List.map (B.xor mask) (k ++ List.repeat (64 - List.length k) 0)
        in
        hash False [ pad 0x5C, hash False (pad 0x36 :: chunks) ]

    else
        hash (mode == 1) chunks
