module SearchableMap exposing
    ( SearchableMap, empty
    , get, update, remove
    , atPrefix, foldl, entries, keys, size
    , fuzzyGet
    )

{-| MiniSearch's `SearchableMap`: a radix tree over UTF-16 keys, persistent.

A node is what MiniSearch's node is, an insertion-ordered map: edge labels to child nodes, and the
empty label (`LEAF`) to the value of the key that ends at the node. The order of a node's entries
is observable (iteration, prefix and fuzzy results), so every operation keeps exactly the order a
JavaScript `Map` would: setting a new key appends it, deleting removes it in place, setting an
existing key keeps its place, and replacing an edge (a split in `createPath`, a merge after a
removal) is setting the new edge and deleting the old one, which moves it to the end.

A node's entries are a list in insertion order, as a `Map`'s keys are: lookups scan it from the
oldest entry as MiniSearch's `for (const k of node.keys())` does, iteration (`TreeIterator`,
newest first) folds it from the right, and the fuzzy walk (oldest first) from the left. No two
edges of a node share their first code unit. Labels are compared and split on code units, so an
edge may end inside a surrogate pair.

The tree is persistent: an update copies the path from the root. A walk over a tree value (prefix
search, vacuuming) therefore sees the tree as it was when the walk began, which is what
MiniSearch's iterator sees too: it snapshots each node's keys, and only the term just visited is
ever deleted during a walk (CONTRACT.md, "The index").

@docs SearchableMap, empty
@docs get, update, remove
@docs atPrefix, foldl, entries, keys, size
@docs fuzzyGet

-}

import Text


{-| A tree and the prefix its keys are relative to: `""` for the index, the prefix for an
`atPrefix` view.
-}
type SearchableMap v
    = SearchableMap String (Node v)


type Node v
    = Node (List (Entry v))


{-| An edge carries its label's first code unit, the unit edges are looked up by: labels never
change once made (a split or a merge makes new edges).
-}
type Entry v
    = Leaf v
    | Edge Int String (Node v)


edge : String -> Node v -> Entry v
edge label child =
    Edge (Text.codeUnitAt 0 label) label child


empty : SearchableMap v
empty =
    SearchableMap "" (Node [])



-- LOOKUP


