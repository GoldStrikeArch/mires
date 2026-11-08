module MathSource exposing (Fingerprint, Native, fingerprint)

{-| Math entropy source. Elm owns the polyfills and the result shape. Native
`Math.acosh` / `asinh` / … values (and `Math.exp` samples the polyfills need)
arrive from the JS host because Elm 0.19 has no those functions.
-}


type alias Native =
    { acos : Float
    , acosh : Float
    , asin : Float
    , asinh : Float
    , atanh : Float
    , atan : Float
    , sin : Float
    , sinh : Float
    , cos : Float
    , cosh : Float
    , tan : Float
    , tanh : Float
    , exp : Float
    , expm1 : Float
    , log1p : Float
    , exp1 : Float
    , exp2 : Float
    }


type alias Fingerprint =
    { acos : Float
    , acosh : Float
    , acoshPf : Float
    , asin : Float
    , asinh : Float
    , asinhPf : Float
    , atanh : Float
    , atanhPf : Float
    , atan : Float
    , sin : Float
    , sinh : Float
    , sinhPf : Float
    , cos : Float
    , cosh : Float
    , coshPf : Float
    , tan : Float
    , tanh : Float
    , tanhPf : Float
    , exp : Float
    , expm1 : Float
    , expm1Pf : Float
    , log1p : Float
    , log1pPf : Float
    , powPI : Float
    }


fingerprint : Native -> Fingerprint
fingerprint native =
    { acos = native.acos
    , acosh = native.acosh
    , acoshPf = acoshPf 1.0e154
    , asin = native.asin
    , asinh = native.asinh
    , asinhPf = asinhPf 1
    , atanh = native.atanh
    , atanhPf = atanhPf 0.5
    , atan = native.atan
    , sin = native.sin
    , sinh = native.sinh
    , sinhPf = native.exp1 - 1 / native.exp1 / 2
    , cos = native.cos
    , cosh = native.cosh
    , coshPf = (native.exp1 + 1 / native.exp1) / 2
    , tan = native.tan
    , tanh = native.tanh
    , tanhPf = (native.exp2 - 1) / (native.exp2 + 1)
    , exp = native.exp
    , expm1 = native.expm1
    , expm1Pf = native.exp1 - 1
    , log1p = native.log1p
    , log1pPf = log1pPf 10
    , powPI = pi ^ -100
    }


acoshPf : Float -> Float
acoshPf value =
    log (value + sqrt (value * value - 1))


asinhPf : Float -> Float
asinhPf value =
    log (value + sqrt (value * value + 1))


atanhPf : Float -> Float
atanhPf value =
    log ((1 + value) / (1 - value)) / 2


log1pPf : Float -> Float
log1pPf value =
    log (1 + value)


log : Float -> Float
log value =
    logBase e value
