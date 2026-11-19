port module Main exposing (main)

{-| Floor port (Elm): the input crosses the request port as JSON, `text` is decoded, and the
same string goes back through the response port.
-}

import Json.Decode as Decode
import Json.Encode as Encode
import Platform


port request : (Decode.Value -> msg) -> Sub msg


port response : Encode.Value -> Cmd msg


handle : Decode.Value -> Encode.Value
handle value =
    case Decode.decodeValue (Decode.field "text" Decode.string) value of
        Ok text ->
            Encode.object [ ( "ok", Encode.string text ) ]

        Err error ->
            Encode.object [ ( "error", Encode.string (Decode.errorToString error) ) ]


main : Program () () Decode.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \value model -> ( model, response (handle value) )
        , subscriptions = \_ -> request identity
        }
