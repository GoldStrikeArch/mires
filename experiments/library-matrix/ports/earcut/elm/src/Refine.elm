module Refine exposing (refine)

import Array exposing (Array)
import Bitwise exposing (and)
import Geometry exposing (point)


at i a =
    Maybe.withDefault -1 (Array.get i a)


next e =
    e - modBy 3 e + modBy 3 (e + 1)


prev e =
    e - modBy 3 e + modBy 3 (e + 2)


orient a b c =
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)


inCircle a b c p =
    let
        dx =
            a.x - p.x

        dy =
            a.y - p.y

        ex =
            b.x - p.x

        ey =
            b.y - p.y

        fx =
            c.x - p.x

        fy =
            c.y - p.y

        ap =
            dx * dx + dy * dy

        bp =
            ex * ex + ey * ey

        cp =
            fx * fx + fy * fy

        s =
            ap + bp + cp
    in
    dx * (ey * cp - bp * fy) - dy * (ex * cp - bp * fx) + ap * (ex * fy - ey * fx) <= 1.0e-13 * s * s


type alias Mesh =
    { tri : Array Int, twin : Array Int, stack : List Int, queued : Array Bool }



-- A slot of -1 is unused and -2 is consumed. Later pairs of a non-manifold edge may probe past consumed slots.


seed : Array Int -> Mesh
seed tri =
    let
        n =
            Array.length tri

        power capacity =
            if capacity < n * 4 then
                power (capacity * 2)

            else
                capacity

        size =
            power 1

        mask =
            size - 1

        -- Half-word products retain the low 32 bits even when full JS multiplication rounds.
        mul a b =
            (and a 65535 * and b 65535)
                + Bitwise.shiftLeftBy 16 (Bitwise.shiftRightZfBy 16 a * and b 65535 + and a 65535 * Bitwise.shiftRightZfBy 16 b)

        loop e table mesh =
            if e == n then
                mesh

            else
                let
                    a =
                        at e tri

                    b =
                        at (next e) tri

                    initial =
                        and mask (Bitwise.xor (mul (min a b) 0x9E3779B1) (mul (max a b) 0x85EBCA6B))

                    probe candidate =
                        let
                            prior =
                                at candidate table
                        in
                        if prior == -1 then
                            ( candidate, -1 )

                        else if prior >= 0 && ((at prior tri == a && at (next prior) tri == b) || (at prior tri == b && at (next prior) tri == a)) then
                            ( candidate, prior )

                        else
                            probe (and mask (candidate + 1))

                    ( slot, s ) =
                        probe initial
                in
                if s < 0 then
                    loop (e + 1) (Array.set slot e table) mesh

                else
                    loop (e + 1)
                        (Array.set slot -2 table)
                        { mesh
                            | twin = mesh.twin |> Array.set s e |> Array.set e s
                            , stack = s :: mesh.stack
                            , queued = Array.set s True mesh.queued
                        }
    in
    loop 0 (Array.repeat size -1) (Mesh tri (Array.repeat n -1) [] (Array.repeat n False))


legalize : Geometry.Coords -> Int -> Mesh -> Array Int
legalize coords dim mesh =
    case mesh.stack of
        [] ->
            mesh.tri

        a :: rest ->
            let
                active =
                    { mesh | stack = rest, queued = Array.set a False mesh.queued }

                b =
                    at a mesh.twin
            in
            if b < 0 then
                legalize coords dim active

            else
                let
                    ar =
                        prev a

                    al =
                        next a

                    bl =
                        prev b

                    br =
                        next b

                    i0 =
                        at ar mesh.tri

                    i1 =
                        at bl mesh.tri

                    p0 =
                        point coords dim i0

                    p1 =
                        point coords dim i1

                    pr =
                        point coords dim (at a mesh.tri)

                    pl =
                        point coords dim (at al mesh.tri)
                in
                if not (inCircle p0 pr pl p1) && orient p0 pr p1 > 0 && orient p0 p1 pl > 0 then
                    let
                        hbl =
                            at bl mesh.twin

                        har =
                            at ar mesh.twin

                        relink from to twin =
                            let
                                changed =
                                    Array.set from to twin
                            in
                            if to >= 0 then
                                Array.set to from changed

                            else
                                changed

                        twins =
                            mesh.twin |> relink a hbl |> relink b har |> relink ar bl

                        enqueue id m =
                            if at id twins >= 0 && Array.get id m.queued == Just False then
                                { m | queued = Array.set id True m.queued, stack = id :: m.stack }

                            else
                                m

                        flipped =
                            { active | twin = twins, tri = mesh.tri |> Array.set a i1 |> Array.set b i0 }
                    in
                    legalize coords dim (flipped |> enqueue a |> enqueue b |> enqueue al |> enqueue br)

                else
                    legalize coords dim active


refine : Geometry.Coords -> Int -> List Int -> List Int
refine coords dim triangles =
    if List.length triangles < 6 then
        triangles

    else
        triangles |> Array.fromList |> seed |> legalize coords dim |> Array.toList
