module Lexer exposing (Lexer, lex, lexInline, new)

{-| Block and inline Lexer + Tokenizer mirroring `src/Lexer.ts` and `src/Tokenizer.ts`
of marked 18.1.1.

Every unbounded loop (`blockTokensLoop`, `inlineTokensLoop`, `drainInlineQueue`,
`parseBlockquoteLoop`, `scanListItems`, `scanListItemLines`, `scanEmStrongRDelim`,
`scanDelRDelim`) is directly self-tail-recursive so Elm 0.19.2 compiles it to a JS
`while` loop with constant stack space.
-}

import Array exposing (Array)
import BlockRules
    exposing
        ( ListMarkerSpec(..)
        , findListTaskCheckbox
        , hasAnyLine
        , hasDoubleBlankLineEnd
        , indentCodeCompensation
        , isBlockquoteAt
        , isBlockquoteBegin
        , isFencesBegin
        , isHeadingBegin
        , isHtmlInterrupt
        , isListTask
        , isNextBullet
        , lineEnd
        , matchBlockCode
        , matchBlockquote
        , matchDef
        , matchFences
        , matchHeading
        , matchHrAt
        , matchHtmlNormal
        , matchHtmlPedantic
        , matchLHeadingAt
        , matchListItem
        , matchListStart
        , matchNewline
        , matchParagraphAt
        , matchTableAt
        , parseTableAlign
        , preprocessPedanticBlock
        , removeCodeIndent
        , replacePedanticListNesting
        , stripBlockquoteMarkers
        , stripListTask
        , stripOneBlockquotePrefix
        , trimTableAlignChars
        , trimTableRowBlankLine
        )
import Dict exposing (Dict)
import Helpers
    exposing
        ( collapseSpaces
        , decodeNumericCharacterReferences
        , expandTabs
        , findClosingParenSpan
        , isBlankLineSpan
        , normalizeLabel
        , normalizeNewlines
        , rtrimChar
        , splitCells
        , trimTrailingBlankLines
        , unescapeAnyPunctuation
        , unescapeLinkBrackets
        )
import InlineRules
    exposing
        ( RDelimKind(..)
        , buildMaskedCodes
        , isEndATag
        , isEndPreScriptTag
        , isLabelEndInsideToken
        , isStartATag
        , isStartPreScriptTag
        , matchAutolink
        , matchBr
        , matchDelL
        , matchEmStrongL
        , matchEscape
        , matchGfmUrl
        , matchInlineCodeAt
        , matchInlineLink
        , matchInlineText
        , matchPedanticHrefTitle
        , matchReflinkAt
        , matchTag
        , nextRDelim
        )
import Tokens
    exposing
        ( Align
        , BlockToken(..)
        , InlineSlot
        , InlineToken(..)
        , LinkDef
        , ListItem
        , Mode
        , Options
        , TableCell
        , appendNewlineToBlockRaw
        , blockRawLen
        , isBreaks
        , isGfm
        , isPedantic
        , modeOf
        )
import Unicode
    exposing
        ( Src
        , at
        , fromString
        , hasCode
        , isInlinePunctuationPrev
        , isUnicodeAlnumPrev
        , slice
        )


type alias Lexer =
    { options : Options
    , mode : Mode
    , links : Dict String LinkDef
    , inlineSlots : Array InlineSlot
    , inLink : Bool
    , inRawBlock : Bool
    , linkEmitted : Bool
    , linkParenPossible : Bool
    , top : Bool
    }


new : Options -> Lexer
new options =
    { options = options
    , mode = modeOf options
    , links = Dict.empty
    , inlineSlots = Array.empty
    , inLink = False
    , inRawBlock = False
    , linkEmitted = False
    , linkParenPossible = True
    , top = True
    }


lex : String -> Lexer -> ( List BlockToken, Array InlineSlot )
lex rawSrc lexer0 =
    let
        normalized =
            normalizeNewlines rawSrc

        ( revBlocks, lexer1 ) =
            blockTokens normalized [] False lexer0

        slotsFinal =
            drainInlineQueue 0 lexer1
    in
    ( List.reverse revBlocks, slotsFinal )


drainInlineQueue : Int -> Lexer -> Array InlineSlot
drainInlineQueue idx lexer =
    let
        total =
            Array.length lexer.inlineSlots
    in
    if idx - total >= 0 then
        lexer.inlineSlots

    else
        case Array.get idx lexer.inlineSlots of
            Nothing ->
                lexer.inlineSlots

            Just slot ->
                let
                    ( revOut, lexer1 ) =
                        inlineTokens slot.src (List.reverse slot.tokens) lexer

                    updatedSlot =
                        { src = "", tokens = List.reverse revOut }

                    lexer2 =
                        { lexer1 | inlineSlots = Array.set idx updatedSlot lexer1.inlineSlots }
                in
                drainInlineQueue (idx + 1) lexer2


lexInline : String -> Lexer -> List InlineToken
lexInline rawSrc lexer0 =
    let
        ( revTokens, _ ) =
            inlineTokens rawSrc [] lexer0
    in
    List.reverse revTokens


queueInline : String -> Lexer -> ( Int, Lexer )
queueInline s lexer =
    let
        id =
            Array.length lexer.inlineSlots
    in
    ( id
    , { lexer | inlineSlots = Array.push { src = s, tokens = [] } lexer.inlineSlots }
    )


updateLastInlineSrc : String -> Lexer -> Lexer
updateLastInlineSrc newSrc lexer =
    let
        lastIdx =
            Array.length lexer.inlineSlots - 1
    in
    case Array.get lastIdx lexer.inlineSlots of
        Just slot ->
            { lexer | inlineSlots = Array.set lastIdx { slot | src = newSrc } lexer.inlineSlots }

        Nothing ->
            lexer



-- BLOCK LEXER


type BlockStep
    = BlockAdvance Int (List BlockToken) Bool Lexer
    | BlockDone


blockTokens : String -> List BlockToken -> Bool -> Lexer -> ( List BlockToken, Lexer )
blockTokens inputSrc revTokens lastParagraphClipped lexer =
    let
        srcStr =
            if lexer.options.pedantic then
                preprocessPedanticBlock inputSrc

            else
                inputSrc

        src =
            fromString srcStr

        ( finalRev, lexer1 ) =
            blockTokensLoop src 0 -1 revTokens lastParagraphClipped lexer
    in
    ( finalRev, { lexer1 | top = True } )


blockTokensLoop : Src -> Int -> Int -> List BlockToken -> Bool -> Lexer -> ( List BlockToken, Lexer )
blockTokensLoop src pos prevPos revTokens lastParagraphClipped lexer =
    if pos - src.len >= 0 || pos - prevPos == 0 then
        ( revTokens, lexer )

    else
        case stepBlockToken src pos revTokens lastParagraphClipped lexer of
            BlockAdvance nextPos nextRev nextClipped nextLexer ->
                blockTokensLoop src nextPos pos nextRev nextClipped nextLexer

            BlockDone ->
                ( revTokens, lexer )


stepBlockToken : Src -> Int -> List BlockToken -> Bool -> Lexer -> BlockStep
stepBlockToken src pos revTokens lastParagraphClipped lexer =
    -- 1. space
    case matchNewline src.codes pos src.len of
        Just len ->
            let
                nextRev =
                    if len == 1 then
                        case revTokens of
                            last :: rest ->
                                appendNewlineToBlockRaw last :: rest

                            [] ->
                                Space { raw = slice pos (pos + len) src } :: revTokens

                    else
                        Space { raw = slice pos (pos + len) src } :: revTokens
            in
            BlockAdvance (pos + len) nextRev lastParagraphClipped lexer

        Nothing ->
            -- 2. code (indented)
            case tokCode src pos lexer of
                Just ( rawLen, codeText ) ->
                    let
                        consume =
                            minInt rawLen (src.len - pos)

                        ( nextRev, nextLexer ) =
                            case revTokens of
                                (Paragraph p) :: rest ->
                                    let
                                        mergedText =
                                            p.text ++ "\n" ++ codeText
                                    in
                                    ( Paragraph { p | rawLen = p.rawLen + 1 + rawLen, text = mergedText } :: rest
                                    , updateLastInlineSrc mergedText lexer
                                    )

                                (BlockText t) :: rest ->
                                    let
                                        mergedText =
                                            t.text ++ "\n" ++ codeText
                                    in
                                    ( BlockText { t | rawLen = t.rawLen + 1 + rawLen, text = mergedText } :: rest
                                    , updateLastInlineSrc mergedText lexer
                                    )

                                _ ->
                                    ( Code { rawLen = rawLen, lang = Nothing, text = codeText } :: revTokens
                                    , lexer
                                    )
                    in
                    BlockAdvance (pos + consume) nextRev lastParagraphClipped nextLexer

                Nothing ->
                    stepBlockAfterCode src pos revTokens lastParagraphClipped lexer


