module Sha3 exposing (Keccak, digest, keccak, update)

{-| noble's sha3.ts: Keccak-f[1600] and the Keccak sponge behind SHA3-256 and Keccak-256.

The sponge state is noble's state32 as a list of 25 lanes in noble's order (x + 5y), each lane
its low (first) and high (second) 32-bit word. keccakP works on the lanes as a record named the
Keccak team's way (row b, g, k, m, s for y = 0..4; column a, e, i, o, u for x = 0..4), so each
step is a function from the whole state to the next one. noble derives the pi, rho and iota
tables at load time; here rho and pi are written out lane by lane in noble's cycle order and the
iota table is literal.

noble XORs each input byte into the state as it arrives. Here a partial block waits in a buffer,
as in HashMD, and is XORed in when it completes; the permutations happen at the same points.

-}

import Array exposing (Array)
import Bitwise exposing (and, complement, or, shiftLeftBy, shiftRightZfBy)
import Words


{-| A lane: its first (low) and second (high) 32-bit word.
-}
type alias Lane =
    { lo : Int, hi : Int }


{-| The 25 lanes during a permutation.
-}
type alias State =
    { aba : Lane, abe : Lane, abi : Lane, abo : Lane, abu : Lane
    , aga : Lane, age : Lane, agi : Lane, ago : Lane, agu : Lane
    , aka : Lane, ake : Lane, aki : Lane, ako : Lane, aku : Lane
    , ama : Lane, ame : Lane, ami : Lane, amo : Lane, amu : Lane
    , asa : Lane, ase : Lane, asi : Lane, aso : Lane, asu : Lane
    }


