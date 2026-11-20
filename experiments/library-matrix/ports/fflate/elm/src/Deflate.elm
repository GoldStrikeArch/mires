module Deflate exposing
    ( CompressOptions
    , DeflateState
    , deflateSync
    , dopt
    , doptOneShot
    , emptyCompressOptions
    , emptyDeflateState
    , shft
    , syncFlushBytes
    )

{-| Raw DEFLATE compressor (`wblk`, `wfblk`, `dflt`, `dopt`, `deflateSync`) matching
`fflate@0.8.3` bit-for-bit, using a sequential bit writer and persistent 8-way trees.
-}

import Bitwise
import ByteBuf exposing (Bytes, IntTree(..), intTreeGet, intTreeInc, intTreeSet, intTreeSwap, shiftForSize)
import Huffman exposing (clen, fdm, flm, hMap, hTree, lc)
import Tables exposing (clim, defaultPlvl, deoChain, deoNice, fdeb, fdt, fleb, flt, revfd, revfl)


type alias CompressOptions =
    { level : Maybe Int
    , mem : Maybe Int
    , dictionary : Maybe Bytes
    , mtime : Maybe Float
    , filename : Maybe String
    }


emptyCompressOptions : CompressOptions
emptyCompressOptions =
    { level = Nothing
    , mem = Nothing
    , dictionary = Nothing
    , mtime = Nothing
    , filename = Nothing
    }


type alias DeflateState =
    { l : Int
    , i : Int
    , w : Int
    , z : Int
    , r : Int
    , h : Maybe IntTree
    , hShift : Int
    , p : Maybe IntTree
    }


emptyDeflateState : DeflateState
emptyDeflateState =
    { l = 0
    , i = 0
    , w = 0
    , z = 0
    , r = 0
    , h = Nothing
    , hShift = 0
    , p = Nothing
    }


shft : Int -> Int
shft p =
    Bitwise.shiftRightBy 3 (p + 7)



-- SEQUENTIAL BIT WRITER


type BitWriter
    = BW Int Int (List Int)


bwWriteBits : Int -> Int -> BitWriter -> BitWriter
bwWriteBits val count (BW acc bits outRev) =
    let
        v =
            Bitwise.or acc (Bitwise.shiftLeftBy bits val)

        b =
            bits + count
    in
    if b < 8 then
        BW v b outRev

    else if b < 16 then
        BW
            (Bitwise.shiftRightZfBy 8 v)
            (b - 8)
            (Bitwise.and 255 v :: outRev)

    else
        BW
            (Bitwise.shiftRightZfBy 16 v)
            (b - 16)
            (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 v) :: Bitwise.and 255 v :: outRev)


{-| Write a stored block (`BTYPE = 00`, byte-align, `LEN`, `NLEN`, raw slice `dat[start..end)`).
Caller has already emitted the 1-bit `BFINAL` flag.
-}
bwWriteStored : Bytes -> Int -> Int -> BitWriter -> BitWriter
bwWriteStored dat start end bw =
    let
        (BW acc1 bits1 rev1) =
            bwWriteBits 0 2 bw

        rev2 =
            if bits1 > 0 then
                Bitwise.and 255 acc1 :: rev1

            else
                rev1

        bl =
            if (end - start) > 0 then
                end - start

            else
                0

        lo =
            Bitwise.and 255 bl

        hi =
            Bitwise.and 255 (Bitwise.shiftRightZfBy 8 bl)

        rev3 =
            Bitwise.xor 255 hi :: Bitwise.xor 255 lo :: hi :: lo :: rev2

        rev4 =
            ByteBuf.foldlRange start end (::) rev3 dat
    in
    BW 0 0 rev4


{-| Exact match for `Deflate.prototype.flush(true)` sync trailer (`wfblk(c, s.r, et)` on 6-byte `c`).
-}
syncFlushBytes : Int -> List Int
syncFlushBytes sr =
    let
        b0 =
            Bitwise.and 255 (Bitwise.shiftRightZfBy 3 sr)

        o =
            shft (sr + 2)

        epBytes =
            let
                raw =
                    Bitwise.shiftRightBy 3 ((o + 4) * 8)
            in
            if raw > 6 then
                6

            else if raw < 0 then
                0

            else
                raw

        byteAt idx =
            if (idx - o) == 0 || (idx - (o + 1)) == 0 then
                0

            else if (idx - (o + 2)) == 0 || (idx - (o + 3)) == 0 then
                255

            else if idx == 0 then
                b0

            else
                0

        build idx acc =
            if (idx - epBytes) >= 0 then
                List.reverse acc

            else
                build (idx + 1) (byteAt idx :: acc)
    in
    build 0 []



