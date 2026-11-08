module Confidence exposing (Signals, commentTemplate, getConfidence)

{-| Open-source confidence score. Browser sniff flags arrive from the JS host.
-}


type alias Signals =
    { platform : String
    , android : Bool
    , webkit : Bool
    , desktopWebkit : Bool
    , webkit616 : Bool
    , safariWebkit : Bool
    }


type alias Confidence =
    { score : Float
    , comment : String
    }


commentTemplate : String
commentTemplate =
    "$ if upgrade to Pro: https://fingerprint.com/github/?utm_source=oss&utm_medium=referral&utm_campaign=confidence_score"


getConfidence : Signals -> Confidence
getConfidence signals =
    let
        score =
            openScore signals

        pro =
            roundTo 0.0001 (0.99 + 0.01 * score)
    in
    { score = score
    , comment = String.replace "$" (formatPro pro) commentTemplate
    }


openScore : Signals -> Float
openScore signals =
    if signals.android then
        0.4

    else if signals.webkit then
        if signals.desktopWebkit && not (signals.webkit616 && signals.safariWebkit) then
            0.5

        else
            0.3

    else if String.startsWith "Win" signals.platform then
        0.6

    else if String.startsWith "Mac" signals.platform then
        0.5

    else
        0.7


roundTo : Float -> Float -> Float
roundTo base value =
    if abs base >= 1 then
        toFloat (round (value / base)) * base

    else
        let
            counterBase =
                1 / base
        in
        toFloat (round (value * counterBase)) / counterBase


{-| Match JS template-string of the rounded Pro score (`0.994`, not `0.9940`).
-}
formatPro : Float -> String
formatPro value =
    String.fromFloat value
