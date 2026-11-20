port module Main exposing (main)

import Decimal
import Json.Decode as D

port request : (D.Value -> msg) -> Sub msg
port response : String -> Cmd msg

main : Program () () D.Value
main = Platform.worker
    { init = \_ -> ((),Cmd.none)
    , update = \input _ -> ((),response (run input))
    , subscriptions = \_ -> request identity
    }

field input key decoder =
    D.decodeValue (D.field key decoder) input |> Result.toMaybe

integer v =
    if toFloat (truncate v)==v then truncate v else -2000001

operation c =
    let
        text key = field c key (D.oneOf [D.string,D.map String.fromFloat D.float]) |> Maybe.withDefault ""
        number key = field c key D.float |> Maybe.map integer
        rm = number "rm" |> Maybe.withDefault 1
    in
    Decimal.operate (text "op") (text "x") (text "y") (number "n") (number "dp" |> Maybe.withDefault 20) rm (number "mode" |> Maybe.withDefault rm)

run input =
    let
        cases = field input "cases" (D.list D.value) |> Maybe.withDefault []
        repeats = field input "repeat" D.int |> Maybe.withDefault 1
    in
    List.range 1 repeats |> List.concatMap (\_ -> List.map operation cases) |> String.join "\n"
