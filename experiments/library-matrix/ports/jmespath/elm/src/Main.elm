port module Main exposing (main)

{-| The Elm side of the port: what libraries/jmespath/reference.mjs does, on the port's jmespath.

The host sends the input as `{ data, queries }`, with the document as postfix instructions
(Values.Op). Each query compiles or searches and gives `{ value }` or `{ error }`; the result array
goes back as postfix instructions too. An evaluation that needs `localeCompare` or `===` from the
host sends that request instead and waits in the model; the answer resumes it.

-}

import Eval exposing (Step(..))
import Json.Decode as D
import Json.Encode as E
import Parser
import Values exposing (V(..))


port request : (D.Value -> msg) -> Sub msg


port collated : (Int -> msg) -> Sub msg


port identical : (Bool -> msg) -> Sub msg


port response : E.Value -> Cmd msg


type Msg
    = Begin D.Value
    | GotCollation Int
    | GotIdentity Bool


type Model
    = Idle
    | Collating (Order -> Step V)
    | ComparingIdentity (Bool -> Step V)


type alias Query =
    { op : String, expression : String }


inputDecoder : D.Decoder ( V, List Query )
inputDecoder =
    D.map2 Tuple.pair
        (D.field "data" (D.list Values.opDecoder) |> D.map Values.fromInput)
        (D.field "queries" (D.list (D.map2 Query (D.field "op" D.string) (D.field "expression" D.string))))


result : String -> V -> V
result key value =
    Obj (Values.fresh ()) [ ( key, value ) ]


{-| The queries in order, each as `{ value }` or `{ error }`; one failing query does not stop the
rest.
-}
runQueries : V -> List Query -> List V -> Step V
runQueries data queries done =
    case queries of
        [] ->
            Done (Arr (Values.fresh ()) (List.reverse done))

        query :: rest ->
            let
                outcome =
                    case Parser.compile query.expression of
                        Err () ->
                            Failed "syntax"

                        Ok ast ->
                            if query.op == "compile" then
                                Done ast

                            else
                                Eval.evaluate ast data

                continue answer =
                    runQueries data rest (answer :: done)
            in
            case outcome of
                Done value ->
                    continue (result "value" value)

                Failed category ->
                    continue (result "error" (Str category))

                suspended ->
                    recover suspended |> Eval.andThen continue


{-| A suspended query's own outcome, its failure included, once the host has answered.
-}
recover : Step V -> Step V
recover step =
    case step of
        Done value ->
            Done (result "value" value)

        Failed category ->
            Done (result "error" (Str category))

        Collate a b resume ->
            Collate a b (resume >> recover)

        Identical target candidates resume ->
            Identical target candidates (resume >> recover)


deliver : Step V -> ( Model, Cmd Msg )
deliver step =
    case step of
        Done value ->
            ( Idle, response (E.object [ ( "value", Values.toOutput value ) ]) )

        Failed category ->
            ( Idle, response (E.object [ ( "error", E.string category ) ]) )

        Collate a b resume ->
            ( Collating resume, response (E.object [ ( "collate", E.list E.string [ a, b ] ) ]) )

        Identical target candidates resume ->
            ( ComparingIdentity resume, response (E.object [ ( "identical", E.list identity (target :: candidates) ) ]) )


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case ( msg, model ) of
        ( Begin input, _ ) ->
            case D.decodeValue inputDecoder input of
                Ok ( data, queries ) ->
                    deliver (runQueries data queries [])

                Err e ->
                    deliver (Failed (D.errorToString e))

        ( GotCollation sign, Collating resume ) ->
            deliver (resume (compare sign 0))

        ( GotIdentity same, ComparingIdentity resume ) ->
            deliver (resume same)

        _ ->
            ( model, Cmd.none )


main : Program () Model Msg
main =
    Platform.worker
        { init = \_ -> ( Idle, Cmd.none )
        , update = update
        , subscriptions = \_ -> Sub.batch [ request Begin, collated GotCollation, identical GotIdentity ]
        }
