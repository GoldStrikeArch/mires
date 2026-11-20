port module Main exposing (main)

import Dict
import Json.Decode as D
import Json.Encode as E

port request : (D.Value -> msg) -> Sub msg
port response : E.Value -> Cmd msg

main : Program () () D.Value
main = Platform.worker
    { init=\_ -> ((),Cmd.none)
    , update=\input _ -> ((),response (route input))
    , subscriptions=\_ -> request identity
    }

field v key decoder =
    D.decodeValue (D.field key decoder) v |> Result.toMaybe

route input =
    let
        method = field input "method" D.string |> Maybe.withDefault ""
        path = field input "path" D.string |> Maybe.withDefault ""
        pairs = D.map2 Tuple.pair (D.index 0 D.string) (D.index 1 D.string)
        entries = field input "entries" (D.list pairs) |> Maybe.withDefault [] |> Dict.fromList
        candidates = field input "candidates" (D.list D.string) |> Maybe.withDefault []
        exact = candidates |> List.filterMap (\key->Dict.get key entries) |> List.head
    in
    if method/="GET" then E.null
    else case exact of
        Just key -> E.object [("kind",E.string "precache"),("key",E.string key)]
        Nothing ->
            if String.startsWith "network/" path then E.object [("kind",E.string "network")]
            else if String.startsWith "first/" path then E.object [("kind",E.string "first")]
            else if String.startsWith "only/" path then E.object [("kind",E.string "only")]
            else E.null
