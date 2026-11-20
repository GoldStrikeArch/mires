module Codec exposing (compress, decompress)

import Array exposing (Array)
import Bitwise
import Dict exposing (Dict)
import Set exposing (Set)

width : String -> Int
width format =
    case format of
        "utf16" -> 15
        "uri" -> 6
        "base64" -> 6
        _ -> 16

alphabet : String -> List Int
alphabet format =
    String.toList
        ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+"
            ++ (if format == "uri" then "-$" else "/="))
        |> List.map Char.toCode

type alias Width = { bits : Int, left : Int }

advance : Width -> Width
advance w =
    if w.left == 1 then
        { bits = w.bits + 1, left = Bitwise.shiftLeftBy w.bits 1 }
    else
        { w | left = w.left - 1 }

type alias Writer = { size : Int, used : Int, word : Int, words : List Int }

write : Int -> Int -> Writer -> Writer
write n value w =
    if n == 0 then w
    else
        let
            word = 2 * w.word + Bitwise.and 1 value
            next =
                if w.used + 1 == w.size then
                    { w | words = word :: w.words, word = 0, used = 0 }
                else
                    { w | word = word, used = w.used + 1 }
        in
        write (n - 1) (Bitwise.shiftRightZfBy 1 value) next

type alias Encoder =
    { dictionary : Dict (List Int) Int, pending : Set Int, next : Int
    , prefix : List Int, writer : Writer, codes : Width
    }

emit : Encoder -> Encoder
emit state =
    let
        code = Dict.get state.prefix state.dictionary |> Maybe.withDefault 0
        ordinary =
            { state | writer = write state.codes.bits code state.writer, codes = advance state.codes }
    in
    case state.prefix of
        [c] ->
            if Set.member c state.pending then
                let
                    w = write state.codes.bits (if c < 256 then 0 else 1) state.writer
                in
                { state | writer = write (if c < 256 then 8 else 16) c w
                    , pending = Set.remove c state.pending, codes = advance (advance state.codes) }
            else ordinary

        _ -> ordinary

special : List Int
special = String.toList "__proto__" |> List.map Char.toCode |> List.reverse

encodeUnit : Int -> Encoder -> Encoder
encodeUnit c state =
    let
        s =
            if Dict.member [c] state.dictionary then state
            else
                { state | dictionary = Dict.insert [c] state.next state.dictionary
                    , pending = Set.insert c state.pending, next = state.next + 1 }
        phrase = c :: s.prefix
    in
    if Dict.member phrase s.dictionary then { s | prefix = phrase }
    else
        let
            out = emit s
            dictionary =
                if phrase == special then out.dictionary else Dict.insert phrase out.next out.dictionary
        in
        { out | prefix = [c], dictionary = dictionary, next = out.next + 1 }

compress : String -> List Int -> List Int
compress format data =
    let
        size = width format
        initial = { dictionary = Dict.empty, pending = Set.empty, next = 3
                  , prefix = [], writer = { size = size, used = 0, word = 0, words = [] }
                  , codes = { bits = 2, left = 2 } }
        state = List.foldl encodeUnit initial data
        end = if List.isEmpty state.prefix then state else emit state
        tagged = write end.codes.bits 2 end.writer
        words = List.reverse (write (size - tagged.used) 0 tagged).words
    in
    case format of
        "raw" -> words
        "utf16" -> List.map ((+) 32) words ++ [32]
        "bytes" -> List.concatMap (\n -> [Bitwise.shiftRightZfBy 8 n, Bitwise.and 255 n]) words
        _ ->
            let
                table = Array.fromList (alphabet format)
                packed = List.map (\n -> Array.get n table |> Maybe.withDefault 0) words
                pad = if format == "base64" then modBy 4 (0 - List.length packed) else 0
            in
            packed ++ List.repeat pad 61

type alias Reader = { words : Array Int, size : Int, index : Int, position : Int }

read : Int -> Reader -> (Int, Reader)
read count reader =
    let
        loop bit value r =
            if bit == count then (value, r)
            else
                let
                    word = Array.get r.index r.words |> Maybe.withDefault 0
                    pos = r.position - 1
                    digit = Bitwise.and 1 (Bitwise.shiftRightZfBy pos word)
                    next = if pos == 0 then { r | index = r.index + 1, position = r.size }
                           else { r | position = pos }
                in
                loop (bit+1) (value + Bitwise.shiftLeftBy bit digit) next
    in
    loop 0 0 reader

unpackBytes : List Int -> List Int -> Result String (List Int)
unpackBytes src rev =
    case src of
        [] -> Ok (List.reverse rev)
        a :: b :: rest -> unpackBytes rest ((a*256+b)::rev)
        _ -> Err "odd byte count"

unpack : String -> List Int -> Result String (List Int)
unpack format data =
    case format of
        "raw" -> Ok data
        "bytes" -> unpackBytes data []
        "utf16" -> Ok (List.map (\n -> n - 32) data)
        _ ->
            let
                table = alphabet format |> List.indexedMap (\i c -> (c,i)) |> Dict.fromList
                value c = Dict.get (if format == "uri" && c == 32 then 43 else c) table |> Maybe.withDefault 0
            in
            Ok (List.map value data)

type alias Phrase = { first : Int, units : List Int }

type alias Decoder =
    { dictionary : Array (Maybe Phrase), previous : Maybe Phrase, output : List (List Int)
    , codes : Width, reader : Reader
    }

decode : Decoder -> Result String (Maybe (List Int))
decode s =
    if s.reader.index >= Array.length s.reader.words then Ok (Just [])
    else
        let (tag, r) = read s.codes.bits s.reader in
        if tag == 2 then Ok (Just (List.concat (List.reverse s.output)))
        else
            let
                (code, state) =
                    if tag <= 1 then
                        let
                            (unit, next) = read (if tag == 0 then 8 else 16) r
                            literal = Just { first = unit, units = [unit] }
                        in
                        (Array.length s.dictionary,
                         { s | dictionary = Array.push literal s.dictionary, reader = next, codes = advance s.codes })
                    else (tag, { s | reader = r })
                missing = code == Array.length state.dictionary
                phrase =
                    if missing then
                        Maybe.map (\p -> { p | units = p.units ++ [p.first] }) state.previous
                    else Array.get code state.dictionary |> Maybe.andThen identity
            in
            case phrase of
                Nothing -> if missing then Err "undefined phrase" else Ok Nothing
                Just p ->
                    let
                        old = Maybe.withDefault { first = 117, units = List.map Char.toCode (String.toList "undefined") } state.previous
                        added = Just { old | units = old.units ++ [p.first] }
                    in
                    decode { state | previous = phrase, dictionary = Array.push added state.dictionary
                           , codes = advance state.codes, output = p.units :: state.output }

decompress : String -> List Int -> Result String (Maybe (List Int))
decompress format data =
    let
        start words =
            let
                size = width format
                (header, r) = read 2 { words = Array.fromList words, size = size, index = 0, position = size }
                (unit, next) = read (if header == 0 then 8 else 16) r
                initial = if header == 3 then Nothing else Just { first = unit, units = [unit] }
            in
            if header == 2 then Ok (Just [])
            else
                decode { dictionary = Array.fromList [Nothing, Nothing, Nothing, initial], previous = initial
                       , output = if header == 3 then [] else [[unit]], codes = { bits = 3, left = 4 }
                       , reader = if header == 3 then r else next }
    in
    if List.isEmpty data then Ok Nothing else unpack format data |> Result.andThen start
