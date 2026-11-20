module Sha3 exposing (digest)

{-| SHA3-256 and Keccak-256 from @noble/hashes 2.4.0 (MIT).
The state names each lane by its x and y coordinates. Each lane holds the
low and high halves of a 64-bit word so every rotation uses exact 32-bit shifts.
-}

import Bitwise exposing (and, complement, or, shiftLeftBy, shiftRightZfBy)


type alias Lane =
    { lo : Int, hi : Int }


type alias State =
    { a00 : Lane
    , a10 : Lane
    , a20 : Lane
    , a30 : Lane
    , a40 : Lane
    , a01 : Lane
    , a11 : Lane
    , a21 : Lane
    , a31 : Lane
    , a41 : Lane
    , a02 : Lane
    , a12 : Lane
    , a22 : Lane
    , a32 : Lane
    , a42 : Lane
    , a03 : Lane
    , a13 : Lane
    , a23 : Lane
    , a33 : Lane
    , a43 : Lane
    , a04 : Lane
    , a14 : Lane
    , a24 : Lane
    , a34 : Lane
    , a44 : Lane
    }


zero : Lane
zero =
    Lane 0 0


empty : State
empty =
    { a00 = zero
    , a10 = zero
    , a20 = zero
    , a30 = zero
    , a40 = zero
    , a01 = zero
    , a11 = zero
    , a21 = zero
    , a31 = zero
    , a41 = zero
    , a02 = zero
    , a12 = zero
    , a22 = zero
    , a32 = zero
    , a42 = zero
    , a03 = zero
    , a13 = zero
    , a23 = zero
    , a33 = zero
    , a43 = zero
    , a04 = zero
    , a14 = zero
    , a24 = zero
    , a34 = zero
    , a44 = zero
    }


mix : Lane -> Lane -> Lane
mix a b =
    Lane (Bitwise.xor a.lo b.lo) (Bitwise.xor a.hi b.hi)


rotate : Int -> Lane -> Lane
rotate n a =
    let
        k = and 31 n

        half x y =
            or (shiftLeftBy k x) (shiftRightZfBy (32-k) y)
    in
    if n > 32 then
        Lane (half a.hi a.lo) (half a.lo a.hi)

    else
        Lane (half a.lo a.hi) (half a.hi a.lo)


chi : Lane -> Lane -> Lane -> Lane
chi a b c =
    Lane
        (Bitwise.xor a.lo (and (complement b.lo) c.lo))
        (Bitwise.xor a.hi (and (complement b.hi) c.hi))


roundConstants : List Lane
roundConstants =
    [ Lane (1) (0)
    , Lane (32898) (0)
    , Lane (32906) (-2147483648)
    , Lane (-2147450880) (-2147483648)
    , Lane (32907) (0)
    , Lane (-2147483647) (0)
    , Lane (-2147450751) (-2147483648)
    , Lane (32777) (-2147483648)
    , Lane (138) (0)
    , Lane (136) (0)
    , Lane (-2147450871) (0)
    , Lane (-2147483638) (0)
    , Lane (-2147450741) (0)
    , Lane (139) (-2147483648)
    , Lane (32905) (-2147483648)
    , Lane (32771) (-2147483648)
    , Lane (32770) (-2147483648)
    , Lane (128) (-2147483648)
    , Lane (32778) (0)
    , Lane (-2147483638) (-2147483648)
    , Lane (-2147450751) (-2147483648)
    , Lane (32896) (-2147483648)
    , Lane (-2147483647) (0)
    , Lane (-2147450872) (-2147483648)
    ]