stepBlockAfterCode : Src -> Int -> List BlockToken -> Bool -> Lexer -> BlockStep
stepBlockAfterCode src pos revTokens lastParagraphClipped lexer =
    -- 3. fences (disabled in pedantic)
    let
        fencesHit =
            if isPedantic lexer.mode then
                Nothing

            else
                tokFences src pos
    in
    case fencesHit of
        Just tok ->
            BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer

        Nothing ->
            -- 4. heading (ATX)
            case tokHeading src pos lexer of
                Just ( tok, lexer1 ) ->
                    BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer1

                Nothing ->
                    -- 5. hr
                    case matchHrAt src.codes pos src.len 3 of
                        Just endIdx ->
                            BlockAdvance endIdx (Hr { rawLen = endIdx - pos } :: revTokens) lastParagraphClipped lexer

                        Nothing ->
                            -- 6. blockquote
                            case tokBlockquote src pos lexer of
                                Just ( tok, lexer1 ) ->
                                    BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer1

                                Nothing ->
                                    -- 7. list
                                    case tokList src pos lexer of
                                        Just ( tok, lexer1 ) ->
                                            BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer1

                                        Nothing ->
                                            -- 8. html
                                            case tokHtml src pos lexer of
                                                Just tok ->
                                                    BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer

                                                Nothing ->
                                                    stepBlockAfterHtml src pos revTokens lastParagraphClipped lexer


stepBlockAfterHtml : Src -> Int -> List BlockToken -> Bool -> Lexer -> BlockStep
stepBlockAfterHtml src pos revTokens lastParagraphClipped lexer =
    -- 9. def
    case tokDef src pos lexer of
        Just ( defRaw, tag, def ) ->
            let
                defLen =
                    String.length defRaw

                consume =
                    minInt defLen (src.len - pos)

                ( nextRev, nextLexer ) =
                    case revTokens of
                        (Paragraph p) :: rest ->
                            let
                                mergedText =
                                    p.text ++ "\n" ++ defRaw
                            in
                            ( Paragraph { p | rawLen = p.rawLen + 1 + defLen, text = mergedText } :: rest
                            , updateLastInlineSrc mergedText lexer
                            )

                        (BlockText t) :: rest ->
                            let
                                mergedText =
                                    t.text ++ "\n" ++ defRaw
                            in
                            ( BlockText { t | rawLen = t.rawLen + 1 + defLen, text = mergedText } :: rest
                            , updateLastInlineSrc mergedText lexer
                            )

                        _ ->
                            if Dict.member tag lexer.links then
                                ( revTokens, lexer )

                            else
                                ( Def { rawLen = defLen } :: revTokens
                                , { lexer | links = Dict.insert tag def lexer.links }
                                )
            in
            BlockAdvance (pos + consume) nextRev lastParagraphClipped nextLexer

        Nothing ->
            -- 10. table (GFM only)
            let
                tableHit =
                    if isGfm lexer.mode then
                        tokTable src pos lexer

                    else
                        Nothing
            in
            case tableHit of
                Just ( tok, lexer1 ) ->
                    BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer1

                Nothing ->
                    -- 11. lheading
                    case tokLHeading src pos lexer of
                        Just ( tok, lexer1 ) ->
                            BlockAdvance (pos + minInt (blockRawLen tok) (src.len - pos)) (tok :: revTokens) lastParagraphClipped lexer1

                        Nothing ->
                            -- 12. top-level paragraph
                            let
                                paraHit =
                                    if lexer.top then
                                        matchParagraphAt src pos lexer.mode False

                                    else
                                        Nothing
                            in
                            case paraHit of
                                Just endIdx ->
                                    let
                                        rawSlice =
                                            slice pos endIdx src

                                        pLen =
                                            endIdx - pos

                                        pText =
                                            if String.endsWith "\n" rawSlice then
                                                String.dropRight 1 rawSlice

                                            else
                                                rawSlice

                                        ( nextRev, nextLexer ) =
                                            if lastParagraphClipped then
                                                case revTokens of
                                                    (Paragraph p) :: rest ->
                                                        let
                                                            mergedText =
                                                                p.text ++ "\n" ++ pText
                                                        in
                                                        ( Paragraph { p | rawLen = p.rawLen + 1 + pLen, text = mergedText } :: rest
                                                        , updateLastInlineSrc mergedText lexer
                                                        )

                                                    _ ->
                                                        let
                                                            ( inlineId, l1 ) =
                                                                queueInline pText lexer
                                                        in
                                                        ( Paragraph { rawLen = pLen, text = pText, inlineId = inlineId } :: revTokens
                                                        , l1
                                                        )

                                            else
                                                let
                                                    ( inlineId, l1 ) =
                                                        queueInline pText lexer
                                                in
                                                ( Paragraph { rawLen = pLen, text = pText, inlineId = inlineId } :: revTokens
                                                , l1
                                                )
                                    in
                                    BlockAdvance endIdx nextRev False nextLexer

                                Nothing ->
                                    -- 13. text (fallback when !top)
                                    let
                                        le =
                                            lineEnd src.codes pos src.len
                                    in
                                    if le - pos > 0 then
                                        let
                                            tSlice =
                                                slice pos le src

                                            tLen =
                                                le - pos

                                            ( nextRev, nextLexer ) =
                                                case revTokens of
                                                    (BlockText t) :: rest ->
                                                        let
                                                            mergedText =
                                                                t.text ++ "\n" ++ tSlice
                                                        in
                                                        ( BlockText { t | rawLen = t.rawLen + 1 + tLen, text = mergedText } :: rest
                                                        , updateLastInlineSrc mergedText lexer
                                                        )

                                                    _ ->
                                                        let
                                                            ( inlineId, l1 ) =
                                                                queueInline tSlice lexer
                                                        in
                                                        ( BlockText { rawLen = tLen, text = tSlice, inlineId = inlineId } :: revTokens
                                                        , l1
                                                        )
                                        in
                                        BlockAdvance le nextRev lastParagraphClipped nextLexer

                                    else
                                        BlockDone


minInt : Int -> Int -> Int
minInt a b =
    if a - b <= 0 then
        a

    else
        b


maxInt : Int -> Int -> Int
maxInt a b =
    if a - b >= 0 then
        a

    else
        b



-- BLOCK TOKENIZERS


tokCode : Src -> Int -> Lexer -> Maybe ( Int, String )
tokCode src pos lexer =
    case matchBlockCode src.codes pos src.len of
        Nothing ->
            Nothing

        Just len ->
            let
                cap0 =
                    slice pos (pos + len) src

                rawSlice =
                    if lexer.options.pedantic then
                        cap0

                    else
                        trimTrailingBlankLines cap0

                text =
                    removeCodeIndent rawSlice
            in
            Just ( String.length rawSlice, text )


tokFences : Src -> Int -> Maybe BlockToken
tokFences src pos =
    case matchFences src.codes pos src.len of
        Nothing ->
            Nothing

        Just m ->
            let
                rawSlice =
                    slice pos m.rawEnd src

                body =
                    slice m.bodyStart m.bodyEnd src

                info =
                    slice m.infoStart m.infoEnd src

                text =
                    indentCodeCompensation rawSlice body

                lang =
                    if String.isEmpty info then
                        Nothing

                    else
                        Just (unescapeAnyPunctuation (String.trim info))
            in
            Just (Code { rawLen = m.rawEnd - pos, lang = lang, text = text })


