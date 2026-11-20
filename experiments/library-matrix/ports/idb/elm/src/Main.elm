port module Main exposing (main)

import Json.Decode as D
import Json.Encode as E

type Phase = Opening | Writing | Reading | Finished
type alias Model = { input:D.Value, remaining:List D.Value, transactions:List D.Value, phase:Phase }
type Msg = Start | Reply D.Value

port start : (() -> msg) -> Sub msg
port reply : (D.Value -> msg) -> Sub msg
port effect : E.Value -> Cmd msg
port output : E.Value -> Cmd msg

command op data = effect (E.object [("op",E.string op),("data",data)])

next model =
    case model.remaining of
        batch :: rest -> ({model | phase=Writing, remaining=rest},command "write" batch)
        [] -> ({model | phase=Reading},command "read" model.input)

update msg model =
    case (msg,model.phase) of
        (Start,Opening) -> (model,command "open" E.null)
        (Reply _,Opening) -> next model
        (Reply value,Writing) -> next {model | transactions=value :: model.transactions}
        (Reply value,Reading) ->
            let
                results = D.decodeValue (D.list D.value) value |> Result.withDefault []
                batches = E.list identity (List.reverse model.transactions)
            in
            ({model | phase=Finished},output (E.list identity (batches::results)))
        _ -> (model,Cmd.none)

main : Program D.Value Model Msg
main = Platform.worker
    { init = \input ->
        let batches=D.decodeValue (D.field "batches" (D.list D.value)) input |> Result.withDefault []
        in ({input=input,remaining=batches,transactions=[],phase=Opening},Cmd.none)
    , update=update
    , subscriptions=\_ -> Sub.batch [start (\_ -> Start),reply Reply]
    }
