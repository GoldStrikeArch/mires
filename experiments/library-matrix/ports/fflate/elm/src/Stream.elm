module Stream exposing
    ( CompressKind(..)
    , DecompressKind(..)
    , StreamStep(..)
    , runCompressStream
    , runDecompressStream
    )

{-| Synchronous streaming classes (`Deflate`, `Gzip`, `Zlib`, `Inflate`, `Gunzip`, `Unzlib`,
`Decompress`) and `[u32le(len), final, ...chunk]` record framing matching `fflate@0.8.3` and
`reference.mjs` bit-for-bit.
-}

import Bitwise
import ByteBuf exposing (Bytes)
import Deflate
    exposing
        ( CompressOptions
        , DeflateState
        , dopt
        , emptyCompressOptions
        , emptyDeflateState
        , shft
        , syncFlushBytes
        )
import Framing
    exposing
        ( AdlerState
        , adlerDigestBytes
        , adlerInit
        , adlerUpdate
        , crcDigest
        , crcInit
        , crcUpdate
        , gzhBytes
        , gzhl
        , gzs
        , isGzipMagic
        , isNotZlibHeader
        , u32LeBytes
        , zlhBytes
        , zls
        )
import Inflate
    exposing
        ( DecompressOptions
        , FlateError(..)
        , InflateBuf(..)
        , InflateState
        , emptyInflateState
        , inflt
        )


type StreamStep
    = StepPush Bytes Bool
    | StepFlush Bool
    | StepUnknown String


type CompressKind
    = KindDeflate
    | KindGzip
    | KindZlib


type DecompressKind
    = KindInflate
    | KindGunzip
    | KindUnzlib
    | KindDecompress


type StreamChunk
    = ChunkBytes Bytes
    | ChunkList (List Int)


prependList : List Int -> List Int -> List Int
prependList xs acc =
    List.foldl (::) acc (List.reverse xs)


frameRecordsRev : List ( StreamChunk, Bool ) -> List Int
frameRecordsRev recordsRev =
    let
        prependRecord ( chunk, isFinal ) acc =
            let
                ( chunkLen, withChunk ) =
                    case chunk of
                        ChunkBytes bs ->
                            ( ByteBuf.length bs, ByteBuf.foldr (::) acc bs )

                        ChunkList lst ->
                            ( List.length lst, prependList lst acc )

                flag =
                    if isFinal then
                        1

                    else
                        0

                u =
                    Bitwise.shiftRightZfBy 0 chunkLen
            in
            Bitwise.and 255 u
                :: Bitwise.and 255 (Bitwise.shiftRightZfBy 8 u)
                :: Bitwise.and 255 (Bitwise.shiftRightZfBy 16 u)
                :: Bitwise.and 255 (Bitwise.shiftRightZfBy 24 u)
                :: flag
                :: withChunk
    in
    List.foldl prependRecord [] recordsRev


tail32768 : Bytes -> Bytes
tail32768 dict =
    let
        len =
            ByteBuf.length dict
    in
    if len > 32768 then
        ByteBuf.slice (len - 32768) len dict

    else
        dict



-- COMPRESS STREAMS: Deflate, Gzip, Zlib


type CompressWrapper
    = WrapRaw
    | WrapGzip Int Int Bool -- crc, totalLen, headerPending
    | WrapZlib AdlerState Bool -- adler, headerPending


type alias CompressStream =
    { hasHandler : Bool
    , opts : CompressOptions
    , s : DeflateState
    , bData : Bytes
    , bCap : Int
    , wrapper : CompressWrapper
    }


initCompressStream : CompressKind -> CompressOptions -> Bool -> CompressStream
initCompressStream kind opts noHandler =
    let
        ( initBData, initI ) =
            case opts.dictionary of
                Just dict ->
                    let
                        d =
                            tail32768 dict

                        dLen =
                            ByteBuf.length d

                        padLen =
                            32768 - dLen
                    in
                    ( ByteBuf.append (ByteBuf.zeros padLen) d, padLen )

                Nothing ->
                    ( ByteBuf.zeros 32768, 32768 )

        wrapper =
            case kind of
                KindDeflate ->
                    WrapRaw

                KindGzip ->
                    WrapGzip crcInit 0 True

                KindZlib ->
                    WrapZlib adlerInit True
    in
    { hasHandler = not noHandler
    , opts = opts
    , s = { emptyDeflateState | l = 0, i = initI, w = 32768, z = 32768 }
    , bData = initBData
    , bCap = 98304
    , wrapper = wrapper
    }