-- BLOCK SELECTION AND EMISSION (`wblk`)


wblk : Bytes -> BitWriter -> Int -> List Int -> IntTree -> IntTree -> Int -> Int -> Int -> BitWriter
wblk dat bw0 final symsRev lf0 df eb bs bl =
    let
        bw1 =
            bwWriteBits final 1 bw0

        lf =
            intTreeInc 6 256 lf0

        dltRes =
            hTree lf 6 288 15

        ddtRes =
            hTree df 3 32 15

        lcltRes =
            lc dltRes.tree dltRes.len

        lcdtRes =
            lc ddtRes.tree ddtRes.len

        addLcFreqs list tree =
            case list of
                [] ->
                    tree

                v :: rest ->
                    addLcFreqs rest (intTreeInc 3 (Bitwise.and 31 v) tree)

        lcfreq =
            addLcFreqs lcdtRes.codes (addLcFreqs lcltRes.codes Empty)

        lctRes =
            hTree lcfreq 3 19 7

        trimNlcc n =
            if n <= 4 then
                4

            else if intTreeGet 6 (clim (n - 1)) lctRes.tree == 0 then
                trimNlcc (n - 1)

            else
                n

        nlcc =
            trimNlcc 19

        flen =
            Bitwise.shiftLeftBy 3 (bl + 5)

        ftlen =
            clen lf 6 flt 6 288 + clen df 3 fdt 3 32 + eb

        dtlen =
            clen lf 6 dltRes.tree 6 dltRes.len
                + clen df 3 ddtRes.tree 6 ddtRes.len
                + eb
                + 14
                + 3
                * nlcc
                + clen lcfreq 3 lctRes.tree 6 lctRes.len
                + 2
                * intTreeGet 3 16 lcfreq
                + 3
                * intTreeGet 3 17 lcfreq
                + 7
                * intTreeGet 3 18 lcfreq
    in
    if bs >= 0 && (flen - ftlen) <= 0 && (flen - dtlen) <= 0 then
        bwWriteStored dat bs (bs + bl) bw1

    else if (dtlen - ftlen) < 0 then
        let
            bw2 =
                bw1
                    |> bwWriteBits 2 2
                    |> bwWriteBits (lcltRes.n - 257) 5
                    |> bwWriteBits (lcdtRes.n - 1) 5
                    |> bwWriteBits (nlcc - 4) 4

            writeClLens i bwAcc =
                if (i - nlcc) >= 0 then
                    bwAcc

                else
                    writeClLens (i + 1) (bwWriteBits (intTreeGet 6 (clim i) lctRes.tree) 3 bwAcc)

            llm =
                hMap lctRes.tree 6 lctRes.len lctRes.maxBits False

            llmShift =
                shiftForSize lctRes.len

            writeClCodes list bwAcc =
                case list of
                    [] ->
                        bwAcc

                    entry :: rest ->
                        let
                            lenSym =
                                Bitwise.and 31 entry

                            bwSym =
                                bwWriteBits (intTreeGet llmShift lenSym llm) (intTreeGet 6 lenSym lctRes.tree) bwAcc

                            bwNext =
                                if lenSym > 15 then
                                    bwWriteBits
                                        (Bitwise.and 127 (Bitwise.shiftRightZfBy 5 entry))
                                        (Bitwise.shiftRightZfBy 12 entry)
                                        bwSym

                                else
                                    bwSym
                        in
                        writeClCodes rest bwNext

            bw3 =
                bw2
                    |> writeClLens 0
                    |> writeClCodes lcltRes.codes
                    |> writeClCodes lcdtRes.codes

            lm =
                hMap dltRes.tree 6 dltRes.len dltRes.maxBits False

            lmShift =
                shiftForSize dltRes.len

            dm =
                hMap ddtRes.tree 6 ddtRes.len ddtRes.maxBits False

            dmShift =
                shiftForSize ddtRes.len
        in
        emitSymbols lm lmShift dltRes.tree 6 dm dmShift ddtRes.tree 6 (List.reverse symsRev) bw3

    else
        let
            bw2 =
                bwWriteBits 1 2 bw1
        in
        emitSymbols flm 6 flt 6 fdm 3 fdt 3 (List.reverse symsRev) bw2


