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
decimals before the run (17999.204999999998 becomes 17999.205), handling "e-" exponent strings by
rounding the mantissa. uPlot first returns members of its fixedDec map unchanged; the string path
returns every member unchanged too. String.fromFloat is the JS string.
-}
fixFloat : Float -> Float
fixFloat v =
    if isInt v then
        v

    else
        let
            str =
                String.fromFloat v
        in
        case decimalsBeforeRun str of
            Just dec ->
                case String.indexes "e-" str of
                    ePos :: _ ->
                        let
                            mant =
                                String.slice 0 ePos str |> String.toFloat |> Maybe.withDefault 0

                            exp =
                                String.dropLeft ePos str
                        in
                        String.fromFloat (roundDec dec mant) ++ exp |> String.toFloat |> Maybe.withDefault v

                    [] ->
                        roundDec dec v

            Nothing ->
                v


incrRound : Float -> Float -> Float
incrRound incr v =
    fixFloat (roundDec 0 (fixFloat (v / incr)) * incr)


incrRoundUp : Float -> Float -> Float
incrRoundUp incr v =
    fixFloat (toFloat (ceiling (fixFloat (v / incr))) * incr)


incrRoundDn : Float -> Float -> Float
incrRoundDn incr v =
    fixFloat (toFloat (floor (fixFloat (v / incr))) * incr)


fmin : Float -> Float -> Float
fmin a b =
    if isNaN a || isNaN b then
        0 / 0

    else if a < b then
        a

    else
        b


fmax : Float -> Float -> Float
fmax a b =
    if isNaN a || isNaN b then
        0 / 0

    else if b < a then
        a

    else
        b


{-| uPlot's rangeNum(min, max, 0.1, true): used by autoScaleX when all x values are equal.
All float comparisons use `<` or `<=` because Elm's `_Utils_cmp` returns GT for NaN.
-}
rangeNum : Float -> Float -> ( Float, Float )
rangeNum minV maxV =
    let
        rawDelta =
            maxV - minV

        scalarMax =
            fmax (abs minV) (abs maxV)

        magDiff =
            abs (logBase 10 scalarMax - logBase 10 rawDelta)

        delta =
            if rawDelta < 1e-24 || (not (isNaN magDiff) && 10 < magDiff) then
                if minV == 0 || maxV == 0 then
                    1e-24

                else
                    0

            else
                rawDelta

        nonZeroDelta =
            if delta /= 0 then
                delta

            else if scalarMax /= 0 then
                scalarMax

            else
                1000

        base =
            10 ^ toFloat (floor (logBase 10 nonZeroDelta))

        padMin =
            nonZeroDelta * (if delta == 0 then if minV == 0 then 0.1 else 1 else 0.1)

        newMin =
            roundDec 24 (incrRoundDn (base / 10) (minV - padMin))

        softMin =
            if not (isNaN newMin) && 0 <= minV && newMin <= 0 then
                0

            else
                1 / 0

        minLim =
            if not (isNaN newMin) && newMin < softMin && softMin <= minV then
                softMin

            else
                fmin softMin newMin

        padMax =
            nonZeroDelta * (if delta == 0 then if maxV == 0 then 0.1 else 1 else 0.1)

        newMax =
            roundDec 24 (incrRoundUp (base / 10) (maxV + padMax))

        softMax =
            if not (isNaN newMax) && maxV <= 0 && 0 <= newMax then
                0

            else
                -1 / 0

        maxLim0 =
            if not (isNaN newMax) && softMax < newMax && maxV <= softMax then
                softMax

            else
                fmax softMax newMax

        maxLim =
            if minLim == maxLim0 && minLim == 0 then
                100

            else
                maxLim0
    in
    ( minLim, maxLim )



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
                        if not (xv < prev.limit) then round (pixel x xv) else prev.column

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
                    -- Use `<=` rather than `>=`: Elm compiles `>=` to `_Utils_cmp > -1`, which is True on NaN,
                    -- whereas `<=` (`_Utils_cmp < 1`) is False on NaN, matching uPlot's `toPx >= frPx`.
                    ( current, False, if inGap && previous <= current then (previous, current) :: rev else rev )

        (_, _, result) =
            List.foldl step (0, False, []) points
    in
    List.reverse result


