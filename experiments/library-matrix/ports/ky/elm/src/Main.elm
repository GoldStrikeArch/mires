port module Main exposing (main)

import Json.Decode as D
import Json.Encode as E

port retry : (D.Value -> msg) -> Sub msg
port decision : E.Value -> Cmd msg

main : Program () () D.Value
main = Platform.worker
    { init=\_ -> ((),Cmd.none)
    , update=\input _ -> ((),decision (plan input))
    , subscriptions=\_ -> retry identity
    }

field v key decoder =
    D.decodeValue (D.field key decoder) v |> Result.toMaybe

canonicalMethod method =
    if List.member (String.toLower method) ["get","post","put","patch","head","delete","query","options","trace"] then String.toUpper method else method

plan input =
    let
        config = field input "config" D.value |> Maybe.withDefault E.null
        count = field input "count" D.int |> Maybe.withDefault 0
        limit = field config "limit" D.int |> Maybe.withDefault 2
        method = field input "method" D.string |> Maybe.withDefault "GET" |> canonicalMethod
        name = field input "name" D.string |> Maybe.withDefault ""
        status = field input "status" D.int |> Maybe.withDefault 0
        header = field input "after" D.string |> Maybe.withDefault "" |> String.trim
        methods = field config "methods" (D.list D.string) |> Maybe.withDefault ["get","put","head","delete","options","trace","query"]
        statuses = field config "statusCodes" (D.list D.int) |> Maybe.withDefault [408,413,429,500,502,503,504]
        eligible =
            case name of
                "HTTPError" -> List.member status statuses && (status/=413 || header/="")
                "NetworkError" -> True
                "TimeoutError" -> field config "retryOnTimeout" D.bool |> Maybe.withDefault False
                _ -> False
        supplied =
            if name=="HTTPError" && List.member status [413,429,503] && header/="" && String.all Char.isDigit header then
                String.toFloat header |> Maybe.map ((*) 1000)
            else Nothing
        backoff = 300 * (2 ^ count)
        limitAfter = field config "maxRetryAfter" D.float |> Maybe.withDefault (1/0)
        limitBackoff = field config "backoffLimit" D.int |> Maybe.withDefault backoff
    in
    if count>=limit || not (List.member method (List.map canonicalMethod methods)) || not eligible then E.null
    else
        case supplied of
            Just ms -> E.object [("ms",E.float (min ms limitAfter)),("backoff",E.null)]
            Nothing -> E.object [("ms",E.int (min backoff limitBackoff)),("backoff",E.int backoff)]
