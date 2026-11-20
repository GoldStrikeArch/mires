port module Main exposing (main)

import Array exposing (Array)
import Earcut
import Geometry
import Json.Decode as D
import Json.Encode as E
import Refine


port request : (D.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg


main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \raw _ -> ( (), response (handle raw) )
        , subscriptions = \_ -> request identity
        }


type alias Polygon =
    { data : Array Float, holes : List Int, dim : Int, refine : Bool }


default field decoder value =
    D.oneOf [ D.field field decoder, D.succeed value ]


dimDecoder =
    default "dim" D.int 2


polygon : D.Decoder Polygon
polygon =
    D.map4 Polygon
        (D.field "data" (D.array D.float))
        (default "holeIndices" (D.list D.int) [])
        dimDecoder
        (default "refine" D.bool False)


triangles : D.Decoder (List Int)
triangles =
    D.field "triangles" (D.list D.int)


triangulate : Bool -> Polygon -> E.Value
triangulate refine p =
    let
        indices =
            Earcut.triangulate p.data p.holes p.dim
    in
    E.list E.int
        (if refine then
            Refine.refine p.data p.dim indices

         else
            indices
        )


flatten : List (List (List Float)) -> D.Decoder E.Value
flatten rings =
    case List.head rings |> Maybe.andThen List.head of
        Nothing ->
            D.fail "flatten: empty rings"

        Just first ->
            let
                dimensions =
                    List.length first

                fold ring ( prev, count, rev ) =
                    let
                        nextCount =
                            count + prev
                    in
                    ( List.length ring
                    , nextCount
                    , if prev == 0 then
                        rev

                      else
                        nextCount :: rev
                    )

                ( _, _, holes ) =
                    List.foldl fold ( 0, 0, [] ) rings

                encodePoint p =
                    List.map E.float (List.take dimensions p) ++ List.repeat (max 0 (dimensions - List.length p)) E.null

                vertices =
                    List.concatMap (List.concatMap encodePoint) rings
            in
            D.succeed (E.object [ ( "vertices", E.list identity vertices ), ( "holes", E.list E.int (List.reverse holes) ), ( "dimensions", E.int dimensions ) ])


operation : String -> D.Decoder E.Value
operation op =
    case op of
        "earcut" ->
            D.map (triangulate False) polygon

        "earcutRefine" ->
            D.map (triangulate True) polygon

        "batch" ->
            D.field "items" (D.list polygon) |> D.map (E.list (\p -> triangulate p.refine p))

        "refine" ->
            D.map3 (\coords dim ids -> E.list E.int (Refine.refine coords dim ids))
                (D.field "coords" (D.array D.float))
                dimDecoder
                triangles

        "deviation" ->
            D.map2 (\p ids -> E.float (Geometry.deviation p.data p.holes p.dim ids)) polygon triangles

        "flatten" ->
            D.field "rings" (D.list (D.list (D.list D.float))) |> D.andThen flatten

        _ ->
            D.fail ("unknown operation " ++ op)


handle : D.Value -> E.Value
handle input =
    case D.decodeValue (D.field "op" D.string |> D.andThen operation) input of
        Ok value ->
            E.object [ ( "ok", value ) ]

        Err error ->
            E.object [ ( "error", E.string (D.errorToString error) ) ]
