port module Main exposing (main)

import Bitwise as B
import Json.Decode as D
import Json.Encode as E
import Platform
import Words as W exposing (Word)

port request : (D.Value -> msg) -> Sub msg
port response : E.Value -> Cmd msg

type alias Command =
    { op : String, s : String, a : Word, b : Word, u : Bool, v : Bool, n : Int, r : Int }

word = D.map2 Word (D.index 0 D.int) (D.index 1 D.int)

commands =
    D.field "cases" (D.list (D.map8 Command
        (D.field "op" D.string) (D.field "s" D.string) (D.field "a" word) (D.field "b" word)
        (D.field "u" D.bool) (D.field "v" D.bool) (D.field "n" D.int) (D.field "r" D.int)))

mix c factor i acc checksum =
    if i >= c.n then W.format acc c.u c.r ++ ":" ++ String.fromInt checksum else
    let
        x = W.bitXor c.b (W.rotl 11 (W.add (W.mul acc factor) (Word (i+1) 0)))
        y = W.add x (W.divide x (Word (3+B.and i 255) 0) c.u)
        next =
            if B.and i 7 /= 0 then checksum else
            let
                s = W.format y c.u c.r
                first = String.uncons s |> Maybe.map (Tuple.first >> Char.toCode) |> Maybe.withDefault 0
            in
            B.or 0 (checksum*33+String.length s+first)
    in
    mix c factor (i+1) y next

evaluate c =
    let
        a = c.a
        b = c.b
        out w = W.format w c.u c.r
    in
    case c.op of
        "fromBits" -> out a
        "toString" -> out a
        "fromString" -> W.parse c.s c.n |> Maybe.map out |> Maybe.withDefault "!"
        "add" -> out (W.add a b)
        "subtract" -> out (W.sub a b)
        "multiply" -> out (W.mul a b)
        "divide" -> if b == W.zero then "!" else out (W.divide a b c.u)
        "modulo" -> if b == W.zero then "!" else out (W.modulo a b c.u)
        "and" -> out (W.bitAnd a b)
        "or" -> out (W.bitOr a b)
        "xor" -> out (W.bitXor a b)
        "shiftLeft" -> out (W.shl c.n a)
        "shiftRight" -> out (W.shr True c.n a)
        "shiftRightUnsigned" -> out (W.shr False c.n a)
        "rotl" -> out (W.rotl c.n a)
        "clz" -> String.fromInt (W.clz a)
        "ctz" -> String.fromInt (W.ctz a)
        "compare" -> String.fromInt (W.compareWords a b c.u c.v)
        "mix" -> if c.r /= 0 && (c.r < 2 || c.r > 36) then "!" else mix c (W.bitOr b W.one) 0 a 0
        _ -> "!"

handle value =
    case D.decodeValue commands value of
        Ok batch -> E.object [("ok", E.string (String.join "\n" (List.map evaluate batch)))]
        Err err -> E.object [("error", E.string (D.errorToString err))]

main : Program () () D.Value
main = Platform.worker
    { init = \_ -> ((),Cmd.none)
    , update = \value model -> (model,response (handle value))
    , subscriptions = \_ -> request identity
    }