round : Lane -> State -> State
round constant s =
    let
        -- Theta mixes the five column parities into every lane.
        c0 =
            s.a00 |> mix s.a01 |> mix s.a02 |> mix s.a03 |> mix s.a04

        c1 =
            s.a10 |> mix s.a11 |> mix s.a12 |> mix s.a13 |> mix s.a14

        c2 =
            s.a20 |> mix s.a21 |> mix s.a22 |> mix s.a23 |> mix s.a24

        c3 =
            s.a30 |> mix s.a31 |> mix s.a32 |> mix s.a33 |> mix s.a34

        c4 =
            s.a40 |> mix s.a41 |> mix s.a42 |> mix s.a43 |> mix s.a44

        d0 =
            mix c4 (rotate 1 c1)

        d1 =
            mix c0 (rotate 1 c2)

        d2 =
            mix c1 (rotate 1 c3)

        d3 =
            mix c2 (rotate 1 c4)

        d4 =
            mix c3 (rotate 1 c0)

        -- Rho and Pi have a fixed 24-lane permutation cycle.
        b00 =
            mix s.a00 d0

        b02 =
            rotate 1 (mix s.a10 d1)

        b21 =
            rotate 3 (mix s.a02 d0)

        b12 =
            rotate 6 (mix s.a21 d2)

        b23 =
            rotate 10 (mix s.a12 d1)

        b33 =
            rotate 15 (mix s.a23 d2)

        b30 =
            rotate 21 (mix s.a33 d3)

        b01 =
            rotate 28 (mix s.a30 d3)

        b13 =
            rotate 36 (mix s.a01 d0)

        b31 =
            rotate 45 (mix s.a13 d1)

        b14 =
            rotate 55 (mix s.a31 d3)

        b44 =
            rotate 2 (mix s.a14 d1)

        b40 =
            rotate 14 (mix s.a44 d4)

        b03 =
            rotate 27 (mix s.a40 d4)

        b34 =
            rotate 41 (mix s.a03 d0)

        b43 =
            rotate 56 (mix s.a34 d3)

        b32 =
            rotate 8 (mix s.a43 d4)

        b22 =
            rotate 25 (mix s.a32 d3)

        b20 =
            rotate 43 (mix s.a22 d2)

        b04 =
            rotate 62 (mix s.a20 d2)

        b42 =
            rotate 18 (mix s.a04 d0)

        b24 =
            rotate 39 (mix s.a42 d4)

        b41 =
            rotate 61 (mix s.a24 d2)

        b11 =
            rotate 20 (mix s.a41 d4)

        b10 =
            rotate 44 (mix s.a11 d1)

    in
    { a00 = mix constant (chi b00 b10 b20)
    , a10 = chi b10 b20 b30
    , a20 = chi b20 b30 b40
    , a30 = chi b30 b40 b00
    , a40 = chi b40 b00 b10
    , a01 = chi b01 b11 b21
    , a11 = chi b11 b21 b31
    , a21 = chi b21 b31 b41
    , a31 = chi b31 b41 b01
    , a41 = chi b41 b01 b11
    , a02 = chi b02 b12 b22
    , a12 = chi b12 b22 b32
    , a22 = chi b22 b32 b42
    , a32 = chi b32 b42 b02
    , a42 = chi b42 b02 b12
    , a03 = chi b03 b13 b23
    , a13 = chi b13 b23 b33
    , a23 = chi b23 b33 b43
    , a33 = chi b33 b43 b03
    , a43 = chi b43 b03 b13
    , a04 = chi b04 b14 b24
    , a14 = chi b14 b24 b34
    , a24 = chi b24 b34 b44
    , a34 = chi b34 b44 b04
    , a44 = chi b44 b04 b14
    }


word : Int -> Int -> Int -> Int -> Int
word a b c d =
    or (or a (shiftLeftBy 8 b)) (or (shiftLeftBy 16 c) (shiftLeftBy 24 d))


-- Decode the 136-byte rate block as 17 little-endian lanes.
decodeLanes : List Int -> List Lane -> List Lane
decodeLanes bytes reversed =
    case bytes of
        a :: b :: c :: d :: e :: f :: g :: h :: rest ->
            decodeLanes rest (Lane (word a b c d) (word e f g h) :: reversed)

        _ ->
            List.reverse reversed


absorb : List Int -> State -> State
absorb bytes state =
    case decodeLanes bytes [] of
        [ l0, l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14, l15, l16 ] ->
            let
                absorbed =
                    { state
                        | a00 = mix state.a00 l0
                        , a10 = mix state.a10 l1
                        , a20 = mix state.a20 l2
                        , a30 = mix state.a30 l3
                        , a40 = mix state.a40 l4
                        , a01 = mix state.a01 l5
                        , a11 = mix state.a11 l6
                        , a21 = mix state.a21 l7
                        , a31 = mix state.a31 l8
                        , a41 = mix state.a41 l9
                        , a02 = mix state.a02 l10
                        , a12 = mix state.a12 l11
                        , a22 = mix state.a22 l12
                        , a32 = mix state.a32 l13
                        , a42 = mix state.a42 l14
                        , a03 = mix state.a03 l15
                        , a13 = mix state.a13 l16
                    }
            in
            List.foldl round absorbed roundConstants

        _ ->
            state


type alias Sponge =
    { state : State, remainder : List Int, used : Int }


update : List Int -> Sponge -> Sponge
update chunk sponge =
    let
        loop bytes state reversed used =
            case bytes of
                [] ->
                    Sponge state reversed used

                byte :: rest ->
                    if used == 135 then
                        loop rest (absorb (List.reverse (byte :: reversed)) state) [] 0

                    else
                        loop rest state (byte :: reversed) (used+1)
    in
    loop chunk sponge.state sponge.remainder sponge.used


bytesOf : Lane -> List Int
bytesOf lane =
    let
        byte i x = and 255 (shiftRightZfBy i x)
    in
    [ byte 0 lane.lo, byte 8 lane.lo, byte 16 lane.lo, byte 24 lane.lo
    , byte 0 lane.hi, byte 8 lane.hi, byte 16 lane.hi, byte 24 lane.hi
    ]


digest : Bool -> List (List Int) -> List Int
digest legacy chunks =
    let
        sponge =
            List.foldl update (Sponge empty [] 0) chunks

        suffix =
            if legacy then 1 else 6

        padded =
            if sponge.used == 135 then
                List.reverse ((or suffix 128) :: sponge.remainder)

            else
                List.reverse (suffix :: sponge.remainder) ++ List.repeat (134 - sponge.used) 0 ++ [128]

        final =
            absorb padded sponge.state
    in
    List.concatMap bytesOf [ final.a00, final.a10, final.a20, final.a30 ]