{-| The value of `key` (`lookup`, then the node's leaf).
-}
get : String -> SearchableMap v -> Maybe v
get key (SearchableMap _ root) =
    lookup key root |> Maybe.andThen leafValue


{-| `lookup`: follow the edges that are prefixes of the rest of the key.
-}
lookup : String -> Node v -> Maybe (Node v)
lookup key ((Node nodeEntries) as node) =
    if String.isEmpty key then
        Just node

    else
        case findEdge (Text.codeUnitAt 0 key) nodeEntries of
            Just ( label, child ) ->
                if String.startsWith label key then
                    lookup (String.dropLeft (String.length label) key) child

                else
                    Nothing

            Nothing ->
                Nothing


leafValue : Node v -> Maybe v
leafValue (Node nodeEntries) =
    findLeaf nodeEntries


findLeaf : List (Entry v) -> Maybe v
findLeaf nodeEntries =
    case nodeEntries of
        [] ->
            Nothing

        (Leaf value) :: _ ->
            Just value

        (Edge _ _ _) :: rest ->
            findLeaf rest


{-| The edge whose label starts with the code unit `unit` (there is at most one).
-}
findEdge : Int -> List (Entry v) -> Maybe ( String, Node v )
findEdge unit nodeEntries =
    case nodeEntries of
        [] ->
            Nothing

        (Edge first label child) :: rest ->
            if sameUnit first unit then
                Just ( label, child )

            else
                findEdge unit rest

        (Leaf _) :: rest ->
            findEdge unit rest


{-| `key[pos] === k[0]`. Elm compiles `==` of two variables to its structural `_Utils_eq` (which
allocates on every call), and `==` against a literal to `===`; edge lookup is the hottest
comparison of indexing.
-}
sameUnit : Int -> Int -> Bool
sameUnit a b =
    a - b == 0


startsWithUnit : Int -> Entry v -> Bool
startsWithUnit unit entry =
    case entry of
        Edge first _ _ ->
            sameUnit first unit

        Leaf _ ->
            False


{-| The entries with the edge starting with `unit` replaced in place.
-}
replaceEdge : Int -> Entry v -> List (Entry v) -> List (Entry v)
replaceEdge unit replacement nodeEntries =
    case nodeEntries of
        [] ->
            []

        entry :: rest ->
            if startsWithUnit unit entry then
                replacement :: rest

            else
                entry :: replaceEdge unit replacement rest


{-| The entries without the edge starting with `unit`.
-}
removeEdge : Int -> List (Entry v) -> List (Entry v)
removeEdge unit nodeEntries =
    case nodeEntries of
        [] ->
            []

        entry :: rest ->
            if startsWithUnit unit entry then
                rest

            else
                entry :: removeEdge unit rest



-- UPDATE


{-| `fetch` / `update`: `createPath(key)`, then the leaf of the node it returns set to
`f (current value)`, appended when the node had no leaf.
-}
update : String -> (Maybe v -> v) -> SearchableMap v -> SearchableMap v
update key f (SearchableMap prefix root) =
    SearchableMap prefix (updateNode key f root)


updateNode : String -> (Maybe v -> v) -> Node v -> Node v
updateNode key f (Node nodeEntries) =
    if String.isEmpty key then
        Node (setLeaf f nodeEntries)

    else
        Node (updateEdges (Text.codeUnitAt 0 key) key f nodeEntries)


{-| `createPath` at one node, in one pass over its entries, oldest first: the edge sharing the
key's first unit is followed in place when the key goes through its whole label; when the key
leaves it part way, it is split: an intermediate node holds the rest of the old edge, and is set
under the shared part while the old edge is deleted, so it moves to the end. With no such edge, a
new edge to a new node holding the leaf is appended.
-}
updateEdges : Int -> String -> (Maybe v -> v) -> List (Entry v) -> List (Entry v)
updateEdges unit key f nodeEntries =
    case nodeEntries of
        [] ->
            [ edge key (Node [ Leaf (f Nothing) ]) ]

        ((Edge first label child) as entry) :: rest ->
            if sameUnit first unit then
                case splitOffset key label of
                    Nothing ->
                        Edge first label (updateNode (String.dropLeft (String.length label) key) f child) :: rest

                    Just offset ->
                        let
                            intermediate =
                                Node [ edge (String.dropLeft offset label) child ]
                        in
                        rest ++ [ Edge first (String.left offset label) (updateNode (String.dropLeft offset key) f intermediate) ]

            else
                entry :: updateEdges unit key f rest

        entry :: rest ->
            entry :: updateEdges unit key f rest


{-| `createPath`'s `offset` when the key leaves the edge part way: the length of the common prefix
of the key and a label whose first unit it shares (`Nothing` when the key goes through the whole
label).
-}
splitOffset : String -> String -> Maybe Int
splitOffset key label =
    if String.startsWith label key then
        Nothing

    else
        let
            limit =
                min (String.length key) (String.length label)

            advance offset =
                if offset < limit && sameUnit (Text.codeUnitAt offset key) (Text.codeUnitAt offset label) then
                    advance (offset + 1)

                else
                    offset
        in
        Just (advance 1)


{-| The leaf set to `f (current value)`: in place, or appended when the node has none.
-}
setLeaf : (Maybe v -> v) -> List (Entry v) -> List (Entry v)
setLeaf f nodeEntries =
    case nodeEntries of
        [] ->
            [ Leaf (f Nothing) ]

        (Leaf value) :: rest ->
            Leaf (f (Just value)) :: rest

        entry :: rest ->
            entry :: setLeaf f rest



-- REMOVE


{-| What a node asks of its parent after a removal below it: nothing; to delete the edge to it,
because it is empty (`cleanup`); or to replace the edge to it by the two labels joined, because
it holds a single edge (`merge`).
-}
type Removal
    = Kept
    | Emptied
    | Single


{-| `delete` (MiniSearch's `remove`): track down to the key's node, delete its leaf, then clean up
and merge on the way back up. Neither happens at the root, which has no parent: what it asks of
one (`cleanup([])`, `merge([])`) is nothing.
-}
remove : String -> SearchableMap v -> SearchableMap v
remove key (SearchableMap prefix root) =
    SearchableMap prefix (Tuple.first (removeFrom key root))


removeFrom : String -> Node v -> ( Node v, Removal )
removeFrom key ((Node nodeEntries) as node) =
    if String.isEmpty key then
        let
            remaining =
                List.filter (not << isLeaf) nodeEntries
        in
        ( Node remaining
        , case remaining of
            [] ->
                Emptied

            [ _ ] ->
                Single

            _ ->
                Kept
        )

    else
        let
            unit =
                Text.codeUnitAt 0 key
        in
        case findEdge unit nodeEntries of
            Just ( label, child ) ->
                if String.startsWith label key then
                    case removeFrom (String.dropLeft (String.length label) key) child of
                        ( kept, Kept ) ->
                            ( Node (replaceEdge unit (Edge unit label kept) nodeEntries), Kept )

                        ( _, Emptied ) ->
                            -- `cleanup`: delete the edge; this node may now be empty, or hold a
                            -- single edge, which merges it into its own parent.
                            let
                                remaining =
                                    removeEdge unit nodeEntries
                            in
                            ( Node remaining
                            , case remaining of
                                [] ->
                                    Emptied

                                [ Edge _ _ _ ] ->
                                    Single

                                _ ->
                                    Kept
                            )

                        ( Node [ Edge _ rest grandchild ], Single ) ->
                            -- `merge`: the joined edge is set, then the old one deleted, so it
                            -- moves to the end.
                            ( Node (removeEdge unit nodeEntries ++ [ Edge unit (label ++ rest) grandchild ]), Kept )

                        ( kept, Single ) ->
                            -- Not reached: a node answers `Single` only when it holds one edge.
                            ( Node (replaceEdge unit (Edge unit label kept) nodeEntries), Kept )

                else
                    ( node, Kept )

            Nothing ->
                ( node, Kept )


isLeaf : Entry v -> Bool
isLeaf entry =
    case entry of
        Leaf _ ->
            True

        Edge _ _ _ ->
            False



-- ITERATION


{-| `atPrefix(prefix)`: the subtree of the keys starting with `prefix`, as a view whose keys are
relative to it. When the prefix ends inside an edge, the view is a node holding only the rest of
that edge.
-}
atPrefix : String -> SearchableMap v -> SearchableMap v
atPrefix prefix (SearchableMap own root) =
    SearchableMap prefix (prefixNode (String.dropLeft (String.length own) prefix) root)


prefixNode : String -> Node v -> Node v
prefixNode key ((Node nodeEntries) as node) =
    if String.isEmpty key then
        node

    else
        case findEdge (Text.codeUnitAt 0 key) nodeEntries of
            Just ( label, child ) ->
                if String.startsWith label key then
                    prefixNode (String.dropLeft (String.length label) key) child

                else if String.startsWith key label then
                    Node [ edge (String.dropLeft (String.length key) label) child ]

                else
                    Node []

            Nothing ->
                Node []


{-| Folds over every key and value in `TreeIterator` order: depth first, each node's entries from
the most recently inserted to the oldest.
-}
foldl : (String -> v -> acc -> acc) -> acc -> SearchableMap v -> acc
foldl f acc (SearchableMap prefix root) =
    foldNode f prefix root acc


foldNode : (String -> v -> acc -> acc) -> String -> Node v -> acc -> acc
foldNode f key (Node nodeEntries) acc =
    List.foldr
        (\entry result ->
            case entry of
                Leaf value ->
                    f key value result

                Edge _ label child ->
                    foldNode f (key ++ label) child result
        )
        acc
        nodeEntries


{-| `entries()`, in iteration order.
-}
entries : SearchableMap v -> List ( String, v )
entries map =
    List.reverse (foldl (\key value list -> ( key, value ) :: list) [] map)


{-| `keys()`, in iteration order.
-}
keys : SearchableMap v -> List String
keys map =
    List.reverse (foldl (\key _ list -> key :: list) [] map)


{-| The number of keys (`size`).
-}
size : SearchableMap v -> Int
size map =
    foldl (\_ _ count -> count + 1) 0 map



-- FUZZY


{-| `fuzzyGet(query, maxDistance)`: every key within `maxDistance` (Levenshtein, over code units)
of `query`, with its value and distance, in the order the walk meets them: depth first, each
node's entries oldest first.

MiniSearch fills a Levenshtein matrix of `query.length + 1` columns and
`query.length + 1 + maxDistance` rows in a `Uint8Array`, one row per label unit on the current
path, each over a band of columns; every other cell keeps its initial value (`maxDistance + 1`,
and `i` in column 0). A row only ever depends on the row above it, and the band of row `i` is the
same on every path, so a row is a pure function of the row above and the label unit: the walk
carries the current row as a list. Inside the parity domain no cell reaches 256, so the
`Uint8Array` never wraps; rows past the matrix (keys longer than `query.length + maxDistance`)
never match, so the walk stops there, as the contract allows (MiniSearch walks on, reading
`undefined`).

-}
fuzzyGet : String -> Int -> SearchableMap v -> List ( String, v, Int )
fuzzyGet query maxDistance (SearchableMap prefix root) =
    let
        columns =
            String.length query + 1

        levenshtein =
            { query = List.map (\i -> Text.codeUnitAt i query) (List.range 0 (columns - 2))
            , maxDistance = maxDistance
            , columns = columns
            , rows = columns + maxDistance
            }
    in
    List.reverse (fuzzyWalk levenshtein root (Row 1 (List.range 0 (columns - 1)) (columns - 1)) prefix [])


type alias Levenshtein =
    { query : List Int -- the query's code units
    , maxDistance : Int
    , columns : Int
    , rows : Int
    }


{-| The last filled row of the matrix: `next` is the index of the row to fill next (MiniSearch's
`m`), `cells` the row's values, `last` its last cell (the distance of the key ending here).
-}
type alias Row =
    { next : Int, cells : List Int, last : Int }


fuzzyWalk : Levenshtein -> Node v -> Row -> String -> List ( String, v, Int ) -> List ( String, v, Int )
fuzzyWalk levenshtein (Node nodeEntries) row prefix results =
    List.foldl
        (\entry found ->
            case entry of
                Leaf value ->
                    if atMost levenshtein.maxDistance row.last then
                        ( prefix, value, row.last ) :: found

                    else
                        found

                Edge _ label child ->
                    case walkLabel levenshtein label 0 row of
                        Just below ->
                            fuzzyWalk levenshtein child below (prefix ++ label) found

                        Nothing ->
                            found
        )
        results
        nodeEntries


{-| The rows for the units of an edge label from `position` on, or nothing when the walk below it
cannot match: a row's minimum exceeds `maxDistance` (distances never decrease down the matrix),
or the label runs past the last row.
-}
walkLabel : Levenshtein -> String -> Int -> Row -> Maybe Row
walkLabel levenshtein label position row =
    if not (atMost (String.length label - 1) position) then
        Just row

    else if not (atMost (levenshtein.rows - 1) row.next) then
        Nothing

    else
        let
            ( filled, minimum ) =
                nextRow levenshtein row.next (Text.codeUnitAt position label) row.cells
        in
        if atMost levenshtein.maxDistance minimum then
            walkLabel levenshtein label (position + 1) filled

        else
            Nothing


{-| Row `i` from row `i - 1` for the label unit `unit`: column 0 is `i`; cell `j + 1` for `j` from
`max(0, i - maxDistance - 1)` to `min(columns - 1, i + maxDistance)` (exclusive) is
`min(above-left + different, above + 1, left + 1)`; every other cell is `maxDistance + 1`. With
the row's minimum over column 0 and the band (MiniSearch's `minDistance`).
-}
nextRow : Levenshtein -> Int -> Int -> List Int -> ( Row, Int )
nextRow levenshtein i unit above =
    let
        first =
            max 0 (i - levenshtein.maxDistance - 1)

        last =
            min (levenshtein.columns - 1) (i + levenshtein.maxDistance)

        band =
            { unit = unit, initial = levenshtein.maxDistance + 1 }
    in
    case above of
        diagonal :: aboveRest ->
            let
                ( cells, final, minimum ) =
                    fillRow band first (last - first) diagonal aboveRest levenshtein.query i i [ i ]
            in
            ( Row (i + 1) cells final, minimum )

        [] ->
            ( Row (i + 1) [] i, i )


{-| Fills a row left to right, from column 1: `before` more cells before the band, then `width`
band cells, then initial cells to the end. `diagonal` is the cell above-left, `above` the cells
above from the current column on, `query` the query units from the column before on, `left` the
cell just filled; `filled` holds the row so far, last first. Returns the row, its last cell and
its minimum.

The counters are compared with literals: Elm compiles a comparison of two variables (and `min`)
to its generic `_Utils_cmp` (and `==` to `_Utils_eq`, see `sameUnit`), and a function of more
than nine parameters to curried closures, all costly in this, the fuzzy walk's inner loop.

-}
fillRow : { unit : Int, initial : Int } -> Int -> Int -> Int -> List Int -> List Int -> Int -> Int -> List Int -> ( List Int, Int, Int )
fillRow band before width diagonal above query left minimum filled =
    case above of
        up :: ups ->
            case query of
                queryUnit :: queryRest ->
                    if before > 0 then
                        fillRow band (before - 1) width up ups queryRest band.initial minimum (band.initial :: filled)

                    else if width <= 0 then
                        fillRow band 0 0 up ups queryRest band.initial minimum (band.initial :: filled)

                    else
                        let
                            different =
                                if sameUnit band.unit queryUnit then
                                    0

                                else
                                    1

                            distance =
                                smaller (smaller (diagonal + different) (up + 1)) (left + 1)
                        in
                        fillRow band 0 (width - 1) up ups queryRest distance (smaller minimum distance) (distance :: filled)

                [] ->
                    ( List.reverse filled, left, minimum )

        [] ->
            ( List.reverse filled, left, minimum )


{-| `min` and `<=` for integers, comparing a difference with a literal (see `fillRow`).
-}
smaller : Int -> Int -> Int
smaller a b =
    if a - b < 0 then
        a

    else
        b


atMost : Int -> Int -> Bool
atMost limit value =
    value - limit <= 0
