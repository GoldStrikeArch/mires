port module Main exposing (main)

{-| The Elm/JS boundary of the `fflate@0.8.3` port, mirroring `libraries/fflate/reference.mjs`.

The JS host (`js/index.js`) sends the JSON corpus input through the `request` port; `Main` decodes
it with `Json.Decode`, executes the requested `fflate` operation in pure Elm, and sends either
`{ ok: [byte, ...] }` or `{ error: message }` back through the `response` port. The worker model
remains `()`, so zero mutable state persists across calls.

-}

import Bitwise
import ByteBuf exposing (Bytes)
import Codec
import Deflate exposing (CompressOptions, emptyCompressOptions)
import Framing
import Inflate exposing (DecompressOptions, FlateError(..), emptyDecompressOptions)
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Platform
import Stream exposing (CompressKind(..), DecompressKind(..), StreamStep(..))


port request : (Decode.Value -> msg) -> Sub msg


port response : Encode.Value -> Cmd msg


main : Program () () Decode.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \value model -> ( model, response (handle value) )
        , subscriptions = \_ -> request identity
        }


type OutputPayload
    = OutBytes Bytes
    | OutList (List Int)


type Request
    = ReqDeflateSync Bytes CompressOptions
    | ReqGzipSync Bytes CompressOptions
    | ReqZlibSync Bytes CompressOptions
    | ReqInflateSync Bytes DecompressOptions
    | ReqGunzipSync Bytes DecompressOptions
    | ReqUnzlibSync Bytes DecompressOptions
    | ReqDecompressSync Bytes DecompressOptions
    | ReqStrToU8 String Bool
    | ReqStrFromU8 Bytes Bool
    | ReqCompressStream CompressKind CompressOptions Bool (List StreamStep)
    | ReqDecompressStream DecompressKind DecompressOptions Bool (List StreamStep)
    | ReqUnknown String


handle : Decode.Value -> Encode.Value
handle value =
    case Decode.decodeValue inputDecoder value of
        Err decodeErr ->
            Encode.object [ ( "error", Encode.string (Decode.errorToString decodeErr) ) ]

        Ok ( op, expectErrorCode, req ) ->
            let
                outcome =
                    execute req
            in
            if not expectErrorCode then
                case outcome of
                    Ok payload ->
                        Encode.object [ ( "ok", encodePayload payload ) ]

                    Err (CodeError _ msg) ->
                        Encode.object [ ( "error", Encode.string msg ) ]

                    Err (PlainError msg) ->
                        Encode.object [ ( "error", Encode.string msg ) ]

            else
                case outcome of
                    Ok _ ->
                        Encode.object [ ( "error", Encode.string ("expected FlateError for op " ++ op ++ ", but call succeeded") ) ]

                    Err (CodeError code _) ->
                        Encode.object [ ( "ok", Encode.list Encode.int [ 255, code ] ) ]

                    Err (PlainError msg) ->
                        Encode.object [ ( "error", Encode.string msg ) ]


encodePayload : OutputPayload -> Encode.Value
encodePayload payload =
    case payload of
        OutBytes bytes ->
            ByteBuf.toJsonValue bytes

        OutList ints ->
            Encode.list Encode.int ints


execute : Request -> Result FlateError OutputPayload
execute req =
    case req of
        ReqDeflateSync dat opts ->
            Ok (OutList (Deflate.deflateSync dat opts))

        ReqGzipSync dat opts ->
            Ok (OutList (Framing.gzipSync dat opts))

        ReqZlibSync dat opts ->
            Ok (OutList (Framing.zlibSync dat opts))

        ReqInflateSync dat opts ->
            Inflate.inflateSync dat opts |> Result.map OutBytes

        ReqGunzipSync dat opts ->
            Framing.gunzipSync dat opts |> Result.map OutBytes

        ReqUnzlibSync dat opts ->
            Framing.unzlibSync dat opts |> Result.map OutBytes

        ReqDecompressSync dat opts ->
            Framing.decompressSync dat opts |> Result.map OutBytes

        ReqStrToU8 text latin1 ->
            Ok (OutBytes (Codec.strToU8 text latin1))

        ReqStrFromU8 dat latin1 ->
            Ok (OutBytes (Codec.strFromU8Utf16Le dat latin1))

        ReqCompressStream kind opts noHandler steps ->
            Stream.runCompressStream kind opts noHandler steps |> Result.map OutList

        ReqDecompressStream kind opts noHandler steps ->
            Stream.runDecompressStream kind opts noHandler steps |> Result.map OutList

        ReqUnknown op ->
            Err (PlainError ("unknown operation " ++ op))



-- DECODERS


inputDecoder : Decoder ( String, Bool, Request )
inputDecoder =
    Decode.map2 Tuple.pair
        (Decode.field "op" Decode.string)
        (fieldOr "expectErrorCode" Decode.bool False)
        |> Decode.andThen
            (\( op, expectErrorCode ) ->
                requestDecoder op
                    |> Decode.map (\req -> ( op, expectErrorCode, req ))
            )


