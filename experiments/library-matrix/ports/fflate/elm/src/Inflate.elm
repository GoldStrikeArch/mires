module Inflate exposing
    ( DecompressOptions
    , FlateError(..)
    , InflateBuf(..)
    , InflateState
    , emptyDecompressOptions
    , emptyInflateState
    , inflateSync
    , inflt
    )

{-| Raw DEFLATE decompressor (`inflt`, `inflateSync`) matching `fflate@0.8.3` bit-for-bit,
including zero-padded OOB reads, dynamic tree decoding, overlapping RLE back-references, custom
dictionary lookback, and `outSize` truncation / stored-block bounds behavior.
-}

import Bitwise
import ByteBuf
    exposing
        ( ByteBuilder
        , Bytes
        , IntTree(..)
        , builderGetByte
        , builderLength
        , builderPushByte
        , builderPushRepeatByte
        , builderToBytes
        , emptyBuilder
        , getBits
        , getBits16
        , intTreeGet
        , intTreeSet
        , shiftForSize
        )
import Deflate exposing (shft)
import Huffman exposing (fdrm, flrm, hMap)
import Tables exposing (clim, fd, fdeb, fl, fleb)


type FlateError
    = CodeError Int String
    | PlainError String


type alias DecompressOptions =
    { dictionary : Maybe Bytes
    , outSize : Maybe Int
    }


emptyDecompressOptions : DecompressOptions
emptyDecompressOptions =
    { dictionary = Nothing
    , outSize = Nothing
    }


type alias InflateState =
    { i : Int
    , f : Int
    , p : Int
    , b : Int
    , l : Maybe IntTree
    , d : Maybe IntTree
    , m : Int
    , n : Int
    }


emptyInflateState : Int -> InflateState
emptyInflateState mode =
    { i = mode
    , f = 0
    , p = 0
    , b = 0
    , l = Nothing
    , d = Nothing
    , m = 0
    , n = 0
    }


type InflateBuf
    = AutoBuf
    | FixedBuf Int
    | StreamBuf Bytes


type alias SymCfg =
    { dat : Bytes
    , tbts : Int
    , noSt : Int
    , maxLen : Int -- -1 means resizable / unlimited
    , dict : Bytes
    , dl : Int
    , lm : IntTree
    , lmShift : Int
    , lms : Int
    , dm : IntTree
    , dmShift : Int
    , dms : Int
    }


type SymOutcome
    = BlockDone Int Int ByteBuilder
    | NeedMore Int Int ByteBuilder
    | SymErr FlateError


type BlockInit
    = BlockErr FlateError
    | BlockStop ByteBuilder InflateState
    | BlockContinue InflateState ByteBuilder
    | BlockHuffman Int Int IntTree Int IntTree Int


type DynTrees
    = DynTrees Int IntTree Int IntTree Int


pushCapped : Int -> Int -> ByteBuilder -> ByteBuilder
pushCapped maxLen byte builder =
    if maxLen < 0 || (builderLength builder - maxLen) < 0 then
        builderPushByte byte builder

    else
        builder


copyDict : Bytes -> Int -> Int -> Int -> Int -> ByteBuilder -> ByteBuilder
copyDict dict shift maxLen bt dend builder =
    if (bt - dend) >= 0 then
        builder

    else
        let
            b =
                ByteBuf.getByte (shift + bt) dict
        in
        copyDict dict shift maxLen (bt + 1) dend (pushCapped maxLen b builder)


copyBackref : Int -> Int -> Int -> Int -> ByteBuilder -> ByteBuilder
copyBackref dt maxLen bt end builder =
    if (bt - end) >= 0 then
        builder

    else if dt == 1 then
        let
            count =
                if maxLen < 0 then
                    end - bt

                else
                    let
                        rem =
                            maxLen - builderLength builder

                        want =
                            end - bt
                    in
                    if rem <= 0 then
                        0

                    else if (rem - want) < 0 then
                        rem

                    else
                        want

            b =
                builderGetByte (bt - 1) builder
        in
        builderPushRepeatByte b count builder

    else
        let
            b =
                builderGetByte (bt - dt) builder
        in
        copyBackref dt maxLen (bt + 1) end (pushCapped maxLen b builder)