emitCompressChunk : CompressStream -> Bytes -> Int -> Bool -> List ( StreamChunk, Bool ) -> ( CompressStream, List ( StreamChunk, Bool ) )
emitCompressChunk st cData cCap isFinal recs =
    let
        ( body, s1 ) =
            dopt cData cCap st.opts st.s

        ( chunkBytes, wrapper1 ) =
            case st.wrapper of
                WrapRaw ->
                    ( body, WrapRaw )

                WrapGzip c totalLen pending ->
                    let
                        hdr =
                            if pending then
                                gzhBytes st.opts

                            else
                                []

                        ftr =
                            if isFinal then
                                u32LeBytes (crcDigest c) ++ u32LeBytes totalLen

                            else
                                []
                    in
                    ( prependList hdr (prependList body ftr)
                    , WrapGzip c totalLen False
                    )

                WrapZlib adler pending ->
                    let
                        hdr =
                            if pending then
                                zlhBytes st.opts

                            else
                                []

                        ftr =
                            if isFinal then
                                adlerDigestBytes adler

                            else
                                []
                    in
                    ( prependList hdr (prependList body ftr)
                    , WrapZlib adler False
                    )
    in
    ( { st | s = s1, wrapper = wrapper1 }
    , ( ChunkList chunkBytes, isFinal ) :: recs
    )


compressPush : Bytes -> Bool -> CompressStream -> List ( StreamChunk, Bool ) -> Result FlateError ( CompressStream, List ( StreamChunk, Bool ) )
compressPush chunk final st0 recs0 =
    let
        chunkLen =
            ByteBuf.length chunk

        stWrapped =
            case st0.wrapper of
                WrapRaw ->
                    st0

                WrapGzip c totalLen pending ->
                    { st0 | wrapper = WrapGzip (crcUpdate chunk c) (totalLen + chunkLen) pending }

                WrapZlib adler pending ->
                    { st0 | wrapper = WrapZlib (adlerUpdate chunk adler) pending }
    in
    if not stWrapped.hasHandler then
        Err (CodeError 5 "no stream handler")

    else if stWrapped.s.l /= 0 then
        Err (CodeError 4 "stream finished")

    else
        let
            endLen =
                chunkLen + stWrapped.s.z

            ( st1, recs1 ) =
                if endLen > stWrapped.bCap then
                    let
                        bCap1 =
                            if endLen > 2 * stWrapped.bCap - 32768 then
                                Bitwise.and -32768 endLen

                            else
                                stWrapped.bCap

                        split =
                            bCap1 - stWrapped.s.z

                        bFull =
                            ByteBuf.append stWrapped.bData (ByteBuf.slice 0 split chunk)

                        sFull =
                            stWrapped.s

                        stBeforeEmit =
                            { stWrapped | bCap = bCap1, bData = bFull, s = { sFull | z = bCap1 } }

                        ( stAfterEmit, recsAfterEmit ) =
                            emitCompressChunk stBeforeEmit bFull bCap1 False recs0

                        bSlid =
                            ByteBuf.append
                                (ByteBuf.slice (bCap1 - 32768) bCap1 bFull)
                                (ByteBuf.slice split chunkLen chunk)

                        sAfterEmit =
                            stAfterEmit.s

                        sSlid =
                            { sAfterEmit
                                | z = chunkLen - split + 32768
                                , i = 32766
                                , w = 32768
                            }
                    in
                    ( { stAfterEmit | bData = bSlid, s = sSlid }, recsAfterEmit )

                else
                    let
                        sCur =
                            stWrapped.s
                    in
                    ( { stWrapped
                        | bData = ByteBuf.append stWrapped.bData chunk
                        , s = { sCur | z = sCur.z + chunkLen }
                      }
                    , recs0
                    )

            sBeforeFinal =
                st1.s

            st2 =
                { st1
                    | s =
                        { sBeforeFinal
                            | l =
                                if final then
                                    1

                                else
                                    0
                        }
                }

            ( st3, recs2 ) =
                if st2.s.z > st2.s.w + 8191 || final then
                    let
                        ( stEmitted, recsEmitted ) =
                            emitCompressChunk st2 st2.bData st2.bCap final recs1

                        sE =
                            stEmitted.s
                    in
                    ( { stEmitted | s = { sE | w = sE.i, i = sE.i - 2 } }, recsEmitted )

                else
                    ( st2, recs1 )

            st4 =
                if final then
                    { st3 | s = emptyDeflateState, opts = emptyCompressOptions, bData = ByteBuf.empty, bCap = 0 }

                else
                    st3
        in
        Ok ( st4, recs2 )