emitSymbols : IntTree -> Int -> IntTree -> Int -> IntTree -> Int -> IntTree -> Int -> List Int -> BitWriter -> BitWriter
emitSymbols lm lmShift ll llShift dm dmShift dl dlShift syms bw =
    let
        go list bwAcc =
            case list of
                [] ->
                    bwWriteBits (intTreeGet lmShift 256 lm) (intTreeGet llShift 256 ll) bwAcc

                sym :: rest ->
                    if sym > 255 then
                        let
                            lenIdx =
                                Bitwise.and 31 (Bitwise.shiftRightZfBy 18 sym)

                            bw1 =
                                bwWriteBits
                                    (intTreeGet lmShift (lenIdx + 257) lm)
                                    (intTreeGet llShift (lenIdx + 257) ll)
                                    bwAcc

                            bw2 =
                                if lenIdx > 7 then
                                    bwWriteBits
                                        (Bitwise.and 31 (Bitwise.shiftRightZfBy 23 sym))
                                        (fleb lenIdx)
                                        bw1

                                else
                                    bw1

                            dstIdx =
                                Bitwise.and 31 sym

                            bw3 =
                                bwWriteBits
                                    (intTreeGet dmShift dstIdx dm)
                                    (intTreeGet dlShift dstIdx dl)
                                    bw2

                            bw4 =
                                if dstIdx > 3 then
                                    bwWriteBits
                                        (Bitwise.and 8191 (Bitwise.shiftRightZfBy 5 sym))
                                        (fdeb dstIdx)
                                        bw3

                                else
                                    bw3
                        in
                        go rest bw4

                    else
                        go rest (bwWriteBits (intTreeGet lmShift sym lm) (intTreeGet llShift sym ll) bwAcc)
    in
    go syms bw



-- LZ77 COMPRESSOR (`dflt`, `dopt`, `deflateSync`)


type alias DfltCfg =
    { dat : Bytes
    , s : Int
    , lst : Int
    , nice : Int
    , chain : Int
    , msk : Int
    , bs1 : Int
    , bs2 : Int
    , headShift : Int
    }


type BlkState
    = BlkState BitWriter Int Int Int Int


type MatchStep
    = MatchStep Int Int Int Bool


type FlushState
    = FlushState BitWriter (List Int) IntTree IntTree Int Int Int Int


type alias SearchCfg =
    { dat : Bytes
    , i : Int
    , maxn : Int
    , maxd : Int
    , ml : Int
    }


hsh : DfltCfg -> Int -> Int
hsh cfg idx =
    let
        raw =
            ByteBuf.getU24Le idx cfg.dat
    in
    Bitwise.and cfg.msk
        (Bitwise.xor
            (Bitwise.xor
                (Bitwise.and 255 raw)
                (Bitwise.shiftLeftBy cfg.bs1 (Bitwise.and 255 (Bitwise.shiftRightZfBy 8 raw)))
            )
            (Bitwise.shiftLeftBy cfg.bs2 (Bitwise.shiftRightZfBy 16 raw))
        )


matchLen : Bytes -> Int -> Int -> Int -> Int -> Int
matchLen dat p1 p2 ml nl =
    if (nl - ml) >= 0 then
        nl

    else if (ByteBuf.getByte (p1 + nl) dat - ByteBuf.getByte (p2 + nl) dat) == 0 then
        matchLen dat p1 p2 ml (nl + 1)

    else
        nl


findRarest : IntTree -> Int -> Int -> Int -> Int -> Int -> Int
findRarest prev base mmd j md bestPimod =
    if (j - mmd) >= 0 then
        bestPimod

    else
        let
            ti =
                Bitwise.and 32767 (base + j)

            pti =
                intTreeGet 12 ti prev

            cd =
                Bitwise.and 32767 (ti - pti)
        in
        if (cd - md) > 0 then
            findRarest prev base mmd (j + 1) cd ti

        else
            findRarest prev base mmd (j + 1) md bestPimod


