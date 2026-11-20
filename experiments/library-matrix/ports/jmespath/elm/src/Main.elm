port module Main exposing (main)

import Eval exposing (Step(..))
import Json.Decode as D
import Json.Encode as E
import Parser
import Platform
import Values exposing (V(..))


port request : (D.Value -> msg) -> Sub msg


port comparison : (Int -> msg) -> Sub msg


port identityResult : (Bool -> msg) -> Sub msg


port response : E.Value -> Cmd msg


type Msg
    = Begin D.Value
    | Compared Int
    | Same Bool


type Model
    = Idle
    | Waiting (Order -> Step V)
    | Equality (Bool -> Step V)


type alias Query =
    { op : String, expression : String }


decoder =
    D.map2 Tuple.pair
        (D.field "data" Values.decode)
        (D.field "queries" (D.list (D.map2 Query (D.field "op" D.string) (D.field "expression" D.string))))


collect data qs acc =
    case qs of
        [] ->
            Done (Arr (List.reverse acc))

        q :: tail ->
            let
                forward step =
                    case step of
                        Done v ->
                            collect data tail (Obj [ ( "value", v ) ] :: acc)

                        Error category ->
                            collect data tail (Obj [ ( "error", Str category ) ] :: acc)

                        Collate a b k ->
                            Collate a b (k >> forward)

                        RefEq a b k ->
                            RefEq a b (k >> forward)
            in
            case Parser.compile q.expression of
                Err e ->
                    forward (Error e)

                Ok ast ->
                    forward
                        (if q.op == "compile" then
                            Done ast

                         else
                            Eval.evaluate ast data
                        )


deliver step =
    case step of
        Done v ->
            ( Idle, response (E.object [ ( "value", Values.wire v ) ]) )

        Error e ->
            ( Idle, response (E.object [ ( "error", E.string e ) ]) )

        Collate a b k ->
            ( Waiting k, response (E.object [ ( "collate", E.list E.string [ a, b ] ) ]) )

        RefEq a b k ->
            ( Equality k, response (E.object [ ( "identity", E.list identity [ a, b ] ) ]) )


update msg model =
    case ( msg, model ) of
        ( Begin input, _ ) ->
            case D.decodeValue decoder input of
                Ok ( data, qs ) ->
                    deliver (collect data qs [])

                Err e ->
                    deliver (Error (D.errorToString e))

        ( Compared i, Waiting resume ) ->
            deliver
                (resume
                    (if i < 0 then
                        LT

                     else if i > 0 then
                        GT

                     else
                        EQ
                    )
                )

        ( Same equal, Equality resume ) ->
            deliver (resume equal)

        _ ->
            ( model, Cmd.none )


main : Program () Model Msg
main =
    Platform.worker { init = \_ -> ( Idle, Cmd.none ), update = update, subscriptions = \_ -> Sub.batch [ request Begin, comparison Compared, identityResult Same ] }
