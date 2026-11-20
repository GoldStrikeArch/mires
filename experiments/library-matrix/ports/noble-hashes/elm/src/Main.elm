port module Main exposing (main)

{-| The boundary: libraries/noble-hashes/reference.mjs in Elm. The JS host sends the corpus
input through `request` and returns what `response` sends back; the input is decoded with
Json.Decode, an optional field only when present, so a present field of the wrong type is an
error as it is for noble.
-}

import Blake2 exposing (Options)
import Hashes exposing (Hash)
import Json.Decode as D
import Json.Encode as E


port request : (D.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg


main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \input _ -> ( (), response (encode (run input)) )
        , subscriptions = \_ -> request identity
        }


encode : Result String String -> E.Value
encode result =
    case result of
        Ok hex ->
            E.object [ ( "ok", E.string hex ) ]

        Err reason ->
            E.object [ ( "error", E.string reason ) ]


{-| A field that may be absent; when present it must decode.
-}
optional : String -> D.Decoder a -> D.Decoder (Maybe a)
optional name decoder =
    D.oneOf [ D.field name D.value |> D.map Just, D.succeed Nothing ]
        |> D.andThen
            (\present ->
                case present of
                    Just _ ->
                        D.field name decoder |> D.map Just

                    Nothing ->
                        D.succeed Nothing
            )


bytes : D.Decoder (List Int)
bytes =
    D.list D.int


{-| What the input supplies: a one-shot message, or chunks fed `repeat` times to one state.
-}
type Message
    = Bytes (List Int)
    | Chunks (List (List Int)) Int


options : D.Decoder Options
options =
    D.map4 Options
        (D.maybe (D.field "dkLen" D.value) |> D.map (Maybe.map (D.decodeValue D.float >> Result.mapError D.errorToString)))
        (optional "key" bytes)
        (optional "salt" bytes)
        (optional "personalization" bytes)


message : D.Decoder Message
message =
    optional "bytes" bytes
        |> D.andThen
            (\one ->
                case one of
                    Just data ->
                        D.succeed (Bytes data)

                    Nothing ->
                        D.map2 Chunks (D.field "chunks" (D.list bytes)) (optional "repeat" D.int |> D.map (Maybe.withDefault 1))
            )


decode : D.Decoder a -> D.Value -> Result String a
decode decoder value =
    D.decodeValue decoder value |> Result.mapError D.errorToString


run : D.Value -> Result String String
run input =
    decode (D.field "algorithm" D.string) input
        |> Result.andThen Hashes.create
        |> Result.andThen (\create -> decode options input |> Result.andThen create)
        |> Result.andThen (\hash -> decode message input |> Result.map (feed hash))
        |> Result.map toHex


feed : Hash -> Message -> List Int
feed hash input =
    case input of
        Bytes data ->
            Hashes.digest (Hashes.update data hash)

        Chunks chunks repeat ->
            Hashes.digest (repeatUpdates repeat chunks hash)


repeatUpdates : Int -> List (List Int) -> Hash -> Hash
repeatUpdates n chunks hash =
    if n <= 0 then
        hash

    else
        repeatUpdates (n - 1) chunks (List.foldl Hashes.update hash chunks)


toHex : List Int -> String
toHex digest =
    let
        nibble n =
            Char.fromCode
                (if n < 10 then
                    48 + n

                 else
                    87 + n
                )
    in
    digest
        |> List.concatMap (\b -> [ nibble (b // 16), nibble (modBy 16 b) ])
        |> String.fromList
