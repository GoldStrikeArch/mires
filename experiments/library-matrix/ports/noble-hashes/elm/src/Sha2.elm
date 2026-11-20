module Sha2 exposing (Sha256, Sha512, sha256, sha256IV, sha512)

{-| noble's sha2.ts: SHA-256 (SHA2\_32B) and SHA-512 (SHA2\_64B), with the IVs from \_md.ts.
-}

import Array exposing (Array)
import Bitwise exposing (or, shiftRightZfBy)
import Md exposing (Md, chi, maj)
import U64 exposing (add, add3H, add3L, add4H, add4L, add5H, add5L, rotrBH, rotrBL, rotrSH, rotrSL, shrSH, shrSL)
import Words exposing (at, rotr)


sha256IV : List Int
sha256IV =
    [ 0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19 ]


sha256K : Array Int
sha256K =
    Array.fromList
        [ 0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5
        , 0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174
        , 0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA
        , 0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7, 0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967
        , 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13, 0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85
        , 0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3, 0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070
        , 0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3
        , 0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7, 0xC67178F2
        ]


{-| noble's SHA2\_32B state: A ... H.
-}
type alias Sha256 =
    { a : Int, b : Int, c : Int, d : Int, e : Int, f : Int, g : Int, h : Int }


sha256 : Md Sha256
sha256 =
    let
        words =
            Array.fromList sha256IV

        iv i =
            at i words
    in
    Md.make 64 32 8 (Sha256 (iv 0) (iv 1) (iv 2) (iv 3) (iv 4) (iv 5) (iv 6) (iv 7)) process256 (\s -> [ s.a, s.b, s.c, s.d, s.e, s.f, s.g, s.h ])


{-| Extends the 16 words of a block into the 64-word message schedule.
-}
schedule256 : Int -> Array Int -> Array Int
schedule256 i w =
    if i == 64 then
        w

    else
        let
            w15 =
                at (i - 15) w

            w2 =
                at (i - 2) w

            s0 =
                Bitwise.xor (Bitwise.xor (rotr w15 7) (rotr w15 18)) (shiftRightZfBy 3 w15)

            s1 =
                Bitwise.xor (Bitwise.xor (rotr w2 17) (rotr w2 19)) (shiftRightZfBy 10 w2)
        in
        schedule256 (i + 1) (Array.push (or 0 (s1 + at (i - 7) w + s0 + at (i - 16) w)) w)


{-| Round `i` onwards of the compression function.
-}
rounds256 : Int -> Array Int -> Sha256 -> Sha256
rounds256 i w s =
    if i == 64 then
        s

    else
        let
            sigma1 =
                Bitwise.xor (Bitwise.xor (rotr s.e 6) (rotr s.e 11)) (rotr s.e 25)

            t1 =
                or 0 (s.h + sigma1 + chi s.e s.f s.g + at i sha256K + at i w)

            sigma0 =
                Bitwise.xor (Bitwise.xor (rotr s.a 2) (rotr s.a 13)) (rotr s.a 22)

            t2 =
                or 0 (sigma0 + maj s.a s.b s.c)
        in
        rounds256 (i + 1) w (Sha256 (or 0 (t1 + t2)) s.a s.b s.c (or 0 (s.d + t1)) s.e s.f s.g)


process256 : List Int -> Sha256 -> Sha256
process256 block s =
    let
        v =
            rounds256 0 (schedule256 16 (Array.fromList (Words.wordsBE block))) s
    in
    -- Add the compressed block to the state.
    Sha256 (or 0 (s.a + v.a))
        (or 0 (s.b + v.b))
        (or 0 (s.c + v.c))
        (or 0 (s.d + v.d))
        (or 0 (s.e + v.e))
        (or 0 (s.f + v.f))
        (or 0 (s.g + v.g))
        (or 0 (s.h + v.h))


sha512IV : List Int
sha512IV =
    [ 0x6A09E667, 0xF3BCC908, 0xBB67AE85, 0x84CAA73B, 0x3C6EF372, 0xFE94F82B, 0xA54FF53A, 0x5F1D36F1
    , 0x510E527F, 0xADE682D1, 0x9B05688C, 0x2B3E6C1F, 0x1F83D9AB, 0xFB41BD6B, 0x5BE0CD19, 0x137E2179
    ]


