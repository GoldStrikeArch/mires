module Huffman exposing
    ( clen
    , fdm
    , fdrm
    , flm
    , flrm
    , hMap
    , hTree
    , lc
    )

{-| Canonical Huffman map construction (`hMap`), UZIP.js length-limited Huffman tree construction
(`hTree`), code-length RLE encoding (`lc`), and bit-cost calculation (`clen`), matching
`fflate@0.8.3` bit-for-bit.
-}

import Bitwise
import ByteBuf exposing (IntTree(..), intTreeGet, intTreeInc, intTreeSet, shiftForSize)
import Tables exposing (fdt, flt, rev15)


{-| Build forward (`isDecode = False`) or reverse (`isDecode = True`) canonical Huffman map.
-}
hMap : IntTree -> Int -> Int -> Int -> Bool -> IntTree
hMap cd cdShift s mb isDecode =
    if mb <= 0 then
        Empty

    else
        let
            -- Step 1: count non-zero code lengths in `l` (indices 0..mb-1, shift 3)
            countLens i l =
                if (i - s) >= 0 then
                    l

                else
                    let
                        c =
                            intTreeGet cdShift i cd
                    in
                    if c /= 0 then
                        countLens (i + 1) (intTreeInc 3 (c - 1) l)

                    else
                        countLens (i + 1) l

            lTree =
                countLens 0 Empty

            -- Step 2: minimum code for each bit length in `le` (u16 mod 65536)
            buildLe i prevLe le =
                if (i - mb) >= 0 then
                    le

                else
                    let
                        nextVal =
                            Bitwise.and 65535
                                (Bitwise.shiftLeftBy 1 (prevLe + intTreeGet 3 (i - 1) lTree))
                    in
                    buildLe (i + 1) nextVal (intTreeSet 3 i nextVal le)

            leTree =
                buildLe 1 0 Empty
        in
        if isDecode then
            let
                coShift =
                    shiftForSize (Bitwise.shiftLeftBy mb 1)

                rvb =
                    15 - mb

                fillSpan v m sv co =
                    if (v - m) > 0 then
                        co

                    else
                        let
                            idx =
                                Bitwise.shiftRightZfBy rvb (rev15 v)
                        in
                        fillSpan (v + 1) m sv (intTreeSet coShift idx sv co)

                fillDecode i le co =
                    if (i - s) >= 0 then
                        co

                    else
                        let
                            c =
                                intTreeGet cdShift i cd
                        in
                        if c /= 0 then
                            let
                                sv =
                                    Bitwise.or (Bitwise.shiftLeftBy 4 i) c

                                r1 =
                                    mb - c

                                code =
                                    intTreeGet 3 (c - 1) le

                                nextLe =
                                    intTreeSet 3 (c - 1) (Bitwise.and 65535 (code + 1)) le

                                vStart =
                                    Bitwise.shiftLeftBy r1 code

                                mEnd =
                                    Bitwise.or vStart (Bitwise.shiftLeftBy r1 1 - 1)
                            in
                            fillDecode (i + 1) nextLe (fillSpan vStart mEnd sv co)

                        else
                            fillDecode (i + 1) le co
            in
            fillDecode 0 leTree Empty

        else
            let
                coShift =
                    shiftForSize s

                fillEncode i le co =
                    if (i - s) >= 0 then
                        co

                    else
                        let
                            c =
                                intTreeGet cdShift i cd
                        in
                        if c /= 0 then
                            let
                                code =
                                    intTreeGet 3 (c - 1) le

                                nextLe =
                                    intTreeSet 3 (c - 1) (Bitwise.and 65535 (code + 1)) le

                                mapped =
                                    Bitwise.shiftRightZfBy (15 - c) (rev15 code)
                            in
                            fillEncode (i + 1) nextLe (intTreeSet coShift i mapped co)

                        else
                            fillEncode (i + 1) le co
            in
            fillEncode 0 leTree Empty


flm : IntTree
flm =
    hMap flt 6 288 9 False


flrm : IntTree
flrm =
    hMap flt 6 288 9 True


fdm : IntTree
fdm =
    hMap fdt 3 32 5 False


fdrm : IntTree
fdrm =
    hMap fdt 3 32 5 True


type HNode
    = HLeaf Int Int
    | HBranch Int HNode HNode


nodeFreq : HNode -> Int
nodeFreq node =
    case node of
        HLeaf _ f ->
            f

        HBranch f _ _ ->
            f