searchChain : SearchCfg -> IntTree -> Int -> Int -> Int -> Int -> Int -> Int -> ( Int, Int )
searchChain sc prev l d ch dif imod pimod =
    if (dif - sc.maxd) > 0 || ch <= 1 || (imod - pimod) == 0 then
        ( l, d )

    else
        let
            (MatchStep l1 d1 pimod1 done) =
                if (ByteBuf.getByte (sc.i + l) sc.dat - ByteBuf.getByte (sc.i + l - dif) sc.dat) == 0 then
                    let
                        nl =
                            matchLen sc.dat sc.i (sc.i - dif) sc.ml 0
                    in
                    if (nl - l) > 0 then
                        if (nl - sc.maxn) > 0 then
                            MatchStep nl dif pimod True

                        else
                            let
                                nl2 =
                                    nl - 2

                                mmd =
                                    if (dif - nl2) < 0 then
                                        dif

                                    else
                                        nl2

                                bestPimod =
                                    findRarest prev (sc.i - dif) mmd 0 0 pimod
                            in
                            MatchStep nl dif bestPimod False

                    else
                        MatchStep l d pimod False

                else
                    MatchStep l d pimod False
        in
        if done then
            ( l1, d1 )

        else
            let
                nextImod =
                    pimod1

                nextPimod =
                    intTreeGet 12 nextImod prev

                nextDif =
                    dif + Bitwise.and 32767 (nextImod - nextPimod)
            in
            searchChain sc prev l1 d1 (ch - 1) nextDif nextImod nextPimod


dfltLoop : DfltCfg -> Int -> Int -> IntTree -> IntTree -> List Int -> IntTree -> IntTree -> BlkState -> ( List Int, DeflateState )
dfltLoop cfg i wi head prev symsRev lf df blkState =
    if (i + 2 - cfg.s) >= 0 then
        let
            tailStart =
                if (i - wi) > 0 then
                    i

                else
                    wi

            addTail idx sRev lfAcc =
                if (idx - cfg.s) >= 0 then
                    ( idx, sRev, lfAcc )

                else
                    let
                        b =
                            ByteBuf.getByte idx cfg.dat
                    in
                    addTail (idx + 1) (b :: sRev) (intTreeInc 6 b lfAcc)

            ( finalI, finalSymsRev, finalLf ) =
                addTail tailStart symsRev lf

            (BlkState bw _ _ eb bs) =
                blkState

            (BW finalAcc finalBits finalRev) =
                wblk cfg.dat bw cfg.lst finalSymsRev finalLf df eb bs (finalI - bs)
        in
        if cfg.lst /= 0 then
            let
                outRev =
                    if finalBits > 0 then
                        Bitwise.and 255 finalAcc :: finalRev

                    else
                        finalRev
            in
            ( List.reverse outRev
            , { l = cfg.lst, i = finalI, w = wi, z = cfg.s, r = 0, h = Just head, hShift = cfg.headShift, p = Just prev }
            )

        else
            let
                nextR =
                    Bitwise.or finalBits (Bitwise.shiftLeftBy 3 (Bitwise.and 255 finalAcc))
            in
            ( List.reverse finalRev
            , { l = 0, i = finalI, w = wi, z = cfg.s, r = nextR, h = Just head, hShift = cfg.headShift, p = Just prev }
            )

    else
        let
            hv =
                hsh cfg i

            imod =
                Bitwise.and 32767 i

            ( pimod, head1 ) =
                intTreeSwap cfg.headShift hv imod head

            prev1 =
                intTreeSet 12 imod pimod prev
        in
        if (wi - i) > 0 then
            dfltLoop cfg (i + 1) wi head1 prev1 symsRev lf df blkState

        else
            let
                rem =
                    cfg.s - i

                (BlkState bw0 li0 lc0 eb0 bs0) =
                    blkState

                (FlushState bw1 syms1 lf1 df1 li1 lc1 eb1 bs1) =
                    if (lc0 > 7000 || li0 > 24576) && (rem > 423 || cfg.lst == 0) then
                        let
                            flushedBw =
                                wblk cfg.dat bw0 0 symsRev lf df eb0 bs0 (i - bs0)
                        in
                        FlushState flushedBw [] Empty Empty 0 0 0 i

                    else
                        FlushState bw0 symsRev lf df li0 lc0 eb0 bs0

                dif =
                    Bitwise.and 32767 (imod - pimod)

                ( matchL, matchD ) =
                    if rem > 2 && (hv - hsh cfg (i - dif)) == 0 then
                        let
                            maxn =
                                (if (cfg.nice - rem) < 0 then
                                    cfg.nice

                                 else
                                    rem
                                )
                                    - 1

                            maxd =
                                if i < 32767 then
                                    i

                                else
                                    32767

                            ml =
                                if rem < 258 then
                                    rem

                                else
                                    258
                        in
                        searchChain { dat = cfg.dat, i = i, maxn = maxn, maxd = maxd, ml = ml } prev1 2 0 cfg.chain dif imod pimod

                    else
                        ( 2, 0 )
            in
            if matchD /= 0 then
                let
                    rfl =
                        revfl matchL

                    rfd =
                        revfd matchD

                    packed =
                        Bitwise.or 268435456 (Bitwise.or (Bitwise.shiftLeftBy 18 rfl) rfd)

                    lin =
                        Bitwise.and 31 rfl

                    din =
                        Bitwise.and 31 rfd

                    nextEb =
                        eb1 + fleb lin + fdeb din
                in
                dfltLoop
                    cfg
                    (i + 1)
                    (i + matchL)
                    head1
                    prev1
                    (packed :: syms1)
                    (intTreeInc 6 (257 + lin) lf1)
                    (intTreeInc 3 din df1)
                    (BlkState bw1 (li1 + 1) (lc1 + 1) nextEb bs1)

            else
                let
                    b =
                        ByteBuf.getByte i cfg.dat
                in
                dfltLoop
                    cfg
                    (i + 1)
                    wi
                    head1
                    prev1
                    (b :: syms1)
                    (intTreeInc 6 b lf1)
                    df1
                    (BlkState bw1 (li1 + 1) lc1 eb1 bs1)


