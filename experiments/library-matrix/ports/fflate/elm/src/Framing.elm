module Framing exposing
    ( AdlerState
    , adlerDigestBytes
    , adlerInit
    , adlerUpdate
    , crcDigest
    , crcInit
    , crcUpdate
    , decompressSync
    , gunzipSync
    , gzhBytes
    , gzhl
    , gzipSync
    , gzl
    , gzs
    , isGzipMagic
    , isNotZlibHeader
    , u32LeBytes
    , unzlibSync
    , zlhBytes
    , zlibSync
    , zls
    )

{-| CRC-32, Adler-32, GZIP/Zlib header/footer framing, and `decompressSync` auto-detection
matching `fflate@0.8.3` bit-for-bit.
-}

import Bitwise
import ByteBuf exposing (Bytes)
import Codec
import Deflate exposing (CompressOptions, doptOneShot)
import Inflate exposing (DecompressOptions, FlateError(..), InflateBuf(..), emptyInflateState, inflateSync, inflt)
import Tables exposing (crct)



-- CRC-32


crcInit : Int
crcInit =
    -1


crcUpdate : Bytes -> Int -> Int
crcUpdate bytes c0 =
    ByteBuf.foldl
        (\b cr -> Bitwise.xor (crct (Bitwise.xor (Bitwise.and 255 cr) b)) (Bitwise.shiftRightZfBy 8 cr))
        c0
        bytes


crcDigest : Int -> Int
crcDigest c =
    Bitwise.shiftRightZfBy 0 (Bitwise.complement c)



-- ADLER-32


type AdlerState
    = AdlerState Int Int


adlerInit : AdlerState
adlerInit =
    AdlerState 1 0


adlerUpdate : Bytes -> AdlerState -> AdlerState
adlerUpdate bytes (AdlerState a0 b0) =
    let
        len =
            ByteBuf.length bytes

        stepBlock start a b =
            if (start - len) >= 0 then
                AdlerState a b

            else
                let
                    end =
                        if (start + 2655 - len) < 0 then
                            start + 2655

                        else
                            len

                    (AdlerState a1 b1) =
                        ByteBuf.foldlRange start
                            end
                            (\byte (AdlerState n m) ->
                                let
                                    n1 =
                                        n + byte
                                in
                                AdlerState n1 (m + n1)
                            )
                            (AdlerState a b)
                            bytes

                    a2 =
                        Bitwise.and 65535 a1 + 15 * Bitwise.shiftRightBy 16 a1

                    b2 =
                        Bitwise.and 65535 b1 + 15 * Bitwise.shiftRightBy 16 b1
                in
                stepBlock end a2 b2
    in
    stepBlock 0 a0 b0


{-| Big-endian 4-byte Adler-32 checksum matching `wbytes(..., adler.d())`.
-}
adlerDigestBytes : AdlerState -> List Int
adlerDigestBytes (AdlerState a b) =
    let
        a1 =
            remainderBy 65521 a

        b1 =
            remainderBy 65521 b
    in
    [ Bitwise.and 255 (Bitwise.shiftRightZfBy 8 b1)
    , Bitwise.and 255 b1
    , Bitwise.and 255 (Bitwise.shiftRightZfBy 8 a1)
    , Bitwise.and 255 a1
    ]


u32LeBytes : Int -> List Int
u32LeBytes v =
    let
        u =
            Bitwise.shiftRightZfBy 0 v
    in
    [ Bitwise.and 255 u
    , Bitwise.and 255 (Bitwise.shiftRightZfBy 8 u)
    , Bitwise.and 255 (Bitwise.shiftRightZfBy 16 u)
    , Bitwise.and 255 (Bitwise.shiftRightZfBy 24 u)
    ]



-- GZIP


gzhl : CompressOptions -> Int
gzhl opts =
    case opts.filename of
        Just fn ->
            if String.isEmpty fn then
                10

            else
                10 + String.length fn + 1

        Nothing ->
            10


gzhBytes : CompressOptions -> List Int
gzhBytes opts =
    let
        fnBytes =
            case opts.filename of
                Just fn ->
                    if String.isEmpty fn then
                        []

                    else
                        Codec.strToU8Latin1List fn ++ [ 0 ]

                Nothing ->
                    []

        flg =
            case fnBytes of
                [] ->
                    0

                _ ->
                    8

        mtimeBytes =
            case opts.mtime of
                Just m ->
                    if m /= 0 then
                        u32LeBytes (floor (m / 1000))

                    else
                        [ 0, 0, 0, 0 ]

                Nothing ->
                    [ 0, 0, 0, 0 ]

        xfl =
            case opts.level of
                Just lvl ->
                    if lvl < 2 then
                        4

                    else if lvl == 9 then
                        2

                    else
                        0

                Nothing ->
                    0
    in
    [ 31, 139, 8, flg ] ++ mtimeBytes ++ [ xfl, 3 ] ++ fnBytes


isGzipMagic : Bytes -> Bool
isGzipMagic d =
    ByteBuf.getByte 0 d == 31 && ByteBuf.getByte 1 d == 139 && ByteBuf.getByte 2 d == 8


