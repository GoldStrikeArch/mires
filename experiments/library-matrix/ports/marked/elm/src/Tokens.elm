module Tokens exposing
    ( Align(..)
    , BlockToken(..)
    , InlineSlot
    , InlineToken(..)
    , LinkDef
    , ListItem
    , Mode(..)
    , Options
    , TableCell
    , appendNewlineToBlockRaw
    , blockRawLen
    , defaultOptions
    , isBreaks
    , isGfm
    , isPedantic
    , modeOf
    )

{-| Options, grammar mode, link definitions, and block/inline token types mirroring marked 18.1.1.
-}


type alias Options =
    { gfm : Bool
    , breaks : Bool
    , pedantic : Bool
    }


defaultOptions : Options
defaultOptions =
    { gfm = True
    , breaks = False
    , pedantic = False
    }


{-| Active grammar rule set selected by `Lexer` constructor (`Lexer.ts`).
-}
type Mode
    = Normal
    | Gfm
    | Breaks
    | Pedantic


modeOf : Options -> Mode
modeOf opts =
    if opts.pedantic then
        Pedantic

    else if opts.gfm then
        if opts.breaks then
            Breaks

        else
            Gfm

    else
        Normal


isPedantic : Mode -> Bool
isPedantic mode =
    case mode of
        Pedantic ->
            True

        _ ->
            False


isGfm : Mode -> Bool
isGfm mode =
    case mode of
        Gfm ->
            True

        Breaks ->
            True

        _ ->
            False


isBreaks : Mode -> Bool
isBreaks mode =
    case mode of
        Breaks ->
            True

        _ ->
            False


type alias LinkDef =
    { href : String
    , title : Maybe String
    }


{-| One queued inline-lexing job (`Lexer.inlineQueue` in `Lexer.ts`), stored in an `Array` arena
so block tokens (`Heading`, `Paragraph`, `Text`, `TableCell`) reference their inline token list by
slot index.
-}
type alias InlineSlot =
    { src : String
    , tokens : List InlineToken
    }


type Align
    = AlignNone
    | AlignLeft
    | AlignCenter
    | AlignRight


type alias TableCell =
    { header : Bool
    , align : Align
    , inlineId : Int
    }


type alias ListItem =
    { raw : String
    , task : Bool
    , checked : Bool
    , loose : Bool
    , text : String
    , tokens : List BlockToken
    }


type BlockToken
    = Space { raw : String }
    | Hr { rawLen : Int }
    | Heading { rawLen : Int, depth : Int, inlineId : Int }
    | Code { rawLen : Int, lang : Maybe String, text : String }
    | Html { rawLen : Int, text : String }
    | Def { rawLen : Int }
    | Blockquote { raw : String, tokens : List BlockToken }
    | ListBlock { raw : String, ordered : Bool, start : Int, loose : Bool, items : List ListItem }
    | BlockCheckbox { checked : Bool }
    | Table { rawLen : Int, header : List TableCell, rows : List (List TableCell) }
    | Paragraph { rawLen : Int, text : String, inlineId : Int }
    | BlockText { rawLen : Int, text : String, inlineId : Int }


blockRawLen : BlockToken -> Int
blockRawLen token =
    case token of
        Space r ->
            String.length r.raw

        Blockquote r ->
            String.length r.raw

        ListBlock r ->
            String.length r.raw

        Hr r ->
            r.rawLen

        Heading r ->
            r.rawLen

        Code r ->
            r.rawLen

        Html r ->
            r.rawLen

        Def r ->
            r.rawLen

        Table r ->
            r.rawLen

        Paragraph r ->
            r.rawLen

        BlockText r ->
            r.rawLen

        BlockCheckbox _ ->
            0


appendNewlineToBlockRaw : BlockToken -> BlockToken
appendNewlineToBlockRaw token =
    case token of
        Space r ->
            Space { raw = r.raw ++ "\n" }

        Blockquote r ->
            Blockquote { r | raw = r.raw ++ "\n" }

        ListBlock r ->
            ListBlock { r | raw = r.raw ++ "\n" }

        Hr r ->
            Hr { rawLen = r.rawLen + 1 }

        Heading r ->
            Heading { r | rawLen = r.rawLen + 1 }

        Code r ->
            Code { r | rawLen = r.rawLen + 1 }

        Html r ->
            Html { r | rawLen = r.rawLen + 1 }

        Def r ->
            Def { rawLen = r.rawLen + 1 }

        Table r ->
            Table { r | rawLen = r.rawLen + 1 }

        Paragraph r ->
            Paragraph { r | rawLen = r.rawLen + 1 }

        BlockText r ->
            BlockText { r | rawLen = r.rawLen + 1 }

        BlockCheckbox _ ->
            token


type InlineToken
    = Escape { text : String }
    | InlineHtml { text : String }
    | Link { href : String, title : Maybe String, text : String, autolink : Bool, tokens : List InlineToken }
    | Image { href : String, title : Maybe String, text : String, tokens : List InlineToken }
    | InlineCheckbox { raw : String, checked : Bool }
    | Strong { text : String, tokens : List InlineToken }
    | Em { text : String, tokens : List InlineToken }
    | Codespan { text : String }
    | Br
    | Del { text : String, tokens : List InlineToken }
    | InlineText { text : String, escaped : Bool }