dflt : Bytes -> Int -> Int -> DeflateState -> ( List Int, DeflateState )
dflt dat lvl plvl st =
    let
        s =
            if st.z /= 0 then
                st.z

            else
                ByteBuf.length dat

        lst =
            st.l

        posBits =
            Bitwise.and 7 st.r
    in
    if lvl /= 0 then
        let
            initAcc =
                if posBits /= 0 then
                    Bitwise.and 255 (Bitwise.shiftRightZfBy 3 st.r)

                else
                    0

            bw0 =
                BW initAcc posBits []

            msk =
                Bitwise.shiftLeftBy plvl 1 - 1

            bs1 =
                (plvl + 2) // 3

            bs2 =
                bs1 + bs1

            ( head, headShift ) =
                case st.h of
                    Just existingHead ->
                        ( existingHead, st.hShift )

                    Nothing ->
                        ( Empty, shiftForSize (msk + 1) )

            prev =
                Maybe.withDefault Empty st.p

            cfg =
                { dat = dat
                , s = s
                , lst = lst
                , nice = deoNice lvl
                , chain = deoChain lvl
                , msk = msk
                , bs1 = bs1
                , bs2 = bs2
                , headShift = headShift
                }
        in
        dfltLoop cfg st.i st.w head prev [] Empty Empty (BlkState bw0 0 0 0 0)

    else
        let
            limit =
                s + lst

            loopLvl0 idx bwAcc =
                if (idx - limit) >= 0 then
                    let
                        (BW finalAcc finalBits finalRev) =
                            bwAcc

                        outRev =
                            if finalBits > 0 then
                                Bitwise.and 255 finalAcc :: finalRev

                            else
                                finalRev
                    in
                    ( List.reverse outRev, { st | i = s } )

                else
                    let
                        eRaw =
                            idx + 65535

                        ( e, finalBit ) =
                            if (eRaw - s) >= 0 then
                                ( s, lst )

                            else
                                ( eRaw, 0 )

                        bwNext =
                            bwAcc
                                |> bwWriteBits finalBit 1
                                |> bwWriteStored dat idx e
                    in
                    loopLvl0 (idx + 65535) bwNext
        in
        loopLvl0 st.w (BW 0 posBits [])


dopt : Bytes -> Int -> CompressOptions -> DeflateState -> ( List Int, DeflateState )
dopt dat datLenForPlvl opts st =
    let
        lvl =
            Maybe.withDefault 6 opts.level

        plvl =
            case opts.mem of
                Just m ->
                    12 + m

                Nothing ->
                    if st.l /= 0 then
                        defaultPlvl datLenForPlvl

                    else
                        20
    in
    dflt dat lvl plvl st


doptOneShot : Bytes -> CompressOptions -> List Int
doptOneShot dat opts =
    let
        ( fullDat, initW ) =
            case opts.dictionary of
                Just dict ->
                    let
                        dLen =
                            ByteBuf.length dict

                        tailDict =
                            if dLen > 32768 then
                                ByteBuf.slice (dLen - 32768) dLen dict

                            else
                                dict
                    in
                    ( ByteBuf.append tailDict dat, ByteBuf.length tailDict )

                Nothing ->
                    ( dat, 0 )

        st =
            { emptyDeflateState | l = 1, w = initW }

        ( outBytes, _ ) =
            dopt fullDat (ByteBuf.length fullDat) opts st
    in
    outBytes


deflateSync : Bytes -> CompressOptions -> List Int
deflateSync dat opts =
    doptOneShot dat opts
