module Morton exposing (Scale, bounds, indexCurve, zOrder)

import Array
import Bitwise exposing (and, or, shiftLeftBy, shiftRightZfBy)
import Geometry exposing (point)
import Ring exposing (around, get, update)


type alias Scale =
    { x : Float, y : Float, factor : Float }


bounds : Geometry.Coords -> Int -> Int -> Scale
bounds coords dim outer =
    if Array.length coords <= 80 * dim then
        Scale 0 0 0

    else
        let
            loop i x y maxX maxY =
                if i == outer then
                    let
                        size =
                            max (maxX - x) (maxY - y)
                    in
                    Scale x
                        y
                        (if size == 0 then
                            0

                         else
                            32767 / size
                        )

                else
                    let
                        p =
                            point coords dim i
                    in
                    loop (i + 1) (min x p.x) (min y p.y) (max maxX p.x) (max maxY p.y)

            first =
                point coords dim 0
        in
        loop 1 first.x first.y first.x first.y


spread v =
    let
        mix bits mask x =
            and mask (or x (shiftLeftBy bits x))
    in
    v |> mix 8 0x00FF00FF |> mix 4 0x0F0F0F0F |> mix 2 0x33333333 |> mix 1 0x55555555


zOrder : Scale -> Float -> Float -> Int
zOrder s x y =
    or (spread (truncate ((x - s.x) * s.factor))) (shiftLeftBy 1 (spread (truncate ((y - s.y) * s.factor))))


insert entry sorted =
    case sorted of
        [] ->
            [ entry ]

        first :: rest ->
            if Tuple.first entry < Tuple.first first then
                entry :: sorted

            else
                first :: insert entry rest



-- Each bucket is built in reverse. Folding it onto the result restores encounter order.


radix : Int -> List ( Int, Int ) -> List ( Int, Int )
radix shift entries =
    let
        add (( z, _ ) as entry) buckets =
            let
                key =
                    and 255 (shiftRightZfBy shift z)

                bucket =
                    Maybe.withDefault [] (Array.get key buckets)
            in
            Array.set key (entry :: bucket) buckets

        filled =
            List.foldl add (Array.repeat 256 []) entries
    in
    Array.foldr (\bucket rest -> List.foldl (::) rest bucket) [] filled


indexCurve : Scale -> Int -> Ring.Arena -> Ring.Arena
indexCurve scale start arena =
    let
        entries =
            List.map (\p -> ( zOrder scale p.x p.y, p.id )) (around arena start)

        sorted =
            if List.length entries <= 32 then
                List.foldl insert [] entries

            else
                entries |> radix 0 |> radix 8 |> radix 16 |> radix 24

        link previous remaining a =
            case remaining of
                [] ->
                    a

                ( z, id ) :: rest ->
                    let
                        next =
                            Maybe.withDefault -1 (Maybe.map Tuple.second (List.head rest))

                        changed =
                            update id (\n -> { n | z = z, prevZ = previous, nextZ = next }) a
                    in
                    link id rest changed
    in
    link -1 sorted arena