decodeSymbols : SymCfg -> Int -> Int -> Int -> ByteBuilder -> SymOutcome
decodeSymbols cfg pos lpos bt builder =
    let
        c =
            intTreeGet cfg.lmShift (Bitwise.and cfg.lms (getBits16 cfg.dat pos)) cfg.lm

        sym =
            Bitwise.shiftRightZfBy 4 c

        pos1 =
            pos + Bitwise.and 15 c
    in
    if (pos1 - cfg.tbts) > 0 then
        if cfg.noSt /= 0 then
            SymErr (CodeError 0 "unexpected EOF")

        else
            NeedMore lpos bt builder

    else if c == 0 then
        SymErr (CodeError 2 "invalid length/literal")

    else if sym < 256 then
        decodeSymbols cfg pos1 pos1 (bt + 1) (pushCapped cfg.maxLen sym builder)

    else if sym == 256 then
        BlockDone pos1 bt builder

    else
        let
            ( add, pos2 ) =
                if sym > 264 then
                    let
                        idx =
                            sym - 257

                        eb =
                            fleb idx
                    in
                    ( getBits cfg.dat pos1 (Bitwise.shiftLeftBy eb 1 - 1) + fl idx
                    , pos1 + eb
                    )

                else
                    ( sym - 254, pos1 )

            d =
                intTreeGet cfg.dmShift (Bitwise.and cfg.dms (getBits16 cfg.dat pos2)) cfg.dm

            dsym =
                Bitwise.shiftRightZfBy 4 d
        in
        if d == 0 then
            SymErr (CodeError 3 "invalid distance")

        else
            let
                pos3 =
                    pos2 + Bitwise.and 15 d

                ( dt, pos4 ) =
                    if dsym > 3 then
                        let
                            eb =
                                fdeb dsym
                        in
                        ( fd dsym + Bitwise.and (Bitwise.shiftLeftBy eb 1 - 1) (getBits16 cfg.dat pos3)
                        , pos3 + eb
                        )

                    else
                        ( fd dsym, pos3 )
            in
            if (pos4 - cfg.tbts) > 0 then
                if cfg.noSt /= 0 then
                    SymErr (CodeError 0 "unexpected EOF")

                else
                    NeedMore lpos bt builder

            else
                let
                    end =
                        bt + add
                in
                if (bt - dt) < 0 then
                    let
                        shift =
                            cfg.dl - dt
                    in
                    if shift + bt < 0 then
                        SymErr (CodeError 3 "invalid distance")

                    else
                        let
                            dend =
                                if (dt - end) < 0 then
                                    dt

                                else
                                    end

                            builder1 =
                                copyDict cfg.dict shift cfg.maxLen bt dend builder

                            builder2 =
                                copyBackref dt cfg.maxLen dend end builder1
                        in
                        decodeSymbols cfg pos4 pos4 end builder2

                else
                    let
                        builder1 =
                            copyBackref dt cfg.maxLen bt end builder
                    in
                    decodeSymbols cfg pos4 pos4 end builder1


