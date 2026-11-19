module OrderedDict exposing (OrderedDict, empty, foldl, get, insert, remove, toList)

{-| A dictionary that remembers insertion order, as a JavaScript `Map` does: inserting a new key
appends it, inserting an existing key keeps its place, removing a key takes it out. MiniSearch's
result maps (`Map<shortId, result>`) and autoSuggest's suggestion map need that order; `Dict` is
ordered by key.

Each key carries the sequence number of its insertion; iteration sorts by it.

-}

import Dict exposing (Dict)


type OrderedDict comparable v
    = OrderedDict Int (Dict comparable ( Int, v ))


empty : OrderedDict comparable v
empty =
    OrderedDict 0 Dict.empty


get : comparable -> OrderedDict comparable v -> Maybe v
get key (OrderedDict _ entries) =
    Maybe.map Tuple.second (Dict.get key entries)


{-| `map.set(key, value)`.
-}
insert : comparable -> v -> OrderedDict comparable v -> OrderedDict comparable v
insert key value (OrderedDict next entries) =
    case Dict.get key entries of
        Just ( sequence, _ ) ->
            OrderedDict next (Dict.insert key ( sequence, value ) entries)

        Nothing ->
            OrderedDict (next + 1) (Dict.insert key ( next, value ) entries)


{-| `map.delete(key)`.
-}
remove : comparable -> OrderedDict comparable v -> OrderedDict comparable v
remove key (OrderedDict next entries) =
    OrderedDict next (Dict.remove key entries)


{-| The entries in insertion order.
-}
toList : OrderedDict comparable v -> List ( comparable, v )
toList (OrderedDict _ entries) =
    Dict.foldr (\key ( sequence, value ) list -> ( sequence, ( key, value ) ) :: list) [] entries
        |> List.sortBy Tuple.first
        |> List.map Tuple.second


{-| Folds over the entries in insertion order.
-}
foldl : (comparable -> v -> acc -> acc) -> acc -> OrderedDict comparable v -> acc
foldl f acc map =
    List.foldl (\( key, value ) result -> f key value result) acc (toList map)