{-| Iota: the low and high words of each round constant (noble's SHA3\_IOTA\_H and SHA3\_IOTA\_L).
-}
iota : Array Lane
iota =
    Array.fromList
        [ Lane 0x01 0, Lane 0x8082 0, Lane 0x808A 0x80000000, Lane 0x80008000 0x80000000
        , Lane 0x808B 0, Lane 0x80000001 0, Lane 0x80008081 0x80000000, Lane 0x8009 0x80000000
        , Lane 0x8A 0, Lane 0x88 0, Lane 0x80008009 0, Lane 0x8000000A 0
        , Lane 0x8000808B 0, Lane 0x8B 0x80000000, Lane 0x8089 0x80000000, Lane 0x8003 0x80000000
        , Lane 0x8002 0x80000000, Lane 0x80 0x80000000, Lane 0x800A 0, Lane 0x8000000A 0x80000000
        , Lane 0x80008081 0x80000000, Lane 0x8080 0x80000000, Lane 0x80000001 0, Lane 0x80008008 0x80000000
        ]


{-| noble's rotlH and rotlL: a left rotation by `s` in 1..63 except 32.
-}
rotl : Int -> Lane -> Lane
rotl s { lo, hi } =
    if s > 32 then
        { lo = or (shiftLeftBy (s - 32) hi) (shiftRightZfBy (64 - s) lo)
        , hi = or (shiftLeftBy (s - 32) lo) (shiftRightZfBy (64 - s) hi)
        }

    else
        { lo = or (shiftLeftBy s lo) (shiftRightZfBy (32 - s) hi)
        , hi = or (shiftLeftBy s hi) (shiftRightZfBy (32 - s) lo)
        }


xor : Lane -> Lane -> Lane
xor a b =
    { lo = Bitwise.xor a.lo b.lo, hi = Bitwise.xor a.hi b.hi }


xor5 : Lane -> Lane -> Lane -> Lane -> Lane -> Lane
xor5 a b c d e =
    xor (xor (xor (xor a b) c) d) e


{-| Theta: each lane takes the parity of the column before it and the rotated parity of the
column after it.
-}
theta : State -> State
theta s =
    let
        c0 =
            xor5 s.aba s.aga s.aka s.ama s.asa

        c1 =
            xor5 s.abe s.age s.ake s.ame s.ase

        c2 =
            xor5 s.abi s.agi s.aki s.ami s.asi

        c3 =
            xor5 s.abo s.ago s.ako s.amo s.aso

        c4 =
            xor5 s.abu s.agu s.aku s.amu s.asu

        d0 =
            xor (rotl 1 c1) c4

        d1 =
            xor (rotl 1 c2) c0

        d2 =
            xor (rotl 1 c3) c1

        d3 =
            xor (rotl 1 c4) c2

        d4 =
            xor (rotl 1 c0) c3
    in
    { aba = xor s.aba d0, abe = xor s.abe d1, abi = xor s.abi d2, abo = xor s.abo d3, abu = xor s.abu d4
    , aga = xor s.aga d0, age = xor s.age d1, agi = xor s.agi d2, ago = xor s.ago d3, agu = xor s.agu d4
    , aka = xor s.aka d0, ake = xor s.ake d1, aki = xor s.aki d2, ako = xor s.ako d3, aku = xor s.aku d4
    , ama = xor s.ama d0, ame = xor s.ame d1, ami = xor s.ami d2, amo = xor s.amo d3, amu = xor s.amu d4
    , asa = xor s.asa d0, ase = xor s.ase d1, asi = xor s.asi d2, aso = xor s.aso d3, asu = xor s.asu d4
    }


{-| Rho and pi: noble's single cycle through the 24 lanes after the first, starting from abe;
each lane moves to the next one's place, rotated by that step's SHA3\_ROTL.
-}
rhoPi : State -> State
rhoPi s =
    { aba = s.aba
    , aka = rotl 1 s.abe
    , agi = rotl 3 s.aka
    , ake = rotl 6 s.agi
    , ami = rotl 10 s.ake
    , amo = rotl 15 s.ami
    , abo = rotl 21 s.amo
    , aga = rotl 28 s.abo
    , ame = rotl 36 s.aga
    , ago = rotl 45 s.ame
    , ase = rotl 55 s.ago
    , asu = rotl 2 s.ase
    , abu = rotl 14 s.asu
    , ama = rotl 27 s.abu
    , aso = rotl 41 s.ama
    , amu = rotl 56 s.aso
    , ako = rotl 8 s.amu
    , aki = rotl 25 s.ako
    , abi = rotl 43 s.aki
    , asa = rotl 62 s.abi
    , aku = rotl 18 s.asa
    , asi = rotl 39 s.aku
    , agu = rotl 61 s.asi
    , age = rotl 20 s.agu
    , abe = rotl 44 s.age
    }


{-| Chi on one lane: `a ^ (~b & c)` for the lanes after it in the row.
-}
chiLane : Lane -> Lane -> Lane -> Lane
chiLane a b c =
    { lo = Bitwise.xor a.lo (and (complement b.lo) c.lo), hi = Bitwise.xor a.hi (and (complement b.hi) c.hi) }


chi : State -> State
chi s =
    { aba = chiLane s.aba s.abe s.abi, abe = chiLane s.abe s.abi s.abo, abi = chiLane s.abi s.abo s.abu, abo = chiLane s.abo s.abu s.aba, abu = chiLane s.abu s.aba s.abe
    , aga = chiLane s.aga s.age s.agi, age = chiLane s.age s.agi s.ago, agi = chiLane s.agi s.ago s.agu, ago = chiLane s.ago s.agu s.aga, agu = chiLane s.agu s.aga s.age
    , aka = chiLane s.aka s.ake s.aki, ake = chiLane s.ake s.aki s.ako, aki = chiLane s.aki s.ako s.aku, ako = chiLane s.ako s.aku s.aka, aku = chiLane s.aku s.aka s.ake
    , ama = chiLane s.ama s.ame s.ami, ame = chiLane s.ame s.ami s.amo, ami = chiLane s.ami s.amo s.amu, amo = chiLane s.amo s.amu s.ama, amu = chiLane s.amu s.ama s.ame
    , asa = chiLane s.asa s.ase s.asi, ase = chiLane s.ase s.asi s.aso, asi = chiLane s.asi s.aso s.asu, aso = chiLane s.aso s.asu s.asa, asu = chiLane s.asu s.asa s.ase
    }


{-| Rounds `round` to 23 of Keccak-f[1600].
-}
rounds : Int -> State -> State
rounds round s =
    case Array.get round iota of
        Just constant ->
            let
                t =
                    chi (rhoPi (theta s))
            in
            rounds (round + 1) { t | aba = xor t.aba constant }

        Nothing ->
            s


{-| noble's keccakP on the lane list.
-}
keccakP : List Lane -> List Lane
keccakP lanes =
    let
        array =
            Array.fromList lanes

        lane i =
            Array.get i array |> Maybe.withDefault (Lane 0 0)

        s =
            rounds 0
                { aba = lane 0, abe = lane 1, abi = lane 2, abo = lane 3, abu = lane 4
                , aga = lane 5, age = lane 6, agi = lane 7, ago = lane 8, agu = lane 9
                , aka = lane 10, ake = lane 11, aki = lane 12, ako = lane 13, aku = lane 14
                , ama = lane 15, ame = lane 16, ami = lane 17, amo = lane 18, amu = lane 19
                , asa = lane 20, ase = lane 21, asi = lane 22, aso = lane 23, asu = lane 24
                }
    in
    [ s.aba, s.abe, s.abi, s.abo, s.abu, s.aga, s.age, s.agi, s.ago, s.agu
    , s.aka, s.ake, s.aki, s.ako, s.aku, s.ama, s.ame, s.ami, s.amo, s.amu
    , s.asa, s.ase, s.asi, s.aso, s.asu
    ]


{-| noble's Keccak sponge with a byte-length rate `blockLen` and domain `suffix`.
-}
type alias Keccak =
    { blockLen : Int
    , suffix : Int
    , outputLen : Int
    , lanes : List Lane
    , buffer : List Int
    , pos : Int
    }


keccak : Int -> Int -> Int -> Keccak
keccak blockLen suffix outputLen =
    { blockLen = blockLen
    , suffix = suffix
    , outputLen = outputLen
    , lanes = List.repeat 25 (Lane 0 0)
    , buffer = []
    , pos = 0
    }


{-| XORs the rate's worth of `block` into the first lanes and permutes.
-}
absorbBlock : List Int -> List Lane -> List Lane
absorbBlock block lanes =
    keccakP (xorLanes (toLanes (Words.wordsLE block)) lanes)


xorLanes : List Lane -> List Lane -> List Lane
xorLanes block lanes =
    case ( block, lanes ) of
        ( b :: blockRest, lane :: laneRest ) ->
            xor b lane :: xorLanes blockRest laneRest

        _ ->
            lanes


toLanes : List Int -> List Lane
toLanes words =
    case words of
        lo :: hi :: rest ->
            Lane lo hi :: toLanes rest

        _ ->
            []


update : List Int -> Keccak -> Keccak
update data k =
    absorb (k.buffer ++ data) (k.pos + List.length data) k


absorb : List Int -> Int -> Keccak -> Keccak
absorb bytes count k =
    if count >= k.blockLen then
        absorb (List.drop k.blockLen bytes) (count - k.blockLen) { k | lanes = absorbBlock (List.take k.blockLen bytes) k.lanes }

    else
        { k | buffer = bytes, pos = count }


{-| noble's finish and writeInto: the suffix holds the domain bits and the first padding bit,
0x80 closes pad10\*1, and the digest is read from the first lanes. Both digests here are shorter
than the rate, so one block is squeezed.
-}
digest : Keccak -> List Int
digest k =
    let
        zeros n =
            List.repeat n 0

        lanes =
            if k.pos == k.blockLen - 1 then
                if and k.suffix 0x80 /= 0 then
                    absorbBlock (zeros (k.blockLen - 1) ++ [ 0x80 ]) (absorbBlock (k.buffer ++ [ k.suffix ]) k.lanes)

                else
                    absorbBlock (k.buffer ++ [ Bitwise.xor k.suffix 0x80 ]) k.lanes

            else
                absorbBlock (k.buffer ++ k.suffix :: zeros (k.blockLen - k.pos - 2) ++ [ 0x80 ]) k.lanes
    in
    lanes
        |> List.concatMap (\lane -> Words.bytesLE lane.lo ++ Words.bytesLE lane.hi)
        |> List.take k.outputLen