tokHeading : Src -> Int -> Lexer -> Maybe ( BlockToken, Lexer )
tokHeading src pos lexer =
    case matchHeading src.codes pos src.len (isPedantic lexer.mode) of
        Nothing ->
            Nothing

        Just m ->
            let
                text0 =
                    String.trim (slice m.textStart m.textEnd src)

                text =
                    if String.endsWith "#" text0 then
                        let
                            trimmed =
                                rtrimChar 35 text0
                        in
                        if
                            lexer.options.pedantic
                                || String.isEmpty trimmed
                                || String.endsWith " " trimmed
                                || String.endsWith "\t" trimmed
                        then
                            String.trim trimmed

                        else
                            text0

                    else
                        text0

                ( inlineId, lexer1 ) =
                    queueInline text lexer
            in
            Just
                ( Heading { rawLen = m.rawEnd - pos, depth = m.depth, inlineId = inlineId }
                , lexer1
                )


tokBlockquote : Src -> Int -> Lexer -> Maybe ( BlockToken, Lexer )
tokBlockquote src pos lexer =
    case matchBlockquote src pos of
        Nothing ->
            Nothing

        Just capEnd ->
            let
                trimmed =
                    rtrimChar 10 (slice pos capEnd src)

                lines =
                    String.split "\n" trimmed

                ( raw, revToks, lexer1 ) =
                    parseBlockquoteLoop lines "" [] lexer
            in
            Just ( Blockquote { raw = raw, tokens = List.reverse revToks }, lexer1 )


splitBqChunk : List String -> Bool -> List String -> ( List String, List String )
splitBqChunk lines inBq acc =
    case lines of
        [] ->
            ( List.reverse acc, [] )

        line :: rest ->
            let
                ls =
                    fromString line
            in
            if isBlockquoteAt ls.codes 0 ls.len then
                splitBqChunk rest True (line :: acc)

            else if not inBq then
                splitBqChunk rest False (line :: acc)

            else
                ( List.reverse acc, lines )


parseBlockquoteLoop : List String -> String -> List BlockToken -> Lexer -> ( String, List BlockToken, Lexer )
parseBlockquoteLoop lines raw revTokens lexer =
    if List.isEmpty lines then
        ( raw, revTokens, lexer )

    else
        let
            ( chunkLines, remaining ) =
                splitBqChunk lines False []

            currentRaw =
                String.join "\n" chunkLines

            currentText =
                stripBlockquoteMarkers currentRaw

            nextRaw =
                if String.isEmpty raw then
                    currentRaw

                else
                    raw ++ "\n" ++ currentRaw

            savedTop =
                lexer.top

            ( revTokens1, lexer1 ) =
                blockTokens currentText revTokens True { lexer | top = True }

            lexer2 =
                { lexer1 | top = savedTop }
        in
        if List.isEmpty remaining then
            ( nextRaw, revTokens1, lexer2 )

        else
            case revTokens1 of
                (Code _) :: _ ->
                    ( nextRaw, revTokens1, lexer2 )

                (Heading _) :: _ ->
                    ( nextRaw, revTokens1, lexer2 )

                (Hr _) :: _ ->
                    ( nextRaw, revTokens1, lexer2 )

                (Html _) :: _ ->
                    ( nextRaw, revTokens1, lexer2 )

                (Blockquote oldBq) :: restToks ->
                    let
                        continuation =
                            String.join "\n" remaining

                        newText =
                            oldBq.raw ++ "\n" ++ stripOneBlockquotePrefix continuation

                        newSrc =
                            fromString newText
                    in
                    case tokBlockquote newSrc 0 lexer2 of
                        Just ( newTok, lexer3 ) ->
                            let
                                newRawLen =
                                    minInt (blockRawLen newTok) (String.length newText)

                                leftover0 =
                                    String.dropLeft newRawLen newText

                                leftover =
                                    if String.startsWith "\n" leftover0 then
                                        String.dropLeft 1 leftover0

                                    else
                                        leftover0

                                leftoverCount =
                                    if String.isEmpty leftover then
                                        0

                                    else
                                        List.length (String.split "\n" leftover)

                                consumedCount =
                                    maxInt 0 (List.length remaining - leftoverCount)

                                finalRaw =
                                    if consumedCount > 0 then
                                        nextRaw ++ "\n" ++ String.join "\n" (List.take consumedCount remaining)

                                    else
                                        nextRaw
                            in
                            ( finalRaw, newTok :: restToks, lexer3 )

                        Nothing ->
                            ( nextRaw, revTokens1, lexer2 )

                (ListBlock oldList) :: restToks ->
                    let
                        newText =
                            oldList.raw ++ "\n" ++ String.join "\n" remaining

                        newSrc =
                            fromString newText
                    in
                    case tokList newSrc 0 lexer2 of
                        Just ( newTok, lexer3 ) ->
                            let
                                newRaw =
                                    case newTok of
                                        ListBlock lb ->
                                            lb.raw

                                        _ ->
                                            ""

                                keepLen =
                                    maxInt 0 (String.length nextRaw - String.length oldList.raw)

                                updatedRaw =
                                    String.left keepLen nextRaw ++ newRaw

                                nextLines =
                                    String.split "\n" (String.dropLeft (String.length newRaw) newText)
                            in
                            parseBlockquoteLoop nextLines updatedRaw (newTok :: restToks) lexer3

                        Nothing ->
                            ( nextRaw, revTokens1, lexer2 )

                _ ->
                    parseBlockquoteLoop remaining nextRaw revTokens1 lexer2


tokList : Src -> Int -> Lexer -> Maybe ( BlockToken, Lexer )
tokList src startPos lexer =
    case matchListStart src.codes startPos src.len of
        Nothing ->
            Nothing

        Just firstCap ->
            let
                bull =
                    String.trim (slice startPos firstCap.bulletEnd src)

                isOrdered =
                    String.length bull > 1

                startNum =
                    if isOrdered then
                        parseDecimal (String.dropRight 1 bull)

                    else
                        0

                lastBulletCode =
                    at (firstCap.bulletEnd - 1) src.codes

                markerSpec =
                    if isOrdered then
                        Ordered lastBulletCode

                    else if lexer.options.pedantic then
                        PedanticAnyUnordered

                    else
                        Unordered lastBulletCode

                ( finalPos, loose0, revItems ) =
                    scanListItems src markerSpec lexer.options.pedantic lexer.options.gfm startPos False False []
            in
            case revItems of
                [] ->
                    Nothing

                lastItem :: restRevItems ->
                    let
                        trimmedLast =
                            { lastItem
                                | raw = String.trimRight lastItem.raw
                                , text = String.trimRight lastItem.text
                            }

                        items0 =
                            List.reverse (trimmedLast :: restRevItems)

                        listRaw =
                            String.trimRight (slice startPos finalPos src)

                        -- Pass 1: tokenize items and finalize `loose` from space tokens
                        ( items1, loose1, lexer1 ) =
                            tokenizeListItems items0 loose0 [] lexer

                        -- Pass 2: place task checkboxes using final `loose`
                        ( items2, lexer2 ) =
                            placeTaskCheckboxes items1 loose1 [] lexer1

                        -- Pass 3: promote `BlockText` to `Paragraph` when `loose`
                        items3 =
                            if loose1 then
                                promoteListItemsLoose items2 []

                            else
                                items2
                    in
                    Just
                        ( ListBlock
                            { raw = listRaw
                            , ordered = isOrdered
                            , start = startNum
                            , loose = loose1
                            , items = items3
                            }
                        , lexer2
                        )