compressFlush : Bool -> CompressStream -> List ( StreamChunk, Bool ) -> Result FlateError ( CompressStream, List ( StreamChunk, Bool ) )
compressFlush sync st0 recs0 =
    if not st0.hasHandler then
        Err (CodeError 5 "no stream handler")

    else if st0.s.l /= 0 then
        Err (CodeError 4 "stream finished")

    else
        let
            ( st1, recs1 ) =
                emitCompressChunk st0 st0.bData st0.bCap False recs0

            s1 =
                st1.s

            st2 =
                { st1 | s = { s1 | w = s1.i, i = s1.i - 2 } }
        in
        if sync then
            let
                syncBytes =
                    syncFlushBytes st2.s.r

                s2 =
                    st2.s
            in
            Ok ( { st2 | s = { s2 | r = 0 } }, ( ChunkList syncBytes, False ) :: recs1 )

        else
            Ok ( st2, recs1 )


runCompressStream : CompressKind -> CompressOptions -> Bool -> List StreamStep -> Result FlateError (List Int)
runCompressStream kind opts noHandler steps =
    let
        go remaining st recs =
            case remaining of
                [] ->
                    Ok (frameRecordsRev recs)

                step :: rest ->
                    case step of
                        StepPush chunk final ->
                            case compressPush chunk final st recs of
                                Ok ( nextSt, nextRecs ) ->
                                    go rest nextSt nextRecs

                                Err err ->
                                    Err err

                        StepFlush sync ->
                            case compressFlush sync st recs of
                                Ok ( nextSt, nextRecs ) ->
                                    go rest nextSt nextRecs

                                Err err ->
                                    Err err

                        StepUnknown action ->
                            Err (PlainError ("unknown stream action " ++ action))
    in
    go steps (initCompressStream kind opts noHandler) []



-- DECOMPRESS STREAMS: Inflate, Gunzip, Unzlib, Decompress


type alias InflateStream =
    { hasHandler : Bool
    , done : Bool
    , s : InflateState
    , o : Bytes
    , p : Bytes
    }


initInflateStream : DecompressOptions -> Bool -> InflateStream
initInflateStream opts noHandler =
    let
        ( initO, initB ) =
            case opts.dictionary of
                Just dict ->
                    let
                        d =
                            tail32768 dict

                        dLen =
                            ByteBuf.length d
                    in
                    ( ByteBuf.append d (ByteBuf.zeros (32768 - dLen)), dLen )

                Nothing ->
                    ( ByteBuf.zeros 32768, 0 )

        s0 =
            emptyInflateState 0
    in
    { hasHandler = not noHandler
    , done = False
    , s = { s0 | b = initB }
    , o = initO
    , p = ByteBuf.empty
    }


inflateE : Bytes -> InflateStream -> Result FlateError InflateStream
inflateE chunk st =
    if not st.hasHandler then
        Err (CodeError 5 "no stream handler")

    else if st.done then
        Err (CodeError 4 "stream finished")

    else if ByteBuf.length st.p == 0 then
        Ok { st | p = chunk }

    else if ByteBuf.length chunk > 0 then
        Ok { st | p = ByteBuf.append st.p chunk }

    else
        Ok st


inflateC : Bool -> InflateStream -> List ( StreamChunk, Bool ) -> Result FlateError ( InflateStream, List ( StreamChunk, Bool ) )
inflateC final st recs =
    let
        s0 =
            st.s

        s1 =
            { s0
                | i =
                    if final then
                        1

                    else
                        0
            }

        bts =
            s1.b
    in
    case inflt st.p s1 (StreamBuf st.o) Nothing of
        Err err ->
            Err err

        Ok ( dt, s2 ) ->
            let
                emitted =
                    ByteBuf.slice bts s2.b dt

                oStart =
                    if s2.b > 32768 then
                        s2.b - 32768

                    else
                        0

                oNew =
                    ByteBuf.slice oStart (ByteBuf.length dt) dt

                pNew =
                    ByteBuf.slice (Bitwise.shiftRightZfBy 3 s2.p) (ByteBuf.length st.p) st.p

                s3 =
                    { s2 | b = ByteBuf.length oNew, p = Bitwise.and 7 s2.p }
            in
            Ok
                ( { st | done = final, s = s3, o = oNew, p = pNew }
                , ( ChunkBytes emitted, final ) :: recs
                )