readDynamicTrees : Bytes -> Int -> DynTrees
readDynamicTrees dat pos0 =
    let
        hLit =
            getBits dat pos0 31 + 257

        hcLen =
            getBits dat (pos0 + 10) 15 + 4

        hDist =
            getBits dat (pos0 + 5) 31 + 1

        tl =
            hLit + hDist

        pos1 =
            pos0 + 14

        readClt i clt maxClb =
            if (i - hcLen) >= 0 then
                ( clt, maxClb )

            else
                let
                    v =
                        getBits dat (pos1 + i * 3) 7

                    nextClb =
                        if (v - maxClb) > 0 then
                            v

                        else
                            maxClb
                in
                readClt (i + 1) (intTreeSet 3 (clim i) v clt) nextClb

        ( cltTree, clb ) =
            readClt 0 Empty 0

        pos2 =
            pos1 + hcLen * 3

        clbmsk =
            Bitwise.shiftLeftBy clb 1 - 1

        clm =
            hMap cltTree 3 19 clb True

        clmShift =
            shiftForSize (Bitwise.shiftLeftBy clb 1)

        fillLdt idx rem val ldt =
            if rem <= 0 then
                ldt

            else if (idx - tl) < 0 then
                fillLdt (idx + 1) (rem - 1) val (intTreeSet 6 idx val ldt)

            else
                ldt

        decodeLdt i pos ldt =
            if (i - tl) >= 0 then
                ( pos, ldt )

            else
                let
                    r =
                        intTreeGet clmShift (getBits dat pos clbmsk) clm

                    posA =
                        pos + Bitwise.and 15 r

                    s =
                        Bitwise.shiftRightZfBy 4 r
                in
                if s < 16 then
                    decodeLdt (i + 1) posA (intTreeSet 6 i s ldt)

                else if s == 16 then
                    let
                        n =
                            3 + getBits dat posA 3

                        c =
                            if i > 0 then
                                intTreeGet 6 (i - 1) ldt

                            else
                                0
                    in
                    decodeLdt (i + n) (posA + 2) (fillLdt i n c ldt)

                else if s == 17 then
                    let
                        n =
                            3 + getBits dat posA 7
                    in
                    decodeLdt (i + n) (posA + 3) (fillLdt i n 0 ldt)

                else
                    let
                        n =
                            11 + getBits dat posA 127
                    in
                    decodeLdt (i + n) (posA + 7) (fillLdt i n 0 ldt)

        ( pos3, ldtTree ) =
            decodeLdt 0 pos2 Empty

        extractLt i maxB acc =
            if (i - hLit) >= 0 then
                ( acc, maxB )

            else
                let
                    v =
                        intTreeGet 6 i ldtTree

                    nextMax =
                        if (v - maxB) > 0 then
                            v

                        else
                            maxB
                in
                extractLt (i + 1) nextMax (intTreeSet 6 i v acc)

        extractDt i maxB acc =
            if (i - hDist) >= 0 then
                ( acc, maxB )

            else
                let
                    v =
                        intTreeGet 6 (hLit + i) ldtTree

                    nextMax =
                        if (v - maxB) > 0 then
                            v

                        else
                            maxB
                in
                extractDt (i + 1) nextMax (intTreeSet 3 i v acc)

        ( ltTree, lbt ) =
            extractLt 0 0 Empty

        ( dtTree, dbt ) =
            extractDt 0 0 Empty

        lm =
            hMap ltTree 6 hLit lbt True

        dm =
            hMap dtTree 3 hDist dbt True
    in
    DynTrees pos3 lm lbt dm dbt