{-| clipGaps for ori 0: rectangles over everything but the gaps, from the plot's left edge.
-}
clipRects x y intervals =
    let
        step (left, right) (end, rev) =
            if not (left < right) then
                (end, rev)
            else
                (toFloat right, if 0 < toFloat left - end then [ end, y.off, toFloat left - end, y.off + y.dim ] :: rev else rev)

        (finalEnd, rects) =
            List.foldl step (x.off, []) intervals

        rest =
            x.off + x.dim - finalEnd
    in
    List.reverse (if 0 < rest then [ finalEnd, y.off - 5, rest, y.off + y.dim + 10 ] :: rects else rects)


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


type alias ScaleState =
    { x : Maybe ( Float, Float )
    , y : Maybe ( Float, Float )
    }


type alias ScaleResult =
    { state : ScaleState
    , xBounds : ( Float, Float )
    , yBounds : ( Float, Float )
    , firstVisible : Int
    , lastVisible : Int
    }


rangeAccepted : Float -> Float -> Bool
rangeAccepted lo hi =
    let
        d =
            hi - lo
    in
    isNaN d || not (d < 1e-16)


applyScales : ScaleState -> Array Float -> ( Float, Float ) -> ( Float, Float ) -> ScaleResult
applyScales state xs (( xMin, xMax ) as xReq) (( yMin, yMax ) as yReq) =
    let
        n =
            Array.length xs

        at i =
            Array.get i xs |> Maybe.withDefault 0

        pendX0 =
            if n <= 1 || rangeAccepted xMin xMax then
                True

            else
                let
                    a0 =
                        at 0

                    a1 =
                        at (n - 1)

                    ( r0, r1 ) =
                        if a0 == a1 then
                            rangeNum a0 a1

                        else
                            ( a0, a1 )

                    ( loA, hiA ) =
                        if not (isNaN r0) && not (isNaN r1) && r1 < r0 then
                            ( r1, r0 )

                        else
                            ( r0, r1 )
                in
                rangeAccepted loA hiA

        pendY0 =
            n <= 1 || rangeAccepted yMin yMax

        ( nextState, firstVisible, lastVisible ) =
            if pendX0 || pendY0 then
                let
                    pendX =
                        pendX0 || state.x == Nothing

                    pendY =
                        pendY0 || state.y == Nothing || pendX

                    nextX =
                        if pendX then
                            Just xReq

                        else
                            state.x

                    nextY =
                        if pendY then
                            Just yReq

                        else
                            state.y

                    ( loVis, hiVis ) =
                        if pendX && n > 0 then
                            let
                                last =
                                    n - 1

                                lo =
                                    nearest xs xMin 0 last

                                hi =
                                    nearest xs xMax 0 last
                            in
                            ( if hi > lo + 1 && at lo < xMin then lo + 1 else lo
                            , if hi > lo + 1 && xMax < at hi then hi - 1 else hi
                            )

                        else if n > 0 then
                            ( 0, n - 1 )

                        else
                            ( 0, 0 )
                in
                ( { x = nextX, y = nextY }, loVis, hiVis )

            else
                ( state, 0, if n > 0 then n - 1 else 0 )
    in
    { state = nextState
    , xBounds = Maybe.withDefault ( 0, 0 ) nextState.x
    , yBounds = Maybe.withDefault ( 0, 0 ) nextState.y
    , firstVisible = firstVisible
    , lastVisible = lastVisible
    }


frame ratio span f state =
    let
        -- The adapter passes CSS sizes; uPlot multiplies them back by pxRatio.
        ( w, h ) = Tuple.mapBoth (\d -> d / ratio) (\d -> d / ratio) f.size
        xs = Array.fromList f.xs
        last = Array.length xs - 1
        scaled =
            applyScales state xs f.xRange f.yRange
        nextState = scaled.state
        firstVisible = scaled.firstVisible
        lastVisible = scaled.lastVisible
        x = scale scaled.xBounds w ratio False
        y = scale scaled.yBounds h ratio True

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

        encoded =
            E.object
                [ ("canvas", E.list E.int [ round (w * ratio), round (h * ratio) ])
                , ("paint", if hasData then paint x y else E.null)
                , ("series", E.list (if hasData then series span x y f.xs else always empty) f.series)
                , ("probes", E.list probe f.probes)
                ]
    in
    ( nextState, encoded )


handle request_ =
    let
        runFrames ratio span frames =
            let
                step f ( st, rev ) =
                    let
                        ( nextSt, enc ) =
                            frame ratio span f st
                    in
                    ( nextSt, enc :: rev )

                ( _, revEncoded ) =
                    List.foldl step ( { x = Nothing, y = Nothing }, [] ) frames
            in
            E.list identity (List.reverse revEncoded)

        decoder =
            D.map3 runFrames
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
