module Canonical exposing (Component, canonicalString, hashComponents)

{-| Canonical `key:value|` join. JSON conversion of component values stays in
the JS host (`JSON.stringify`); Elm sorts keys, escapes `:|\\`, and joins.
-}

import Hashing


type alias Component =
    { key : String
    , value : String
    }


canonicalString : List Component -> String
canonicalString components =
    components
        |> List.sortBy .key
        |> List.map (\item -> escapeKey item.key ++ ":" ++ item.value)
        |> String.join "|"


hashComponents : List Component -> String
hashComponents components =
    Hashing.x64hash128 (canonicalString components) 0


escapeKey : String -> String
escapeKey key =
    String.foldl
        (\char acc ->
            if char == ':' || char == '|' || char == '\\' then
                acc ++ "\\" ++ String.fromChar char

            else
                acc ++ String.fromChar char
        )
        ""
        key