scanListItems : Src -> ListMarkerSpec -> Bool -> Bool -> Int -> Bool -> Bool -> List ListItem -> ( Int, Bool, List ListItem )
scanListItems src markerSpec pedantic gfm pos loose endsWithBlank revItems =
    if pos - src.len >= 0 then
        ( pos, loose, revItems )

    else
        case matchListItem src.codes pos src.len markerSpec of
            Nothing ->
                ( pos, loose, revItems )

            Just cap ->
                case matchHrAt src.codes pos src.len 3 of
                    Just _ ->
                        ( pos, loose, revItems )

                    Nothing ->
                        let
                            itemStart =
                                pos

                            bulletIndent =
                                cap.bulletEnd - pos

                            firstLine =
                                slice cap.restStart cap.restEnd src

                            pos1 =
                                cap.rawEnd

                            line0 =
                                if pedantic then
                                    expandTabs bulletIndent firstLine

                                else
                                    expandLeadingSpaceTab bulletIndent firstLine

                            nextLineEnd =
                                lineEnd src.codes pos1 src.len

                            blankLine0 =
                                String.isEmpty (String.trim line0)

                            ( indent, contents0 ) =
                                if pedantic then
                                    ( 2, [ String.trimLeft line0 ] )

                                else if blankLine0 then
                                    ( bulletIndent + 1, [ "" ] )

                                else
                                    let
                                        firstNs =
                                            firstNonSpaceIndex line0

                                        rel =
                                            if firstNs < 0 then
                                                0

                                            else if firstNs > 4 then
                                                1

                                            else
                                                firstNs
                                    in
                                    ( rel + bulletIndent, [ String.dropLeft rel line0 ] )

                            ( itemEndPos, contentsRev ) =
                                if blankLine0 && isBlankLineSpan src.codes pos1 nextLineEnd then
                                    ( minInt (nextLineEnd + 1) src.len, contents0 )

                                else
                                    let
                                        maxInd =
                                            maxInt 0 (minInt 3 (indent - 1))
                                    in
                                    scanListItemLines src pedantic indent maxInd pos1 blankLine0 line0 contents0

                            itemContents =
                                String.join "\n" (List.reverse contentsRev)

                            itemRaw =
                                slice itemStart itemEndPos src

                            ( nextLoose, nextEndsBlank ) =
                                if loose then
                                    ( True, endsWithBlank )

                                else if endsWithBlank then
                                    ( True, True )

                                else if hasDoubleBlankLineEnd src.codes itemStart itemEndPos then
                                    ( False, True )

                                else
                                    ( False, False )

                            isTask =
                                gfm && isListTask itemContents

                            item =
                                { raw = itemRaw
                                , task = isTask
                                , checked = False
                                , loose = False
                                , text = itemContents
                                , tokens = []
                                }
                        in
                        scanListItems src markerSpec pedantic gfm itemEndPos nextLoose nextEndsBlank (item :: revItems)


scanListItemLines : Src -> Bool -> Int -> Int -> Int -> Bool -> String -> List String -> ( Int, List String )
scanListItemLines src pedantic indent maxInd pos blankLine prevLine contentsRev =
    if pos - src.len >= 0 then
        ( pos, contentsRev )

    else
        let
            rawLineEnd =
                lineEnd src.codes pos src.len

            rawLine =
                slice pos rawLineEnd src

            ( mStr, gStr, interrupted ) =
                if pedantic then
                    let
                        pLine =
                            replacePedanticListNesting rawLine

                        pSrc =
                            fromString pLine

                        hit =
                            isListLineInterrupted pSrc.str pSrc.codes 0 pSrc.len maxInd
                    in
                    ( pLine, pLine, hit )

                else
                    let
                        hit =
                            isListLineInterrupted src.str src.codes pos rawLineEnd maxInd
                    in
                    ( rawLine, expandLeadingSpaceTab 0 rawLine, hit )
        in
        if interrupted then
            ( pos, contentsRev )

        else
            let
                mBlank =
                    String.isEmpty (String.trim mStr)

                firstNsG =
                    firstNonSpaceIndex gStr

                dedentable =
                    (firstNsG >= 0 && firstNsG - indent >= 0) || mBlank
            in
            if dedentable then
                let
                    dedented =
                        String.dropLeft indent gStr

                    nextPos =
                        minInt (rawLineEnd + 1) src.len
                in
                scanListItemLines src pedantic indent maxInd nextPos mBlank dedented (dedented :: contentsRev)

            else if blankLine then
                ( pos, contentsRev )

            else
                let
                    lb =
                        fromString prevLine

                    prevStops =
                        isLineIndented4WithTabs lb.codes lb.len
                            || isFencesBegin lb.codes 0 lb.len maxInd
                            || isHeadingBegin lb.codes 0 lb.len maxInd
                            || isJust (matchHrAt lb.codes 0 lb.len maxInd)
                in
                if prevStops then
                    ( pos, contentsRev )

                else
                    let
                        nextPos =
                            minInt (rawLineEnd + 1) src.len

                        nextPrev =
                            String.dropLeft indent gStr
                    in
                    scanListItemLines src pedantic indent maxInd nextPos mBlank nextPrev (mStr :: contentsRev)


isListLineInterrupted : String -> Array Int -> Int -> Int -> Int -> Bool
isListLineInterrupted str codes start end maxInd =
    isFencesBegin codes start end maxInd
        || isHeadingBegin codes start end maxInd
        || isHtmlInterrupt str codes start end maxInd True
        || isBlockquoteBegin codes start end maxInd
        || isNextBullet codes start end maxInd
        || isJust (matchHrAt codes start end maxInd)


tokenizeListItems : List ListItem -> Bool -> List ListItem -> Lexer -> ( List ListItem, Bool, Lexer )
tokenizeListItems items loose acc lexer =
    case items of
        [] ->
            ( List.reverse acc, loose, lexer )

        item :: rest ->
            let
                ( revToks, lexer1 ) =
                    blockTokens item.text [] False { lexer | top = False }

                itemToks =
                    List.reverse revToks

                nextLoose =
                    if loose then
                        True

                    else
                        hasLooseSpacer itemToks

                updatedItem =
                    { item | tokens = itemToks }
            in
            tokenizeListItems rest nextLoose (updatedItem :: acc) lexer1


hasLooseSpacer : List BlockToken -> Bool
hasLooseSpacer tokens =
    case tokens of
        [] ->
            False

        (Space s) :: rest ->
            if hasAnyLine s.raw then
                True

            else
                hasLooseSpacer rest

        _ :: rest ->
            hasLooseSpacer rest


placeTaskCheckboxes : List ListItem -> Bool -> List ListItem -> Lexer -> ( List ListItem, Lexer )
placeTaskCheckboxes items loose acc lexer =
    case items of
        [] ->
            ( List.reverse acc, lexer )

        item :: rest ->
            if not item.task then
                placeTaskCheckboxes rest loose (item :: acc) lexer

            else
                case item.tokens of
                    firstTok :: restToks ->
                        case extractTextOrPara firstTok of
                            Just ( firstSlotId, strippedFirst ) ->
                                let
                                    strippedItemText =
                                        stripListTask item.text

                                    slots1 =
                                        stripLastTaskSlot lexer.inlineSlots (Array.length lexer.inlineSlots - 1)

                                    lexer1 =
                                        { lexer | inlineSlots = slots1 }
                                in
                                case findListTaskCheckbox item.raw of
                                    Just cbSlice ->
                                        let
                                            checked =
                                                cbSlice /= "[ ]"

                                            cbRaw =
                                                cbSlice ++ " "

                                            ( newToks, lexer2 ) =
                                                if loose then
                                                    let
                                                        prefixedFirst =
                                                            prependBlockText cbRaw strippedFirst

                                                        slots2 =
                                                            case Array.get firstSlotId lexer1.inlineSlots of
                                                                Just slot ->
                                                                    Array.set firstSlotId
                                                                        { slot | tokens = InlineCheckbox { raw = cbRaw, checked = checked } :: slot.tokens }
                                                                        lexer1.inlineSlots

                                                                Nothing ->
                                                                    lexer1.inlineSlots
                                                    in
                                                    ( prefixedFirst :: restToks
                                                    , { lexer1 | inlineSlots = slots2 }
                                                    )

                                                else
                                                    ( BlockCheckbox { checked = checked } :: strippedFirst :: restToks
                                                    , lexer1
                                                    )

                                            updatedItem =
                                                { item
                                                    | text = strippedItemText
                                                    , checked = checked
                                                    , tokens = newToks
                                                }
                                        in
                                        placeTaskCheckboxes rest loose (updatedItem :: acc) lexer2

                                    Nothing ->
                                        let
                                            updatedItem =
                                                { item
                                                    | text = strippedItemText
                                                    , tokens = strippedFirst :: restToks
                                                }
                                        in
                                        placeTaskCheckboxes rest loose (updatedItem :: acc) lexer1

                            Nothing ->
                                placeTaskCheckboxes rest loose ({ item | task = False } :: acc) lexer

                    [] ->
                        placeTaskCheckboxes rest loose ({ item | task = False } :: acc) lexer