{-| Build length-limited Huffman code lengths from frequency table `d` (`0..dLen-1`).
Returns `{ tree : IntTree (shift 6), len : Int, maxBits : Int }`.
-}
hTree : IntTree -> Int -> Int -> Int -> { tree : IntTree, len : Int, maxBits : Int }
hTree d dShift dLen mb =
    let
        collect i count curMax acc =
            if (i - dLen) >= 0 then
                ( count, curMax, List.reverse acc )

            else
                let
                    f =
                        intTreeGet dShift i d
                in
                if f /= 0 then
                    collect (i + 1) (count + 1) i ({ s = i, f = f } :: acc)

                else
                    collect (i + 1) count curMax acc

        ( s, maxSym, t2 ) =
            collect 0 0 0 []
    in
    if s == 0 then
        { tree = Empty, len = 0, maxBits = 0 }

    else if s == 1 then
        { tree = intTreeSet 6 maxSym 1 Empty, len = maxSym + 1, maxBits = 1 }

    else
        let
            sortedLeaves =
                t2
                    |> List.sortWith
                        (\a b ->
                            if (a.f - b.f) /= 0 then
                                compare a.f b.f

                            else
                                compare a.s b.s
                        )
                    |> List.map (\e -> HLeaf e.s e.f)

            root =
                case sortedLeaves of
                    first :: second :: restLeaves ->
                        let
                            firstBranch =
                                HBranch (nodeFreq first + nodeFreq second) first second
                        in
                        mergeQueue (s - 2) [ firstBranch ] [] restLeaves firstBranch

                    _ ->
                        HLeaf 0 0

            ( tr0, mbt0 ) =
                assignDepths root 0 Empty 0
        in
        if (mbt0 - mb) <= 0 then
            { tree = tr0, len = maxSym + 1, maxBits = mbt0 }

        else
            let
                t2Sorted =
                    List.sortWith
                        (\a b ->
                            let
                                da =
                                    intTreeGet 6 a.s tr0

                                db =
                                    intTreeGet 6 b.s tr0
                            in
                            if (db - da) /= 0 then
                                compare db da

                            else if (a.f - b.f) /= 0 then
                                compare a.f b.f

                            else
                                compare a.s b.s
                        )
                        t2

                buildSymsTree idx list acc =
                    case list of
                        [] ->
                            acc

                        x :: xs ->
                            buildSymsTree (idx + 1) xs (intTreeSet 6 idx x.s acc)

                symsTree =
                    buildSymsTree 0 t2Sorted Empty

                lft =
                    mbt0 - mb

                cst =
                    Bitwise.shiftLeftBy lft 1

                clampOver i dt tr =
                    if (i - s) >= 0 then
                        ( i, dt, tr )

                    else
                        let
                            sym =
                                intTreeGet 6 i symsTree

                            depth =
                                intTreeGet 6 sym tr
                        in
                        if (depth - mb) > 0 then
                            clampOver
                                (i + 1)
                                (dt + cst - Bitwise.shiftLeftBy (mbt0 - depth) 1)
                                (intTreeSet 6 sym mb tr)

                        else
                            ( i, dt, tr )

                ( i1, dt1, tr1 ) =
                    clampOver 0 0 tr0

                dt2 =
                    Bitwise.shiftRightBy lft dt1

                payDebt i dt tr =
                    if dt <= 0 then
                        ( i, dt, tr )

                    else
                        let
                            sym =
                                intTreeGet 6 i symsTree

                            depth =
                                intTreeGet 6 sym tr
                        in
                        if (depth - mb) < 0 then
                            payDebt
                                i
                                (dt - Bitwise.shiftLeftBy (mb - depth - 1) 1)
                                (intTreeSet 6 sym (depth + 1) tr)

                        else
                            payDebt (i + 1) dt tr

                ( i2, dt3, tr2 ) =
                    payDebt i1 dt2 tr1

                rollback i dt tr =
                    if i < 0 || dt == 0 then
                        tr

                    else
                        let
                            sym =
                                intTreeGet 6 i symsTree

                            depth =
                                intTreeGet 6 sym tr
                        in
                        if (depth - mb) == 0 then
                            rollback (i - 1) (dt + 1) (intTreeSet 6 sym (depth - 1) tr)

                        else
                            rollback (i - 1) dt tr
            in
            { tree = rollback i2 dt3 tr2, len = maxSym + 1, maxBits = mb }


type PopRes
    = PopRes HNode (List HNode) (List HNode) (List HNode)


