module JsMath exposing (jsMax, jsMin, truthy)

{-| JavaScript number semantics that d3 relies on and elm/core does not give.

Elm compiles `a > b` and `a >= b` between two non-literal Floats to `_Utils_cmp(a, b) > 0` and
`>= 0`, and `_Utils_cmp` answers "greater" when an operand is NaN, so both are true where
JavaScript's are false. `<` and `<=` stay false for NaN, so the layouts write d3's `a > b` as
`b < a`. `max` and `min` compare the same way, and pick between zeros by argument order.

-}


{-| JavaScript's ToBoolean of a number: false for 0, -0 and NaN. d3 tests numbers this way
(`value ? … : …`, `if (d2)`, `+x || 0`).
-}
truthy : Float -> Bool
truthy x =
    not (x == 0 || isNaN x)


negativeZero : Float -> Bool
negativeZero x =
    x == 0 && 1 / x < 0


{-| `Math.max(a, b)`: NaN if either is NaN, and +0 above -0.
-}
jsMax : Float -> Float -> Float
jsMax a b =
    if isNaN a then
        a

    else if isNaN b then
        b

    else if a == b then
        if negativeZero a then
            b

        else
            a

    else if a < b then
        b

    else
        a


{-| `Math.min(a, b)`: NaN if either is NaN, and -0 below +0.
-}
jsMin : Float -> Float -> Float
jsMin a b =
    if isNaN a then
        a

    else if isNaN b then
        b

    else if a == b then
        if negativeZero a then
            a

        else
            b

    else if a < b then
        a

    else
        b
