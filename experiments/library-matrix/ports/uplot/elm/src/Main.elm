port module Main exposing (main)

{-| uPlot 1.6.32's linear-series slice, as the reference chart runs it: calcSize's plot box with
incrRound, getHPos/getVPos and posToVal, closestIdx and setScales' window, paths.linear with its
gaps, and drawPath's settings. The x scale has ori 0 and the y scale ori 1, both dir 1; the series
has pxAlign 1, alignGaps 0, width 1, no fill, no bands and no points.

The JS host sends the input with devicePixelRatio and applies what comes back: the canvas size,
each frame's stroke settings, and each series' vertices and clip rectangles.

-}

import Array exposing (Array)
import Json.Decode as D
import Json.Encode as E
import Platform


port request : (D.Value -> msg) -> Sub msg


port response : E.Value -> Cmd msg



-- uPlot's rounding helpers


{-| Number.EPSILON, 2^-52.
-}
epsilon : Float
epsilon =
    2.220446049250313e-16


{-| Number.isInteger.
-}
isInt : Float -> Bool
isInt v =
    not (isNaN v) && not (isInfinite v) && toFloat (floor v) == v


{-| roundDec: rounds half away from zero at `dec` decimals.
-}
roundDec : Int -> Float -> Float
roundDec dec v =
    if isInt v then
        v

    else
        let
            p =
                10 ^ toFloat dec
        in
        toFloat (round (v * p * (1 + epsilon))) / p


leadingDigits : String -> String
leadingDigits text =
    case String.uncons text of
        Just ( c, rest ) ->
            if Char.isDigit c then
                String.cons c (leadingDigits rest)

            else
                ""

        Nothing ->
            ""


{-| uPlot's regex6, /\\.\\d\*?(?=9{6,}|0{6,})/: the number of digits between the point and the
first run of six or more 9s or 0s, if there is one.
-}
decimalsBeforeRun : String -> Maybe Int
decimalsBeforeRun text =
    case String.split "." text of
        [ _, fraction ] ->
            let
                digits =
                    leadingDigits fraction

                find k =
                    if k + 6 > String.length digits then
                        Nothing

                    else if List.member (String.slice k (k + 6) digits) [ "999999", "000000" ] then
                        Just k

                    else
                        find (k + 1)
            in
            find 0

        _ ->
            Nothing


{-| fixFloat: snaps a value whose JS string has a run of six or more 9s or 0s after the point to the
decimals before the run (17999.204999999998 becomes 17999.205). uPlot first returns members of its
fixedDec map unchanged; the string path returns every member unchanged too. Its branch for strings
with "e-" (values below 1e-6) is not reached by plot sizes of 1 px or more. String.fromFloat is
the JS string.
-}
fixFloat : Float -> Float
fixFloat v =
    if isInt v then
        v

    else
        case decimalsBeforeRun (String.fromFloat v) of
            Just dec ->
                roundDec dec v

            Nothing ->
                v


incrRound : Float -> Float -> Float
incrRound incr v =
    fixFloat (roundDec 0 (fixFloat (v / incr)) * incr)



-- Scales


{-| One axis of the plot: the scale range, calcSize's offset and size in canvas pixels, and the CSS
size uPlot was given (the adapter divides the physical size by pxRatio).
-}
type alias Scale =
    { low : Float, high : Float, off : Float, dim : Float, css : Float, ratio : Float, vertical : Bool }


scale : ( Float, Float ) -> Float -> Float -> Bool -> Scale
scale ( low, high ) css ratio vertical =
    { low = low
    , high = high
    , off = incrRound 0.5 (0 * ratio)
    , dim = incrRound 0.5 (css * ratio)
    , css = css
    , ratio = ratio
    , vertical = vertical
    }


{-| getHPos / getVPos (dir 1) in canvas pixels, keeping uPlot's `off +`, which turns -0 into 0.
-}
pixel s v =
    let
        pct =
            (v - s.low) / (s.high - s.low)
    in
    s.off + s.dim * (if s.vertical then 1 - pct else pct)