mergeQueue : Int -> List HNode -> List HNode -> List HNode -> HNode -> HNode
mergeQueue remaining intFront intBackRev leaves lastBranch =
    if remaining <= 0 then
        lastBranch

    else
        let
            (PopRes l intF1 intB1 leaves1) =
                popMin intFront intBackRev leaves

            (PopRes r intF2 intB2 leaves2) =
                popMin intF1 intB1 leaves1

            combined =
                HBranch (nodeFreq l + nodeFreq r) l r
        in
        mergeQueue (remaining - 1) intF2 (combined :: intB2) leaves2 combined


popMin : List HNode -> List HNode -> List HNode -> PopRes
popMin intFront intBackRev leaves =
    let
        ( front, backRev ) =
            case intFront of
                [] ->
                    ( List.reverse intBackRev, [] )

                _ ->
                    ( intFront, intBackRev )
    in
    case ( front, leaves ) of
        ( ih :: it, lh :: lt ) ->
            -- Strict `<`: ties pick the leaf, matching `t[i0].f < t[i2].f` in `browser.js`.
            if (nodeFreq ih - nodeFreq lh) < 0 then
                PopRes ih it backRev leaves

            else
                PopRes lh front backRev lt

        ( ih :: it, [] ) ->
            PopRes ih it backRev []

        ( [], lh :: lt ) ->
            PopRes lh [] [] lt

        ( [], [] ) ->
            PopRes (HLeaf 0 0) [] [] []


assignDepths : HNode -> Int -> IntTree -> Int -> ( IntTree, Int )
assignDepths node depth tr maxD =
    case node of
        HLeaf sym _ ->
            let
                nextMax =
                    if (depth - maxD) > 0 then
                        depth

                    else
                        maxD
            in
            ( intTreeSet 6 sym depth tr, nextMax )

        HBranch _ left right ->
            let
                ( tr1, max1 ) =
                    assignDepths left (depth + 1) tr maxD
            in
            assignDepths right (depth + 1) tr1 max1


{-| RLE-encode code lengths `c` (`0..cLen-1`, shift 6) matching `fflate`'s `lc(c)`.
Returns `{ codes : List Int, n : Int }`.
-}
lc : IntTree -> Int -> { codes : List Int, n : Int }
lc c cLen =
    let
        trim s =
            if s <= 0 then
                1

            else if intTreeGet 6 (s - 1) c == 0 then
                trim (s - 1)

            else
                s

        sTrimmed =
            trim cLen

        emitRepeat val count acc =
            if count <= 0 then
                acc

            else
                emitRepeat val (count - 1) (val :: acc)

        emitZeroRuns cls acc =
            if cls > 138 then
                emitZeroRuns (cls - 138) (32754 :: acc)

            else if cls > 2 then
                let
                    entry =
                        if cls > 10 then
                            Bitwise.or (Bitwise.shiftLeftBy 5 (cls - 11)) 28690

                        else
                            Bitwise.or (Bitwise.shiftLeftBy 5 (cls - 3)) 12305
                in
                entry :: acc

            else
                emitRepeat 0 cls acc

        emitNonZeroRuns cln cls acc =
            if cls > 6 then
                emitNonZeroRuns cln (cls - 6) (8304 :: acc)

            else if cls > 2 then
                Bitwise.or (Bitwise.shiftLeftBy 5 (cls - 3)) 8208 :: acc

            else
                emitRepeat cln cls acc

        flushRun cln cls acc =
            if cln == 0 && cls > 2 then
                emitZeroRuns cls acc

            else if cls > 3 then
                emitNonZeroRuns cln (cls - 1) (cln :: acc)

            else
                emitRepeat cln cls acc

        scan i cln cls acc =
            if (i - sTrimmed) > 0 then
                List.reverse acc

            else
                let
                    ci =
                        if (i - sTrimmed) < 0 then
                            intTreeGet 6 i c

                        else
                            -1
                in
                if (ci - cln) == 0 && (i - sTrimmed) /= 0 then
                    scan (i + 1) cln (cls + 1) acc

                else
                    scan (i + 1) ci 1 (flushRun cln cls acc)
    in
    { codes = scan 1 (intTreeGet 6 0 c) 1 [], n = sTrimmed }


{-| Sum `cf[i] * cl[i]` for `i = 0 .. len - 1`.
-}
clen : IntTree -> Int -> IntTree -> Int -> Int -> Int
clen cf cfShift cl clShift len =
    let
        go i acc =
            if (i - len) >= 0 then
                acc

            else
                go (i + 1) (acc + intTreeGet cfShift i cf * intTreeGet clShift i cl)
    in
    go 0 0
