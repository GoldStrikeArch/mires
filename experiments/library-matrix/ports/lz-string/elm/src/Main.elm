port module Main exposing (main)
import Codec
import Json.Decode as D
import Json.Encode as E

port request : (D.Value -> msg) -> Sub msg
port response : E.Value -> Cmd msg

run : D.Value -> E.Value
run input =
    let
        field key decoder = D.decodeValue (D.field key decoder) input |> Result.toMaybe
        format = field "format" D.string |> Maybe.withDefault ""
        op = field "op" D.string |> Maybe.withDefault ""
        execute data =
            if op == "decompress" then Codec.decompress format data
            else
                let
                    times = field "repeat" D.int |> Maybe.withDefault 1
                    text = if times == 1 then data else List.concat (List.repeat times data)
                    packed = Codec.compress format text
                in
                if op == "compress" then Ok (Just packed)
                else if op == "roundtrip" && Codec.decompress format packed == Ok (Just text) then
                    Ok (Just [List.length packed, List.length text])
                else Err "unknown operation or roundtrip mismatch"

        result = case field "data" (D.list D.int) of
                     Just data -> execute data
                     Nothing -> Ok (Just [])
    in
    case result of
        Err msg -> E.object [("error", E.string msg)]
        Ok a -> E.object [("ok", Maybe.map (E.list E.int) a |> Maybe.withDefault E.null)]

main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \input _ -> ( (), response (run input) )
        , subscriptions = \_ -> request identity
        }
