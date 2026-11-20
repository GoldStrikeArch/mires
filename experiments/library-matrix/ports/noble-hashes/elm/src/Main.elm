port module Main exposing (main)

import Blake2
import Json.Decode as D
import Json.Encode as E
import Sha2
import Sha3

port request : (D.Value -> msg) -> Sub msg
port response : E.Value -> Cmd msg

main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \input _ -> ( (), response (run input) )
        , subscriptions = \_ -> request identity
        }

run : D.Value -> E.Value
run input =
    let
        field name decoder =
            D.decodeValue (D.field name decoder) input |> Result.toMaybe

        bytes = D.list D.int
        parts = field "chunks" (D.list bytes) |> Maybe.withDefault []
        chunks =
            case field "bytes" bytes of
                Just one -> [one]
                Nothing -> List.concat (List.repeat (field "repeat" D.int |> Maybe.withDefault 1) parts)

        algorithm = field "algorithm" D.string |> Maybe.withDefault ""
        key = field "key" bytes
        value = Maybe.withDefault [] key
        salt = field "salt" bytes
        personal = field "personalization" bytes
        wide = algorithm == "blake2b"
        limit = if wide then 64 else 32
        size = field "dkLen" D.float |> Maybe.withDefault (toFloat limit)
        dk = truncate size
        valid n a = Maybe.map (List.length >> (==) n) a |> Maybe.withDefault True

        result =
            case algorithm of
                "sha256" -> Ok (Sha2.digest 0 chunks value)
                "sha512" -> Ok (Sha2.digest 1 chunks value)
                "hmac_sha256" ->
                    case key of
                        Nothing -> Err "missing key"
                        Just k -> Ok (Sha2.digest 2 chunks k)
                "sha3_256" -> Ok (Sha3.digest False chunks)
                "keccak_256" -> Ok (Sha3.digest True chunks)
                _ ->
                    if algorithm /= "blake2s" && not wide then Err "unknown hash"
                    else if dk < 1 || dk > limit || toFloat dk /= size then Err "invalid digest size"
                    else if key /= Nothing && (List.isEmpty value || List.length value > limit) then Err "invalid key"
                    else if not (valid (limit // 4) salt && valid (limit // 4) personal) then Err "invalid parameter"
                    else Ok (Blake2.digest wide chunks dk value (Maybe.withDefault [] salt) (Maybe.withDefault [] personal))

        nibble n = Char.fromCode (if n < 10 then 48 + n else 87 + n)
        hex b = [nibble (b // 16), nibble (modBy 16 b)]
    in
    case result of
        Ok digest -> E.object [("ok", E.string (String.fromList (List.concatMap hex digest)))]
        Err reason -> E.object [("error", E.string reason)]