inflatePush : Bytes -> Bool -> InflateStream -> List ( StreamChunk, Bool ) -> Result FlateError ( InflateStream, List ( StreamChunk, Bool ) )
inflatePush chunk final st recs =
    case inflateE chunk st of
        Err err ->
            Err err

        Ok st1 ->
            inflateC final st1 recs


type alias GunzipStream =
    { inf : InflateStream
    , v : Int
    , r : Int
    }


initGunzipStream : DecompressOptions -> Bool -> GunzipStream
initGunzipStream opts noHandler =
    { inf = initInflateStream opts noHandler
    , v = 1
    , r = 0
    }


gunzipPush : Bytes -> Bool -> GunzipStream -> List ( StreamChunk, Bool ) -> Result FlateError ( GunzipStream, List ( StreamChunk, Bool ) )
gunzipPush chunk final gst recs =
    case inflateE chunk gst.inf of
        Err err ->
            Err err

        Ok inf1 ->
            let
                r1 =
                    gst.r + ByteBuf.length chunk

                afterHeader =
                    if gst.v /= 0 then
                        let
                            pSub =
                                ByteBuf.slice (gst.v - 1) (ByteBuf.length inf1.p) inf1.p

                            pLen =
                                ByteBuf.length pSub

                            sRes =
                                if pLen > 3 then
                                    gzs pSub

                                else
                                    Ok 4
                        in
                        case sRes of
                            Err err ->
                                Err err

                            Ok s ->
                                if s > pLen && not final then
                                    Ok Nothing

                                else
                                    Ok (Just ( { inf1 | p = ByteBuf.slice s pLen pSub }, 0 ))

                    else
                        Ok (Just ( inf1, 0 ))
            in
            case afterHeader of
                Err err ->
                    Err err

                Ok Nothing ->
                    Ok ( { gst | inf = inf1, r = r1 }, recs )

                Ok (Just ( inf2, v2 )) ->
                    case inflateC False inf2 recs of
                        Err err ->
                            Err err

                        Ok ( inf3, recs1 ) ->
                            if inf3.s.f /= 0 && inf3.s.l == Nothing then
                                let
                                    vNext =
                                        shft inf3.s.p + 9

                                    infReset =
                                        { inf3 | s = emptyInflateState 0, o = ByteBuf.empty }
                                in
                                gunzipPush ByteBuf.empty final { inf = infReset, v = vNext, r = r1 } recs1

                            else if final then
                                case inflateC True inf3 recs1 of
                                    Err err ->
                                        Err err

                                    Ok ( inf4, recs2 ) ->
                                        Ok ( { inf = inf4, v = v2, r = r1 }, recs2 )

                            else
                                Ok ( { inf = inf3, v = v2, r = r1 }, recs1 )


type alias UnzlibStream =
    { inf : InflateStream
    , v : Int
    }


initUnzlibStream : DecompressOptions -> Bool -> UnzlibStream
initUnzlibStream opts noHandler =
    let
        v0 =
            case opts.dictionary of
                Just _ ->
                    2

                Nothing ->
                    1
    in
    { inf = initInflateStream opts noHandler
    , v = v0
    }


unzlibPush : Bytes -> Bool -> UnzlibStream -> List ( StreamChunk, Bool ) -> Result FlateError ( UnzlibStream, List ( StreamChunk, Bool ) )
unzlibPush chunk final ust recs =
    case inflateE chunk ust.inf of
        Err err ->
            Err err

        Ok inf1 ->
            let
                afterHeader =
                    if ust.v /= 0 then
                        if ByteBuf.length inf1.p < 6 && not final then
                            Ok Nothing

                        else
                            case zls inf1.p (ust.v - 1 /= 0) of
                                Err err ->
                                    Err err

                                Ok start ->
                                    Ok (Just ( { inf1 | p = ByteBuf.slice start (ByteBuf.length inf1.p) inf1.p }, 0 ))

                    else
                        Ok (Just ( inf1, 0 ))
            in
            case afterHeader of
                Err err ->
                    Err err

                Ok Nothing ->
                    Ok ( { ust | inf = inf1 }, recs )

                Ok (Just ( inf2, v2 )) ->
                    let
                        afterTrailer =
                            if final then
                                let
                                    pLen =
                                        ByteBuf.length inf2.p
                                in
                                if pLen < 4 then
                                    Err (CodeError 6 "invalid zlib data")

                                else
                                    Ok { inf2 | p = ByteBuf.slice 0 (pLen - 4) inf2.p }

                            else
                                Ok inf2
                    in
                    case afterTrailer of
                        Err err ->
                            Err err

                        Ok inf3 ->
                            case inflateC final inf3 recs of
                                Err err ->
                                    Err err

                                Ok ( inf4, recs1 ) ->
                                    Ok ( { inf = inf4, v = v2 }, recs1 )