{-| posToVal(p, key, true): the CSS position (the plot's CSS offset is 0), from the bottom for y.
-}
valueAt s p =
    let
        pos =
            p / s.ratio

        q =
            if s.vertical then s.css - pos else pos
    in
    s.low + (s.high - s.low) * (q / s.css)


{-| closestIdx: JS `hi || arr.length - 1` makes a zero `hi` the last index; an empty array gives -1.
-}
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



-- paths.linear


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


{-| clipGaps for ori 0: rectangles over everything but the gaps, from the plot's left edge.
-}
clipRects x y intervals =
    let
        step (left, right) (end, rev) =
            if right <= left then
                (end, rev)
            else
                (toFloat right, if toFloat left - end > 0 then [ end, y.off, toFloat left - end, y.off + y.dim ] :: rev else rev)

        (finalEnd, rects) =
            List.foldl step (x.off, []) intervals

        rest =
            x.off + x.dim - finalEnd
    in
    List.reverse (if rest > 0 then [ finalEnd, y.off - 5, rest, y.off + y.dim + 10 ] :: rects else rects)


pair ( a, b ) =
    E.list E.int [ a, b ]


series span x y xs ys =
    let
        points =
            List.map2 Tuple.pair xs ys |> trimMissing |> List.reverse |> trimMissing |> List.reverse

        intervals =
            if span then [] else gaps x points

        clip =
            if List.isEmpty intervals then E.null
            else E.list (E.list E.float) (clipRects x y intervals)
    in
    E.object
        [ ("geometry", E.list identity [ E.list pair (vertices x y points), if span then E.null else E.list pair intervals ])
        , ("clip", clip)
        ]


{-| drawPath's settings for this frame: the series width 1 in canvas pixels, the half-pixel
translate (only when it is not 0), setCtxStyle's values and the bounds clip.
-}
paint x y =
    let
        width =
            roundDec 3 x.ratio

        offset =
            (width - 2 * toFloat (truncate (width / 2))) / 2
    in
    E.object
        [ ("translate", if offset > 0 then E.float offset else E.null)
        , ("strokeStyle", E.string "#172839")
        , ("lineWidth", E.float width)
        , ("lineJoin", E.string "round")
        , ("lineCap", E.string "butt")
        , ("bounds", E.list E.float [ x.off - width / 2, y.off - width / 2, x.dim + width, y.dim + width ])
        , ("stroke", E.bool (width > 0))
        ]



-- The boundary


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


frame ratio span f =
    let
        -- The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
        ( w, h ) = Tuple.mapBoth (\d -> d / ratio) (\d -> d / ratio) f.size
        x = scale f.xRange w ratio False
        y = scale f.yRange h ratio True
        xs = Array.fromList f.xs
        last = Array.length xs - 1
        lo = nearest xs x.low 0 last
        hi = nearest xs x.high 0 last
        at i = Array.get i xs |> Maybe.withDefault 0
        firstVisible = if hi > lo + 1 && at lo < x.low then lo + 1 else lo
        lastVisible = if hi > lo + 1 && at hi > x.high then hi - 1 else hi

        -- The adapter's precise(): Math.round(x * 1e6) || 0, which maps -0 and NaN to 0.
        precise v =
            let
                n = toFloat (round (v * 1000000))
            in
            if n == 0 || isNaN n then 0 else n

        probe q =
            case q of
                [ xv, yv, px, py ] ->
                    E.list identity
                        [ E.float (precise (pixel x xv)), E.float (precise (pixel y yv))
                        , E.float (precise (valueAt x px)), E.float (precise (valueAt y py))
                        , E.int (nearest xs xv 0 last), E.int (nearest xs (valueAt x px) firstVisible lastVisible) ]
                _ -> E.null

        -- drawSeries runs only with data; the adapter then reports an empty trace for each series.
        hasData = not (List.isEmpty f.xs)
        empty = E.object [ ("geometry", E.list identity [ E.list identity [], if span then E.null else E.list identity [] ]), ("clip", E.null) ]
    in
    E.object
        [ ("canvas", E.list E.int [ round (w * ratio), round (h * ratio) ])
        , ("paint", if hasData then paint x y else E.null)
        , ("series", E.list (if hasData then series span x y f.xs else always empty) f.series)
        , ("probes", E.list probe f.probes)
        ]


handle request_ =
    let
        decoder =
            D.map3 (\ratio span frames -> E.list (frame ratio span) frames)
                (D.field "pxRatio" D.float)
                (D.at [ "input", "spanGaps" ] D.bool)
                (D.at [ "input", "frames" ] (D.list frameDecoder))
    in
    case D.decodeValue decoder request_ of
        Ok frames -> E.object [ ("ok", frames) ]
        Err error -> E.object [ ("error", E.string (D.errorToString error)) ]


main : Program () () D.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \v m -> ( m, response (handle v) )
        , subscriptions = \_ -> request identity
        }
