module Arena exposing (Arena, Node, alter, blank, get, walk)

import Array exposing (Array)
import Json.Encode as E


{-| `id` is the id stratify gave the node, "" where d3 leaves `node.id` undefined; `label` is the
datum's id as the input has it (a string, null or undefined), which the output shows then.
-}
type alias Node =
    { id : String
    , label : E.Value
    , parent : Int
    , kids : List Int
    , depth : Int
    , height : Int
    , value : Float
    , x : Float
    , y : Float
    , r : Float
    , x0 : Float
    , y0 : Float
    , x1 : Float
    , y1 : Float
    , z : Float
    , m : Float
    , c : Float
    , s : Float
    , thread : Int
    , ancestor : Int
    , defaultAncestor : Int
    , number : Int
    }


type alias Arena =
    Array Node


blank : Node
blank =
    { id = ""
    , label = E.null
    , parent = -1
    , kids = []
    , depth = 0
    , height = 0
    , value = 0
    , x = 0
    , y = 0
    , r = 0
    , x0 = 0
    , y0 = 0
    , x1 = 0
    , y1 = 0
    , z = 0
    , m = 0
    , c = 0
    , s = 0
    , thread = -1
    , ancestor = -1
    , defaultAncestor = -1
    , number = 0
    }


get i ns =
    Maybe.withDefault blank (Array.get i ns)


alter i f ns =
    Array.set i (f (get i ns)) ns


walk post root ns =
    let
        loop pending acc =
            case pending of
                [] ->
                    if post then
                        acc

                    else
                        List.reverse acc

                i :: rest ->
                    let
                        kids =
                            (get i ns).kids
                    in
                    loop
                        ((if post then
                            List.reverse kids

                          else
                            kids
                         )
                            ++ rest
                        )
                        (i :: acc)
    in
    loop [ root ] []
