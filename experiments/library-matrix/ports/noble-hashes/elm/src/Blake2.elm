module Blake2 exposing (digest)

import Array exposing (Array)
import Bitwise as B


type alias Word =
    { lo : Int, hi : Int }


zero : Word
zero =
    Word 0 0


ivLow : Array Int
ivLow =
    Array.fromList [ -205731576, -2067093701, -23791573, 1595750129, -1377402159, 725511199, -79577749, 327033209 ]


ivHigh : Array Int
ivHigh =
    Array.fromList [ 1779033703, -1150833019, 1013904242, -1521486534, 1359893119, -1694144372, 528734635, 1541459225 ]


sigma : Array (Array Int)
sigma =
    Array.fromList
        [ Array.fromList [ 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15 ]
        , Array.fromList [ 14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3 ]
        , Array.fromList [ 11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4 ]
        , Array.fromList [ 7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8 ]
        , Array.fromList [ 9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13 ]
        , Array.fromList [ 2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9 ]
        , Array.fromList [ 12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11 ]
        , Array.fromList [ 13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10 ]
        , Array.fromList [ 6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5 ]
        , Array.fromList [ 10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0 ]
        ]


getWord : Int -> Array Word -> Word
getWord i words =
    Maybe.withDefault zero (Array.get i words)


getInt : Int -> Array Int -> Int
getInt i words =
    Maybe.withDefault 0 (Array.get i words)


add : Word -> Word -> Word
add a b =
    let
        low =
            B.or 0 (a.lo + b.lo)

        carry =
            if B.xor low -2147483648 < B.xor a.lo -2147483648 then
                1

            else
                0
    in
    Word low (B.or 0 (a.hi + b.hi + carry))


xor : Word -> Word -> Word
xor a b =
    Word (B.xor a.lo b.lo) (B.xor a.hi b.hi)


rotate : Bool -> Int -> Word -> Word
rotate wide n w =
    let
        combine l h bits =
            B.or (B.shiftRightZfBy bits l) (B.shiftLeftBy (32 - bits) h)
    in
    if not wide then
        Word (combine w.lo w.lo n) 0

    else if n == 32 then
        Word w.hi w.lo

    else if n > 32 then
        Word (combine w.hi w.lo (n - 32)) (combine w.lo w.hi (n - 32))

    else
        Word (combine w.lo w.hi n) (combine w.hi w.lo n)


initial : Bool -> Int -> Word
initial wide i =
    if wide then
        Word (getInt i ivLow) (getInt i ivHigh)

    else
        Word (getInt i ivHigh) 0


load32 : Array Int -> Int -> Int
load32 bytes pos =
    B.or (B.or (getInt pos bytes) (B.shiftLeftBy 8 (getInt (pos + 1) bytes)))
        (B.or (B.shiftLeftBy 16 (getInt (pos + 2) bytes)) (B.shiftLeftBy 24 (getInt (pos + 3) bytes)))


load : Bool -> Array Int -> Int -> Word
load wide bytes i =
    if wide then
        Word (load32 bytes (8 * i)) (load32 bytes (8 * i + 4))

    else
        Word (load32 bytes (4 * i)) 0


type alias Rotations =
    { wide : Bool, r1 : Int, r2 : Int, r3 : Int, r4 : Int }


mix : Rotations -> Int -> Int -> Int -> Int -> Word -> Word -> Array Word -> Array Word
mix rot a b c d x y v =
    let
        a1 = add (add (getWord a v) (getWord b v)) x
        d1 = rotate rot.wide rot.r1 (xor (getWord d v) a1)
        c1 = add (getWord c v) d1
        b1 = rotate rot.wide rot.r2 (xor (getWord b v) c1)
        a2 = add (add a1 b1) y
        d2 = rotate rot.wide rot.r3 (xor d1 a2)
        c2 = add c1 d2
        b2 = rotate rot.wide rot.r4 (xor b1 c2)
    in
    v |> Array.set a a2 |> Array.set b b2 |> Array.set c c2 |> Array.set d d2


compress : Bool -> Array Word -> Array Int -> Word -> Bool -> Array Word
compress wide h block count last =
    let
        initialLane i =
            if i < 8 then
                getWord i h

            else
                let
                    w = initial wide (i - 8)
                in
                if i == 12 then
                    xor w (Word count.lo (if wide then count.hi else 0))

                else if i == 13 && not wide then
                    xor w (Word count.hi 0)

                else if i == 14 && last then
                    xor w (Word -1 -1)

                else
                    w

        rot =
            if wide then Rotations True 32 24 16 63 else Rotations False 16 12 8 7

        msg =
            Array.initialize 16 (load wide block)

        mixRound r v =
            let
                schedule = Maybe.withDefault Array.empty (Array.get (modBy 10 r) sigma)
                m j = getWord (getInt j schedule) msg
                column i = mix rot i (i+4) (i+8) (i+12) (m (2*i)) (m (2*i+1))
                diagonal i = mix rot i (4+modBy 4 (i+1)) (8+modBy 4 (i+2)) (12+modBy 4 (i+3)) (m (8+2*i)) (m (9+2*i))
            in
            v |> column 0 |> column 1 |> column 2 |> column 3 |> diagonal 0 |> diagonal 1 |> diagonal 2 |> diagonal 3

        final =
            List.foldl mixRound (Array.initialize 16 initialLane) (List.range 0 (if wide then 11 else 9))
    in
    Array.indexedMap (\i w -> xor w (xor (getWord i final) (getWord (i+8) final))) h


type alias Pending =
    { h : Array Word, rev : List Int, len : Int, compressed : Word }


digest : Bool -> List (List Int) -> Int -> List Int -> List Int -> List Int -> List Int
digest wide chunks dkLen key salt personalization =
    let
        stride = if wide then 8 else 4
        blockLen = 16 * stride
        saltBytes = Array.fromList salt
        personalBytes = Array.fromList personalization
        init i =
            let
                w = initial wide i
            in
            if i == 0 then
                xor w (Word (B.or 0x01010000 (B.or (B.shiftLeftBy 8 (List.length key)) dkLen)) 0)

            else if i >= 4 && i < 6 && not (List.isEmpty salt) then
                xor w (load wide saltBytes (i - 4))

            else if i >= 6 && not (List.isEmpty personalization) then
                xor w (load wide personalBytes (i - 6))

            else
                w

        initialBlock =
            if List.isEmpty key then
                []

            else
                List.repeat (blockLen - List.length key) 0 ++ List.reverse key

        feed byte state =
            if state.len == blockLen then
                let
                    count = add state.compressed (Word blockLen 0)
                    nextH = compress wide state.h (Array.fromList (List.reverse state.rev)) count False
                in
                Pending nextH [ byte ] 1 count

            else
                { state | rev = byte :: state.rev, len = state.len + 1 }

        start = Pending (Array.initialize 8 init) initialBlock (List.length initialBlock) zero
        st = List.foldl (\chunk acc -> List.foldl feed acc chunk) start chunks
        lastBlock = Array.fromList (List.reverse st.rev ++ List.repeat (blockLen - st.len) 0)
        h = compress wide st.h lastBlock (add st.compressed (Word st.len 0)) True

        read i =
            let
                w = getWord (i // stride) h
                limb = if modBy stride i >= 4 then w.hi else w.lo
            in
            B.and 255 (B.shiftRightZfBy (8 * modBy 4 i) limb)
    in
    List.map read (List.range 0 (dkLen - 1))
