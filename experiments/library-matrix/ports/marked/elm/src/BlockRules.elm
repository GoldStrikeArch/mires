module BlockRules exposing
    ( DefSpan
    , FencesSpan
    , HeadingSpan
    , LHeadingSpan
    , ListBulletSpan
    , ListMarkerSpec(..)
    , TableSpan
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
    , matchBlockLabel
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

{-| Deterministic native matchers for all block-level rules in `marked@18.1.1` (`src/rules.ts`).
All loops are tail-recursive and operate directly on UTF-16-aligned `Src` / `Array Int` cursors.
-}

import Array exposing (Array)
import Tokens exposing (Align(..), Mode(..), isGfm, isPedantic)
import Unicode
    exposing
        ( Src
        , at
        , charWidth
        , fromString
        , hasCode
        , isAllJsSpace
        , isAsciiAlnum
        , isAsciiAlpha
        , isAsciiDigit
        , isAsciiWord
        , isJsSpace
        , startsWithAsciiCi
        )


lineEnd : Array Int -> Int -> Int -> Int
lineEnd codes pos end =
    if pos - end >= 0 || at pos codes == 10 then
        pos

    else
        lineEnd codes (pos + 1) end


leadingSpacesUpTo : Array Int -> Int -> Int -> Int -> Int
leadingSpacesUpTo codes pos end maxSpaces =
    leadingSpacesLoop codes pos end maxSpaces 0


leadingSpacesLoop : Array Int -> Int -> Int -> Int -> Int -> Int
leadingSpacesLoop codes pos end maxSpaces c =
    if c - maxSpaces < 0 && pos + c - end < 0 && at (pos + c) codes == 32 then
        leadingSpacesLoop codes pos end maxSpaces (c + 1)

    else
        c



-- HTML TAG TABLES


isHtmlBlockTagLower : String -> Bool
isHtmlBlockTagLower key =
    case key of
        "address" ->
            True

        "article" ->
            True

        "aside" ->
            True

        "base" ->
            True

        "basefont" ->
            True

        "blockquote" ->
            True

        "body" ->
            True

        "caption" ->
            True

        "center" ->
            True

        "col" ->
            True

        "colgroup" ->
            True

        "dd" ->
            True

        "details" ->
            True

        "dialog" ->
            True

        "dir" ->
            True

        "div" ->
            True

        "dl" ->
            True

        "dt" ->
            True

        "fieldset" ->
            True

        "figcaption" ->
            True

        "figure" ->
            True

        "footer" ->
            True

        "form" ->
            True

        "frame" ->
            True

        "frameset" ->
            True

        "h1" ->
            True

        "h2" ->
            True

        "h3" ->
            True

        "h4" ->
            True

        "h5" ->
            True

        "h6" ->
            True

        "head" ->
            True

        "header" ->
            True

        "hr" ->
            True

        "html" ->
            True

        "iframe" ->
            True

        "legend" ->
            True

        "li" ->
            True

        "link" ->
            True

        "main" ->
            True

        "menu" ->
            True

        "menuitem" ->
            True

        "meta" ->
            True

        "nav" ->
            True

        "noframes" ->
            True

        "ol" ->
            True

        "optgroup" ->
            True

        "option" ->
            True

        "p" ->
            True

        "param" ->
            True

        "search" ->
            True

        "section" ->
            True

        "summary" ->
            True

        "table" ->
            True

        "tbody" ->
            True

        "td" ->
            True

        "tfoot" ->
            True

        "th" ->
            True

        "thead" ->
            True

        "title" ->
            True

        "tr" ->
            True

        "track" ->
            True

        "ul" ->
            True

        _ ->
            False


isHtmlBlockTagStr : String -> Int -> Int -> Bool -> Bool
isHtmlBlockTagStr str ns ne caseInsensitive =
    let
        len =
            ne - ns
    in
    if len <= 0 || len > 10 then
        False

    else
        let
            raw =
                String.slice ns ne str
        in
        isHtmlBlockTagLower
            (if caseInsensitive then
                String.toLower raw

             else
                raw
            )


isHtmlType1TagLower : String -> Bool
isHtmlType1TagLower key =
    case key of
        "pre" ->
            True

        "style" ->
            True

        "script" ->
            True

        "textarea" ->
            True

        _ ->
            False


isHtmlType1TagStr : String -> Int -> Int -> Bool -> Bool
isHtmlType1TagStr str ns ne caseInsensitive =
    let
        len =
            ne - ns
    in
    if len < 3 || len > 8 then
        False

    else
        let
            raw =
                String.slice ns ne str
        in
        isHtmlType1TagLower
            (if caseInsensitive then
                String.toLower raw

             else
                raw
            )


isPedanticInlineTag : String -> Bool
isPedanticInlineTag key =
    case key of
        "a" ->
            True

        "abbr" ->
            True

        "b" ->
            True

        "bdi" ->
            True

        "bdo" ->
            True

        "br" ->
            True

        "cite" ->
            True

        "code" ->
            True

        "data" ->
            True

        "del" ->
            True

        "dfn" ->
            True

        "em" ->
            True

        "i" ->
            True

        "img" ->
            True

        "ins" ->
            True

        "kbd" ->
            True

        "mark" ->
            True

        "q" ->
            True

        "rp" ->
            True

        "rt" ->
            True

        "ruby" ->
            True

        "s" ->
            True

        "samp" ->
            True

        "small" ->
            True

        "span" ->
            True

        "strong" ->
            True

        "sub" ->
            True

        "sup" ->
            True

        "time" ->
            True

        "u" ->
            True

        "var" ->
            True

        "wbr" ->
            True

        _ ->
            False



-- 1. NEWLINE / SPACE


{-| Matches `block.newline`: `/^(?:[ \t]*(?:\n|$))+/`. Returns length from `pos` if > 0.
-}
matchNewline : Array Int -> Int -> Int -> Maybe Int
matchNewline codes pos end =
    let
        matchedEnd =
            matchNewlineLoop codes end pos pos
    in
    if matchedEnd - pos > 0 then
        Just (matchedEnd - pos)

    else
        Nothing


matchNewlineLoop : Array Int -> Int -> Int -> Int -> Int
matchNewlineLoop codes end i matchedEnd =
    if i - end >= 0 then
        matchedEnd

    else
        let
            j =
                skipSpacesTabs codes i end
        in
        if j - end < 0 && at j codes == 10 then
            matchNewlineLoop codes end (j + 1) (j + 1)

        else if j - end == 0 then
            if j - i > 0 then
                end

            else
                matchedEnd

        else
            matchedEnd


skipSpacesTabs : Array Int -> Int -> Int -> Int
skipSpacesTabs codes i end =
    if i - end < 0 then
        let
            c =
                at i codes
        in
        if c == 32 || c == 9 then
            skipSpacesTabs codes (i + 1) end

        else
            i

    else
        i


skipSpaces : Array Int -> Int -> Int -> Int
skipSpaces codes i end =
    if i - end < 0 && at i codes == 32 then
        skipSpaces codes (i + 1) end

    else
        i


skipNewlines : Array Int -> Int -> Int -> Int
skipNewlines codes i end =
    if i - end < 0 && at i codes == 10 then
        skipNewlines codes (i + 1) end

    else
        i



-- 2. INDENTED CODE


matchCodeIndent : Array Int -> Int -> Int -> Maybe Int
matchCodeIndent codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 4
    in
    if sp == 4 then
        Just (pos + 4)

    else if pos + sp - end < 0 && at (pos + sp) codes == 9 then
        Just (pos + sp + 1)

    else
        Nothing


{-| Matches `block.code` at `codes[pos..end)`, returning length from `pos`.
-}
matchBlockCode : Array Int -> Int -> Int -> Maybe Int
matchBlockCode codes pos end =
    matchBlockCodeLoop codes end pos pos False


matchBlockCodeLoop : Array Int -> Int -> Int -> Int -> Bool -> Maybe Int
matchBlockCodeLoop codes end startPos i any =
    if i - end >= 0 then
        if any then
            Just (i - startPos)

        else
            Nothing

    else
        case matchCodeIndent codes i end of
            Nothing ->
                if any then
                    Just (i - startPos)

                else
                    Nothing

            Just afterIndent ->
                if afterIndent - end >= 0 || at afterIndent codes == 10 then
                    if any then
                        Just (i - startPos)

                    else
                        Nothing

                else
                    let
                        le =
                            lineEnd codes afterIndent end
                    in
                    if le - end < 0 && at le codes == 10 then
                        let
                            nextI =
                                skipBlankLinesAfterNl codes end (le + 1)
                        in
                        matchBlockCodeLoop codes end startPos nextI True

                    else
                        Just (le - startPos)


skipBlankLinesAfterNl : Array Int -> Int -> Int -> Int
skipBlankLinesAfterNl codes end i =
    let
        j =
            skipSpacesTabs codes i end
    in
    if j - end < 0 && at j codes == 10 then
        skipBlankLinesAfterNl codes end (j + 1)

    else if j - end == 0 then
        end

    else
        i



-- 3. FENCES


type alias FencesSpan =
    { rawEnd : Int
    , infoStart : Int
    , infoEnd : Int
    , bodyStart : Int
    , bodyEnd : Int
    }


matchClosingFence : Array Int -> Int -> Int -> Int -> Int -> Maybe Int
matchClosingFence codes pos end fenceChar fenceLen =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        i0 =
            pos + sp

        i1 =
            skipSameChar codes i0 end fenceChar
    in
    if (i1 - i0) - fenceLen < 0 then
        Nothing

    else
        let
            i2 =
                skipFenceTrailingChars codes i1 end

            i3 =
                skipSpaces codes i2 end
        in
        if i3 - end == 0 || at i3 codes == 10 then
            Just i3

        else
            Nothing


skipSameChar : Array Int -> Int -> Int -> Int -> Int
skipSameChar codes i end ch =
    if i - end < 0 && at i codes - ch == 0 then
        skipSameChar codes (i + 1) end ch

    else
        i


skipFenceTrailingChars : Array Int -> Int -> Int -> Int
skipFenceTrailingChars codes i end =
    if i - end < 0 then
        let
            c =
                at i codes
        in
        if c == 126 || c == 96 then
            skipFenceTrailingChars codes (i + 1) end

        else
            i

    else
        i


matchFences : Array Int -> Int -> Int -> Maybe FencesSpan
matchFences codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        fStart =
            pos + sp
    in
    if fStart - end >= 0 then
        Nothing

    else
        let
            ch =
                at fStart codes
        in
        if ch /= 96 && ch /= 126 then
            Nothing

        else
            let
                fenceEnd =
                    skipSameChar codes fStart end ch

                fenceLen =
                    fenceEnd - fStart
            in
            if fenceLen < 3 then
                Nothing

            else
                let
                    openLineEnd =
                        lineEnd codes fenceEnd end
                in
                if ch == 96 && hasCode 96 codes fenceEnd openLineEnd then
                    Nothing

                else if openLineEnd - end == 0 || openLineEnd + 1 - end == 0 then
                    Just
                        { rawEnd = end
                        , infoStart = fenceEnd
                        , infoEnd = openLineEnd
                        , bodyStart = end
                        , bodyEnd = end
                        }

                else
                    let
                        bodyStart =
                            openLineEnd + 1
                    in
                    case matchClosingFence codes bodyStart end ch fenceLen of
                        Just closeEnd ->
                            Just
                                { rawEnd = closeEnd
                                , infoStart = fenceEnd
                                , infoEnd = openLineEnd
                                , bodyStart = bodyStart
                                , bodyEnd = bodyStart
                                }

                        Nothing ->
                            scanFencesBody codes end fenceEnd openLineEnd bodyStart ch fenceLen bodyStart


scanFencesBody : Array Int -> Int -> Int -> Int -> Int -> Int -> Int -> Int -> Maybe FencesSpan
scanFencesBody codes end infoStart infoEnd bodyStart ch fenceLen cur =
    if cur - end >= 0 then
        let
            bodyEnd =
                if end - bodyStart > 0 && at (end - 1) codes == 10 then
                    end - 1

                else
                    end
        in
        Just
            { rawEnd = end
            , infoStart = infoStart
            , infoEnd = infoEnd
            , bodyStart = bodyStart
            , bodyEnd = bodyEnd
            }

    else
        let
            le =
                lineEnd codes cur end
        in
        if le - end < 0 then
            case matchClosingFence codes (le + 1) end ch fenceLen of
                Just closeEnd ->
                    Just
                        { rawEnd = closeEnd
                        , infoStart = infoStart
                        , infoEnd = infoEnd
                        , bodyStart = bodyStart
                        , bodyEnd = le
                        }

                Nothing ->
                    scanFencesBody codes end infoStart infoEnd bodyStart ch fenceLen (le + 1)

        else
            let
                bodyEnd =
                    if end - bodyStart > 0 && at (end - 1) codes == 10 then
                        end - 1

                    else
                        end
            in
            Just
                { rawEnd = end
                , infoStart = infoStart
                , infoEnd = infoEnd
                , bodyStart = bodyStart
                , bodyEnd = bodyEnd
                }


isFencesBegin : Array Int -> Int -> Int -> Int -> Bool
isFencesBegin codes pos end maxIndent =
    let
        sp =
            leadingSpacesUpTo codes pos end maxIndent

        i =
            pos + sp
    in
    if i + 3 - end > 0 then
        False

    else
        let
            c0 =
                at i codes
        in
        (c0 == 96 && at (i + 1) codes == 96 && at (i + 2) codes == 96)
            || (c0 == 126 && at (i + 1) codes == 126 && at (i + 2) codes == 126)


isFencesInterrupt : Array Int -> Int -> Int -> Bool
isFencesInterrupt codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        i =
            pos + sp
    in
    if i + 3 - end > 0 then
        False

    else
        let
            c0 =
                at i codes
        in
        if c0 == 96 && at (i + 1) codes == 96 && at (i + 2) codes == 96 then
            let
                j =
                    skipSameChar codes (i + 3) end 96

                le =
                    lineEnd codes j end
            in
            not (hasCode 96 codes j le)

        else
            c0 == 126 && at (i + 1) codes == 126 && at (i + 2) codes == 126



-- 4. ATX HEADING


type alias HeadingSpan =
    { rawEnd : Int
    , depth : Int
    , textStart : Int
    , textEnd : Int
    }


matchHeading : Array Int -> Int -> Int -> Bool -> Maybe HeadingSpan
matchHeading codes pos end pedantic =
    let
        sp =
            if pedantic then
                0

            else
                leadingSpacesUpTo codes pos end 3

        hStart =
            pos + sp

        hEnd =
            skipHashesUpTo6 codes hStart end 0

        depth =
            hEnd - hStart
    in
    if depth == 0 then
        Nothing

    else if not pedantic && hEnd - end < 0 && not (isJsSpace (at hEnd codes)) then
        Nothing

    else
        let
            le =
                lineEnd codes hEnd end
        in
        Just
            { rawEnd = le
            , depth = depth
            , textStart = hEnd
            , textEnd = le
            }


skipHashesUpTo6 : Array Int -> Int -> Int -> Int -> Int
skipHashesUpTo6 codes i end count =
    if count < 6 && i - end < 0 && at i codes == 35 then
        skipHashesUpTo6 codes (i + 1) end (count + 1)

    else
        i


isHeadingInterrupt : Array Int -> Int -> Int -> Bool
isHeadingInterrupt codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        start =
            pos + sp

        i =
            skipHashesUpTo6 codes start end 0
    in
    if i - start == 0 then
        False

    else
        i - end == 0 || isJsSpace (at i codes)


{-| Checks pedantic paragraph heading interrupt: ` *#{1,6} *[^\n]`.
-}
isPedanticHeadingInterrupt : Array Int -> Int -> Int -> Bool
isPedanticHeadingInterrupt codes pos end =
    let
        start =
            skipSpaces codes pos end

        i =
            skipHashesUpTo6 codes start end 0

        hashes =
            i - start
    in
    if hashes == 0 then
        False

    else
        hashes >= 2 || (i - end < 0 && at i codes /= 10)


isHeadingBegin : Array Int -> Int -> Int -> Int -> Bool
isHeadingBegin codes pos end maxIndent =
    let
        sp =
            leadingSpacesUpTo codes pos end maxIndent
    in
    pos + sp - end < 0 && at (pos + sp) codes == 35



-- 5. HR


matchHrAt : Array Int -> Int -> Int -> Int -> Maybe Int
matchHrAt codes pos end maxIndent =
    let
        sp =
            leadingSpacesUpTo codes pos end maxIndent

        i =
            pos + sp
    in
    if i - end >= 0 then
        Nothing

    else
        let
            marker =
                at i codes
        in
        if marker /= 45 && marker /= 95 && marker /= 42 then
            Nothing

        else
            matchHrLoop codes end marker i 0


matchHrLoop : Array Int -> Int -> Int -> Int -> Int -> Maybe Int
matchHrLoop codes end marker i count =
    if i - end >= 0 then
        if count >= 3 then
            Just i

        else
            Nothing

    else
        let
            b =
                at i codes
        in
        if b - marker == 0 then
            matchHrLoop codes end marker (i + 1) (count + 1)

        else if b == 32 || b == 9 then
            matchHrLoop codes end marker (i + 1) count

        else if b == 10 then
            if count >= 3 then
                Just i

            else
                Nothing

        else
            Nothing



-- 6. BLOCKQUOTE BEGIN / AT


isBlockquoteBegin : Array Int -> Int -> Int -> Int -> Bool
isBlockquoteBegin codes pos end maxIndent =
    let
        sp =
            leadingSpacesUpTo codes pos end maxIndent
    in
    pos + sp - end < 0 && at (pos + sp) codes == 62


isBlockquoteAt : Array Int -> Int -> Int -> Bool
isBlockquoteAt codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 3
    in
    pos + sp - end < 0 && at (pos + sp) codes == 62



-- 7. LIST BULLETS


matchBulletAt : Array Int -> Int -> Int -> Int -> Maybe Int
matchBulletAt codes pos end maxIndent =
    let
        sp =
            leadingSpacesUpTo codes pos end maxIndent

        i =
            pos + sp
    in
    if i - end >= 0 then
        Nothing

    else
        let
            b =
                at i codes
        in
        if b == 42 || b == 43 || b == 45 then
            Just (i + 1)

        else if isAsciiDigit b then
            let
                j =
                    skipDigitsUpTo9 codes i end 0
            in
            if j - end < 0 then
                let
                    d =
                        at j codes
                in
                if d == 46 || d == 41 then
                    Just (j + 1)

                else
                    Nothing

            else
                Nothing

        else
            Nothing


skipDigitsUpTo9 : Array Int -> Int -> Int -> Int -> Int
skipDigitsUpTo9 codes i end count =
    if count < 9 && i - end < 0 && isAsciiDigit (at i codes) then
        skipDigitsUpTo9 codes (i + 1) end (count + 1)

    else
        i


isNextBullet : Array Int -> Int -> Int -> Int -> Bool
isNextBullet codes pos end maxIndent =
    case matchBulletAt codes pos end maxIndent of
        Nothing ->
            False

        Just after ->
            after - end == 0
                || at after codes
                == 32
                || at after codes
                == 9
                || at after codes
                == 10


isParagraphListInterrupt : Array Int -> Int -> Int -> Bool
isParagraphListInterrupt codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        i =
            pos + sp
    in
    if i - end >= 0 then
        False

    else
        let
            b =
                at i codes

            after =
                if b == 42 || b == 43 || b == 45 then
                    i + 1

                else if b == 49 && i + 1 - end < 0 && (at (i + 1) codes == 46 || at (i + 1) codes == 41) then
                    i + 2

                else
                    -1
        in
        if after < 0 then
            False

        else
            let
                wsEnd =
                    skipSpacesTabs codes after end
            in
            wsEnd - after > 0 && wsEnd - end < 0 && at wsEnd codes /= 10


isTableListInterrupt : Array Int -> Int -> Int -> Bool
isTableListInterrupt codes pos end =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        i =
            pos + sp
    in
    if i - end >= 0 then
        False

    else
        let
            b =
                at i codes

            after =
                if b == 42 || b == 43 || b == 45 then
                    i + 1

                else if b == 49 && i + 1 - end < 0 && (at (i + 1) codes == 46 || at (i + 1) codes == 41) then
                    i + 2

                else
                    -1
        in
        if after < 0 then
            False

        else
            after - end < 0 && (at after codes == 32 || at after codes == 9)


isBlockquoteListInterrupt : Array Int -> Int -> Int -> Bool
isBlockquoteListInterrupt codes pos end =
    isNextBullet codes pos end 3


type alias ListBulletSpan =
    { rawEnd : Int
    , bulletEnd : Int
    , restStart : Int
    , restEnd : Int
    }


finishListBullet : Array Int -> Int -> Int -> Maybe ListBulletSpan
finishListBullet codes end after =
    if after - end == 0 then
        Just
            { rawEnd = end
            , bulletEnd = after
            , restStart = end
            , restEnd = end
            }

    else
        let
            c =
                at after codes
        in
        if c == 10 then
            Just
                { rawEnd = after + 1
                , bulletEnd = after
                , restStart = after
                , restEnd = after
                }

        else if c /= 32 && c /= 9 then
            Nothing

        else
            let
                le =
                    lineEnd codes after end

                rawEnd =
                    if le - end < 0 then
                        le + 1

                    else
                        end
            in
            Just
                { rawEnd = rawEnd
                , bulletEnd = after
                , restStart = after
                , restEnd = le
                }


matchListStart : Array Int -> Int -> Int -> Maybe ListBulletSpan
matchListStart codes pos end =
    case matchBulletAt codes pos end 3 of
        Nothing ->
            Nothing

        Just after ->
            finishListBullet codes end after


type ListMarkerSpec
    = Ordered Int
    | Unordered Int
    | PedanticAnyUnordered


matchListItem : Array Int -> Int -> Int -> ListMarkerSpec -> Maybe ListBulletSpan
matchListItem codes pos end spec =
    let
        sp =
            leadingSpacesUpTo codes pos end 3

        i =
            pos + sp
    in
    if i - end >= 0 then
        Nothing

    else
        let
            maybeAfter =
                case spec of
                    Ordered delim ->
                        if not (isAsciiDigit (at i codes)) then
                            Nothing

                        else
                            let
                                j =
                                    skipDigitsUpTo9 codes i end 0
                            in
                            if j - end < 0 && at j codes - delim == 0 then
                                Just (j + 1)

                            else
                                Nothing

                    Unordered ch ->
                        if at i codes - ch == 0 then
                            Just (i + 1)

                        else
                            Nothing

                    PedanticAnyUnordered ->
                        let
                            b =
                                at i codes
                        in
                        if b == 42 || b == 43 || b == 45 then
                            Just (i + 1)

                        else
                            Nothing
        in
        case maybeAfter of
            Nothing ->
                Nothing

            Just after ->
                finishListBullet codes end after



-- 8. HTML BLOCKS & INTERRUPTS


cdataLowerCodes : List Int
cdataLowerCodes =
    [ 60, 33, 91, 99, 100, 97, 116, 97, 91 ]


isHtmlInterrupt : String -> Array Int -> Int -> Int -> Int -> Bool -> Bool
isHtmlInterrupt str codes pos end maxIndent caseInsensitive =
    let
        sp =
            leadingSpacesUpTo codes pos end maxIndent

        i =
            pos + sp
    in
    if i + 2 - end > 0 || at i codes /= 60 then
        False

    else
        let
            b1 =
                at (i + 1) codes
        in
        if b1 == 63 then
            -- '<?'
            True

        else if b1 == 33 then
            -- '<!'
            if i + 2 - end < 0 && isAsciiAlpha (at (i + 2) codes) then
                True

            else if i + 4 - end <= 0 && at (i + 2) codes == 45 && at (i + 3) codes == 45 then
                True

            else if caseInsensitive then
                startsWithAsciiCi codes i end cdataLowerCodes

            else
                i + 9 - end <= 0 && String.slice i (i + 9) str == "<![CDATA["

        else
            let
                isClose =
                    b1 == 47

                nameStart =
                    if isClose then
                        i + 2

                    else
                        i + 1

                nameEnd =
                    skipAsciiAlnum codes nameStart end
            in
            if nameEnd - nameStart == 0 then
                False

            else if
                not isClose
                    && isHtmlType1TagStr str nameStart nameEnd caseInsensitive
                    && (nameEnd - end == 0 || at nameEnd codes == 62 || isJsSpace (at nameEnd codes))
            then
                True

            else if isHtmlBlockTagStr str nameStart nameEnd caseInsensitive then
                if nameEnd - end == 0 then
                    caseInsensitive

                else
                    let
                        next =
                            at nameEnd codes
                    in
                    if next == 32 || next == 62 then
                        True

                    else if not caseInsensitive && next == 10 then
                        True

                    else
                        next == 47 && nameEnd + 1 - end < 0 && at (nameEnd + 1) codes == 62

            else
                False


skipAsciiAlnum : Array Int -> Int -> Int -> Int
skipAsciiAlnum codes i end =
    if i - end < 0 && isAsciiAlnum (at i codes) then
        skipAsciiAlnum codes (i + 1) end

    else
        i


skipAsciiAlpha : Array Int -> Int -> Int -> Int
skipAsciiAlpha codes i end =
    if i - end < 0 && isAsciiAlpha (at i codes) then
        skipAsciiAlpha codes (i + 1) end

    else
        i


skipTagNameHyphen : Array Int -> Int -> Int -> Int
skipTagNameHyphen codes i end =
    if i - end < 0 then
        let
            c =
                at i codes
        in
        if isAsciiAlnum c || c == 45 then
            skipTagNameHyphen codes (i + 1) end

        else
            i

    else
        i


scanToBlankLineOrEof : Array Int -> Int -> Int -> Int
scanToBlankLineOrEof codes start end =
    if start - end >= 0 then
        end

    else if at start codes == 10 then
        let
            ( jEnd, blankCount ) =
                scanConsecutiveBlankLines codes end start 0
        in
        if blankCount > 0 then
            jEnd + 1

        else
            scanToBlankLineOrEof codes (start + 1) end

    else
        scanToBlankLineOrEof codes (start + 1) end


scanConsecutiveBlankLines : Array Int -> Int -> Int -> Int -> ( Int, Int )
scanConsecutiveBlankLines codes end j blankCount =
    if j - end < 0 && at j codes == 10 then
        let
            k =
                skipSpacesTabs codes (j + 1) end
        in
        if k - end < 0 && at k codes == 10 then
            scanConsecutiveBlankLines codes end k (blankCount + 1)

        else
            ( j, blankCount )

    else
        ( j, blankCount )


matchBlockHtmlAttribute : Array Int -> Int -> Int -> Maybe Int
matchBlockHtmlAttribute codes pos end =
    let
        i0 =
            skipSpaces codes pos end
    in
    if i0 - pos == 0 || i0 - end >= 0 then
        Nothing

    else
        let
            b0 =
                at i0 codes
        in
        if not (isAsciiAlpha b0 || b0 == 58 || b0 == 95) then
            Nothing

        else
            let
                save =
                    skipAttrNameRest codes (i0 + 1) end

                iEq =
                    skipSpaces codes save end
            in
            if iEq - end < 0 && at iEq codes == 61 then
                let
                    iVal =
                        skipSpaces codes (iEq + 1) end
                in
                if iVal - end < 0 && at iVal codes == 34 then
                    let
                        qEnd =
                            scanUntilCharOrNl codes (iVal + 1) end 34
                    in
                    if qEnd - end < 0 && at qEnd codes == 34 then
                        Just (qEnd + 1)

                    else
                        Just save

                else if iVal - end < 0 && at iVal codes == 39 then
                    let
                        qEnd =
                            scanUntilCharOrNl codes (iVal + 1) end 39
                    in
                    if qEnd - end < 0 && at qEnd codes == 39 then
                        Just (qEnd + 1)

                    else
                        Just save

                else
                    let
                        vEnd =
                            scanUnquotedAttrVal codes iVal end
                    in
                    if vEnd - iVal > 0 then
                        Just vEnd

                    else
                        Just save

            else
                Just save


skipAttrNameRest : Array Int -> Int -> Int -> Int
skipAttrNameRest codes i end =
    if i - end < 0 then
        let
            b =
                at i codes
        in
        if isAsciiAlnum b || b == 95 || b == 46 || b == 58 || b == 45 then
            skipAttrNameRest codes (i + 1) end

        else
            i

    else
        i


scanUntilCharOrNl : Array Int -> Int -> Int -> Int -> Int
scanUntilCharOrNl codes i end ch =
    if i - end < 0 then
        let
            b =
                at i codes
        in
        if b - ch == 0 || b == 10 then
            i

        else
            scanUntilCharOrNl codes (i + 1) end ch

    else
        i


scanUnquotedAttrVal : Array Int -> Int -> Int -> Int
scanUnquotedAttrVal codes i end =
    if i - end < 0 then
        let
            b =
                at i codes
        in
        if b == 34 || b == 39 || b == 61 || b == 60 || b == 62 || b == 96 || isJsSpace b then
            i

        else
            scanUnquotedAttrVal codes (i + charWidth b) end

    else
        i


{-| Matches `block.html` (normal/GFM) at `src[pos..]`. Returns `rawEnd`.
-}
matchHtmlNormal : Src -> Int -> Maybe Int
matchHtmlNormal src pos =
    let
        codes =
            src.codes

        n =
            src.len

        sp =
            leadingSpacesUpTo codes pos n 3

        i =
            pos + sp
    in
    if i + 2 - n > 0 || at i codes /= 60 then
        Nothing

    else
        let
            b1 =
                at (i + 1) codes
        in
        -- Type 1: <(script|pre|style|textarea)[\s>][\s\S]*?(?:</\1>[^\n]*\n*|$)
        case tryHtmlType1 src i b1 of
            Just endIdx ->
                Just endIdx

            Nothing ->
                -- Type 2: <!--(?:-?>|[\s\S]*?(?:-->|$))[^\n]*(\n+|$)
                if b1 == 33 && i + 4 - n <= 0 && at (i + 2) codes == 45 && at (i + 3) codes == 45 then
                    let
                        after =
                            if i + 4 - n < 0 && at (i + 4) codes == 62 then
                                i + 5

                            else if i + 5 - n < 0 && at (i + 4) codes == 45 && at (i + 5) codes == 62 then
                                i + 6

                            else
                                scanHtmlCommentEnd codes (i + 4) n

                        end1 =
                            lineEnd codes after n
                    in
                    Just (skipNewlines codes end1 n)

                else if b1 == 63 then
                    -- Type 3: <\?
                    Just (scanPiEnd codes (i + 2) n)

                else if b1 == 33 && i + 2 - n < 0 && isAsciiAlpha (at (i + 2) codes) then
                    -- Type 4: <![A-Za-z]
                    Just (scanDeclEnd codes (i + 3) n)

                else if b1 == 33 && startsWithAsciiCi codes i n cdataLowerCodes then
                    -- Type 5: <![CDATA[
                    Just (scanCdataEnd codes (i + 9) n)

                else
                    -- Type 6 & 7
                    case tryHtmlType6 src i b1 of
                        Just endIdx ->
                            Just endIdx

                        Nothing ->
                            tryHtmlType7 src i b1


tryHtmlType1 : Src -> Int -> Int -> Maybe Int
tryHtmlType1 src i b1 =
    if not (isAsciiAlpha b1) then
        Nothing

    else
        let
            codes =
                src.codes

            n =
                src.len

            ne =
                skipAsciiAlpha codes (i + 1) n
        in
        if isHtmlType1TagStr src.str (i + 1) ne True && ne - n < 0 then
            let
                delim =
                    at ne codes
            in
            if delim == 62 || isJsSpace delim then
                let
                    tagLower =
                        String.toLower (String.slice (i + 1) ne src.str)

                    tagLen =
                        ne - (i + 1)
                in
                Just (scanType1Close src.str codes n tagLower tagLen (ne + 1))

            else
                Nothing

        else
            Nothing


scanType1Close : String -> Array Int -> Int -> String -> Int -> Int -> Int
scanType1Close str codes n tagLower tagLen scan =
    if scan - n >= 0 then
        n

    else if
        at scan codes
            == 60
            && scan + 2 + tagLen - n
            < 0
            && at (scan + 1) codes
            == 47
            && at (scan + 2 + tagLen) codes
            == 62
            && String.toLower (String.slice (scan + 2) (scan + 2 + tagLen) str)
            == tagLower
    then
        let
            le =
                lineEnd codes (scan + 3 + tagLen) n
        in
        skipNewlines codes le n

    else
        scanType1Close str codes n tagLower tagLen (scan + 1)


scanHtmlCommentEnd : Array Int -> Int -> Int -> Int
scanHtmlCommentEnd codes j n =
    if j + 2 - n >= 0 then
        n

    else if at j codes == 45 && at (j + 1) codes == 45 && at (j + 2) codes == 62 then
        j + 3

    else
        scanHtmlCommentEnd codes (j + 1) n


scanPiEnd : Array Int -> Int -> Int -> Int
scanPiEnd codes j n =
    if j + 1 - n >= 0 then
        n

    else if at j codes == 63 && at (j + 1) codes == 62 then
        skipNewlines codes (lineEnd codes (j + 2) n) n

    else
        scanPiEnd codes (j + 1) n


scanDeclEnd : Array Int -> Int -> Int -> Int
scanDeclEnd codes j n =
    if j - n >= 0 then
        n

    else if at j codes == 62 then
        skipNewlines codes (lineEnd codes (j + 1) n) n

    else
        scanDeclEnd codes (j + 1) n


scanCdataEnd : Array Int -> Int -> Int -> Int
scanCdataEnd codes j n =
    if j + 2 - n >= 0 then
        n

    else if at j codes == 93 && at (j + 1) codes == 93 && at (j + 2) codes == 62 then
        skipNewlines codes (lineEnd codes (j + 3) n) n

    else
        scanCdataEnd codes (j + 1) n


tryHtmlType6 : Src -> Int -> Int -> Maybe Int
tryHtmlType6 src i b1 =
    let
        codes =
            src.codes

        n =
            src.len

        isClose =
            b1 == 47

        ns =
            if isClose then
                i + 2

            else
                i + 1

        ne =
            skipAsciiAlnum codes ns n
    in
    if ne - ns > 0 && isHtmlBlockTagStr src.str ns ne True && ne - n < 0 then
        let
            c =
                at ne codes

            maybeAfter =
                if c == 32 then
                    Just (skipSpaces codes ne n)

                else if c == 10 || c == 62 then
                    Just (ne + 1)

                else if c == 47 && ne + 1 - n < 0 && at (ne + 1) codes == 62 then
                    Just (ne + 2)

                else
                    Nothing
        in
        case maybeAfter of
            Just after ->
                Just (scanToBlankLineOrEof codes after n)

            Nothing ->
                Nothing

    else
        Nothing


tryHtmlType7 : Src -> Int -> Int -> Maybe Int
tryHtmlType7 src i b1 =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if isAsciiAlpha b1 then
        let
            ns =
                i + 1

            ne =
                skipTagNameHyphen codes (ns + 1) n
        in
        if isHtmlType1TagStr src.str ns ne True then
            Nothing

        else
            scanType7OpenAttrs codes n ne

    else if b1 == 47 && i + 2 - n < 0 && isAsciiAlpha (at (i + 2) codes) then
        let
            cur0 =
                skipTagNameHyphen codes (i + 3) n

            cur =
                skipJsSpaces codes cur0 n
        in
        if cur - n < 0 && at cur codes == 62 then
            let
                afterGt =
                    cur + 1

                w =
                    skipSpacesTabs codes afterGt n
            in
            if w - n == 0 || at w codes == 10 then
                Just (scanToBlankLineOrEof codes afterGt n)

            else
                Nothing

        else
            Nothing

    else
        Nothing


scanType7OpenAttrs : Array Int -> Int -> Int -> Maybe Int
scanType7OpenAttrs codes n cur =
    let
        c0 =
            skipSpaces codes cur n

        c1 =
            if c0 - n < 0 && at c0 codes == 47 then
                c0 + 1

            else
                c0

        matchedEnd =
            if c1 - n < 0 && at c1 codes == 62 then
                let
                    afterGt =
                        c1 + 1

                    w =
                        skipSpacesTabs codes afterGt n
                in
                if w - n == 0 || at w codes == 10 then
                    Just (scanToBlankLineOrEof codes afterGt n)

                else
                    Nothing

            else
                Nothing
    in
    case matchedEnd of
        Just endIdx ->
            Just endIdx

        Nothing ->
            case matchBlockHtmlAttribute codes cur n of
                Just nextCur ->
                    scanType7OpenAttrs codes n nextCur

                Nothing ->
                    Nothing


skipJsSpaces : Array Int -> Int -> Int -> Int
skipJsSpaces codes i end =
    if i - end < 0 then
        let
            c =
                at i codes
        in
        if isJsSpace c then
            skipJsSpaces codes (i + charWidth c) end

        else
            i

    else
        i


matchPedanticHtmlTagName : Src -> Int -> Maybe Int
matchPedanticHtmlTagName src pos =
    let
        codes =
            src.codes

        n =
            src.len

        endIdx =
            skipAsciiWords codes pos n
    in
    if endIdx - pos == 0 then
        Nothing

    else if isPedanticInlineTag (String.slice pos endIdx src.str) then
        Nothing

    else if endIdx - n < 0 && at endIdx codes == 58 then
        Nothing

    else if pedanticTagHasAtSign codes endIdx n then
        Nothing

    else
        Just endIdx


skipAsciiWords : Array Int -> Int -> Int -> Int
skipAsciiWords codes i end =
    if i - end < 0 && isAsciiWord (at i codes) then
        skipAsciiWords codes (i + 1) end

    else
        i


pedanticTagHasAtSign : Array Int -> Int -> Int -> Bool
pedanticTagHasAtSign codes j n =
    if j - n >= 0 then
        False

    else
        let
            b =
                at j codes
        in
        if b == 64 then
            True

        else if isAsciiWord b || isJsSpace b then
            False

        else
            pedanticTagHasAtSign codes (j + charWidth b) n


{-| Matches `blockPedantic.html` at `src[pos..]`, returning `rawEnd`.
-}
matchHtmlPedantic : Src -> Int -> Maybe Int
matchHtmlPedantic src pos =
    let
        codes =
            src.codes

        n =
            src.len

        i =
            skipSpaces codes pos n
    in
    if i + 2 - n > 0 || at i codes /= 60 then
        Nothing

    else if i + 4 - n <= 0 && at (i + 1) codes == 33 && at (i + 2) codes == 45 && at (i + 3) codes == 45 then
        -- Alt 1: comment *(?:\n|\s*$)
        let
            checkAfter after =
                let
                    j =
                        skipSpaces codes after n
                in
                if j - n < 0 && at j codes == 10 then
                    Just (j + 1)

                else if isAllJsSpace codes j n then
                    Just n

                else
                    Nothing

            shortResult =
                if i + 4 - n < 0 && at (i + 4) codes == 62 then
                    checkAfter (i + 5)

                else if i + 5 - n < 0 && at (i + 4) codes == 45 && at (i + 5) codes == 62 then
                    checkAfter (i + 6)

                else
                    Nothing
        in
        case shortResult of
            Just endIdx ->
                Just endIdx

            Nothing ->
                Just (scanPedanticCommentEnd codes n (i + 4))

    else
        case matchPedanticHtmlTagName src (i + 1) of
            Nothing ->
                Nothing

            Just nameEnd ->
                let
                    tag =
                        String.slice (i + 1) nameEnd src.str

                    tagLen =
                        nameEnd - (i + 1)
                in
                -- Alt 2: <(tag)[\s\S]+?</\1> *(?:\n{2,}|\s*$)
                case scanPedanticAlt2 src.str codes n tag tagLen (nameEnd + 1) of
                    Just endIdx ->
                        Just endIdx

                    Nothing ->
                        -- Alt 3: <tag(?:"[^"]*"|'[^']*'|\s[^'"/>\s]*)*?/?> *(?:\n{2,}|\s*$)
                        scanPedanticAlt3 codes n nameEnd


scanPedanticCommentEnd : Array Int -> Int -> Int -> Int
scanPedanticCommentEnd codes n j =
    if j + 2 - n >= 0 then
        n

    else if at j codes == 45 && at (j + 1) codes == 45 && at (j + 2) codes == 62 then
        let
            k =
                skipSpaces codes (j + 3) n
        in
        if k - n < 0 && at k codes == 10 then
            k + 1

        else if isAllJsSpace codes k n then
            n

        else
            scanPedanticCommentEnd codes n (j + 1)

    else
        scanPedanticCommentEnd codes n (j + 1)


checkPedanticBlockTail : Array Int -> Int -> Int -> Maybe Int
checkPedanticBlockTail codes n after =
    let
        end0 =
            skipSpaces codes after n
    in
    if end0 + 1 - n < 0 && at end0 codes == 10 && at (end0 + 1) codes == 10 then
        Just (skipNewlines codes end0 n)

    else if isAllJsSpace codes end0 n then
        Just n

    else
        Nothing


scanPedanticAlt2 : String -> Array Int -> Int -> String -> Int -> Int -> Maybe Int
scanPedanticAlt2 str codes n tag tagLen scan =
    if scan - n >= 0 then
        Nothing

    else if
        at scan codes
            == 60
            && scan + 2 + tagLen - n
            < 0
            && at (scan + 1) codes
            == 47
            && at (scan + 2 + tagLen) codes
            == 62
            && String.slice (scan + 2) (scan + 2 + tagLen) str
            == tag
    then
        case checkPedanticBlockTail codes n (scan + 3 + tagLen) of
            Just endIdx ->
                Just endIdx

            Nothing ->
                scanPedanticAlt2 str codes n tag tagLen (scan + 1)

    else
        scanPedanticAlt2 str codes n tag tagLen (scan + 1)


scanPedanticAlt3 : Array Int -> Int -> Int -> Maybe Int
scanPedanticAlt3 codes n cur =
    let
        c0 =
            if cur - n < 0 && at cur codes == 47 then
                cur + 1

            else
                cur

        tailHit =
            if c0 - n < 0 && at c0 codes == 62 then
                checkPedanticBlockTail codes n (c0 + 1)

            else
                Nothing
    in
    case tailHit of
        Just endIdx ->
            Just endIdx

        Nothing ->
            if cur - n >= 0 then
                Nothing

            else
                let
                    b =
                        at cur codes
                in
                if b == 34 then
                    let
                        j =
                            scanUntilChar codes (cur + 1) n 34
                    in
                    if j - n < 0 then
                        scanPedanticAlt3 codes n (j + 1)

                    else
                        Nothing

                else if b == 39 then
                    let
                        j =
                            scanUntilChar codes (cur + 1) n 39
                    in
                    if j - n < 0 then
                        scanPedanticAlt3 codes n (j + 1)

                    else
                        Nothing

                else if isJsSpace b then
                    let
                        nextCur =
                            scanPedanticAttrWord codes (cur + charWidth b) n
                    in
                    scanPedanticAlt3 codes n nextCur

                else
                    Nothing


scanUntilChar : Array Int -> Int -> Int -> Int -> Int
scanUntilChar codes j n ch =
    if j - n < 0 && at j codes - ch /= 0 then
        scanUntilChar codes (j + 1) n ch

    else
        j


scanPedanticAttrWord : Array Int -> Int -> Int -> Int
scanPedanticAttrWord codes cur n =
    if cur - n >= 0 then
        cur

    else
        let
            b =
                at cur codes
        in
        if b == 39 || b == 34 || b == 47 || b == 62 || isJsSpace b then
            cur

        else
            scanPedanticAttrWord codes (cur + charWidth b) n



-- 9. DEF


type alias DefSpan =
    { rawEnd : Int
    , labelStart : Int
    , labelEnd : Int
    , hrefStart : Int
    , hrefEnd : Int
    , title : Maybe ( Int, Int )
    }


{-| Parses `_blockLabel`: `(?!\s*\])(?:\\[\s\S]|[^\[\]\\])+` at `codes[pos..end)`,
returning `]` index.
-}
matchBlockLabel : Array Int -> Int -> Int -> Maybe Int -> Maybe Int
matchBlockLabel codes pos end maxItems =
    let
        w =
            skipJsSpaces codes pos end
    in
    if w - end < 0 && at w codes == 93 then
        Nothing

    else
        let
            limit =
                case maxItems of
                    Just m ->
                        m

                    Nothing ->
                        -1
        in
        matchBlockLabelLoop codes end limit pos 0


matchBlockLabelLoop : Array Int -> Int -> Int -> Int -> Int -> Maybe Int
matchBlockLabelLoop codes end limit i items =
    if i - end >= 0 then
        Nothing

    else if limit >= 0 && items - limit >= 0 then
        if items > 0 && at i codes == 93 then
            Just i

        else
            Nothing

    else
        let
            b =
                at i codes
        in
        if b == 93 then
            if items > 0 then
                Just i

            else
                Nothing

        else if b == 91 then
            Nothing

        else if b == 92 then
            if i + 1 - end >= 0 then
                Nothing

            else
                matchBlockLabelLoop codes end limit (i + 1 + charWidth (at (i + 1) codes)) (items + 1)

        else
            matchBlockLabelLoop codes end limit (i + charWidth b) (items + 1)


matchDefTitle : Array Int -> Int -> Int -> Maybe Int
matchDefTitle codes pos end =
    if pos - end >= 0 then
        Nothing

    else
        let
            b =
                at pos codes
        in
        if b == 34 then
            scanDefTitleDouble codes (pos + 1) end

        else if b == 39 then
            scanDefTitleSingle codes (pos + 1) end

        else if b == 40 then
            scanDefTitleParen codes (pos + 1) end

        else
            Nothing


scanDefTitleDouble : Array Int -> Int -> Int -> Maybe Int
scanDefTitleDouble codes i end =
    if i - end >= 0 then
        Nothing

    else
        let
            b =
                at i codes
        in
        if b == 92 then
            if i + 1 - end < 0 && at (i + 1) codes == 34 then
                scanDefTitleDouble codes (i + 2) end

            else
                scanDefTitleDouble codes (i + 1) end

        else if b == 34 then
            Just (i + 1)

        else
            scanDefTitleDouble codes (i + 1) end


scanDefTitleSingle : Array Int -> Int -> Int -> Maybe Int
scanDefTitleSingle codes i end =
    let
        i1 =
            scanUntilCharOrNl codes i end 39
    in
    scanDefTitleSingleNl codes i1 end


scanDefTitleSingleNl : Array Int -> Int -> Int -> Maybe Int
scanDefTitleSingleNl codes i end =
    if i - end < 0 && at i codes == 10 then
        let
            afterNl =
                i + 1
        in
        if afterNl - end < 0 && at afterNl codes == 39 then
            Just (afterNl + 1)

        else
            let
                j =
                    scanUntilCharOrNl codes afterNl end 39
            in
            if j - afterNl == 0 then
                Nothing

            else
                scanDefTitleSingleNl codes j end

    else if i - end < 0 && at i codes == 39 then
        Just (i + 1)

    else
        Nothing


scanDefTitleParen : Array Int -> Int -> Int -> Maybe Int
scanDefTitleParen codes i end =
    if i - end >= 0 then
        Nothing

    else
        let
            b =
                at i codes
        in
        if b == 40 then
            Nothing

        else if b == 41 then
            Just (i + 1)

        else
            scanDefTitleParen codes (i + 1) end


matchDefEnd : Array Int -> Int -> Int -> Maybe Int
matchDefEnd codes pos end =
    let
        i =
            skipSpaces codes pos end
    in
    if i - end == 0 || at i codes == 10 then
        Just i

    else
        Nothing


matchDef : Src -> Int -> Bool -> Maybe DefSpan
matchDef src pos pedantic =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if pedantic then
        matchDefPedantic codes pos n

    else
        let
            sp =
                leadingSpacesUpTo codes pos n 3

            open =
                pos + sp
        in
        if open - n >= 0 || at open codes /= 91 then
            Nothing

        else
            let
                labelStart =
                    open + 1
            in
            case matchBlockLabel codes labelStart n Nothing of
                Nothing ->
                    Nothing

                Just labelEnd ->
                    if labelEnd + 1 - n >= 0 || at (labelEnd + 1) codes /= 58 then
                        Nothing

                    else
                        let
                            i0 =
                                skipSpaces codes (labelEnd + 2) n

                            i1 =
                                if i0 - n < 0 && at i0 codes == 10 then
                                    skipSpacesTabs codes (i0 + 1) n

                                else
                                    i0
                        in
                        if i1 - n >= 0 then
                            Nothing

                        else if at i1 codes == 60 then
                            -- `<.*?>` lazy backtracking across `>` on the same line
                            scanDefAngleHref codes n labelStart labelEnd i1 (i1 + 1)

                        else if isJsSpace (at i1 codes) then
                            Nothing

                        else
                            let
                                j =
                                    skipNonJsSpace codes i1 n
                            in
                            if j - i1 == 0 then
                                Nothing

                            else
                                matchDefTail codes n labelStart labelEnd i1 j


scanDefAngleHref : Array Int -> Int -> Int -> Int -> Int -> Int -> Maybe DefSpan
scanDefAngleHref codes n labelStart labelEnd hrefStart j =
    if j - n >= 0 || at j codes == 10 then
        Nothing

    else if at j codes == 62 then
        case matchDefTail codes n labelStart labelEnd hrefStart (j + 1) of
            Just span ->
                Just span

            Nothing ->
                scanDefAngleHref codes n labelStart labelEnd hrefStart (j + 1)

    else
        scanDefAngleHref codes n labelStart labelEnd hrefStart (j + 1)


skipNonJsSpace : Array Int -> Int -> Int -> Int
skipNonJsSpace codes i end =
    if i - end < 0 then
        let
            c =
                at i codes
        in
        if isJsSpace c then
            i

        else
            skipNonJsSpace codes (i + charWidth c) end

    else
        i


matchDefTail : Array Int -> Int -> Int -> Int -> Int -> Int -> Maybe DefSpan
matchDefTail codes n labelStart labelEnd hrefStart hrefEnd =
    let
        t0 =
            skipSpaces codes hrefEnd n

        hasSpace =
            t0 - hrefEnd > 0

        ( t1, hasNl ) =
            if t0 - n < 0 && at t0 codes == 10 then
                ( skipSpacesTabs codes (t0 + 1) n, True )

            else
                ( t0, False )

        withTitle =
            if hasSpace || hasNl then
                case matchDefTitle codes t1 n of
                    Just titleEnd ->
                        case matchDefEnd codes titleEnd n of
                            Just rawEnd ->
                                Just
                                    { rawEnd = rawEnd
                                    , labelStart = labelStart
                                    , labelEnd = labelEnd
                                    , hrefStart = hrefStart
                                    , hrefEnd = hrefEnd
                                    , title = Just ( t1, titleEnd )
                                    }

                            Nothing ->
                                Nothing

                    Nothing ->
                        Nothing

            else
                Nothing
    in
    case withTitle of
        Just span ->
            Just span

        Nothing ->
            case matchDefEnd codes hrefEnd n of
                Just rawEnd ->
                    Just
                        { rawEnd = rawEnd
                        , labelStart = labelStart
                        , labelEnd = labelEnd
                        , hrefStart = hrefStart
                        , hrefEnd = hrefEnd
                        , title = Nothing
                        }

                Nothing ->
                    Nothing


matchDefPedantic : Array Int -> Int -> Int -> Maybe DefSpan
matchDefPedantic codes pos n =
    let
        i0 =
            skipSpaces codes pos n
    in
    if i0 - n >= 0 || at i0 codes /= 91 then
        Nothing

    else
        let
            labelStart =
                i0 + 1

            labelEnd =
                scanUntilChar codes labelStart n 93
        in
        if labelEnd - labelStart == 0 || labelEnd + 1 - n >= 0 || at (labelEnd + 1) codes /= 58 then
            Nothing

        else
            let
                i1 =
                    skipSpaces codes (labelEnd + 2) n

                i2 =
                    if i1 - n < 0 && at i1 codes == 60 then
                        i1 + 1

                    else
                        i1

                hrefStart =
                    i2

                hrefEnd =
                    scanPedanticDefHref codes hrefStart n
            in
            if hrefEnd - hrefStart == 0 then
                Nothing

            else
                let
                    afterHref =
                        if hrefEnd - n < 0 && at hrefEnd codes == 62 then
                            hrefEnd + 1

                        else
                            hrefEnd

                    le =
                        lineEnd codes afterHref n
                in
                if afterHref - le < 0 && at afterHref codes == 32 then
                    let
                        ts =
                            skipSpaces codes afterHref le
                    in
                    if ts - le < 0 && (at ts codes == 34 || at ts codes == 40) then
                        let
                            te =
                                skipSpacesRev codes ts le
                        in
                        if te - (ts + 3) >= 0 && (at (te - 1) codes == 34 || at (te - 1) codes == 41) then
                            Just
                                { rawEnd = le
                                , labelStart = labelStart
                                , labelEnd = labelEnd
                                , hrefStart = hrefStart
                                , hrefEnd = hrefEnd
                                , title = Just ( ts, te )
                                }

                        else
                            finishPedanticDefNoTitle codes n labelStart labelEnd hrefStart hrefEnd afterHref

                    else
                        finishPedanticDefNoTitle codes n labelStart labelEnd hrefStart hrefEnd afterHref

                else
                    finishPedanticDefNoTitle codes n labelStart labelEnd hrefStart hrefEnd afterHref


finishPedanticDefNoTitle : Array Int -> Int -> Int -> Int -> Int -> Int -> Int -> Maybe DefSpan
finishPedanticDefNoTitle codes n labelStart labelEnd hrefStart hrefEnd afterHref =
    case matchDefEnd codes afterHref n of
        Just rawEnd ->
            Just
                { rawEnd = rawEnd
                , labelStart = labelStart
                , labelEnd = labelEnd
                , hrefStart = hrefStart
                , hrefEnd = hrefEnd
                , title = Nothing
                }

        Nothing ->
            Nothing


scanPedanticDefHref : Array Int -> Int -> Int -> Int
scanPedanticDefHref codes i n =
    if i - n >= 0 then
        i

    else
        let
            c =
                at i codes
        in
        if c == 62 || isJsSpace c then
            i

        else
            scanPedanticDefHref codes (i + charWidth c) n


skipSpacesRev : Array Int -> Int -> Int -> Int
skipSpacesRev codes start end =
    if end - start > 0 && at (end - 1) codes == 32 then
        skipSpacesRev codes start (end - 1)

    else
        end



-- 10. GFM TABLE


type alias TableSpan =
    { rawEnd : Int
    , headerStart : Int
    , headerEnd : Int
    , alignStart : Int
    , alignEnd : Int
    , body : Maybe ( Int, Int )
    }


matchTableAlignLine : Array Int -> Int -> Int -> Maybe ( Int, Int )
matchTableAlignLine codes pos n =
    let
        sp =
            leadingSpacesUpTo codes pos n 3

        alignStart =
            pos + sp

        i0 =
            if alignStart - n < 0 && at alignStart codes == 124 then
                skipSpaces codes (alignStart + 1) n

            else
                alignStart

        i1 =
            if i0 - n < 0 && at i0 codes == 58 then
                i0 + 1

            else
                i0

        i2 =
            skipSameChar codes i1 n 45
    in
    if i2 - i1 == 0 then
        Nothing

    else
        let
            i3 =
                if i2 - n < 0 && at i2 codes == 58 then
                    i2 + 1

                else
                    i2

            i4 =
                skipSpaces codes i3 n
        in
        scanTableAlignPipes codes n alignStart i4


scanTableAlignPipes : Array Int -> Int -> Int -> Int -> Maybe ( Int, Int )
scanTableAlignPipes codes n alignStart i =
    if i - n < 0 && at i codes == 124 then
        let
            i1 =
                skipSpaces codes (i + 1) n
        in
        if i1 - n == 0 || at i1 codes == 10 then
            Just ( alignStart, i1 )

        else
            let
                i2 =
                    if at i1 codes == 58 then
                        i1 + 1

                    else
                        i1

                i3 =
                    skipSameChar codes i2 n 45
            in
            if i3 - i2 == 0 then
                finishTableAlignLine codes n alignStart i

            else
                let
                    i4 =
                        if i3 - n < 0 && at i3 codes == 58 then
                            i3 + 1

                        else
                            i3

                    i5 =
                        skipSpaces codes i4 n
                in
                scanTableAlignPipes codes n alignStart i5

    else
        finishTableAlignLine codes n alignStart i


finishTableAlignLine : Array Int -> Int -> Int -> Int -> Maybe ( Int, Int )
finishTableAlignLine codes n alignStart i =
    let
        iEnd =
            if i - n < 0 && at i codes == 124 then
                skipSpaces codes (i + 1) n

            else
                i
    in
    if iEnd - n == 0 || at iEnd codes == 10 then
        Just ( alignStart, iEnd )

    else
        Nothing


matchTableAt : Src -> Int -> Maybe TableSpan
matchTableAt src pos =
    let
        codes =
            src.codes

        n =
            src.len

        headerStart =
            skipSpaces codes pos n
    in
    if headerStart - n >= 0 || at headerStart codes == 10 then
        Nothing

    else
        let
            headerEnd =
                lineEnd codes headerStart n
        in
        if headerEnd - n >= 0 then
            Nothing

        else
            case matchTableAlignLine codes (headerEnd + 1) n of
                Nothing ->
                    Nothing

                Just ( alignStart, alignEnd ) ->
                    if alignEnd - n == 0 then
                        Just
                            { rawEnd = alignEnd
                            , headerStart = headerStart
                            , headerEnd = headerEnd
                            , alignStart = alignStart
                            , alignEnd = alignEnd
                            , body = Nothing
                            }

                    else
                        let
                            bodyStart =
                                alignEnd + 1

                            curEnd =
                                scanTableBodyRows src bodyStart

                            rawEnd =
                                rtrimNewlinesIdx codes pos curEnd
                        in
                        Just
                            { rawEnd = rawEnd
                            , headerStart = headerStart
                            , headerEnd = headerEnd
                            , alignStart = alignStart
                            , alignEnd = alignEnd
                            , body = Just ( bodyStart, curEnd )
                            }


scanTableBodyRows : Src -> Int -> Int
scanTableBodyRows src cur =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if cur - n >= 0 then
        cur

    else
        let
            s =
                skipSpaces codes cur n
        in
        if s - n < 0 && at s codes == 10 then
            cur

        else if
            isJust (matchHrAt codes cur n 3)
                || isHeadingInterrupt codes cur n
                || isBlockquoteAt codes cur n
                || isCodeIndentNonBlank codes cur n
                || isFencesInterrupt codes cur n
                || isTableListInterrupt codes cur n
                || isHtmlInterrupt src.str codes cur n 3 False
        then
            cur

        else
            let
                le =
                    lineEnd codes cur n

                nextCur =
                    if le - n < 0 then
                        le + 1

                    else
                        n
            in
            scanTableBodyRows src nextCur


isCodeIndentNonBlank : Array Int -> Int -> Int -> Bool
isCodeIndentNonBlank codes cur n =
    case matchCodeIndent codes cur n of
        Just a ->
            a - n < 0 && at a codes /= 10

        Nothing ->
            False


rtrimNewlinesIdx : Array Int -> Int -> Int -> Int
rtrimNewlinesIdx codes pos end =
    if end - pos > 0 && at (end - 1) codes == 10 then
        rtrimNewlinesIdx codes pos (end - 1)

    else
        end


isJust : Maybe a -> Bool
isJust m =
    case m of
        Just _ ->
            True

        Nothing ->
            False



-- 11. SETEXT LHEADING


matchSetextUnderline : Array Int -> Int -> Int -> Maybe ( Int, Int )
matchSetextUnderline codes pos n =
    let
        sp =
            leadingSpacesUpTo codes pos n 3

        i0 =
            pos + sp
    in
    if i0 - n >= 0 then
        Nothing

    else
        let
            ch =
                at i0 codes
        in
        if ch /= 61 && ch /= 45 then
            Nothing

        else
            let
                i1 =
                    skipSameChar codes i0 n ch

                i2 =
                    skipSpaces codes i1 n
            in
            if i2 - n == 0 || at i2 codes == 10 then
                Just ( ch, i2 )

            else
                Nothing


isLHeadingHtmlAt : Array Int -> Int -> Int -> Bool
isLHeadingHtmlAt codes pos n =
    let
        sp =
            leadingSpacesUpTo codes pos n 3

        i =
            pos + sp
    in
    if i - n >= 0 || at i codes /= 60 then
        False

    else
        let
            j =
                scanUntilGtOrNl codes (i + 1) n
        in
        j - (i + 1) > 0 && j + 1 - n < 0 && at j codes == 62 && at (j + 1) codes == 10


scanUntilGtOrNl : Array Int -> Int -> Int -> Int
scanUntilGtOrNl codes j n =
    if j - n < 0 then
        let
            b =
                at j codes
        in
        if b == 10 || b == 62 then
            j

        else
            scanUntilGtOrNl codes (j + 1) n

    else
        j


isLHeadingTableAt : Array Int -> Int -> Int -> Bool
isLHeadingTableAt codes pos n =
    let
        sp =
            leadingSpacesUpTo codes pos n 3

        i0 =
            pos + sp

        i1 =
            if i0 - n < 0 && at i0 codes == 124 then
                i0 + 1

            else
                i0
    in
    scanLHeadingTableLoop codes i1 n 0


scanLHeadingTableLoop : Array Int -> Int -> Int -> Int -> Bool
scanLHeadingTableLoop codes i n pipes =
    if i - n >= 0 then
        False

    else
        let
            b =
                at i codes
        in
        if b == 10 then
            pipes >= 1

        else if b == 124 then
            scanLHeadingTableLoop codes (i + 1) n (pipes + 1)

        else if b == 58 || b == 45 || b == 32 then
            scanLHeadingTableLoop codes (i + 1) n pipes

        else
            False


isLHeadingBullAt : Array Int -> Int -> Int -> Bool
isLHeadingBullAt codes pos n =
    case matchBulletAt codes pos n 3 of
        Just after ->
            after - n < 0 && at after codes == 32

        Nothing ->
            False


type alias LHeadingSpan =
    { rawEnd : Int
    , depth : Int
    , textStart : Int
    , textEnd : Int
    }


matchLHeadingAt : Src -> Int -> Mode -> Maybe LHeadingSpan
matchLHeadingAt src pos mode =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if pos - n >= 0 then
        Nothing

    else if isPedantic mode then
        let
            le =
                lineEnd codes pos n
        in
        if le - pos == 0 || le - n >= 0 then
            Nothing

        else
            case matchSetextUnderline codes (le + 1) n of
                Just ( ch, uEnd ) ->
                    Just
                        { rawEnd = uEnd
                        , depth =
                            if ch == 61 then
                                1

                            else
                                2
                        , textStart = pos
                        , textEnd = le
                        }

                Nothing ->
                    Nothing

    else
        let
            gfm =
                isGfm mode
        in
        if
            isLHeadingBullAt codes pos n
                || isJust (matchCodeIndent codes pos n)
                || isFencesBegin codes pos n 3
                || isBlockquoteAt codes pos n
                || isHeadingInterrupt codes pos n
                || isLHeadingHtmlAt codes pos n
                || (gfm && isLHeadingTableAt codes pos n)
        then
            Nothing

        else
            scanLHeadingLines codes n gfm pos pos


scanLHeadingLines : Array Int -> Int -> Bool -> Int -> Int -> Maybe LHeadingSpan
scanLHeadingLines codes n gfm startPos cur =
    let
        le =
            lineEnd codes cur n
    in
    if le - n >= 0 then
        Nothing

    else
        let
            nextStart =
                le + 1

            foundUnderline =
                if le - startPos > 0 then
                    matchSetextUnderline codes nextStart n

                else
                    Nothing
        in
        case foundUnderline of
            Just ( ch, uEnd ) ->
                Just
                    { rawEnd = uEnd
                    , depth =
                        if ch == 61 then
                            1

                        else
                            2
                    , textStart = startPos
                    , textEnd = le
                    }

            Nothing ->
                let
                    nextLe =
                        lineEnd codes nextStart n
                in
                if nextLe - n < 0 && isAllJsSpace codes nextStart nextLe then
                    Nothing

                else if
                    isLHeadingBullAt codes nextStart n
                        || isFencesBegin codes nextStart n 3
                        || isBlockquoteAt codes nextStart n
                        || isHeadingInterrupt codes nextStart n
                        || isJust (matchHrAt codes nextStart n 3)
                        || isLHeadingHtmlAt codes nextStart n
                        || (gfm && isLHeadingTableAt codes nextStart n)
                then
                    Nothing

                else
                    scanLHeadingLines codes n gfm startPos nextStart



-- 12. PARAGRAPH & BLOCKQUOTE


isSpaceTabBlankLine : Array Int -> Int -> Int -> Bool
isSpaceTabBlankLine codes pos n =
    let
        i =
            skipSpacesTabs codes pos n
    in
    i - pos > 0 && i - n < 0 && at i codes == 10


{-| Matches `paragraph` (or `blockquoteParagraph`) at `src[pos..]`, returning `end` index.
-}
matchParagraphAt : Src -> Int -> Mode -> Bool -> Maybe Int
matchParagraphAt src pos mode forBlockquote =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if pos - n >= 0 || at pos codes == 10 then
        Nothing

    else
        let
            firstEnd =
                lineEnd codes pos n
        in
        Just (scanParagraphLines src mode forBlockquote firstEnd)


scanParagraphLines : Src -> Mode -> Bool -> Int -> Int
scanParagraphLines src mode forBlockquote cur =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if cur - n >= 0 || at cur codes /= 10 then
        cur

    else
        let
            next =
                cur + 1
        in
        if next - n >= 0 || at next codes == 10 then
            cur

        else if
            isSpaceTabBlankLine codes next n
                || isJust (matchHrAt codes next n 3)
                || isBlockquoteAt codes next n
        then
            cur

        else if not forBlockquote && isPedantic mode then
            if isPedanticHeadingInterrupt codes next n || isJust (matchLHeadingAt src next Normal) then
                cur

            else
                scanParagraphLines src mode forBlockquote (lineEnd codes next n)

        else if
            isHeadingInterrupt codes next n
                || isFencesInterrupt codes next n
                || isHtmlInterrupt src.str codes next n 3 False
        then
            cur

        else if forBlockquote then
            if isBlockquoteListInterrupt codes next n then
                cur

            else
                scanParagraphLines src mode forBlockquote (lineEnd codes next n)

        else if isParagraphListInterrupt codes next n || (isGfm mode && isJust (matchTableAt src next)) then
            cur

        else
            scanParagraphLines src mode forBlockquote (lineEnd codes next n)


{-| Matches `block.blockquote` at `src[pos..]`, returning `end` index.
-}
matchBlockquote : Src -> Int -> Maybe Int
matchBlockquote src pos =
    matchBlockquoteLoop src pos False


matchBlockquoteLoop : Src -> Int -> Bool -> Maybe Int
matchBlockquoteLoop src i any =
    let
        codes =
            src.codes

        n =
            src.len
    in
    if i - n >= 0 then
        if any then
            Just i

        else
            Nothing

    else
        let
            sp =
                leadingSpacesUpTo codes i n 3

            gt =
                i + sp
        in
        if gt - n >= 0 || at gt codes /= 62 then
            if any then
                Just i

            else
                Nothing

        else
            let
                body =
                    if gt + 1 - n < 0 && at (gt + 1) codes == 32 then
                        gt + 2

                    else
                        gt + 1

                afterBody =
                    case matchParagraphAt src body Normal True of
                        Just pend ->
                            pend

                        Nothing ->
                            lineEnd codes body n

                nextI =
                    if afterBody - n < 0 && at afterBody codes == 10 then
                        afterBody + 1

                    else
                        afterBody
            in
            matchBlockquoteLoop src nextI True



-- LINE TRANSFORMATION HELPERS


stripBlockquoteMarkers : String -> String
stripBlockquoteMarkers chunk =
    String.split "\n" chunk
        |> stripBqMarkersLoop True []
        |> List.reverse
        |> String.join "\n"


stripBqMarkersLoop : Bool -> List String -> List String -> List String
stripBqMarkersLoop isFirst acc lines =
    case lines of
        [] ->
            acc

        line :: rest ->
            let
                s =
                    fromString line

                sp =
                    leadingSpacesUpTo s.codes 0 s.len 3
            in
            if not isFirst && isSetextOnlyLine s.codes sp s.len then
                stripBqMarkersLoop False (("    " ++ String.dropLeft sp line) :: acc) rest

            else if sp - s.len < 0 && at sp s.codes == 62 then
                let
                    after =
                        if sp + 1 - s.len < 0 && (at (sp + 1) s.codes == 32 || at (sp + 1) s.codes == 9) then
                            sp + 2

                        else
                            sp + 1
                in
                stripBqMarkersLoop False (String.dropLeft after line :: acc) rest

            else
                stripBqMarkersLoop False (line :: acc) rest


isSetextOnlyLine : Array Int -> Int -> Int -> Bool
isSetextOnlyLine codes sp len =
    if sp - len >= 0 then
        False

    else
        let
            ch =
                at sp codes
        in
        if ch /= 61 && ch /= 45 then
            False

        else
            let
                k1 =
                    skipSameChar codes sp len ch

                k2 =
                    skipSpaces codes k1 len
            in
            k2 - len == 0


stripOneBlockquotePrefix : String -> String
stripOneBlockquotePrefix text =
    String.split "\n" text
        |> List.foldl
            (\line acc ->
                let
                    s =
                        fromString line

                    sp =
                        leadingSpacesUpTo s.codes 0 s.len 3
                in
                if sp - s.len < 0 && at sp s.codes == 62 then
                    let
                        after =
                            if sp + 1 - s.len < 0 && (at (sp + 1) s.codes == 32 || at (sp + 1) s.codes == 9) then
                                sp + 2

                            else
                                sp + 1
                    in
                    String.dropLeft after line :: acc

                else
                    line :: acc
            )
            []
        |> List.reverse
        |> String.join "\n"


hasDoubleBlankLineEnd : Array Int -> Int -> Int -> Bool
hasDoubleBlankLineEnd codes start end =
    let
        i1 =
            skipSpacesTabsRev codes start end
    in
    if i1 - start <= 0 || at (i1 - 1) codes /= 10 then
        False

    else
        let
            i2 =
                skipSpacesTabsRev codes start (i1 - 1)
        in
        i2 - start > 0 && at (i2 - 1) codes == 10


skipSpacesTabsRev : Array Int -> Int -> Int -> Int
skipSpacesTabsRev codes start end =
    if end - start > 0 then
        let
            c =
                at (end - 1) codes
        in
        if c == 32 || c == 9 then
            skipSpacesTabsRev codes start (end - 1)

        else
            end

    else
        end


hasAnyLine : String -> Bool
hasAnyLine s =
    let
        src =
            fromString s
    in
    countNewlinesAtLeast2 src.codes 0 src.len 0


countNewlinesAtLeast2 : Array Int -> Int -> Int -> Int -> Bool
countNewlinesAtLeast2 codes i len count =
    if i - len >= 0 then
        False

    else if at i codes == 10 then
        if count >= 1 then
            True

        else
            countNewlinesAtLeast2 codes (i + 1) len (count + 1)

    else
        countNewlinesAtLeast2 codes (i + 1) len count


isListTask : String -> Bool
isListTask s =
    if String.length s < 5 then
        False

    else if not (String.startsWith "[ ] " s || String.startsWith "[x] " s || String.startsWith "[X] " s) then
        False

    else
        let
            src =
                fromString s

            i =
                skipSpaces src.codes 3 src.len
        in
        i > 3 && i - src.len < 0 && not (isJsSpace (at i src.codes))


stripListTask : String -> String
stripListTask s =
    if String.startsWith "[ ] " s || String.startsWith "[x] " s || String.startsWith "[X] " s then
        let
            src =
                fromString s

            i =
                skipSpaces src.codes 3 src.len
        in
        String.dropLeft i s

    else
        s


findListTaskCheckbox : String -> Maybe String
findListTaskCheckbox s =
    let
        src =
            fromString s
    in
    findListTaskCheckboxLoop src.str src.codes src.len 0


findListTaskCheckboxLoop : String -> Array Int -> Int -> Int -> Maybe String
findListTaskCheckboxLoop str codes len i =
    if i + 2 - len >= 0 then
        Nothing

    else if at i codes == 91 && at (i + 2) codes == 93 then
        let
            mid =
                at (i + 1) codes
        in
        if mid == 32 || mid == 120 || mid == 88 then
            Just (String.slice i (i + 3) str)

        else
            findListTaskCheckboxLoop str codes len (i + 1)

    else
        findListTaskCheckboxLoop str codes len (i + 1)


replacePedanticListNesting : String -> String
replacePedanticListNesting line =
    if not (String.startsWith " " line) then
        line

    else
        let
            src =
                fromString line

            sp =
                skipSpaces src.codes 0 src.len
        in
        if sp >= 1 && sp - src.len < 0 then
            let
                kStart =
                    if sp < 4 then
                        sp

                    else
                        4
            in
            case findPedanticNestK sp kStart of
                Just k ->
                    "  " ++ String.dropLeft k line

                Nothing ->
                    line

        else
            line


findPedanticNestK : Int -> Int -> Maybe Int
findPedanticNestK sp k =
    if k < 1 then
        Nothing

    else if modBy 4 (sp - k) == 0 then
        Just k

    else
        findPedanticNestK sp (k - 1)


trimTableRowBlankLine : String -> String
trimTableRowBlankLine s =
    let
        src =
            fromString s

        i =
            skipSpacesTabsRev src.codes 0 src.len
    in
    if i > 0 && at (i - 1) src.codes == 10 then
        String.slice 0 (i - 1) s

    else
        s


trimTableAlignChars : String -> String
trimTableAlignChars s =
    let
        s1 =
            if String.startsWith "|" s then
                String.dropLeft 1 s

            else
                s

        src =
            fromString s1

        end =
            skipSpacesRev src.codes 0 src.len
    in
    if end > 0 && at (end - 1) src.codes == 124 then
        String.slice 0 (end - 1) s1

    else
        s1


parseTableAlign : String -> Align
parseTableAlign cell =
    let
        src =
            fromString cell

        start =
            skipSpaces src.codes 0 src.len

        end =
            skipSpacesRev src.codes start src.len
    in
    if end - start <= 0 then
        AlignNone

    else
        let
            left =
                at start src.codes == 58

            right =
                at (end - 1) src.codes == 58

            dStart =
                if left then
                    start + 1

                else
                    start

            dEnd =
                if right then
                    end - 1

                else
                    end
        in
        if dEnd - dStart <= 0 then
            AlignNone

        else if allDashes src.codes dStart dEnd then
            case ( left, right ) of
                ( False, True ) ->
                    AlignRight

                ( True, True ) ->
                    AlignCenter

                ( True, False ) ->
                    AlignLeft

                ( False, False ) ->
                    AlignNone

        else
            AlignNone


allDashes : Array Int -> Int -> Int -> Bool
allDashes codes i end =
    if i - end >= 0 then
        True

    else if at i codes == 45 then
        allDashes codes (i + 1) end

    else
        False


removeCodeIndent : String -> String
removeCodeIndent raw =
    String.split "\n" raw
        |> List.foldl
            (\line acc ->
                let
                    s =
                        fromString line

                    sp =
                        leadingSpacesUpTo s.codes 0 s.len 4

                    strip =
                        if sp < 4 && sp - s.len < 0 && at sp s.codes == 9 then
                            sp + 1

                        else
                            sp
                in
                String.dropLeft strip line :: acc
            )
            []
        |> List.reverse
        |> String.join "\n"


indentCodeCompensation : String -> String -> String
indentCodeCompensation raw text =
    let
        rSrc =
            fromString raw

        ( indentLen, indentChars ) =
            countLeadingJsSpace rSrc.codes 0 rSrc.len 0
    in
    if
        indentLen
            == 0
            || indentLen + 3 - rSrc.len
            > 0
            || at indentLen rSrc.codes
            /= 96
            || at (indentLen + 1) rSrc.codes
            /= 96
            || at (indentLen + 2) rSrc.codes
            /= 96
    then
        text

    else
        String.split "\n" text
            |> List.foldl
                (\line acc ->
                    let
                        ls =
                            fromString line

                        stripUnits =
                            stripUpToNJsSpaces ls.codes 0 ls.len indentChars 0
                    in
                    String.dropLeft stripUnits line :: acc
                )
                []
            |> List.reverse
            |> String.join "\n"


countLeadingJsSpace : Array Int -> Int -> Int -> Int -> ( Int, Int )
countLeadingJsSpace codes i len chars =
    if i - len >= 0 then
        ( i, chars )

    else
        let
            c =
                at i codes
        in
        if isJsSpace c then
            countLeadingJsSpace codes (i + charWidth c) len (chars + 1)

        else
            ( i, chars )


stripUpToNJsSpaces : Array Int -> Int -> Int -> Int -> Int -> Int
stripUpToNJsSpaces codes i len maxChars seen =
    if i - len >= 0 || seen - maxChars >= 0 then
        i

    else
        let
            c =
                at i codes
        in
        if isJsSpace c then
            stripUpToNJsSpaces codes (i + charWidth c) len maxChars (seen + 1)

        else
            i


preprocessPedanticBlock : String -> String
preprocessPedanticBlock src =
    String.split "\n" src
        |> List.foldl
            (\line acc ->
                let
                    noTab =
                        if String.contains "\t" line then
                            String.replace "\t" "    " line

                        else
                            line
                in
                if isAllAsciiSpacesNonEmpty noTab then
                    "" :: acc

                else
                    noTab :: acc
            )
            []
        |> List.reverse
        |> String.join "\n"


isAllAsciiSpacesNonEmpty : String -> Bool
isAllAsciiSpacesNonEmpty s =
    if String.isEmpty s then
        False

    else if not (String.startsWith " " s) then
        False

    else
        let
            src =
                fromString s
        in
        skipSpaces src.codes 0 src.len - src.len == 0