{-| The 80 round constants of RFC 6234 §5.2, split into high and low halves as noble's K512 is.
-}
sha512Kh : Array Int
sha512Kh =
    Array.fromList
        [ 0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5, 0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5
        , 0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3, 0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174
        , 0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC, 0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA
        , 0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7, 0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967
        , 0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13, 0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85
        , 0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3, 0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070
        , 0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5, 0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3
        , 0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208, 0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7, 0xC67178F2
        , 0xCA273ECE, 0xD186B8C7, 0xEADA7DD6, 0xF57D4F7F, 0x06F067AA, 0x0A637DC5, 0x113F9804, 0x1B710B35
        , 0x28DB77F5, 0x32CAAB7B, 0x3C9EBE0A, 0x431D67C4, 0x4CC5D4BE, 0x597F299C, 0x5FCB6FAB, 0x6C44198C
        ]


sha512Kl : Array Int
sha512Kl =
    Array.fromList
        [ 0xD728AE22, 0x23EF65CD, 0xEC4D3B2F, 0x8189DBBC, 0xF348B538, 0xB605D019, 0xAF194F9B, 0xDA6D8118
        , 0xA3030242, 0x45706FBE, 0x4EE4B28C, 0xD5FFB4E2, 0xF27B896F, 0x3B1696B1, 0x25C71235, 0xCF692694
        , 0x9EF14AD2, 0x384F25E3, 0x8B8CD5B5, 0x77AC9C65, 0x592B0275, 0x6EA6E483, 0xBD41FBD4, 0x831153B5
        , 0xEE66DFAB, 0x2DB43210, 0x98FB213F, 0xBEEF0EE4, 0x3DA88FC2, 0x930AA725, 0xE003826F, 0x0A0E6E70
        , 0x46D22FFC, 0x5C26C926, 0x5AC42AED, 0x9D95B3DF, 0x8BAF63DE, 0x3C77B2A8, 0x47EDAEE6, 0x1482353B
        , 0x4CF10364, 0xBC423001, 0xD0F89791, 0x0654BE30, 0xD6EF5218, 0x5565A910, 0x5771202A, 0x32BBD1B8
        , 0xB8D2D0C8, 0x5141AB53, 0xDF8EEB99, 0xE19B48A8, 0xC5C95A63, 0xE3418ACB, 0x7763E373, 0xD6B2B8A3
        , 0x5DEFB2FC, 0x43172F60, 0xA1F0AB72, 0x1A6439EC, 0x23631E28, 0xDE82BDE9, 0xB2C67915, 0xE372532B
        , 0xEA26619C, 0x21C0C207, 0xCDE0EB1E, 0xEE6ED178, 0x72176FBA, 0xA2C898A6, 0xBEF90DAE, 0x131C471B
        , 0x23047D84, 0x40C72493, 0x15C9BEBC, 0x9C100D4C, 0xCB3E42B6, 0xFC657E2A, 0x3AD6FAEC, 0x4A475817
        ]


{-| noble's SHA2\_64B state: Ah, Al, ..., Hh, Hl.
-}
type alias Sha512 =
    { ah : Int, al : Int, bh : Int, bl : Int, ch : Int, cl : Int, dh : Int, dl : Int
    , eh : Int, el : Int, fh : Int, fl : Int, gh : Int, gl : Int, hh : Int, hl : Int
    }


sha512 : Md Sha512
sha512 =
    Md.make 128 64 16 (fromWords sha512IV) process512 toWords


fromWords : List Int -> Sha512
fromWords words =
    let
        array =
            Array.fromList words

        word i =
            at i array
    in
    { ah = word 0, al = word 1, bh = word 2, bl = word 3, ch = word 4, cl = word 5, dh = word 6, dl = word 7
    , eh = word 8, el = word 9, fh = word 10, fl = word 11, gh = word 12, gl = word 13, hh = word 14, hl = word 15
    }


toWords : Sha512 -> List Int
toWords s =
    [ s.ah, s.al, s.bh, s.bl, s.ch, s.cl, s.dh, s.dl, s.eh, s.el, s.fh, s.fl, s.gh, s.gl, s.hh, s.hl ]