extractTextOrPara : BlockToken -> Maybe ( Int, BlockToken )
extractTextOrPara tok =
    case tok of
        BlockText t ->
            Just ( t.inlineId, BlockText { t | text = stripListTask t.text } )

        Paragraph p ->
            Just ( p.inlineId, Paragraph { p | text = stripListTask p.text } )

        _ ->
            Nothing


prependBlockText : String -> BlockToken -> BlockToken
prependBlockText prefix tok =
    case tok of
        BlockText t ->
            BlockText { t | text = prefix ++ t.text }

        Paragraph p ->
            Paragraph { p | text = prefix ++ p.text }

        _ ->
            tok


stripLastTaskSlot : Array InlineSlot -> Int -> Array InlineSlot
stripLastTaskSlot slots idx =
    if idx < 0 then
        slots

    else
        case Array.get idx slots of
            Just slot ->
                if isListTask slot.src then
                    Array.set idx { slot | src = stripListTask slot.src } slots

                else
                    stripLastTaskSlot slots (idx - 1)

            Nothing ->
                slots


promoteListItemsLoose : List ListItem -> List ListItem -> List ListItem
promoteListItemsLoose items acc =
    case items of
        [] ->
            List.reverse acc

        item :: rest ->
            let
                promotedToks =
                    promoteBlockTexts item.tokens []
            in
            promoteListItemsLoose rest ({ item | loose = True, tokens = promotedToks } :: acc)


promoteBlockTexts : List BlockToken -> List BlockToken -> List BlockToken
promoteBlockTexts toks acc =
    case toks of
        [] ->
            List.reverse acc

        (BlockText t) :: rest ->
            promoteBlockTexts rest (Paragraph { rawLen = t.rawLen, text = t.text, inlineId = t.inlineId } :: acc)

        tok :: rest ->
            promoteBlockTexts rest (tok :: acc)


tokHtml : Src -> Int -> Lexer -> Maybe BlockToken
tokHtml src pos lexer =
    let
        maybeEnd =
            if isPedantic lexer.mode then
                matchHtmlPedantic src pos

            else
                matchHtmlNormal src pos
    in
    case maybeEnd of
        Nothing ->
            Nothing

        Just endIdx ->
            let
                rawSlice =
                    trimTrailingBlankLines (slice pos endIdx src)
            in
            Just (Html { rawLen = String.length rawSlice, text = rawSlice })


tokDef : Src -> Int -> Lexer -> Maybe ( String, String, LinkDef )
tokDef src pos lexer =
    case matchDef src pos (isPedantic lexer.mode) of
        Nothing ->
            Nothing

        Just m ->
            let
                hrefRaw =
                    slice m.hrefStart m.hrefEnd src

                badBareParens =
                    not (String.startsWith "<" hrefRaw)
                        && findClosingParenSpan src.codes m.hrefStart m.hrefEnd
                        /= -1
            in
            if badBareParens then
                Nothing

            else
                let
                    tag =
                        collapseSpaces (normalizeLabel (slice m.labelStart m.labelEnd src))

                    hrefInner =
                        if String.length hrefRaw >= 2 && String.startsWith "<" hrefRaw && String.endsWith ">" hrefRaw then
                            String.slice 1 (String.length hrefRaw - 1) hrefRaw

                        else
                            hrefRaw

                    href =
                        unescapeAnyPunctuation hrefInner

                    title =
                        case m.title of
                            Just ( ts, te ) ->
                                Just (unescapeAnyPunctuation (slice (ts + 1) (te - 1) src))

                            Nothing ->
                                Nothing
                in
                Just ( slice pos m.rawEnd src, tag, { href = href, title = title } )


tokTable : Src -> Int -> Lexer -> Maybe ( BlockToken, Lexer )
tokTable src pos lexer =
    case matchTableAt src pos of
        Nothing ->
            Nothing

        Just m ->
            if not (hasCode 58 src.codes m.alignStart m.alignEnd || hasCode 124 src.codes m.alignStart m.alignEnd) then
                Nothing

            else
                let
                    headers =
                        splitCells Nothing (slice m.headerStart m.headerEnd src)

                    alignSlice =
                        slice m.alignStart m.alignEnd src

                    alignParts =
                        String.split "|" (trimTableAlignChars alignSlice)

                    headerCount =
                        List.length headers
                in
                if headerCount - List.length alignParts /= 0 then
                    Nothing

                else
                    let
                        aligns =
                            List.foldl (\a acc -> parseTableAlign a :: acc) [] alignParts
                                |> List.reverse

                        ( headerCells, lexer1 ) =
                            buildTableRow True headers aligns [] lexer

                        ( rows, lexer2 ) =
                            case m.body of
                                Nothing ->
                                    ( [], lexer1 )

                                Just ( bs, be ) ->
                                    let
                                        bodyStr =
                                            slice bs be src
                                    in
                                    if String.isEmpty (String.trim bodyStr) then
                                        ( [], lexer1 )

                                    else
                                        let
                                            trimmed =
                                                trimTableRowBlankLine bodyStr

                                            rowStrs =
                                                String.split "\n" trimmed
                                        in
                                        buildTableBodyRows rowStrs headerCount aligns [] lexer1
                    in
                    Just
                        ( Table { rawLen = m.rawEnd - pos, header = headerCells, rows = rows }
                        , lexer2
                        )


buildTableRow : Bool -> List String -> List Align -> List TableCell -> Lexer -> ( List TableCell, Lexer )
buildTableRow isHeader cells aligns acc lexer =
    case ( cells, aligns ) of
        ( c :: restC, a :: restA ) ->
            let
                ( inlineId, lexer1 ) =
                    queueInline c lexer

                cell =
                    { header = isHeader, align = a, inlineId = inlineId }
            in
            buildTableRow isHeader restC restA (cell :: acc) lexer1

        _ ->
            ( List.reverse acc, lexer )


buildTableBodyRows : List String -> Int -> List Align -> List (List TableCell) -> Lexer -> ( List (List TableCell), Lexer )
buildTableBodyRows rowStrs colCount aligns acc lexer =
    case rowStrs of
        [] ->
            ( List.reverse acc, lexer )

        rowStr :: rest ->
            let
                cells =
                    splitCells (Just colCount) rowStr

                ( rowCells, lexer1 ) =
                    buildTableRow False cells aligns [] lexer
            in
            buildTableBodyRows rest colCount aligns (rowCells :: acc) lexer1


tokLHeading : Src -> Int -> Lexer -> Maybe ( BlockToken, Lexer )
tokLHeading src pos lexer =
    case matchLHeadingAt src pos lexer.mode of
        Nothing ->
            Nothing

        Just m ->
            let
                text =
                    String.trim (slice m.textStart m.textEnd src)

                ( inlineId, lexer1 ) =
                    queueInline text lexer
            in
            Just
                ( Heading { rawLen = m.rawEnd - pos, depth = m.depth, inlineId = inlineId }
                , lexer1
                )



-- INLINE LEXER


type InlineStep
    = InlineAdvance Int Int Bool (Maybe (Array Int)) (List InlineToken) Lexer
    | InlineDone


