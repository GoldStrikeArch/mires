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

-- The Big constructor's conversion: a number goes through String(), except that -0 becomes "-0".
numberText f =
    if f == 0 && 1 / f < 0 then "-0" else String.fromFloat f

-- An absent, undefined or null option fails D.float, so dp, rm and repeat take their defaults as
-- the adapter's ?? does; n and mode stay Maybe and reach Decimal as upstream's optional arguments.
operation c =
    let
        text key = field c key (D.oneOf [D.string,D.map numberText D.float]) |> Maybe.withDefault ""
        number key = field c key D.float
        rm = number "rm" |> Maybe.withDefault 1
    in
    Decimal.operate (text "op") (text "x") (text "y") (number "n") (number "dp" |> Maybe.withDefault 20) rm (number "mode" |> Maybe.withDefault rm)

run input =
    let
        cases = field input "cases" (D.list D.value) |> Maybe.withDefault []
        repeats = field input "repeat" D.int |> Maybe.withDefault 1
    in
    List.range 1 repeats |> List.concatMap (\_ -> List.map operation cases) |> String.join "\n"