{-| Extends the 16 words of a block into the 80-word message schedule, as high halves `wh` and
low halves `wl` (noble's SHA512\_W\_H and SHA512\_W\_L).
-}
schedule512 : Int -> Array Int -> Array Int -> ( Array Int, Array Int )
schedule512 i wh wl =
    if i == 80 then
        ( wh, wl )

    else
        let
            -- s0 = rotr(w[i-15], 1) ^ rotr(w[i-15], 8) ^ (w[i-15] >>> 7)
            w15h =
                at (i - 15) wh

            w15l =
                at (i - 15) wl

            s0h =
                Bitwise.xor (Bitwise.xor (rotrSH w15h w15l 1) (rotrSH w15h w15l 8)) (shrSH w15h w15l 7)

            s0l =
                Bitwise.xor (Bitwise.xor (rotrSL w15h w15l 1) (rotrSL w15h w15l 8)) (shrSL w15h w15l 7)

            -- s1 = rotr(w[i-2], 19) ^ rotr(w[i-2], 61) ^ (w[i-2] >>> 6)
            w2h =
                at (i - 2) wh

            w2l =
                at (i - 2) wl

            s1h =
                Bitwise.xor (Bitwise.xor (rotrSH w2h w2l 19) (rotrBH w2h w2l 61)) (shrSH w2h w2l 6)

            s1l =
                Bitwise.xor (Bitwise.xor (rotrSL w2h w2l 19) (rotrBL w2h w2l 61)) (shrSL w2h w2l 6)

            sumL =
                add4L s0l s1l (at (i - 7) wl) (at (i - 16) wl)

            sumH =
                add4H sumL s0h s1h (at (i - 7) wh) (at (i - 16) wh)
        in
        schedule512 (i + 1) (Array.push sumH wh) (Array.push (or 0 sumL) wl)


{-| Round `i` onwards of the compression function.
-}
rounds512 : Int -> Array Int -> Array Int -> Sha512 -> Sha512
rounds512 i wh wl s =
    if i == 80 then
        s

    else
        let
            -- S1 = rotr(e, 14) ^ rotr(e, 18) ^ rotr(e, 41)
            sigma1h =
                Bitwise.xor (Bitwise.xor (rotrSH s.eh s.el 14) (rotrSH s.eh s.el 18)) (rotrBH s.eh s.el 41)

            sigma1l =
                Bitwise.xor (Bitwise.xor (rotrSL s.eh s.el 14) (rotrSL s.eh s.el 18)) (rotrBL s.eh s.el 41)

            -- T1 = H + S1 + Chi(E, F, G) + K[i] + W[i]
            t1ll =
                add5L s.hl sigma1l (chi s.el s.fl s.gl) (at i sha512Kl) (at i wl)

            t1h =
                add5H t1ll s.hh sigma1h (chi s.eh s.fh s.gh) (at i sha512Kh) (at i wh)

            t1l =
                or 0 t1ll

            -- S0 = rotr(a, 28) ^ rotr(a, 34) ^ rotr(a, 39)
            sigma0h =
                Bitwise.xor (Bitwise.xor (rotrSH s.ah s.al 28) (rotrBH s.ah s.al 34)) (rotrBH s.ah s.al 39)

            sigma0l =
                Bitwise.xor (Bitwise.xor (rotrSL s.ah s.al 28) (rotrBL s.ah s.al 34)) (rotrBL s.ah s.al 39)

            e =
                add s.dh s.dl t1h t1l

            all =
                add3L t1l sigma0l (maj s.al s.bl s.cl)
        in
        rounds512 (i + 1)
            wh
            wl
            { ah = add3H all t1h sigma0h (maj s.ah s.bh s.ch)
            , al = or 0 all
            , bh = s.ah
            , bl = s.al
            , ch = s.bh
            , cl = s.bl
            , dh = s.ch
            , dl = s.cl
            , eh = e.h
            , el = e.l
            , fh = s.eh
            , fl = s.el
            , gh = s.fh
            , gl = s.fl
            , hh = s.gh
            , hl = s.gl
            }


process512 : List Int -> Sha512 -> Sha512
process512 block s =
    let
        words =
            Array.fromList (Words.wordsBE block)

        -- Even words are the high halves, odd words the low ones.
        ( wh, wl ) =
            schedule512 16 (Array.initialize 16 (\i -> at (2 * i) words)) (Array.initialize 16 (\i -> at (2 * i + 1) words))

        v =
            rounds512 0 wh wl s

        -- Add the compressed block to the state.
        a =
            add s.ah s.al v.ah v.al

        b =
            add s.bh s.bl v.bh v.bl

        c =
            add s.ch s.cl v.ch v.cl

        d =
            add s.dh s.dl v.dh v.dl

        e =
            add s.eh s.el v.eh v.el

        f =
            add s.fh s.fl v.fh v.fl

        g =
            add s.gh s.gl v.gh v.gl

        h =
            add s.hh s.hl v.hh v.hl
    in
    { ah = a.h, al = a.l, bh = b.h, bl = b.l, ch = c.h, cl = c.l, dh = d.h, dl = d.l
    , eh = e.h, el = e.l, fh = f.h, fl = f.l, gh = g.h, gl = g.l, hh = h.h, hl = h.l
    }
