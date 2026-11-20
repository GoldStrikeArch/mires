port module Main exposing (main)

import Array exposing (Array)
import Json.Decode as D
import Json.Encode as E
import Platform


port request : (D.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg


type alias Scale =
    { low : Float, high : Float, dim : Float, requested : Float, vertical : Bool }


pixel scale v =
    let
        pct =
            (v - scale.low) / (scale.high - scale.low)
    in
    scale.dim * (if scale.vertical then 1 - pct else pct)


valueAt scale p =
    let
        q =
            if scale.vertical then scale.requested - p else p
    in
    scale.low + (scale.high - scale.low) * (q / scale.requested)


nearest xs v lower upper =
    let
        x i =
            Array.get i xs |> Maybe.withDefault 0

        search lo hi =
            if hi <= lo + 1 then
                if v - x lo <= x hi - v then lo else hi
            else
                let
                    mid = (lo + hi) // 2
                in
                if x mid < v then search mid hi else search lo mid
    in
    if Array.isEmpty xs then -1 else search lower (if upper == 0 then Array.length xs - 1 else upper)


type alias Bucket =
    { first : Float, last : Float, low : Float, high : Float }


flush y column bucket rev =
    case bucket of
        Nothing ->
            rev

        Just b ->
            let
                ordinate v =
                    round (pixel y v)

                low = ordinate b.low
                high = ordinate b.high
                first = ordinate b.first
                last = ordinate b.last

                add v points =
                    if v /= first && v /= last then (column, v) :: points else points
            in
            if low == high then rev else (column, last) :: add high (add low rev)


vertices x y points =
    let
        coordinate ( xv, yv ) =
            Maybe.map (\v -> (round (pixel x xv), round (pixel y v))) yv
    in
    if toFloat (List.length points - 1) < x.dim * 4 then
        List.filterMap coordinate points
    else
        let
            start =
                List.head points |> Maybe.map (Tuple.first >> pixel x >> round) |> Maybe.withDefault 0

            step ( xv, yv ) prev =
                let
                    next =
                        if xv >= prev.limit then round (pixel x xv) else prev.column

                    state =
                        if next == prev.column then prev
                        else
                            { column = next, limit = valueAt x (toFloat next + 1), bucket = Nothing
                            , rev = flush y prev.column prev.bucket prev.rev }
                in
                case ( yv, state.bucket ) of
                    ( Nothing, _ ) ->
                        state

                    ( Just v, Nothing ) ->
                        { state | bucket = Just { first = v, last = v, low = v, high = v }
                        , rev = ( state.column, round (pixel y v) ) :: state.rev }

                    ( Just v, Just b ) ->
                        { state | bucket = Just { b | last = v, low = min b.low v, high = max b.high v } }

            final =
                List.foldl step { column = start, limit = valueAt x (toFloat start + 1), bucket = Nothing, rev = [] } points
        in
        List.reverse (flush y final.column final.bucket final.rev)


trimMissing points =
    case points of
        (_, Nothing) :: rest -> trimMissing rest
        _ -> points


gaps x points =
    let
        step ( xv, yv ) ( previous, inGap, rev ) =
            case yv of
                Nothing ->
                    ( previous, True, rev )

                Just _ ->
                    let
                        current = round (pixel x xv)
                    in
                    ( current, False, if inGap && current >= previous then (previous, current) :: rev else rev )

        (_, _, result) =
            List.foldl step (0, False, []) points
    in
    List.reverse result


clipRects w h intervals =
    let
        step (left, right) (end, rev) =
            if right <= left then
                (end, rev)
            else
                (right, if left > end then [ toFloat end, 0, toFloat (left - end), h ] :: rev else rev)

        (finalEnd, rects) =
            List.foldl step (0, []) intervals

        endPos = toFloat finalEnd
    in
    List.reverse (if w > endPos then [ endPos, -5, w - endPos, h + 10 ] :: rects else rects)


pair ( a, b ) =
    E.list E.int [ a, b ]


series span x y xs ys =
    let
        points =
            List.map2 Tuple.pair xs ys |> trimMissing |> List.reverse |> trimMissing |> List.reverse

        intervals =
            if span then [] else gaps x points

        geometry =
            E.list identity
                [ E.list pair (vertices x y points)
                , if span then E.null else E.list pair intervals
                ]

        mask =
            if List.isEmpty intervals then E.null
            else E.list (E.list E.float) (clipRects x.dim y.dim intervals)
    in
    E.object [ ("geometry", geometry), ("mask", mask) ]


type alias Frame =
    { size : ( Float, Float ), xRange : ( Float, Float ), yRange : ( Float, Float )
    , xs : List Float, series : List (List (Maybe Float)), probes : List (List Float) }


frameDecoder =
    let
        range =
            D.map2 Tuple.pair (D.index 0 D.float) (D.index 1 D.float)

        columns =
            D.list (D.list (D.nullable D.float)) |> D.map (List.drop 1)
    in
    D.map6 Frame (D.field "size" range) (D.field "xRange" range) (D.field "yRange" range)
        (D.field "data" (D.index 0 (D.list D.float)))
        (D.field "data" columns) (D.field "probes" (D.list (D.list D.float)))


frame span f =
    let
        ( w, h ) = f.size
        ( xMin, xMax ) = f.xRange
        ( yMin, yMax ) = f.yRange
        plotDim d = toFloat (round (d * 2)) / 2
        x = Scale xMin xMax (plotDim w) w False
        y = Scale yMin yMax (plotDim h) h True
        xs = Array.fromList f.xs
        last = Array.length xs - 1
        lo = nearest xs x.low 0 last
        hi = nearest xs x.high 0 last
        at i = Array.get i xs |> Maybe.withDefault 0
        firstVisible = if hi > lo + 1 && at lo < x.low then lo + 1 else lo
        lastVisible = if hi > lo + 1 && at hi > x.high then hi - 1 else hi

        precise v =
            let
                n = round (v * 1000000)
            in
            if n == 0 then 0 else n

        probe q =
            case q of
                [ xv, yv, px, py ] ->
                    E.list E.int
                        [ precise (pixel x xv), precise (pixel y yv)
                        , precise (valueAt x px), precise (valueAt y py)
                        , nearest xs xv 0 last, nearest xs (valueAt x px) firstVisible lastVisible ]
                _ -> E.null
    in
    E.object
        [ ("size", E.list E.float [ w, h ])
        , ("bounds", E.list E.float [ x.dim, y.dim ])
        , ("series", E.list (series span x y f.xs) f.series)
        , ("probes", E.list probe f.probes)
        ]


handle input =
    case D.decodeValue (D.map2 Tuple.pair (D.field "spanGaps" D.bool) (D.field "frames" (D.list frameDecoder))) input of
        Ok (span, frames) -> E.object [ ("ok", E.list (frame span) frames) ]
        Err error -> E.object [ ("error", E.string (D.errorToString error)) ]


main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \v m -> ( m, response (handle v) )
        , subscriptions = \_ -> request identity
        }
