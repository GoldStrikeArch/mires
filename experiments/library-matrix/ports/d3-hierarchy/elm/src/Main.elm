port module Main exposing (main)

import Arena exposing (Arena, alter, blank, get, walk)
import Array
import Circles
import Dict
import Json.Decode as D
import Json.Encode as E
import JsMath exposing (truthy)
import Platform
import Rectangles
import Tree


port request : (D.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg


{-| A stratify row. d3 reads every field with `!= null` or `??`, so a null field is absent too;
`label` is the row's id as the input has it.
-}
type alias Row =
    { id : String, parentId : String, value : Float, label : D.Value }


type alias Input =
    { op : String, source : String, data : Maybe D.Value, rows : List Row, w : Float, h : Float, p : Float, sort : Bool, nodeSize : Bool, round : Bool, tile : String }


opt k fieldDecoder fallback =
    D.oneOf [ D.field k fieldDecoder, D.succeed fallback ]


{-| The id field as the input has it: a string, null, or `absent` (the host's `undefined`).
-}
rawId : D.Value -> D.Decoder D.Value
rawId absent =
    D.oneOf [ D.field "id" D.value, D.succeed absent ]


rowDecoder : D.Value -> D.Decoder Row
rowDecoder absent =
    D.map4 Row
        (opt "id" D.string "")
        (opt "parentId" D.string "")
        (opt "value" D.float 0)
        (rawId absent)


{-| The request: the input, and JavaScript's `undefined` from the host. The nested data stays a raw
value: `nested` reads it one node at a time, so no decoder recurses through the nesting.
-}
requestDecoder : D.Decoder ( Input, D.Value )
requestDecoder =
    D.field "undefined" D.value
        |> D.andThen (\absent -> D.map (\input -> ( input, absent )) (D.field "input" (decoder absent)))


decoder absent =
    D.map2
        (\( op, source, dataRows ) cfg ->
            let
                ( data, rows ) =
                    dataRows
            in
            { op = op, source = source, data = data, rows = rows, w = cfg.w, h = cfg.h, p = cfg.p, sort = cfg.sort, nodeSize = cfg.fixed, round = cfg.round, tile = cfg.tile }
        )
        (D.map4 (\op src d rows -> ( op, src, ( d, rows ) ))
            (D.field "op" D.string)
            (D.field "source" D.string)
            (D.maybe (D.field "data" D.value))
            (opt "rows" (D.list (rowDecoder absent)) [])
        )
        (D.map7 (\w h p sort fixed round tile -> { w = w, h = h, p = p, sort = sort, fixed = fixed, round = round, tile = tile })
            (D.field "width" D.float)
            (D.field "height" D.float)
            (D.field "padding" D.float)
            (D.field "sort" D.bool)
            (D.field "nodeSize" D.bool)
            (D.field "round" D.bool)
            (D.field "tile" D.string)
        )


{-| One node of the nested data: its id as the input has it, its value and its children, still
raw.
-}
datum : D.Value -> D.Decoder ( D.Value, Float, List D.Value )
datum absent =
    D.map3 (\label weight kids -> ( label, weight, kids ))
        (rawId absent)
        (opt "value" D.float 0)
        (opt "children" (D.list D.value) [])


{-| `hierarchy(data)`, built on an explicit stack as d3 builds it: the arena holds the nodes in
pre-order, each with its children in order.
-}
nested : D.Value -> D.Value -> Result String ( Arena, Int )
nested absent data =
    let
        build pending arena =
            case pending of
                [] ->
                    Ok ( Array.map (\n -> { n | kids = List.reverse n.kids }) arena, 0 )

                ( value, parent ) :: rest ->
                    case D.decodeValue (datum absent) value of
                        Err err ->
                            Err (D.errorToString err)

                        Ok ( label, weight, kids ) ->
                            let
                                i =
                                    Array.length arena

                                added =
                                    Array.push { blank | label = label, parent = parent, value = weight } arena
                            in
                            build (List.map (\k -> ( k, i )) kids ++ rest)
                                (if parent < 0 then
                                    added

                                 else
                                    alter parent (\n -> { n | kids = i :: n.kids }) added
                                )
    in
    build [ ( data, -1 ) ] Array.empty


slash path i =
    let
        char j =
            String.slice j (j + 1) path

        count j =
            if j > 0 && char (j - 1) == "\\" then
                count (j - 1)

            else
                j
    in
    i >= 0 && i < String.length path && char i == "/" && modBy 2 (i - count i) == 0


normalize path =
    let
        n =
            String.length path

        p =
            if slash path (n - 1) && not (slash path (n - 2)) then
                String.dropRight 1 path

            else
                path
    in
    if String.startsWith "/" p then
        p

    else
        "/" ++ p


parentPath path =
    let
        loop i =
            if i <= 1 || slash path i then
                String.left i path

            else
                loop (i - 1)
    in
    if String.length path < 2 then
        ""

    else
        loop (String.length path - 1)


stratify paths rows =
    let
        make r ( ns, byId, pids ) =
            let
                i =
                    Array.length ns

                id =
                    if paths then
                        normalize r.id

                    else
                        r.id

                p =
                    if paths then
                        parentPath id

                    else
                        r.parentId

                ids =
                    if id == "" then
                        byId

                    else
                        Dict.insert id
                            (if Dict.member id byId then
                                -1

                             else
                                i
                            )
                            byId
            in
            ( Array.push { blank | id = id, label = r.label, value = r.value } ns, ids, Array.push p pids )

        ( given, map0, pids0 ) =
            List.foldl make ( Array.empty, Dict.empty, Array.empty ) rows

        impute j ns ids pids =
            if not paths || j >= Array.length pids then
                ( ns, ids, pids )

            else
                let
                    p =
                        Array.get j pids |> Maybe.withDefault ""
                in
                if p /= "" && not (Dict.member p ids) then
                    impute (j + 1) (Array.push { blank | id = p } ns) (Dict.insert p (Array.length ns) ids) (Array.push (parentPath p) pids)

                else
                    impute (j + 1) ns ids pids

        ( all, mapping, parents ) =
            impute 0 given map0 pids0

        link i ns root =
            if i >= Array.length ns then
                if root < 0 then
                    Err "no root"

                else
                    Ok ( Array.map (\n -> { n | kids = List.reverse n.kids }) ns, root )

            else
                let
                    key =
                        Maybe.withDefault "" (Array.get i parents)
                in
                if key == "" then
                    if root >= 0 then
                        Err "multiple roots"

                    else
                        link (i + 1) ns i

                else
                    let
                        p =
                            Dict.get key mapping |> Maybe.withDefault -1
                    in
                    if p < 0 then
                        Err "missing or ambiguous parent"

                    else
                        link (i + 1) (ns |> alter i (\n -> { n | parent = p }) |> alter p (\n -> { n | kids = i :: n.kids })) root

        trim ns root n =
            case (get root ns).kids of
                [ c ] ->
                    if paths && root >= Array.length given then
                        trim ns c (n - 1)

                    else
                        ( root, n )

                _ ->
                    ( root, n )

        finish ( ns, r ) =
            let
                ( root, expected ) =
                    trim ns r (Array.length all)

                arr =
                    alter root (\n -> { n | parent = -1 }) ns
            in
            if List.length (walk False root arr) == expected then
                Ok ( arr, root )

            else
                Err "cycle"
    in
    link 0 all -1 |> Result.andThen finish


layout input ( initial, root ) =
    let
        sum i ns =
            let
                n =
                    get i ns

                add c ( total, high ) =
                    let
                        child =
                            get c ns
                    in
                    ( total + child.value, max high (child.height + 1) )

                ( value, height ) =
                    -- `+value(node.data) || 0`, then the children's values, the last child first
                    List.foldr add
                        ( if truthy n.value then
                            n.value

                          else
                            0
                        , 0
                        )
                        n.kids

                -- d3's comparator `b.value - a.value`: a NaN difference is a tie, as
                -- Array.prototype.sort counts it as 0 (`compare` would answer GT).
                order a b =
                    let
                        difference =
                            (get b ns).value - (get a ns).value
                    in
                    if difference < 0 then
                        LT

                    else if 0 < difference then
                        GT

                    else
                        EQ

                kids =
                    if input.sort then
                        List.sortWith order n.kids

                    else
                        n.kids
            in
            alter i (\x -> { x | value = value, height = height, kids = kids }) ns

        sums =
            List.foldl sum initial (walk True root initial)

        pre =
            walk False root sums

        post =
            walk True root sums

        depth i ns =
            let
                n =
                    get i ns
            in
            if n.parent < 0 then
                ns

            else
                alter i (\x -> { x | depth = (get n.parent ns).depth + 1 }) ns

        ready =
            List.foldl depth sums pre

        w =
            input.w

        h =
            input.h

        prepared =
            case input.op of
                "tree" ->
                    Ok (Tree.tidy pre post w h input.nodeSize ready)

                "cluster" ->
                    Ok (Tree.cluster pre post w h input.nodeSize ready)

                "pack" ->
                    Circles.layout pre post root w h input.p ready

                "treemap" ->
                    Ok (Rectangles.layout pre root w h input.p False input.tile input.round ready)

                "partition" ->
                    Ok (Rectangles.layout pre root w h input.p True input.tile input.round ready)

                "stratify" ->
                    Ok ready

                "hierarchy" ->
                    Ok ready

                "iterator" ->
                    Ok ready

                _ ->
                    Err "unknown operation"

        emit ns =
            let
                -- The reference's `n.id ?? n.data.id`.
                name n =
                    if n.id == "" then
                        n.label

                    else
                        E.string n.id

                breadth current acc =
                    if List.isEmpty current then
                        List.reverse acc

                    else
                        breadth (List.concatMap (\i -> (get i ns).kids) current) (List.reverse current ++ acc)

                order =
                    if input.op == "iterator" then
                        breadth [ root ] []

                    else
                        pre

                row i =
                    let
                        n =
                            get i ns

                        p =
                            if n.parent < 0 then
                                E.null

                            else
                                name (get n.parent ns)

                        coords =
                            case input.op of
                                "tree" ->
                                    [ n.x, n.y ]

                                "cluster" ->
                                    [ n.x, n.y ]

                                "pack" ->
                                    [ n.x, n.y, n.r ]

                                "treemap" ->
                                    [ n.x0, n.y0, n.x1, n.y1 ]

                                "partition" ->
                                    [ n.x0, n.y0, n.x1, n.y1 ]

                                _ ->
                                    []
                    in
                    E.list identity ([ name n, p, E.int n.depth, E.int n.height, E.float n.value ] ++ List.map E.float coords)
            in
            E.list row order
    in
    Result.map emit prepared


handle value =
    let
        run ( input, absent ) =
            let
                tree =
                    case input.data of
                        Just data ->
                            nested absent data

                        Nothing ->
                            stratify (input.source == "path") input.rows
            in
            Result.andThen (layout input) tree
    in
    case D.decodeValue requestDecoder value |> Result.mapError D.errorToString |> Result.andThen run of
        Ok output ->
            E.object [ ( "ok", output ) ]

        Err err ->
            E.object [ ( "error", E.string err ) ]


main : Program () () D.Value
main =
    Platform.worker { init = \_ -> ( (), Cmd.none ), update = \v m -> ( m, response (handle v) ), subscriptions = \_ -> request identity }