inflt : Bytes -> InflateState -> InflateBuf -> Maybe Bytes -> Result FlateError ( Bytes, InflateState )
inflt dat st0 inflBuf maybeDict =
    let
        sl =
            ByteBuf.length dat
    in
    if sl == 0 || (st0.f /= 0 && st0.l == Nothing) then
        let
            earlyOut =
                case inflBuf of
                    AutoBuf ->
                        ByteBuf.empty

                    FixedBuf outSize ->
                        ByteBuf.zeros outSize

                    StreamBuf initO ->
                        initO
        in
        Ok ( earlyOut, st0 )

    else
        let
            ( maxLen, initBuilder ) =
                case inflBuf of
                    AutoBuf ->
                        ( -1, emptyBuilder )

                    FixedBuf outSize ->
                        ( outSize, emptyBuilder )

                    StreamBuf initO ->
                        ( -1, ByteBuf.fromBytes (ByteBuf.slice 0 st0.b initO) )

            dict =
                Maybe.withDefault ByteBuf.empty maybeDict

            dl =
                ByteBuf.length dict

            noSt =
                st0.i

            tbts =
                sl * 8

            runBlocks st builder =
                let
                    blockInit =
                        case st.l of
                            Nothing ->
                                let
                                    final =
                                        getBits dat st.p 1

                                    btype =
                                        getBits dat (st.p + 1) 3

                                    posAfterHdr =
                                        st.p + 3
                                in
                                if btype == 0 then
                                    let
                                        s =
                                            shft posAfterHdr + 4

                                        l =
                                            ByteBuf.getU16Le (s - 4) dat

                                        t =
                                            s + l
                                    in
                                    if (t - sl) > 0 then
                                        if noSt /= 0 then
                                            BlockErr (CodeError 0 "unexpected EOF")

                                        else
                                            BlockStop builder st

                                    else if maxLen >= 0 && (st.b + l - maxLen) > 0 then
                                        BlockErr (PlainError "offset is out of bounds")

                                    else
                                        let
                                            nextBuilder =
                                                ByteBuf.appendSlice s t dat builder

                                            nextSt =
                                                { st | b = st.b + l, p = t * 8, f = final }
                                        in
                                        if final == 0 then
                                            BlockContinue nextSt nextBuilder

                                        else
                                            BlockStop nextBuilder nextSt

                                else if btype == 1 then
                                    if (posAfterHdr - tbts) > 0 then
                                        if noSt /= 0 then
                                            BlockErr (CodeError 0 "unexpected EOF")

                                        else
                                            BlockStop builder st

                                    else
                                        BlockHuffman final posAfterHdr flrm 9 fdrm 5

                                else if btype == 2 then
                                    let
                                        (DynTrees posAfterDyn lm lbt dm dbt) =
                                            readDynamicTrees dat posAfterHdr
                                    in
                                    if (posAfterDyn - tbts) > 0 then
                                        if noSt /= 0 then
                                            BlockErr (CodeError 0 "unexpected EOF")

                                        else
                                            BlockStop builder st

                                    else
                                        BlockHuffman final posAfterDyn lm lbt dm dbt

                                else
                                    BlockErr (CodeError 1 "invalid block type")

                            Just lm ->
                                BlockHuffman st.f st.p lm st.m (Maybe.withDefault Empty st.d) st.n
                in
                case blockInit of
                    BlockErr err ->
                        Err err

                    BlockStop outBuilder outSt ->
                        Ok ( builderToBytes outBuilder, outSt )

                    BlockContinue nextSt nextBuilder ->
                        runBlocks nextSt nextBuilder

                    BlockHuffman final pos lm lbt dm dbt ->
                        let
                            cfg =
                                { dat = dat
                                , tbts = tbts
                                , noSt = noSt
                                , maxLen = maxLen
                                , dict = dict
                                , dl = dl
                                , lm = lm
                                , lmShift = shiftForSize (Bitwise.shiftLeftBy lbt 1)
                                , lms = Bitwise.shiftLeftBy lbt 1 - 1
                                , dm = dm
                                , dmShift = shiftForSize (Bitwise.shiftLeftBy dbt 1)
                                , dms = Bitwise.shiftLeftBy dbt 1 - 1
                                }
                        in
                        case decodeSymbols cfg pos pos st.b builder of
                            SymErr err ->
                                Err err

                            BlockDone nextPos nextBt nextBuilder ->
                                let
                                    nextSt =
                                        { st | l = Nothing, p = nextPos, b = nextBt, f = final }
                                in
                                if final == 0 then
                                    runBlocks nextSt nextBuilder

                                else
                                    Ok ( builderToBytes nextBuilder, nextSt )

                            NeedMore savedLpos nextBt nextBuilder ->
                                let
                                    nextSt =
                                        { st
                                            | l = Just lm
                                            , p = savedLpos
                                            , b = nextBt
                                            , f = final
                                            , m = lbt
                                            , d = Just dm
                                            , n = dbt
                                        }
                                in
                                Ok ( builderToBytes nextBuilder, nextSt )
        in
        runBlocks st0 initBuilder


inflateSync : Bytes -> DecompressOptions -> Result FlateError Bytes
inflateSync dat opts =
    let
        inflBuf =
            case opts.outSize of
                Just n ->
                    FixedBuf n

                Nothing ->
                    AutoBuf
    in
    inflt dat (emptyInflateState 2) inflBuf opts.dictionary
        |> Result.map Tuple.first
