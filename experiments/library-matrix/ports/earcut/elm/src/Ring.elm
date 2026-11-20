module Ring exposing (Arena, Block, Node, addTriangle, around, blank, expand, filterPoints, get, linkedList, locallyInside, node, remove, split, update)

import Array exposing (Array)
import Geometry exposing (area, equal, point, signedArea)


type alias Node =
    { id : Int
    , i : Int
    , x : Float
    , y : Float
    , prev : Int
    , next : Int
    , z : Int
    , prevZ : Int
    , nextZ : Int
    , steiner : Bool
    }


type alias Block =
    { head : Int, stop : Int, minX : Float, minY : Float, maxX : Float, maxY : Float }


type alias Arena =
    { nodes : Array Node, blocks : Array Block, indexing : Bool, triangles : List Int }


blank : Arena
blank =
    { nodes = Array.empty, blocks = Array.empty, indexing = False, triangles = [] }


node : Int -> Int -> Float -> Float -> Node
node id i x y =
    Node id i x y id id 0 -1 -1 False


get : Arena -> Int -> Node
get a id =
    case Array.get id a.nodes of
        Just n ->
            n

        Nothing ->
            node -1 -1 0 0


update : Int -> (Node -> Node) -> Arena -> Arena
update id f a =
    { a | nodes = Array.set id (f (get a id)) a.nodes }


link : Int -> Int -> Arena -> Arena
link from to a =
    a |> update from (\n -> { n | next = to }) |> update to (\n -> { n | prev = from })


append : Int -> Float -> Float -> Arena -> ( Int, Arena )
append i x y a =
    let
        id =
            Array.length a.nodes
    in
    ( id, { a | nodes = Array.push (node id i x y) a.nodes } )


linkedList : Geometry.Coords -> Int -> Int -> Int -> Bool -> Arena -> ( Int, Arena )
linkedList coords dim start end clockwise arena =
    let
        forward =
            clockwise == (signedArea coords dim start end > 0)

        step =
            if forward then
                1

            else
                -1

        loop left i last a =
            if left <= 0 then
                if last >= 0 then
                    let
                        n =
                            get a last
                    in
                    if equal n (get a n.next) then
                        ( n.next, remove n a )

                    else
                        ( last, a )

                else
                    ( last, a )

            else
                let
                    p =
                        point coords dim i

                    ( id, created ) =
                        append i p.x p.y a

                    ring =
                        if last < 0 then
                            created

                        else
                            created |> link id (get a last).next |> link last id
                in
                loop (left - 1) (i + step) id ring
    in
    loop (end - start)
        (if forward then
            start

         else
            end - 1
        )
        -1
        arena


expand : Node -> Block -> Block
expand p b =
    { b | minX = min b.minX p.x, minY = min b.minY p.y, maxX = max b.maxX p.x, maxY = max b.maxY p.y }



-- Keep the removed node in the arena. Block walks follow its old links to the next live node.


remove : Node -> Arena -> Arena
remove n a =
    let
        changed =
            link n.prev n.next a

        before =
            if n.prevZ >= 0 then
                update n.prevZ (\p -> { p | nextZ = n.nextZ }) changed

            else
                changed

        after =
            if n.nextZ >= 0 then
                update n.nextZ (\p -> { p | prevZ = n.prevZ }) before

            else
                before
    in
    if a.indexing then
        let
            index =
                (get a n.prev).z
        in
        case Array.get index a.blocks of
            Just b ->
                { after | blocks = Array.set index (expand (get a n.next) b) a.blocks }

            Nothing ->
                after

    else
        after


split : Int -> Int -> Arena -> ( Int, Arena )
split ai bi a =
    let
        p =
            get a ai

        q =
            get a bi

        ( a2, withA ) =
            append p.i p.x p.y a

        ( b2, withB ) =
            append q.i q.x q.y withA
    in
    ( b2, withB |> link ai bi |> link a2 p.next |> link b2 a2 |> link q.prev b2 )



-- A whole-ring sweep runs to a fixed point. A bridge or diagonal cut heals only its dirty window.


filterPoints : Int -> Int -> Arena -> ( Int, Arena, Bool )
filterPoints start bound a =
    let
        full =
            start == bound

        loop id end removed state =
            let
                p =
                    get state id

                n =
                    get state p.next

                before =
                    get state p.prev
            in
            if id /= p.next && not p.steiner && (equal p n || area before p n == 0) then
                loop p.prev
                    (if full || id == end then
                        p.prev

                     else
                        end
                    )
                    True
                    (remove p state)

            else if full && p.next /= end then
                loop p.next end removed state

            else if not full && id /= end then
                loop p.next end removed state

            else
                ( end, state, removed )
    in
    loop start bound False a


around : Arena -> Int -> List Node
around a start =
    let
        loop id rev =
            let
                n =
                    get a id
            in
            if n.next == start then
                List.reverse (n :: rev)

            else
                loop n.next (n :: rev)
    in
    loop start []


locallyInside : Arena -> Node -> Node -> Bool
locallyInside arena a b =
    let
        p =
            get arena a.prev

        n =
            get arena a.next
    in
    if area p a n < 0 then
        area a b n >= 0 && area a p b >= 0

    else
        area a b p < 0 || area a n b < 0


addTriangle : Node -> Node -> Node -> Arena -> Arena
addTriangle a b c state =
    { state | triangles = c.i :: b.i :: a.i :: state.triangles }
