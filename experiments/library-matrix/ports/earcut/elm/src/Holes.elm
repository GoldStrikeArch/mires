module Holes exposing (eliminate)

import Array
import Geometry exposing (area, equal, inTriangle)
import Ring exposing (Arena, Block, Node, around, expand, filterPoints, get, linkedList, locallyInside, split, update)


indexSegment : Int -> Int -> Arena -> Arena
indexSegment start stop state =
    let
        block head a =
            let
                n =
                    get a head

                index =
                    Array.length a.blocks

                edges id remaining box arena =
                    let
                        p =
                            get arena id

                        q =
                            get arena p.next

                        next =
                            update id (\old -> { old | z = index }) arena

                        bounds =
                            expand q box
                    in
                    if remaining == 1 || p.next == stop then
                        ( p.next, { next | blocks = Array.push { bounds | stop = p.next } next.blocks } )

                    else
                        edges p.next (remaining - 1) bounds next

                ( end, indexed ) =
                    edges head 16 (Block head head n.x n.y n.x n.y) a
            in
            if end == stop then
                indexed

            else
                block end indexed
    in
    block start state


live : Arena -> Int -> Int
live a id =
    let
        n =
            get a id
    in
    if (get a n.prev).next == id then
        id

    else
        live a n.next


type Search state
    = Continue state
    | Found Int



-- Block insertion order breaks bridge ties. Each walk advances past stale head and stop nodes.


scanBlocks : Arena -> (Block -> state -> Bool) -> (Node -> state -> Search state) -> state -> Search state
scanBlocks a skip visit initial =
    let
        walk id stop state =
            let
                p =
                    get a id

                result =
                    if (get a p.prev).next == id then
                        visit p state

                    else
                        Continue state
            in
            case result of
                Found _ ->
                    result

                Continue next ->
                    if p.next == stop then
                        result

                    else
                        walk p.next stop next

        blocks i state =
            case Array.get i a.blocks of
                Nothing ->
                    Continue state

                Just b ->
                    if skip b state then
                        blocks (i + 1) state

                    else
                        case walk (live a b.head) (live a b.stop) state of
                            Found id ->
                                Found id

                            Continue next ->
                                blocks (i + 1) next
    in
    blocks 0 initial


bridge : Node -> Int -> Arena -> Int
bridge hole outer a =
    let
        raySkip b ( qx, _ ) =
            hole.y < b.minY || hole.y > b.maxY || b.minX > hole.x || b.maxX <= qx

        ray p (( qx, _ ) as state) =
            let
                n =
                    get a p.next
            in
            if equal hole n then
                Found n.id

            else if hole.y <= p.y && hole.y >= n.y && n.y /= p.y then
                let
                    x =
                        p.x + (hole.y - p.y) * (n.x - p.x) / (n.y - p.y)
                in
                if x <= hole.x && x > qx then
                    let
                        id =
                            if p.x < n.x then
                                p.id

                            else
                                n.id
                    in
                    if x == hole.x then
                        Found id

                    else
                        Continue ( x, id )

                else
                    Continue state

            else
                Continue state

        candidates qx endpoint =
            let
                m =
                    get a endpoint

                low =
                    min m.y hole.y

                high =
                    max m.y hole.y

                v1 =
                    { x =
                        if hole.y < m.y then
                            hole.x

                        else
                            qx
                    , y = hole.y
                    }

                v3 =
                    { x =
                        if hole.y < m.y then
                            qx

                        else
                            hole.x
                    , y = hole.y
                    }

                skip b _ =
                    b.maxX < m.x || b.minX > hole.x || b.maxY < low || b.minY > high

                visit p (( bestTan, best ) as state) =
                    if hole.x > p.x && p.x >= m.x && inTriangle v1 m v3 p then
                        let
                            n =
                                get a p.next

                            angle =
                                abs (hole.y - p.y) / (hole.x - p.x)

                            betterTie =
                                p.x
                                    > best.x
                                    || (p.x
                                            == best.x
                                            && area (get a best.prev) best (get a p.prev)
                                            < 0
                                            && area n best (get a best.next)
                                            < 0
                                       )
                        in
                        if
                            (locallyInside a p hole || (p.y == hole.y && n.y == hole.y && n.x > hole.x))
                                && (angle < bestTan || (angle == bestTan && betterTie))
                        then
                            Continue ( angle, p )

                        else
                            Continue state

                    else
                        Continue state
            in
            case scanBlocks a skip visit ( 1 / 0, m ) of
                Continue ( _, best ) ->
                    best.id

                Found id ->
                    id
    in
    if equal hole (get a outer) then
        outer

    else
        case scanBlocks a raySkip ray ( -1 / 0, -1 ) of
            Found id ->
                id

            Continue ( qx, id ) ->
                if id < 0 then
                    -1

                else
                    candidates qx id


merge : Int -> ( Int, Arena ) -> ( Int, Arena )
merge hole ( outer, a ) =
    let
        id =
            bridge (get a hole) outer a
    in
    if id < 0 then
        ( outer, a )

    else
        let
            ( reverseBridge, spliced ) =
                split id hole a

            bridge2 =
                (get spliced reverseBridge).next

            indexed =
                indexSegment id (get spliced bridge2).next spliced

            ( _, healed, _ ) =
                filterPoints reverseBridge bridge2 indexed

            ( root, done, _ ) =
                filterPoints id (get healed id).next healed
        in
        ( root, done )


eliminate : Geometry.Coords -> Int -> List Int -> Int -> Arena -> ( Int, Arena )
eliminate coords dim holes outer arena =
    let
        lowest p best =
            if p.x < best.x || (p.x == best.x && p.y < best.y) then
                p

            else
                best

        rings remaining reverseQueue a =
            case remaining of
                [] ->
                    ( List.reverse reverseQueue, a )

                h :: rest ->
                    let
                        end =
                            Maybe.withDefault (Array.length coords // dim) (List.head rest)

                        ( last, linked ) =
                            linkedList coords dim h end False a

                        marked =
                            if (get linked last).next == last then
                                update last (\p -> { p | steiner = True }) linked

                            else
                                linked

                        left =
                            List.foldl lowest (get marked last) (around marked last)
                    in
                    rings rest (left.id :: reverseQueue) marked

        ( queue, built ) =
            rings holes [] arena

        slope p =
            let
                n =
                    get built p.next
            in
            (n.y - p.y) / (n.x - p.x)

        compareHoles aid bid =
            let
                p =
                    get built aid

                q =
                    get built bid

                angle =
                    slope p - slope q
            in
            if p.x /= q.x then
                compare p.x q.x

            else if p.y /= q.y then
                compare p.y q.y

            else if angle < 0 then
                LT

            else if angle > 0 then
                GT

            else
                EQ

        indexed =
            indexSegment outer outer built

        ( start, merged ) =
            List.foldl merge ( outer, { indexed | indexing = True } ) (List.sortWith compareHoles queue)

        ( root, healed, _ ) =
            filterPoints start start { merged | indexing = False }
    in
    ( root, healed )