inlineTokens : String -> List InlineToken -> Lexer -> ( List InlineToken, Lexer )
inlineTokens srcStr revTokens lexer =
    let
        prevLinkParen =
            lexer.linkParenPossible

        linkParenNow =
            prevLinkParen && String.contains ")" srcStr

        src =
            fromString srcStr

        ( finalRev, lexer1 ) =
            inlineTokensLoop src 0 -1 -1 False Nothing revTokens { lexer | linkParenPossible = linkParenNow }
    in
    ( finalRev, { lexer1 | linkParenPossible = prevLinkParen } )


inlineTokensLoop : Src -> Int -> Int -> Int -> Bool -> Maybe (Array Int) -> List InlineToken -> Lexer -> ( List InlineToken, Lexer )
inlineTokensLoop src pos prevPos prevChar0 keepPrevChar maskedCache revTokens lexer =
    if pos - src.len >= 0 || pos - prevPos == 0 then
        ( revTokens, lexer )

    else
        let
            prevChar =
                if keepPrevChar then
                    prevChar0

                else
                    -1
        in
        case stepInlineToken src pos prevChar maskedCache revTokens lexer of
            InlineAdvance nextPos nextPrev nextKeep nextMasked nextRev nextLexer ->
                inlineTokensLoop src nextPos pos nextPrev nextKeep nextMasked nextRev nextLexer

            InlineDone ->
                ( revTokens, lexer )


stepInlineToken : Src -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineToken src pos prevChar maskedCache revTokens lexer =
    let
        b0 =
            at pos src.codes
    in
    -- 1. escape
    if b0 == 92 then
        case matchEscape src.codes pos src.len of
            Just chCode ->
                let
                    tok =
                        Escape { text = String.fromChar (Char.fromCode chCode) }
                in
                InlineAdvance (pos + 2) prevChar False maskedCache (tok :: revTokens) lexer

            Nothing ->
                stepInlineAfterEscape src pos b0 prevChar maskedCache revTokens lexer

    else
        stepInlineAfterEscape src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterEscape : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterEscape src pos b0 prevChar maskedCache revTokens lexer =
    -- 2. tag
    if b0 == 60 then
        case matchTag src.codes pos src.len of
            Just endIdx ->
                let
                    nextInLink =
                        if not lexer.inLink && isStartATag src.codes pos endIdx then
                            True

                        else if lexer.inLink && isEndATag src.codes pos endIdx then
                            False

                        else
                            lexer.inLink

                    nextInRaw =
                        if not lexer.inRawBlock && isStartPreScriptTag src.codes pos endIdx then
                            True

                        else if lexer.inRawBlock && isEndPreScriptTag src.codes pos endIdx then
                            False

                        else
                            lexer.inRawBlock

                    tok =
                        InlineHtml { text = slice pos endIdx src }

                    lexer1 =
                        { lexer | inLink = nextInLink, inRawBlock = nextInRaw }
                in
                InlineAdvance endIdx prevChar False maskedCache (tok :: revTokens) lexer1

            Nothing ->
                stepInlineAfterTag src pos b0 prevChar maskedCache revTokens lexer

    else
        stepInlineAfterTag src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterTag : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterTag src pos b0 prevChar maskedCache revTokens lexer =
    -- 3. link
    if (b0 == 91 || b0 == 33) && lexer.linkParenPossible then
        case tokLink src pos lexer of
            Just ( tok, endIdx, lexer1 ) ->
                InlineAdvance endIdx prevChar False maskedCache (tok :: revTokens) lexer1

            Nothing ->
                stepInlineAfterLink src pos b0 prevChar maskedCache revTokens lexer

    else
        stepInlineAfterLink src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterLink : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterLink src pos b0 prevChar maskedCache revTokens lexer =
    -- 4. reflink / nolink
    if b0 == 91 || b0 == 33 then
        case tokReflink src pos lexer of
            Just ( tok, endIdx, lexer1 ) ->
                let
                    nextRev =
                        case ( tok, revTokens ) of
                            ( InlineText t, (InlineText last) :: rest ) ->
                                InlineText { last | text = last.text ++ t.text } :: rest

                            _ ->
                                tok :: revTokens
                in
                InlineAdvance endIdx prevChar False maskedCache nextRev lexer1

            Nothing ->
                stepInlineAfterReflink src pos b0 prevChar maskedCache revTokens lexer

    else
        stepInlineAfterReflink src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterReflink : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterReflink src pos b0 prevChar maskedCache revTokens lexer =
    -- 5. em & strong
    if b0 == 42 || b0 == 95 then
        let
            masked =
                case maskedCache of
                    Just m ->
                        m

                    Nothing ->
                        buildMaskedCodes src lexer.links (isPedantic lexer.mode)

            nextCache =
                Just masked
        in
        case tokEmStrong src pos masked prevChar lexer of
            Just ( tok, endIdx, lexer1 ) ->
                InlineAdvance endIdx prevChar False nextCache (tok :: revTokens) lexer1

            Nothing ->
                stepInlineAfterEmStrong src pos b0 prevChar nextCache revTokens lexer

    else
        stepInlineAfterEmStrong src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterEmStrong : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterEmStrong src pos b0 prevChar maskedCache revTokens lexer =
    -- 6. codespan
    if b0 == 96 then
        case matchInlineCodeAt src.codes pos src.len of
            Just ( endIdx, k ) ->
                let
                    rawInner =
                        slice (pos + k) (endIdx - k) src

                    withSpaces =
                        if String.contains "\n" rawInner then
                            String.replace "\n" " " rawInner

                        else
                            rawInner

                    hasNonSpace =
                        hasNonAsciiSpace withSpaces

                    finalText =
                        if hasNonSpace && String.length withSpaces >= 2 && String.startsWith " " withSpaces && String.endsWith " " withSpaces then
                            String.slice 1 (String.length withSpaces - 1) withSpaces

                        else
                            withSpaces

                    tok =
                        Codespan { text = finalText }
                in
                InlineAdvance endIdx prevChar False maskedCache (tok :: revTokens) lexer

            Nothing ->
                stepInlineAfterCodespan src pos b0 prevChar maskedCache revTokens lexer

    else
        stepInlineAfterCodespan src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterCodespan : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterCodespan src pos b0 prevChar maskedCache revTokens lexer =
    -- 7. br
    if b0 == 32 || b0 == 92 || (b0 == 10 && isBreaks lexer.mode) then
        case matchBr src.codes pos src.len (isBreaks lexer.mode) of
            Just endIdx ->
                InlineAdvance endIdx prevChar False maskedCache (Br :: revTokens) lexer

            Nothing ->
                stepInlineAfterBr src pos b0 prevChar maskedCache revTokens lexer

    else
        stepInlineAfterBr src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterBr : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterBr src pos b0 prevChar maskedCache revTokens lexer =
    -- 8. del (GFM / breaks only)
    if b0 == 126 && isGfm lexer.mode then
        let
            masked =
                case maskedCache of
                    Just m ->
                        m

                    Nothing ->
                        buildMaskedCodes src lexer.links (isPedantic lexer.mode)

            nextCache =
                Just masked
        in
        case tokDel src pos masked prevChar lexer of
            Just ( tok, endIdx, lexer1 ) ->
                InlineAdvance endIdx prevChar False nextCache (tok :: revTokens) lexer1

            Nothing ->
                stepInlineAfterDel src pos b0 prevChar nextCache revTokens lexer

    else
        stepInlineAfterDel src pos b0 prevChar maskedCache revTokens lexer


stepInlineAfterDel : Src -> Int -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterDel src pos b0 prevChar maskedCache revTokens lexer =
    -- 9. autolink
    if b0 == 60 then
        case matchAutolink src.codes pos src.len of
            Just ( endIdx, isEmail ) ->
                let
                    inner =
                        slice (pos + 1) (endIdx - 1) src

                    href =
                        if isEmail then
                            "mailto:" ++ inner

                        else
                            inner

                    tok =
                        Link { href = href, title = Nothing, text = inner, autolink = True, tokens = [] }
                in
                InlineAdvance endIdx prevChar False maskedCache (tok :: revTokens) lexer

            Nothing ->
                stepInlineAfterAutolink src pos prevChar maskedCache revTokens lexer

    else
        stepInlineAfterAutolink src pos prevChar maskedCache revTokens lexer


