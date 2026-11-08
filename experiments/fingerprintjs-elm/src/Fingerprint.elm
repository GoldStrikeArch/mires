port module Fingerprint exposing (main)

{-| Elm side of the FingerprintJS 5.2.0 port.

Ports are the only Elm/JS boundary. Incoming requests are handled
synchronously by the Elm 0.19 scheduler, so the JS host can read the
outgoing `response` in the same `send` turn.
-}

import Canonical
import Confidence
import Hashing
import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import MathSource
import Platform
import Sources


port request : (Decode.Value -> msg) -> Sub msg


port response : Encode.Value -> Cmd msg


type Msg
    = Incoming Decode.Value


main : Program () () Msg
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = update
        , subscriptions = \_ -> request Incoming
        }


update : Msg -> () -> ( (), Cmd Msg )
update (Incoming value) _ =
    ( (), response (handle value) )


handle : Decode.Value -> Encode.Value
handle value =
    case Decode.decodeValue requestDecoder value of
        Ok req ->
            ok (run req)

        Err err ->
            fail (Decode.errorToString err)


type Request
    = Hash String Int
    | HashUnits (List Int) Int
    | Canonical (List Canonical.Component)
    | HashComponents (List Canonical.Component)
    | GetConfidence Confidence.Signals
    | Math MathSource.Native
    | SourceOrder


requestDecoder : Decoder Request
requestDecoder =
    Decode.field "op" Decode.string
        |> Decode.andThen
            (\op ->
                case op of
                    "hash" ->
                        let
                            seedDecoder =
                                Decode.oneOf [ Decode.field "seed" Decode.int, Decode.succeed 0 ]
                        in
                        Decode.oneOf
                            [ Decode.map2 HashUnits (Decode.field "units" (Decode.list Decode.int)) seedDecoder
                            , Decode.map2 Hash (Decode.field "input" Decode.string) seedDecoder
                            ]

                    "canonical" ->
                        Decode.map Canonical (Decode.field "items" (Decode.list componentDecoder))

                    "hashComponents" ->
                        Decode.map HashComponents (Decode.field "items" (Decode.list componentDecoder))

                    "confidence" ->
                        Decode.map GetConfidence signalsDecoder

                    "math" ->
                        Decode.map Math nativeMathDecoder

                    "sourceOrder" ->
                        Decode.succeed SourceOrder

                    _ ->
                        Decode.fail ("unknown op: " ++ op)
            )


componentDecoder : Decoder Canonical.Component
componentDecoder =
    Decode.map2 Canonical.Component
        (Decode.field "key" Decode.string)
        (Decode.field "value" Decode.string)


signalsDecoder : Decoder Confidence.Signals
signalsDecoder =
    Decode.map6 Confidence.Signals
        (Decode.oneOf [ Decode.field "platform" Decode.string, Decode.succeed "" ])
        (Decode.field "android" Decode.bool)
        (Decode.field "webkit" Decode.bool)
        (Decode.field "desktopWebkit" Decode.bool)
        (Decode.field "webkit616" Decode.bool)
        (Decode.field "safariWebkit" Decode.bool)


nativeMathDecoder : Decoder MathSource.Native
nativeMathDecoder =
    Decode.map7
        (\acos acosh asin asinh atanh atan sin ->
            { acos = acos
            , acosh = acosh
            , asin = asin
            , asinh = asinh
            , atanh = atanh
            , atan = atan
            , sin = sin
            , sinh = 0
            , cos = 0
            , cosh = 0
            , tan = 0
            , tanh = 0
            , exp = 0
            , expm1 = 0
            , log1p = 0
            , exp1 = 0
            , exp2 = 0
            }
        )
        (Decode.field "acos" Decode.float)
        (Decode.field "acosh" Decode.float)
        (Decode.field "asin" Decode.float)
        (Decode.field "asinh" Decode.float)
        (Decode.field "atanh" Decode.float)
        (Decode.field "atan" Decode.float)
        (Decode.field "sin" Decode.float)
        |> Decode.andThen
            (\partial ->
                Decode.map8
                    (\sinh cos cosh tan tanh exp expm1 log1p ->
                        { partial
                            | sinh = sinh
                            , cos = cos
                            , cosh = cosh
                            , tan = tan
                            , tanh = tanh
                            , exp = exp
                            , expm1 = expm1
                            , log1p = log1p
                        }
                    )
                    (Decode.field "sinh" Decode.float)
                    (Decode.field "cos" Decode.float)
                    (Decode.field "cosh" Decode.float)
                    (Decode.field "tan" Decode.float)
                    (Decode.field "tanh" Decode.float)
                    (Decode.field "exp" Decode.float)
                    (Decode.field "expm1" Decode.float)
                    (Decode.field "log1p" Decode.float)
            )
        |> Decode.andThen
            (\partial ->
                Decode.map2
                    (\exp1 exp2 -> { partial | exp1 = exp1, exp2 = exp2 })
                    (Decode.field "exp1" Decode.float)
                    (Decode.field "exp2" Decode.float)
            )


run : Request -> Encode.Value
run req =
    case req of
        Hash input seed ->
            Encode.string (Hashing.x64hash128 input seed)

        HashUnits units seed ->
            Encode.string (Hashing.x64hash128FromUnits units seed)

        Canonical items ->
            Encode.string (Canonical.canonicalString items)

        HashComponents items ->
            Encode.string (Canonical.hashComponents items)

        GetConfidence signals ->
            let
                confidence =
                    Confidence.getConfidence signals
            in
            Encode.object
                [ ( "score", Encode.float confidence.score )
                , ( "comment", Encode.string confidence.comment )
                ]

        Math native ->
            encodeMath (MathSource.fingerprint native)

        SourceOrder ->
            Encode.list Encode.string Sources.order


encodeMath : MathSource.Fingerprint -> Encode.Value
encodeMath math =
    Encode.object
        [ ( "acos", Encode.float math.acos )
        , ( "acosh", Encode.float math.acosh )
        , ( "acoshPf", Encode.float math.acoshPf )
        , ( "asin", Encode.float math.asin )
        , ( "asinh", Encode.float math.asinh )
        , ( "asinhPf", Encode.float math.asinhPf )
        , ( "atanh", Encode.float math.atanh )
        , ( "atanhPf", Encode.float math.atanhPf )
        , ( "atan", Encode.float math.atan )
        , ( "sin", Encode.float math.sin )
        , ( "sinh", Encode.float math.sinh )
        , ( "sinhPf", Encode.float math.sinhPf )
        , ( "cos", Encode.float math.cos )
        , ( "cosh", Encode.float math.cosh )
        , ( "coshPf", Encode.float math.coshPf )
        , ( "tan", Encode.float math.tan )
        , ( "tanh", Encode.float math.tanh )
        , ( "tanhPf", Encode.float math.tanhPf )
        , ( "exp", Encode.float math.exp )
        , ( "expm1", Encode.float math.expm1 )
        , ( "expm1Pf", Encode.float math.expm1Pf )
        , ( "log1p", Encode.float math.log1p )
        , ( "log1pPf", Encode.float math.log1pPf )
        , ( "powPI", Encode.float math.powPI )
        ]


ok : Encode.Value -> Encode.Value
ok value =
    Encode.object [ ( "result", value ) ]


fail : String -> Encode.Value
fail message =
    Encode.object [ ( "error", Encode.string message ) ]