gzs : Bytes -> Result FlateError Int
gzs d =
    if not (isGzipMagic d) then
        Err (CodeError 6 "invalid gzip data")

    else
        let
            flg =
                ByteBuf.getByte 3 d

            st0 =
                if Bitwise.and 4 flg /= 0 then
                    10 + ByteBuf.getU16Le 10 d + 2

                else
                    10

            zs0 =
                Bitwise.and 1 (Bitwise.shiftRightZfBy 3 flg)
                    + Bitwise.and 1 (Bitwise.shiftRightZfBy 4 flg)

            scanNul st zs =
                if zs <= 0 then
                    st

                else
                    let
                        b =
                            ByteBuf.getByte st d
                    in
                    if b == 0 then
                        scanNul (st + 1) (zs - 1)

                    else
                        scanNul (st + 1) zs

            st1 =
                scanNul st0 zs0
        in
        Ok (st1 + Bitwise.and 2 flg)


gzl : Bytes -> Int
gzl d =
    ByteBuf.getU32Le (ByteBuf.length d - 4) d


gzipSync : Bytes -> CompressOptions -> List Int
gzipSync dat opts =
    let
        c =
            crcDigest (crcUpdate dat crcInit)

        len =
            ByteBuf.length dat

        body =
            doptOneShot dat opts
    in
    gzhBytes opts ++ body ++ u32LeBytes c ++ u32LeBytes len


gunzipSync : Bytes -> DecompressOptions -> Result FlateError Bytes
gunzipSync dat opts =
    case gzs dat of
        Err err ->
            Err err

        Ok st ->
            let
                len =
                    ByteBuf.length dat
            in
            if st + 8 > len then
                Err (CodeError 6 "invalid gzip data")

            else
                let
                    outLen =
                        case opts.outSize of
                            Just n ->
                                n

                            Nothing ->
                                gzl dat

                    body =
                        ByteBuf.slice st (len - 8) dat
                in
                inflt body (emptyInflateState 2) (FixedBuf outLen) opts.dictionary
                    |> Result.map Tuple.first



-- ZLIB


zlhBytes : CompressOptions -> List Int
zlhBytes opts =
    let
        fl =
            case opts.level of
                Nothing ->
                    2

                Just lvl ->
                    if lvl == 0 then
                        0

                    else if lvl < 6 then
                        1

                    else if lvl == 9 then
                        3

                    else
                        2

        hasDict =
            case opts.dictionary of
                Just _ ->
                    True

                Nothing ->
                    False

        c0 =
            120

        c1a =
            Bitwise.or (Bitwise.shiftLeftBy 6 fl)
                (if hasDict then
                    32

                 else
                    0
                )

        c1 =
            Bitwise.or c1a (31 - remainderBy 31 (Bitwise.or (Bitwise.shiftLeftBy 8 c0) c1a))

        dictIdBytes =
            case opts.dictionary of
                Just dict ->
                    adlerDigestBytes (adlerUpdate dict adlerInit)

                Nothing ->
                    []
    in
    c0 :: c1 :: dictIdBytes


isNotZlibHeader : Bytes -> Bool
isNotZlibHeader d =
    let
        b0 =
            ByteBuf.getByte 0 d

        b1 =
            ByteBuf.getByte 1 d
    in
    Bitwise.and 15 b0
        /= 8
        || Bitwise.shiftRightZfBy 4 b0
        > 7
        || remainderBy 31 (Bitwise.or (Bitwise.shiftLeftBy 8 b0) b1)
        /= 0


zls : Bytes -> Bool -> Result FlateError Int
zls d hasDict =
    if isNotZlibHeader d then
        Err (CodeError 6 "invalid zlib data")

    else
        let
            b1 =
                ByteBuf.getByte 1 d

            fdict =
                Bitwise.and 1 (Bitwise.shiftRightZfBy 5 b1)

            expectOpposite =
                if hasDict then
                    0

                else
                    1
        in
        if fdict == expectOpposite then
            Err
                (CodeError 6
                    ("invalid zlib data: "
                        ++ (if Bitwise.and 32 b1 /= 0 then
                                "need"

                            else
                                "unexpected"
                           )
                        ++ " dictionary"
                    )
                )

        else
            Ok (Bitwise.and 4 (Bitwise.shiftRightZfBy 3 b1) + 2)


zlibSync : Bytes -> CompressOptions -> List Int
zlibSync dat opts =
    let
        a =
            adlerUpdate dat adlerInit

        body =
            doptOneShot dat opts
    in
    zlhBytes opts ++ body ++ adlerDigestBytes a


unzlibSync : Bytes -> DecompressOptions -> Result FlateError Bytes
unzlibSync dat opts =
    let
        hasDict =
            case opts.dictionary of
                Just _ ->
                    True

                Nothing ->
                    False
    in
    case zls dat hasDict of
        Err err ->
            Err err

        Ok start ->
            let
                len =
                    ByteBuf.length dat

                end =
                    if len > 4 then
                        len - 4

                    else
                        0

                inflBuf =
                    case opts.outSize of
                        Just n ->
                            FixedBuf n

                        Nothing ->
                            AutoBuf
            in
            inflt (ByteBuf.slice start end dat) (emptyInflateState 2) inflBuf opts.dictionary
                |> Result.map Tuple.first



-- AUTO-DETECT DECOMPRESS


decompressSync : Bytes -> DecompressOptions -> Result FlateError Bytes
decompressSync dat opts =
    if isGzipMagic dat then
        gunzipSync dat opts

    else if isNotZlibHeader dat then
        inflateSync dat opts

    else
        unzlibSync dat opts
