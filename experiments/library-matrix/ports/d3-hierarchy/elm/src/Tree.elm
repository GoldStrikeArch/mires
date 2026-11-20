module Tree exposing (cluster, tidy)

import Arena exposing (alter, get)


head xs =
    Maybe.withDefault -1 (List.head xs)


last xs =
    head (List.reverse xs)


separation ns a b =
    if (get a ns).parent == (get b ns).parent then
        1

    else
        2


cluster pre post w h fixed ns =
    let
        first i ( old, previous, x ) =
            let
                n =
                    get i old
            in
            case n.kids of
                [] ->
                    let
                        xx =
                            if previous < 0 then
                                0

                            else
                                x + separation old i previous
                    in
                    ( alter i (\v -> { v | x = xx, y = 0 }) old, i, xx )

                kids ->
                    let
                        xx =
                            List.foldl (\c t -> t + (get c old).x) 0 kids / toFloat (List.length kids)

                        yy =
                            1 + List.foldl (\c t -> max t (get c old).y) 0 kids
                    in
                    ( alter i (\v -> { v | x = xx, y = yy }) old, previous, x )

        ( stage, right, _ ) =
            List.foldl first ( ns, -1, 0 ) post

        left =
            head post

        x0 =
            (get left stage).x - separation stage left right / 2

        x1 =
            (get right stage).x + separation stage right left / 2

        root =
            get (head pre) stage

        second n =
            if fixed then
                { n | x = (n.x - root.x) * w, y = (root.y - n.y) * h }

            else
                { n
                    | x = (n.x - x0) / (x1 - x0) * w
                    , y =
                        (1
                            - (if root.y /= 0 then
                                n.y / root.y

                               else
                                1
                              )
                        )
                            * h
                }
    in
    List.foldl (\v -> alter v second) stage post


contour left ns i =
    let
        n =
            get i ns
    in
    if List.isEmpty n.kids then
        n.thread

    else if left then
        head n.kids

    else
        last n.kids


move wm v shift ns =
    let
        d =
            shift / toFloat ((get v ns).number - (get wm ns).number)
    in
    ns
        |> alter v (\n -> { n | c = n.c - d, s = n.s + shift, z = n.z + shift, m = n.m + shift })
        |> alter wm (\n -> { n | c = n.c + d })


apportion v prev ancestor ns =
    if prev < 0 then
        ( ns, ancestor )

    else
        let
            first =
                head (get (get v ns).parent ns).kids

            loop cur vim vip vom vop sim sip som sop =
                let
                    im =
                        contour False cur vim

                    ip =
                        contour True cur vip
                in
                if im < 0 || ip < 0 then
                    let
                        t1 =
                            if im >= 0 && contour False cur vop < 0 then
                                alter vop (\n -> { n | thread = im, m = n.m + (sim - sop) }) cur

                            else
                                cur
                    in
                    if ip >= 0 && contour True t1 vom < 0 then
                        ( alter vom (\n -> { n | thread = ip, m = n.m + (sip - som) }) t1, v )

                    else
                        ( t1, ancestor )

                else
                    let
                        om =
                            contour True cur vom

                        op =
                            contour False cur vop

                        modified =
                            alter op (\n -> { n | ancestor = v }) cur

                        shift =
                            (get im modified).z + sim - (get ip modified).z - sip + separation modified im ip

                        anc =
                            (get im modified).ancestor

                        wm =
                            if (get anc modified).parent == (get v modified).parent then
                                anc

                            else
                                ancestor

                        moved =
                            if shift > 0 then
                                move wm v shift modified

                            else
                                modified

                        delta =
                            if shift > 0 then
                                shift

                            else
                                0
                    in
                    loop moved im ip om op (sim + (get im moved).m) (sip + delta + (get ip moved).m) (som + (get om moved).m) (sop + delta + (get op moved).m)
        in
        loop ns prev v first v (get prev ns).m (get v ns).m (get first ns).m (get v ns).m


tidy pre post w h fixed ns =
    let
        initialize v old =
            let
                n =
                    get v old
            in
            List.indexedMap Tuple.pair n.kids
                |> List.foldl (\( j, c ) -> alter c (\child -> { child | number = j })) (alter v (\a -> { a | ancestor = v }) old)

        firstWalk v old =
            let
                n =
                    get v old

                siblings =
                    (get n.parent old).kids

                prev =
                    if n.number > 0 then
                        head (List.drop (n.number - 1) siblings)

                    else
                        -1

                shifts c ( arr, shift, change ) =
                    let
                        item =
                            get c arr
                    in
                    ( alter c (\a -> { a | z = a.z + shift, m = a.m + shift }) arr, shift + (item.s + (change + item.c)), change + item.c )

                ( shifted, _, _ ) =
                    List.foldr shifts ( old, 0, 0 ) n.kids

                mid =
                    ((get (head n.kids) shifted).z + (get (last n.kids) shifted).z) / 2

                pos =
                    if prev >= 0 then
                        (get prev shifted).z + separation shifted v prev

                    else
                        mid

                placed =
                    if List.isEmpty n.kids then
                        if prev >= 0 then
                            alter v (\a -> { a | z = pos }) old

                        else
                            old

                    else
                        alter v
                            (\a ->
                                { a
                                    | z = pos
                                    , m =
                                        if prev >= 0 then
                                            pos - mid

                                        else
                                            a.m
                                }
                            )
                            shifted

                initial =
                    (get n.parent placed).defaultAncestor
            in
            if n.parent < 0 then
                placed

            else
                let
                    ( result, anc ) =
                        apportion v
                            prev
                            (if initial < 0 then
                                head siblings

                             else
                                initial
                            )
                            placed
                in
                alter n.parent (\p -> { p | defaultAncestor = anc }) result

        stage1 =
            List.foldl firstWalk (List.foldl initialize ns pre) post

        root =
            head pre

        offset =
            -(get root stage1).z

        second v old =
            let
                n =
                    get v old

                modifier =
                    if n.parent < 0 then
                        offset

                    else
                        (get n.parent old).m
            in
            alter v (\a -> { a | x = a.z + modifier, m = a.m + modifier }) old

        stage2 =
            List.foldl second stage1 pre

        bounds v ( lo, hi, deep ) =
            let
                n =
                    get v stage2
            in
            ( if n.x < (get lo stage2).x then
                v

              else
                lo
            , if n.x > (get hi stage2).x then
                v

              else
                hi
            , max deep n.depth
            )

        ( left, right, depth ) =
            List.foldl bounds ( root, root, 0 ) pre

        margin =
            if left == right then
                1

            else
                separation stage2 left right / 2

        tx =
            margin - (get left stage2).x

        kx =
            w / ((get right stage2).x + margin + tx)

        ky =
            h / toFloat (max 1 depth)

        scale n =
            if fixed then
                { n | x = n.x * w, y = toFloat n.depth * h }

            else
                { n | x = (n.x + tx) * kx, y = toFloat n.depth * ky }
    in
    List.foldl (\i -> alter i scale) stage2 pre