stepInlineAfterAutolink : Src -> Int -> Int -> Maybe (Array Int) -> List InlineToken -> Lexer -> InlineStep
stepInlineAfterAutolink src pos prevChar maskedCache revTokens lexer =
    -- 10. url (GFM / breaks only, and !inLink)
    let
        urlHit =
            if isGfm lexer.mode && not lexer.inLink then
                matchGfmUrl src.codes pos src.len

            else
                Nothing
    in
    case urlHit of
        Just m ->
            let
                raw =
                    slice pos m.rawEnd src

                href =
                    if m.isBareEmail then
                        "mailto:" ++ raw

                    else if m.isWww then
                        "http://" ++ raw

                    else
                        raw

                tok =
                    Link { href = href, title = Nothing, text = raw, autolink = True, tokens = [] }
            in
            InlineAdvance m.rawEnd prevChar False maskedCache (tok :: revTokens) lexer

        Nothing ->
            -- 11. inlineText
            case matchInlineText src.codes pos src.len lexer.mode of
                Just endIdx ->
                    let
                        rawSlice =
                            slice pos endIdx src

                        lastCode =
                            at (endIdx - 1) src.codes

                        nextPrevChar =
                            if lastCode == 95 then
                                prevChar

                            else
                                lastCode

                        escaped =
                            lexer.inRawBlock

                        chunk =
                            if escaped then
                                rawSlice

                            else
                                decodeNumericCharacterReferences rawSlice

                        nextRev =
                            case revTokens of
                                (InlineText last) :: rest ->
                                    InlineText { last | text = last.text ++ chunk } :: rest

                                _ ->
                                    InlineText { text = chunk, escaped = escaped } :: revTokens
                    in
                    InlineAdvance endIdx nextPrevChar True maskedCache nextRev lexer

                Nothing ->
                    InlineDone



-- INLINE TOKENIZERS


outputLink : Bool -> String -> String -> Maybe String -> Lexer -> Maybe ( InlineToken, Lexer )
outputLink isImage rawLabel href maybeTitle lexer =
    let
        text =
            unescapeLinkBrackets rawLabel

        outerInLink =
            lexer.inLink

        outerLinkEmitted =
            lexer.linkEmitted

        ( revChildren, lexer1 ) =
            inlineTokens text [] { lexer | inLink = True, linkEmitted = False }

        textHasLink =
            lexer1.linkEmitted

        lexer2 =
            { lexer1 | linkEmitted = outerLinkEmitted, inLink = outerInLink }

        title =
            case maybeTitle of
                Just t ->
                    if String.isEmpty t then
                        Nothing

                    else
                        Just t

                Nothing ->
                    Nothing

        children =
            List.reverse revChildren
    in
    if not isImage then
        if textHasLink then
            Nothing

        else
            Just
                ( Link { href = href, title = title, text = text, autolink = False, tokens = children }
                , { lexer2 | linkEmitted = True }
                )

    else
        Just
            ( Image { href = href, title = title, text = text, tokens = children }
            , lexer2
            )


tokLink : Src -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
tokLink src pos lexer =
    case matchInlineLink src.codes pos src.len (isPedantic lexer.mode) of
        Nothing ->
            Nothing

        Just m ->
            if not lexer.options.pedantic && isLabelEndInsideToken src.codes src.len m.labelStart m.labelEnd then
                Nothing

            else
                let
                    rawSlice0 =
                        slice pos m.rawEnd src

                    labelStr =
                        slice m.labelStart m.labelEnd src

                    capHref0 =
                        slice m.hrefStart m.hrefEnd src

                    capTitle0 =
                        case m.title of
                            Just ( ts, te ) ->
                                Just (slice ts te src)

                            Nothing ->
                                Nothing

                    trimmedUrl =
                        String.trim capHref0

                    checked =
                        if not lexer.options.pedantic && String.startsWith "<" trimmedUrl then
                            if not (String.endsWith ">" trimmedUrl) then
                                Nothing

                            else
                                let
                                    withoutGt =
                                        String.dropRight 1 trimmedUrl

                                    rtrimSlash =
                                        rtrimChar 92 withoutGt
                                in
                                if modBy 2 (String.length trimmedUrl - String.length rtrimSlash) == 0 then
                                    Nothing

                                else
                                    Just ( rawSlice0, capHref0, capTitle0 )

                        else
                            let
                                lastParen =
                                    findClosingParenSpan src.codes m.hrefStart m.hrefEnd
                            in
                            if lastParen == -2 then
                                Nothing

                            else if lastParen > -1 then
                                let
                                    startU16 =
                                        if m.isImage then
                                            5

                                        else
                                            4

                                    linkLen =
                                        startU16 + (m.labelEnd - m.labelStart) + lastParen

                                    capHref1 =
                                        String.slice 0 lastParen capHref0

                                    rawSlice1 =
                                        String.trim (String.slice 0 linkLen rawSlice0)
                                in
                                Just ( rawSlice1, capHref1, Nothing )

                            else
                                Just ( rawSlice0, capHref0, capTitle0 )
                in
                case checked of
                    Nothing ->
                        Nothing

                    Just ( rawSlice, capHref, capTitle ) ->
                        let
                            ( href0, title0 ) =
                                if lexer.options.pedantic then
                                    case matchPedanticHrefTitle capHref of
                                        Just ( h, t ) ->
                                            ( h, t )

                                        Nothing ->
                                            ( capHref, "" )

                                else
                                    case capTitle of
                                        Just t ->
                                            if String.length t >= 2 then
                                                ( capHref, String.slice 1 (String.length t - 1) t )

                                            else
                                                ( capHref, "" )

                                        Nothing ->
                                            ( capHref, "" )

                            href1 =
                                String.trim href0

                            href2 =
                                if String.startsWith "<" href1 then
                                    if lexer.options.pedantic && not (String.endsWith ">" trimmedUrl) then
                                        String.dropLeft 1 href1

                                    else if String.length href1 >= 2 then
                                        String.slice 1 (String.length href1 - 1) href1

                                    else
                                        ""

                                else
                                    href1

                            finalHref =
                                if String.isEmpty href2 then
                                    ""

                                else
                                    unescapeAnyPunctuation href2

                            finalTitle =
                                if String.isEmpty title0 then
                                    Nothing

                                else
                                    Just (unescapeAnyPunctuation title0)

                            endIdx =
                                pos + String.length rawSlice
                        in
                        case outputLink m.isImage labelStr finalHref finalTitle lexer of
                            Just ( tok, lexer1 ) ->
                                Just ( tok, endIdx, lexer1 )

                            Nothing ->
                                Nothing


tokReflink : Src -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
tokReflink src pos lexer =
    case matchReflinkAt src.codes pos src.len (isPedantic lexer.mode) False of
        Nothing ->
            Nothing

        Just m ->
            if not lexer.options.pedantic && isLabelEndInsideToken src.codes src.len m.labelStart m.labelEnd then
                Nothing

            else
                let
                    labelStr =
                        slice m.labelStart m.labelEnd src

                    lookupRaw =
                        case m.refSpan of
                            Just ( rs, re ) ->
                                slice rs re src

                            Nothing ->
                                labelStr

                    key =
                        normalizeLabel (collapseSpaces lookupRaw)
                in
                case Dict.get key lexer.links of
                    Nothing ->
                        Just
                            ( InlineText { text = slice pos (pos + 1) src, escaped = False }
                            , pos + 1
                            , lexer
                            )

                    Just def ->
                        case outputLink m.isImage labelStr def.href def.title lexer of
                            Just ( tok, lexer1 ) ->
                                Just ( tok, m.rawEnd, lexer1 )

                            Nothing ->
                                Nothing


