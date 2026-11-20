port module Main exposing (main)

{-| The boundary of the Elm port of `marked@18.1.1` (`libraries/marked/CONTRACT.md`),
mirroring `libraries/marked/reference.mjs`.

The JS host (`js/index.js`) sends the corpus input through the `request` port; it is decoded
with `Json.Decode` (`op`, `markdown`, optional `options: { gfm, breaks, pedantic }`), compiled
through `Lexer` and `Renderer`, and answered as `{ ok: html }` (or `{ error: message }` for
an unknown operation or malformed input) through the `response` port.
-}

import Json.Decode as Decode exposing (Decoder)
import Json.Encode as Encode
import Lexer
import Platform
import Renderer
import Tokens exposing (Options, defaultOptions)


port request : (Decode.Value -> msg) -> Sub msg


port response : Encode.Value -> Cmd msg


main : Program () () Decode.Value
main =
    Platform.worker
        { init = \_ -> ( (), Cmd.none )
        , update = \value model -> ( model, response (handle value) )
        , subscriptions = \_ -> request identity
        }


type alias Request =
    { op : String
    , markdown : String
    , options : Options
    }


handle : Decode.Value -> Encode.Value
handle value =
    case Decode.decodeValue requestDecoder value of
        Ok req ->
            case req.op of
                "parse" ->
                    let
                        ( blocks, slots ) =
                            Lexer.lex req.markdown (Lexer.new req.options)
                    in
                    Encode.object [ ( "ok", Encode.string (Renderer.renderBlocks blocks slots) ) ]

                "parseInline" ->
                    let
                        inlines =
                            Lexer.lexInline req.markdown (Lexer.new req.options)
                    in
                    Encode.object [ ( "ok", Encode.string (Renderer.renderInline inlines) ) ]

                other ->
                    Encode.object [ ( "error", Encode.string ("unknown operation " ++ other) ) ]

        Err err ->
            Encode.object [ ( "error", Encode.string (Decode.errorToString err) ) ]


requestDecoder : Decoder Request
requestDecoder =
    Decode.map3 Request
        (Decode.field "op" Decode.string)
        (Decode.field "markdown" Decode.string)
        (Decode.maybe (Decode.field "options" optionsDecoder)
            |> Decode.map (Maybe.withDefault defaultOptions)
        )


optionsDecoder : Decoder Options
optionsDecoder =
    Decode.map3 Options
        (optionalBool "gfm" defaultOptions.gfm)
        (optionalBool "breaks" defaultOptions.breaks)
        (optionalBool "pedantic" defaultOptions.pedantic)


optionalBool : String -> Bool -> Decoder Bool
optionalBool field fallback =
    Decode.maybe (Decode.field field Decode.bool)
        |> Decode.map (Maybe.withDefault fallback)
