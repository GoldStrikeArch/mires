module Geometry exposing (Coords, Point, area, coord, deviation, equal, inTriangle, intersects, point, signedArea)

import Array exposing (Array)


type alias Coords =
    Array Float


type alias Point =
    { x : Float, y : Float }


coord : Int -> Coords -> Float
coord i coords =
    Maybe.withDefault 0 (Array.get i coords)


point : Coords -> Int -> Int -> Point
point coords dim i =
    { x = coord (dim * i) coords, y = coord (dim * i + 1) coords }


equal p q =
    p.x == q.x && p.y == q.y


area p q r =
    (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)


inTriangle a b c p =
    (c.x - p.x)
        * (a.y - p.y)
        >= (a.x - p.x)
        * (c.y - p.y)
        && (a.x - p.x)
        * (b.y - p.y)
        >= (b.x - p.x)
        * (a.y - p.y)
        && (b.x - p.x)
        * (c.y - p.y)
        >= (c.x - p.x)
        * (b.y - p.y)


onSegment p q r =
    q.x <= max p.x r.x && q.x >= min p.x r.x && q.y <= max p.y r.y && q.y >= min p.y r.y


intersects boundary a b c d =
    let
        o1 =
            area a b c

        o2 =
            area a b d

        o3 =
            area c d a

        o4 =
            area c d b

        opposite x y =
            (x < 0 && y > 0) || (x > 0 && y < 0)
    in
    (opposite o1 o2 && opposite o3 o4)
        || (boundary
                && ((o1 == 0 && onSegment a c b)
                        || (o2 == 0 && onSegment a d b)
                        || (o3 == 0 && onSegment c a d)
                        || (o4 == 0 && onSegment c b d)
                   )
           )


signedArea : Coords -> Int -> Int -> Int -> Float
signedArea coords dim start end =
    let
        loop i j sum =
            if i >= end then
                sum

            else
                loop (i + 1)
                    i
                    (sum
                        + (coord (j * dim) coords - coord (i * dim) coords)
                        * (coord (i * dim + 1) coords + coord (j * dim + 1) coords)
                    )
    in
    loop start (end - 1) 0


deviation : Coords -> List Int -> Int -> List Int -> Float
deviation coords holes dim tris =
    let
        n =
            Array.length coords // dim

        outer =
            Maybe.withDefault n (List.head holes)

        subtract hs a =
            case hs of
                [] ->
                    a

                h :: rest ->
                    subtract rest (a - abs (signedArea coords dim h (Maybe.withDefault n (List.head rest))))

        polygonArea =
            subtract holes (abs (signedArea coords dim 0 outer))

        triangleArea ids total =
            case ids of
                ai :: bi :: ci :: rest ->
                    let
                        a =
                            point coords dim ai

                        b =
                            point coords dim bi

                        c =
                            point coords dim ci
                    in
                    triangleArea rest (total + abs ((a.x - c.x) * (b.y - a.y) - (a.x - b.x) * (c.y - a.y)))

                _ ->
                    total

        sum =
            triangleArea tris 0

        maxCoord i highest =
            if i >= n then
                highest

            else
                let
                    p =
                        point coords dim i
                in
                maxCoord (i + 1) (max highest (max (abs p.x) (abs p.y)))
    in
    if sum == 0 then
        let
            m =
                maxCoord 0 0
        in
        if abs polygonArea <= toFloat (Array.length coords) * m * m * 2.220446049250313e-16 then
            0

        else
            1

    else
        abs ((sum - polygonArea) / polygonArea)