type InnerDecompress
    = InnerInflate InflateStream
    | InnerGunzip GunzipStream
    | InnerUnzlib UnzlibStream


type DecompressState
    = StateInflate InflateStream
    | StateGunzip GunzipStream
    | StateUnzlib UnzlibStream
    | StateAuto
        { hasHandler : Bool
        , opts : DecompressOptions
        , inner : Maybe InnerDecompress
        , pending : Bytes
        }


initDecompressState : DecompressKind -> DecompressOptions -> Bool -> DecompressState
initDecompressState kind opts noHandler =
    case kind of
        KindInflate ->
            StateInflate (initInflateStream opts noHandler)

        KindGunzip ->
            StateGunzip (initGunzipStream opts noHandler)

        KindUnzlib ->
            StateUnzlib (initUnzlibStream opts noHandler)

        KindDecompress ->
            StateAuto
                { hasHandler = not noHandler
                , opts = opts
                , inner = Nothing
                , pending = ByteBuf.empty
                }


pushInner : Bytes -> Bool -> InnerDecompress -> List ( StreamChunk, Bool ) -> Result FlateError ( InnerDecompress, List ( StreamChunk, Bool ) )
pushInner chunk final inner recs =
    case inner of
        InnerInflate st ->
            inflatePush chunk final st recs |> Result.map (Tuple.mapFirst InnerInflate)

        InnerGunzip st ->
            gunzipPush chunk final st recs |> Result.map (Tuple.mapFirst InnerGunzip)

        InnerUnzlib st ->
            unzlibPush chunk final st recs |> Result.map (Tuple.mapFirst InnerUnzlib)


decompressPush : Bytes -> Bool -> DecompressState -> List ( StreamChunk, Bool ) -> Result FlateError ( DecompressState, List ( StreamChunk, Bool ) )
decompressPush chunk final state recs =
    case state of
        StateInflate st ->
            inflatePush chunk final st recs |> Result.map (Tuple.mapFirst StateInflate)

        StateGunzip st ->
            gunzipPush chunk final st recs |> Result.map (Tuple.mapFirst StateGunzip)

        StateUnzlib st ->
            unzlibPush chunk final st recs |> Result.map (Tuple.mapFirst StateUnzlib)

        StateAuto auto ->
            if not auto.hasHandler then
                Err (CodeError 5 "no stream handler")

            else
                case auto.inner of
                    Just inner ->
                        pushInner chunk final inner recs
                            |> Result.map (\( nextInner, nextRecs ) -> ( StateAuto { auto | inner = Just nextInner }, nextRecs ))

                    Nothing ->
                        let
                            p1 =
                                if ByteBuf.length auto.pending > 0 then
                                    ByteBuf.append auto.pending chunk

                                else
                                    chunk
                        in
                        if ByteBuf.length p1 > 2 then
                            let
                                selected =
                                    if isGzipMagic p1 then
                                        InnerGunzip (initGunzipStream auto.opts False)

                                    else if isNotZlibHeader p1 then
                                        InnerInflate (initInflateStream auto.opts False)

                                    else
                                        InnerUnzlib (initUnzlibStream auto.opts False)
                            in
                            pushInner p1 final selected recs
                                |> Result.map
                                    (\( nextInner, nextRecs ) ->
                                        ( StateAuto { auto | inner = Just nextInner, pending = ByteBuf.empty }
                                        , nextRecs
                                        )
                                    )

                        else
                            Ok ( StateAuto { auto | pending = p1 }, recs )


runDecompressStream : DecompressKind -> DecompressOptions -> Bool -> List StreamStep -> Result FlateError (List Int)
runDecompressStream kind opts noHandler steps =
    let
        go remaining state recs =
            case remaining of
                [] ->
                    Ok (frameRecordsRev recs)

                step :: rest ->
                    case step of
                        StepPush chunk final ->
                            case decompressPush chunk final state recs of
                                Ok ( nextState, nextRecs ) ->
                                    go rest nextState nextRecs

                                Err err ->
                                    Err err

                        StepFlush _ ->
                            Err (PlainError "flush is not supported on decompression streams")

                        StepUnknown action ->
                            Err (PlainError ("unknown stream action " ++ action))
    in
    go steps (initDecompressState kind opts noHandler) []