type alias EmStrongCtx =
    { src : Src
    , pos : Int
    , delimChar : Int
    , lLen : Int
    , baseOffset : Int
    , fullMasked : Array Int
    , midRun : Bool
    }


tokEmStrong : Src -> Int -> Array Int -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
tokEmStrong src pos fullMasked prevChar lexer =
    case matchEmStrongL src.codes pos src.len lexer.mode of
        Nothing ->
            Nothing

        Just lm ->
            if lm.isUndNonPunct && isUnicodeAlnumPrev prevChar then
                Nothing

            else if not lm.nextIsPunct || prevChar < 0 || isInlinePunctuationPrev prevChar then
                let
                    ctx =
                        { src = src
                        , pos = pos
                        , delimChar = lm.delimChar
                        , lLen = lm.lLen
                        , baseOffset = pos + lm.lLen
                        , fullMasked = fullMasked
                        , midRun = prevChar - lm.delimChar == 0
                        }
                in
                scanEmStrongRDelim ctx 0 lm.lLen 0 lexer

            else
                Nothing


scanEmStrongRDelim : EmStrongCtx -> Int -> Int -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
scanEmStrongRDelim ctx scanPos delimTotal midDelimTotal lexer =
    case nextRDelim ctx.fullMasked ctx.baseOffset ctx.src.len scanPos ctx.delimChar lexer.mode False of
        Nothing ->
            Nothing

        Just hit ->
            let
                rLen =
                    hit.delimLen
            in
            case hit.kind of
                LeftOnly ->
                    scanEmStrongRDelim ctx hit.nextPos (delimTotal + rLen) midDelimTotal lexer

                Both ->
                    if modBy 3 ctx.lLen /= 0 && modBy 3 (ctx.lLen + rLen) == 0 then
                        scanEmStrongRDelim ctx hit.nextPos delimTotal (midDelimTotal + rLen) lexer

                    else if ctx.midRun then
                        Nothing

                    else
                        let
                            newDelimTotal =
                                delimTotal - rLen
                        in
                        if newDelimTotal > 0 then
                            scanEmStrongRDelim ctx hit.nextPos newDelimTotal midDelimTotal lexer

                        else
                            let
                                effRLen =
                                    maxInt 0 (minInt rLen (rLen + newDelimTotal + midDelimTotal))

                                rawEnd =
                                    ctx.baseOffset + hit.delimStart + effRLen
                            in
                            emitEmStrong ctx.src ctx.pos ctx.lLen effRLen rawEnd lexer

                RightOnly ->
                    let
                        newDelimTotal =
                            delimTotal - rLen
                    in
                    if newDelimTotal > 0 then
                        scanEmStrongRDelim ctx hit.nextPos newDelimTotal midDelimTotal lexer

                    else
                        let
                            effRLen =
                                maxInt 0 (minInt rLen (rLen + newDelimTotal + midDelimTotal))

                            rawEnd =
                                ctx.baseOffset + hit.delimStart + effRLen
                        in
                        emitEmStrong ctx.src ctx.pos ctx.lLen effRLen rawEnd lexer


emitEmStrong : Src -> Int -> Int -> Int -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
emitEmStrong src pos lLen effRLen rawEnd lexer =
    if modBy 2 (minInt lLen effRLen) == 1 then
        let
            inner =
                slice (pos + 1) (rawEnd - 1) src

            ( revChildren, lexer1 ) =
                inlineTokens inner [] lexer
        in
        Just ( Em { text = inner, tokens = List.reverse revChildren }, rawEnd, lexer1 )

    else
        let
            inner =
                slice (pos + 2) (rawEnd - 2) src

            ( revChildren, lexer1 ) =
                inlineTokens inner [] lexer
        in
        Just ( Strong { text = inner, tokens = List.reverse revChildren }, rawEnd, lexer1 )


tokDel : Src -> Int -> Array Int -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
tokDel src pos fullMasked prevChar lexer =
    case matchDelL src.codes pos src.len of
        Nothing ->
            Nothing

        Just ( lLen, nextIsPunct ) ->
            if not nextIsPunct || prevChar < 0 || isInlinePunctuationPrev prevChar then
                scanDelRDelim src pos lLen (pos + lLen) fullMasked 0 lLen lexer

            else
                Nothing


scanDelRDelim : Src -> Int -> Int -> Int -> Array Int -> Int -> Int -> Lexer -> Maybe ( InlineToken, Int, Lexer )
scanDelRDelim src pos lLen baseOffset fullMasked scanPos delimTotal lexer =
    case nextRDelim fullMasked baseOffset src.len scanPos 126 lexer.mode True of
        Nothing ->
            Nothing

        Just hit ->
            let
                rLen =
                    hit.delimLen
            in
            if rLen - lLen /= 0 then
                scanDelRDelim src pos lLen baseOffset fullMasked hit.nextPos delimTotal lexer

            else
                case hit.kind of
                    LeftOnly ->
                        scanDelRDelim src pos lLen baseOffset fullMasked hit.nextPos (delimTotal + rLen) lexer

                    _ ->
                        let
                            newDelimTotal =
                                delimTotal - rLen
                        in
                        if newDelimTotal > 0 then
                            scanDelRDelim src pos lLen baseOffset fullMasked hit.nextPos newDelimTotal lexer

                        else
                            let
                                effRLen =
                                    maxInt 0 (minInt rLen (rLen + newDelimTotal))

                                rawEnd =
                                    baseOffset + hit.delimStart + effRLen

                                inner =
                                    slice (pos + lLen) (rawEnd - lLen) src

                                ( revChildren, lexer1 ) =
                                    inlineTokens inner [] lexer
                            in
                            Just ( Del { text = inner, tokens = List.reverse revChildren }, rawEnd, lexer1 )



-- SMALL HELPERS


parseDecimal : String -> Int
parseDecimal s =
    String.foldl (\ch acc -> acc * 10 + (Char.toCode ch - 48)) 0 s


expandLeadingSpaceTab : Int -> String -> String
expandLeadingSpaceTab indent line =
    if not (String.contains "\t" line) then
        line

    else
        let
            src =
                fromString line

            end =
                skipSpacesTabsStart src.codes 0 src.len
        in
        if end == 0 || not (hasCode 9 src.codes 0 end) then
            line

        else
            expandTabs indent (String.slice 0 end line) ++ String.dropLeft end line


skipSpacesTabsStart : Array Int -> Int -> Int -> Int
skipSpacesTabsStart codes i len =
    if i - len < 0 then
        let
            b =
                at i codes
        in
        if b == 32 || b == 9 then
            skipSpacesTabsStart codes (i + 1) len

        else
            i

    else
        i


firstNonSpaceIndex : String -> Int
firstNonSpaceIndex s =
    let
        src =
            fromString s
    in
    findFirstNonSpace src.codes 0 src.len


findFirstNonSpace : Array Int -> Int -> Int -> Int
findFirstNonSpace codes i len =
    if i - len >= 0 then
        -1

    else if at i codes /= 32 then
        i

    else
        findFirstNonSpace codes (i + 1) len


hasNonAsciiSpace : String -> Bool
hasNonAsciiSpace s =
    let
        src =
            fromString s
    in
    findFirstNonSpace src.codes 0 src.len >= 0


isLineIndented4WithTabs : Array Int -> Int -> Bool
isLineIndented4WithTabs codes len =
    isLineIndented4Loop codes len 0 0


isLineIndented4Loop : Array Int -> Int -> Int -> Int -> Bool
isLineIndented4Loop codes len i spaces =
    if i - len >= 0 then
        False

    else
        let
            b =
                at i codes
        in
        if b == 32 then
            isLineIndented4Loop codes len (i + 1) (spaces + 1)

        else if b == 9 then
            isLineIndented4Loop codes len (i + 1) (spaces + 4)

        else
            spaces >= 4


isJust : Maybe a -> Bool
isJust m =
    case m of
        Just _ ->
            True

        Nothing ->
            False