requestDecoder : String -> Decoder Request
requestDecoder op =
    case op of
        "deflateSync" ->
            Decode.map2 ReqDeflateSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" compressOptionsDecoder emptyCompressOptions)

        "gzipSync" ->
            Decode.map2 ReqGzipSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" compressOptionsDecoder emptyCompressOptions)

        "zlibSync" ->
            Decode.map2 ReqZlibSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" compressOptionsDecoder emptyCompressOptions)

        "inflateSync" ->
            Decode.map2 ReqInflateSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" decompressOptionsDecoder emptyDecompressOptions)

        "gunzipSync" ->
            Decode.map2 ReqGunzipSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" decompressOptionsDecoder emptyDecompressOptions)

        "unzlibSync" ->
            Decode.map2 ReqUnzlibSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" decompressOptionsDecoder emptyDecompressOptions)

        "decompressSync" ->
            Decode.map2 ReqDecompressSync
                (Decode.field "data" payloadDecoder)
                (fieldOr "options" decompressOptionsDecoder emptyDecompressOptions)

        "strToU8" ->
            Decode.map2 ReqStrToU8
                (Decode.field "text" Decode.string)
                (fieldOr "latin1" Decode.bool False)

        "strFromU8" ->
            Decode.map2 ReqStrFromU8
                (Decode.field "data" payloadDecoder)
                (fieldOr "latin1" Decode.bool False)

        "Deflate" ->
            compressStreamDecoder KindDeflate

        "Gzip" ->
            compressStreamDecoder KindGzip

        "Zlib" ->
            compressStreamDecoder KindZlib

        "Inflate" ->
            decompressStreamDecoder KindInflate

        "Gunzip" ->
            decompressStreamDecoder KindGunzip

        "Unzlib" ->
            decompressStreamDecoder KindUnzlib

        "Decompress" ->
            decompressStreamDecoder KindDecompress

        _ ->
            Decode.succeed (ReqUnknown op)


compressStreamDecoder : CompressKind -> Decoder Request
compressStreamDecoder kind =
    Decode.map3 (ReqCompressStream kind)
        (fieldOr "options" compressOptionsDecoder emptyCompressOptions)
        (fieldOr "noHandler" Decode.bool False)
        (Decode.field "steps" (Decode.list streamStepDecoder))


decompressStreamDecoder : DecompressKind -> Decoder Request
decompressStreamDecoder kind =
    Decode.map3 (ReqDecompressStream kind)
        (fieldOr "options" decompressOptionsDecoder emptyDecompressOptions)
        (fieldOr "noHandler" Decode.bool False)
        (Decode.field "steps" (Decode.list streamStepDecoder))


payloadDecoder : Decoder Bytes
payloadDecoder =
    optionalField "bytes" (Decode.list Decode.int)
        |> Decode.andThen
            (\maybeBytes ->
                case maybeBytes of
                    Just intList ->
                        Decode.succeed (ByteBuf.fromList (List.map (Bitwise.and 255) intList))

                    Nothing ->
                        Decode.map2 Codec.strToU8
                            (Decode.field "text" Decode.string)
                            (fieldOr "latin1" Decode.bool False)
            )


compressOptionsDecoder : Decoder CompressOptions
compressOptionsDecoder =
    Decode.map5 CompressOptions
        (optionalField "level" Decode.int)
        (optionalField "mem" Decode.int)
        (optionalField "dictionary" payloadDecoder)
        (optionalField "mtime" Decode.float)
        (optionalField "filename" Decode.string)


decompressOptionsDecoder : Decoder DecompressOptions
decompressOptionsDecoder =
    Decode.map2 DecompressOptions
        (optionalField "dictionary" payloadDecoder)
        (optionalField "outSize" Decode.int)


streamStepDecoder : Decoder StreamStep
streamStepDecoder =
    Decode.field "action" Decode.string
        |> Decode.andThen
            (\action ->
                case action of
                    "push" ->
                        Decode.map2 StepPush
                            (Decode.field "data" payloadDecoder)
                            (fieldOr "final" Decode.bool False)

                    "flush" ->
                        Decode.map StepFlush
                            (fieldOr "sync" Decode.bool False)

                    _ ->
                        Decode.succeed (StepUnknown action)
            )


optionalField : String -> Decoder a -> Decoder (Maybe a)
optionalField name decoder =
    Decode.maybe (Decode.field name Decode.value)
        |> Decode.andThen
            (\found ->
                case found of
                    Just _ ->
                        Decode.map Just (Decode.field name decoder)

                    Nothing ->
                        Decode.succeed Nothing
            )


fieldOr : String -> Decoder a -> a -> Decoder a
fieldOr name decoder fallback =
    optionalField name decoder |> Decode.map (Maybe.withDefault fallback)
