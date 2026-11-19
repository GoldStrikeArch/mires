port module Main exposing (main)

{-| Elm side of the pilot port. The boundary: the JS host (js/index.js) sends the corpus input
through the `request` port as JSON, it is decoded into `{ text, top }`, and the result string
comes back through the `response` port.
-}

import Json.Decode as Decode
import Json.Encode as Encode
import Pilot
import Platform


port request : (Decode.Value -> msg) -> Sub msg


port response : Encode.Value -> Cmd msg


type alias Input =
    { text : String, top : Int }


decoder : Decode.Decoder Input
decoder =
    Decode.map2 Input (Decode.field "text" Decode.string) (Decode.field "top" Decode.int)


handle : Decode.Value -> Encode.Value
handle value =
    case Decode.decodeValue decoder value of
        Ok input ->
            Encode.object [ ( "ok", Encode.string (Pilot.analyse input.text input.top) ) ]

        Err error ->
            Encode.object [ ( "error", Encode.string (Decode.errorToString error) ) ]


main : Program () () Decode.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \value model -> ( model, response (handle value) )
        , subscriptions = \_ -> request identity
        }
