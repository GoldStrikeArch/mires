module InlineRules exposing
    ( EmStrongLMatch
    , GfmUrlSpan
    , LinkSpan
    , RDelimHit
    , RDelimKind(..)
    , ReflinkSpan
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

{-| Deterministic native matchers and delimiter maskers for all inline-level rules in
`marked@18.1.1` (`src/rules.ts`, `src/Lexer.ts`, `src/Tokenizer.ts`).
All loops are directly self-tail-recursive and operate on UTF-16-aligned `Src` / `Array Int`.
-}

import Array exposing (Array)
import BlockRules exposing (dotRunEnd, matchBlockLabel)
import Dict exposing (Dict)
import Helpers exposing (normalizeLabel)
import Tokens exposing (LinkDef, Mode, isBreaks, isGfm, isPedantic)
import Unicode
    exposing
        ( Src
        , at
        , charWidth
        , decodeRevAt
        , fromString
        , hasCode
        , isAllJsSpace
        , isAsciiAlnum
        , isAsciiAlpha
        , isJsSpace
        , isLineTerminator
        , isOpenQuote
        , isPunct
        , isPunctOrSpace
        , startsWithAsciiCi
        , toAsciiLower
        )



-- 1. ESCAPE


isAsciiEscapePunct : Int -> Bool
isAsciiEscapePunct b =
    (b >= 33 && b <= 47)
        || (b >= 58 && b <= 64)
        || (b >= 91 && b <= 96)
        || (b >= 123 && b <= 126)


matchEscape : Array Int -> Int -> Int -> Maybe Int
matchEscape codes pos len =
    if pos + 1 - len < 0 && at pos codes == 92 then
        let
            next =
                at (pos + 1) codes
        in
        if isAsciiEscapePunct next then
            Just next

        else
            Nothing

    else
        Nothing



-- 2. INLINE HTML TAG


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


scanUntilChar : Array Int -> Int -> Int -> Int -> Int
scanUntilChar codes i end ch =
    if i - end < 0 && at i codes - ch /= 0 then
        scanUntilChar codes (i + 1) end ch

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


matchInlineHtmlAttribute : Array Int -> Int -> Int -> Maybe Int
matchInlineHtmlAttribute codes pos end =
    let
        i0 =
            skipJsSpaces codes pos end
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
                    skipJsSpaces codes save end
            in
            if iEq - end < 0 && at iEq codes == 61 then
                let
                    iVal =
                        skipJsSpaces codes (iEq + 1) end
                in
                if iVal - end < 0 && at iVal codes == 34 then
                    let
                        qEnd =
                            scanUntilChar codes (iVal + 1) end 34
                    in
                    if qEnd - end < 0 && at qEnd codes == 34 then
                        Just (qEnd + 1)

                    else
                        Just save

                else if iVal - end < 0 && at iVal codes == 39 then
                    let
                        qEnd =
                            scanUntilChar codes (iVal + 1) end 39
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


skipTagNameHyphen : Array Int -> Int -> Int -> Int
skipTagNameHyphen codes i end =
    if i - end < 0 then
        let
            b =
                at i codes
        in
        if isAsciiAlnum b || b == 45 then
            skipTagNameHyphen codes (i + 1) end

        else
            i

    else
        i


skipAsciiAlpha : Array Int -> Int -> Int -> Int
skipAsciiAlpha codes i end =
    if i - end < 0 && isAsciiAlpha (at i codes) then
        skipAsciiAlpha codes (i + 1) end

    else
        i


{-| Matches `inline.tag` at `codes[pos..end)`. Returns end index.
-}
matchTag : Array Int -> Int -> Int -> Maybe Int
matchTag codes pos end =
    if pos + 3 - end > 0 || at pos codes /= 60 then
        Nothing

    else
        let
            b1 =
                at (pos + 1) codes
        in
        -- 1. Inline comment: `<!--(?:-?>|[\s\S]*?-->)`
        if b1 == 33 && pos + 4 - end <= 0 && at (pos + 2) codes == 45 && at (pos + 3) codes == 45 then
            if pos + 5 - end <= 0 && at (pos + 4) codes == 62 then
                Just (pos + 5)

            else if pos + 6 - end <= 0 && at (pos + 4) codes == 45 && at (pos + 5) codes == 62 then
                Just (pos + 6)

            else
                scanInlineCommentEnd codes (pos + 4) end

        else if b1 == 47 && pos + 4 - end <= 0 && isAsciiAlpha (at (pos + 2) codes) then
            -- 2. Closing tag: `</[a-zA-Z][a-zA-Z0-9-]*\s*>`
            let
                i1 =
                    skipTagNameHyphen codes (pos + 3) end

                i2 =
                    skipJsSpaces codes i1 end
            in
            if i2 - end < 0 && at i2 codes == 62 then
                Just (i2 + 1)

            else
                Nothing

        else if isAsciiAlpha b1 then
            -- 3. Open tag: `<[a-zA-Z][a-zA-Z0-9-]*(?:attribute)*?\s*/?>`
            let
                cur0 =
                    skipTagNameHyphen codes (pos + 2) end
            in
            scanInlineOpenTagAttrs codes end cur0

        else if b1 == 63 then
            -- 4. Processing instruction: `<\?[\s\S]*?\?>`
            scanInlinePiEnd codes (pos + 2) end

        else if b1 == 33 then
            -- 5 & 6. CDATA or Declaration
            if
                pos + 9 - end
                    <= 0
                    && at (pos + 2) codes
                    == 91
                    && at (pos + 3) codes
                    == 67
                    && at (pos + 4) codes
                    == 68
                    && at (pos + 5) codes
                    == 65
                    && at (pos + 6) codes
                    == 84
                    && at (pos + 7) codes
                    == 65
                    && at (pos + 8) codes
                    == 91
            then
                scanInlineCdataEnd codes (pos + 9) end

            else if isAsciiAlpha (at (pos + 2) codes) then
                let
                    i1 =
                        skipAsciiAlpha codes (pos + 3) end
                in
                if i1 - end < 0 && isJsSpace (at i1 codes) then
                    let
                        gt =
                            scanUntilChar codes (i1 + charWidth (at i1 codes)) end 62
                    in
                    if gt - end < 0 then
                        Just (gt + 1)

                    else
                        Nothing

                else
                    Nothing

            else
                Nothing

        else
            Nothing


scanInlineCommentEnd : Array Int -> Int -> Int -> Maybe Int
scanInlineCommentEnd codes i end =
    if i + 2 - end >= 0 then
        Nothing

    else if at i codes == 45 && at (i + 1) codes == 45 && at (i + 2) codes == 62 then
        Just (i + 3)

    else
        scanInlineCommentEnd codes (i + 1) end


scanInlineOpenTagAttrs : Array Int -> Int -> Int -> Maybe Int
scanInlineOpenTagAttrs codes end cur =
    let
        c0 =
            skipJsSpaces codes cur end

        c1 =
            if c0 - end < 0 && at c0 codes == 47 then
                c0 + 1

            else
                c0
    in
    if c1 - end < 0 && at c1 codes == 62 then
        Just (c1 + 1)

    else
        case matchInlineHtmlAttribute codes cur end of
            Just nextCur ->
                scanInlineOpenTagAttrs codes end nextCur

            Nothing ->
                Nothing


scanInlinePiEnd : Array Int -> Int -> Int -> Maybe Int
scanInlinePiEnd codes i end =
    if i + 1 - end >= 0 then
        Nothing

    else if at i codes == 63 && at (i + 1) codes == 62 then
        Just (i + 2)

    else
        scanInlinePiEnd codes (i + 1) end


scanInlineCdataEnd : Array Int -> Int -> Int -> Maybe Int
scanInlineCdataEnd codes i end =
    if i + 2 - end >= 0 then
        Nothing

    else if at i codes == 93 && at (i + 1) codes == 93 && at (i + 2) codes == 62 then
        Just (i + 3)

    else
        scanInlineCdataEnd codes (i + 1) end


isStartATag : Array Int -> Int -> Int -> Bool
isStartATag codes pos end =
    end - pos >= 3 && at pos codes == 60 && (at (pos + 1) codes == 97 || at (pos + 1) codes == 65) && at (pos + 2) codes == 32


isEndATag : Array Int -> Int -> Int -> Bool
isEndATag codes pos end =
    end - pos >= 4 && at pos codes == 60 && at (pos + 1) codes == 47 && (at (pos + 2) codes == 97 || at (pos + 2) codes == 65) && at (pos + 3) codes == 62


matchRawBlockTagName : Array Int -> Int -> Int -> Maybe Int
matchRawBlockTagName codes start end =
    let
        rem =
            end - start
    in
    if rem >= 3 && startsWithAsciiCi codes start end [ 112, 114, 101 ] then
        -- pre
        Just (start + 3)

    else if rem >= 4 && startsWithAsciiCi codes start end [ 99, 111, 100, 101 ] then
        -- code
        Just (start + 4)

    else if rem >= 3 && startsWithAsciiCi codes start end [ 107, 98, 100 ] then
        -- kbd
        Just (start + 3)

    else if rem >= 6 && startsWithAsciiCi codes start end [ 115, 99, 114, 105, 112, 116 ] then
        -- script
        Just (start + 6)

    else
        Nothing


isStartPreScriptTag : Array Int -> Int -> Int -> Bool
isStartPreScriptTag codes pos end =
    if end - pos < 4 || at pos codes /= 60 then
        False

    else
        case matchRawBlockTagName codes (pos + 1) end of
            Just after ->
                after - end < 0 && (at after codes == 62 || isJsSpace (at after codes))

            Nothing ->
                False


isEndPreScriptTag : Array Int -> Int -> Int -> Bool
isEndPreScriptTag codes pos end =
    if end - pos < 5 || at pos codes /= 60 || at (pos + 1) codes /= 47 then
        False

    else
        case matchRawBlockTagName codes (pos + 2) end of
            Just after ->
                after - end < 0 && (at after codes == 62 || isJsSpace (at after codes))

            Nothing ->
                False



-- 3. CODESPAN


skipSameChar : Array Int -> Int -> Int -> Int -> Int
skipSameChar codes i end ch =
    if i - end < 0 && at i codes - ch == 0 then
        skipSameChar codes (i + 1) end ch

    else
        i


{-| Matches `inline.code` at `codes[pos..end)`, returning `Just ( endPos, tickLen )`.
-}
matchInlineCodeAt : Array Int -> Int -> Int -> Maybe ( Int, Int )
matchInlineCodeAt codes pos end =
    if pos - end >= 0 || at pos codes /= 96 then
        Nothing

    else
        let
            bodyStart =
                skipSameChar codes pos end 96

            k =
                bodyStart - pos
        in
        if bodyStart - end >= 0 then
            Nothing

        else
            scanClosingBackticks codes end k (bodyStart + 1)


scanClosingBackticks : Array Int -> Int -> Int -> Int -> Maybe ( Int, Int )
scanClosingBackticks codes end k i =
    if i - end >= 0 then
        Nothing

    else if at i codes == 96 then
        let
            runEnd =
                skipSameChar codes i end 96

            runLen =
                runEnd - i
        in
        if runLen - k == 0 then
            Just ( runEnd, k )

        else
            scanClosingBackticks codes end k runEnd

    else
        scanClosingBackticks codes end k (i + 1)



-- 4. AUTOLINK


{-| Matches `inline.autolink` at `codes[pos..end)`, returning `Just ( endPos, isEmail )`.
-}
matchAutolink : Array Int -> Int -> Int -> Maybe ( Int, Bool )
matchAutolink codes pos end =
    if pos + 4 - end > 0 || at pos codes /= 60 then
        Nothing

    else
        case scanAutolinkGt codes (pos + 1) end of
            Nothing ->
                Nothing

            Just gt ->
                let
                    innerStart =
                        pos + 1
                in
                if isAutolinkUri codes innerStart gt then
                    Just ( gt + 1, False )

                else if isAutolinkEmail codes innerStart gt then
                    Just ( gt + 1, True )

                else
                    Nothing


scanAutolinkGt : Array Int -> Int -> Int -> Maybe Int
scanAutolinkGt codes j end =
    if j - end >= 0 then
        Nothing

    else
        let
            b =
                at j codes
        in
        if b == 62 then
            Just j

        else if b == 60 || b < 0x20 then
            Nothing

        else
            scanAutolinkGt codes (j + 1) end


isAutolinkUri : Array Int -> Int -> Int -> Bool
isAutolinkUri codes start end =
    if start - end >= 0 || not (isAsciiAlpha (at start codes)) then
        False

    else
        let
            j =
                scanSchemeRest codes (start + 1) end 1
        in
        if j - start >= 2 && j - end < 0 && at j codes == 58 then
            checkUriBody codes (j + 1) end

        else
            False


scanSchemeRest : Array Int -> Int -> Int -> Int -> Int
scanSchemeRest codes j end count =
    if count < 32 && j - end < 0 then
        let
            b =
                at j codes
        in
        if isAsciiAlnum b || b == 43 || b == 46 || b == 45 then
            scanSchemeRest codes (j + 1) end (count + 1)

        else
            j

    else
        j


checkUriBody : Array Int -> Int -> Int -> Bool
checkUriBody codes k end =
    if k - end >= 0 then
        True

    else
        let
            b =
                at k codes
        in
        if b < 0x20 || b == 60 || b == 62 || isJsSpace b then
            False

        else
            checkUriBody codes (k + charWidth b) end


isAutolinkEmailLocalChar : Int -> Bool
isAutolinkEmailLocalChar b =
    isAsciiAlnum b
        || b
        == 46
        || b
        == 33
        || b
        == 35
        || b
        == 36
        || b
        == 37
        || b
        == 38
        || b
        == 39
        || b
        == 42
        || b
        == 43
        || b
        == 47
        || b
        == 61
        || b
        == 63
        || b
        == 94
        || b
        == 95
        || b
        == 96
        || b
        == 123
        || b
        == 124
        || b
        == 125
        || b
        == 126
        || b
        == 45


isAutolinkEmail : Array Int -> Int -> Int -> Bool
isAutolinkEmail codes start end =
    case findCodeInSpan codes start end 64 of
        Nothing ->
            False

        Just atIdx ->
            if atIdx - start <= 0 then
                False

            else if allAutolinkLocal codes start atIdx then
                isAutolinkEmailDomain codes (atIdx + 1) end 0

            else
                False


findCodeInSpan : Array Int -> Int -> Int -> Int -> Maybe Int
findCodeInSpan codes i end target =
    if i - end >= 0 then
        Nothing

    else if at i codes - target == 0 then
        Just i

    else
        findCodeInSpan codes (i + 1) end target


allAutolinkLocal : Array Int -> Int -> Int -> Bool
allAutolinkLocal codes k end =
    if k - end >= 0 then
        True

    else if isAutolinkEmailLocalChar (at k codes) then
        allAutolinkLocal codes (k + 1) end

    else
        False


isAutolinkEmailDomain : Array Int -> Int -> Int -> Int -> Bool
isAutolinkEmailDomain codes start end parts =
    let
        segEnd =
            scanUntilDot codes start end

        segLen =
            segEnd - start
    in
    if segLen <= 0 || segLen > 63 then
        False

    else if not (isAsciiAlnum (at start codes)) || not (isAsciiAlnum (at (segEnd - 1) codes)) then
        False

    else if not (allDomainSegChars codes start segEnd) then
        False

    else if segEnd - end == 0 then
        parts + 1 >= 2

    else
        isAutolinkEmailDomain codes (segEnd + 1) end (parts + 1)


scanUntilDot : Array Int -> Int -> Int -> Int
scanUntilDot codes i end =
    if i - end < 0 && at i codes /= 46 then
        scanUntilDot codes (i + 1) end

    else
        i


allDomainSegChars : Array Int -> Int -> Int -> Bool
allDomainSegChars codes k end =
    if k - end >= 0 then
        True

    else
        let
            b =
                at k codes
        in
        if isAsciiAlnum b || b == 45 then
            allDomainSegChars codes (k + 1) end

        else
            False



-- 5. INLINE LINK & REFLINK LABELS


skipInnerBrackets : Array Int -> Int -> Int -> Maybe Int
skipInnerBrackets codes pos end =
    if pos - end >= 0 || at pos codes /= 91 then
        Nothing

    else
        skipInnerBracketsLoop codes (pos + 1) end


skipInnerBracketsLoop : Array Int -> Int -> Int -> Maybe Int
skipInnerBracketsLoop codes i end =
    if i - end >= 0 then
        Nothing

    else
        let
            b =
                at i codes
        in
        if b == 93 then
            Just (i + 1)

        else if b == 91 then
            Nothing

        else if b == 92 then
            if i + 1 - end >= 0 then
                Nothing

            else
                skipInnerBracketsLoop codes (i + 1 + charWidth (at (i + 1) codes)) end

        else
            skipInnerBracketsLoop codes (i + 1) end


skipOuterBrackets : Array Int -> Int -> Int -> Maybe Int
skipOuterBrackets codes pos end =
    if pos - end >= 0 || at pos codes /= 91 then
        Nothing

    else
        skipOuterBracketsLoop codes (pos + 1) end


skipOuterBracketsLoop : Array Int -> Int -> Int -> Maybe Int
skipOuterBracketsLoop codes i end =
    if i - end >= 0 then
        Nothing

    else
        let
            b =
                at i codes
        in
        if b == 93 then
            Just (i + 1)

        else if b == 91 then
            case skipInnerBrackets codes i end of
                Just nextI ->
                    skipOuterBracketsLoop codes nextI end

                Nothing ->
                    Nothing

        else if b == 92 then
            if i + 1 - end >= 0 then
                Nothing

            else
                skipOuterBracketsLoop codes (i + 1 + charWidth (at (i + 1) codes)) end

        else
            skipOuterBracketsLoop codes (i + 1) end


{-| Returns candidate closing `]` indices for `_inlineLabel` / `_boundedInlineLabel` (`rules.ts:465,498`).

The greedy `` `+(?!`)[^`]*?`+(?!`) `` alternative precedes `` ``+(?=\]) ``, so a direct `]`
reached after pairing backticks is returned first, followed by every `` ``] `` fallback along
the path in LIFO order (latest/rightmost first).
-}
findInlineLabelEnds : Array Int -> Int -> Int -> Int -> List Int
findInlineLabelEnds codes start end maxSpecials =
    findInlineLabelEndsLoop codes end maxSpecials start 0 []


findInlineLabelEndsLoop : Array Int -> Int -> Int -> Int -> Int -> List Int -> List Int
findInlineLabelEndsLoop codes end maxSpecials i specials altEnds =
    if i - end >= 0 then
        altEnds

    else
        let
            b =
                at i codes
        in
        if b == 93 then
            i :: altEnds

        else if b == 91 then
            if maxSpecials >= 0 && specials - maxSpecials >= 0 then
                altEnds

            else
                case skipOuterBrackets codes i end of
                    Just nextI ->
                        findInlineLabelEndsLoop codes end maxSpecials nextI (specials + 1) altEnds

                    Nothing ->
                        altEnds

        else if b == 92 then
            if maxSpecials >= 0 && specials - maxSpecials >= 0 then
                altEnds

            else if i + 1 - end >= 0 then
                altEnds

            else
                findInlineLabelEndsLoop codes end maxSpecials (i + 1 + charWidth (at (i + 1) codes)) (specials + 1) altEnds

        else if b == 96 then
            if maxSpecials >= 0 && specials - maxSpecials >= 0 then
                altEnds

            else
                let
                    afterRun =
                        skipSameChar codes i end 96

                    run =
                        afterRun - i

                    nextAlts =
                        if run >= 2 && afterRun - end < 0 && at afterRun codes == 93 then
                            afterRun :: altEnds

                        else
                            altEnds

                    j =
                        scanUntilChar codes afterRun end 96
                in
                if j - end < 0 && at j codes == 96 then
                    let
                        afterClose =
                            skipSameChar codes j end 96
                    in
                    findInlineLabelEndsLoop codes end maxSpecials afterClose (specials + 1) nextAlts

                else
                    nextAlts

        else
            findInlineLabelEndsLoop codes end maxSpecials (i + 1) specials altEnds


{-| Exact port of `isLabelEndInsideToken` (`Tokenizer.ts`).
-}
isLabelEndInsideToken : Array Int -> Int -> Int -> Int -> Bool
isLabelEndInsideToken codes totalLen labelStart labelEnd =
    if not (hasCode 60 codes labelStart labelEnd) then
        False

    else
        isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd 0


isLabelEndInsideTokenLoop : Array Int -> Int -> Int -> Int -> Int -> Bool
isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd i =
    let
        labelLen =
            labelEnd - labelStart
    in
    if i - labelLen >= 0 then
        False

    else
        let
            absPos =
                labelStart + i

            b =
                at absPos codes
        in
        if b == 92 then
            let
                nextI =
                    if i + 1 - labelLen < 0 then
                        i + 1 + charWidth (at (absPos + 1) codes)

                    else
                        i + 1
            in
            isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd nextI

        else if b == 96 then
            case matchInlineCodeAt codes absPos labelEnd of
                Just ( endAbs, _ ) ->
                    isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd (endAbs - labelStart)

                Nothing ->
                    isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd (i + 1)

        else if b /= 60 then
            isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd (i + 1)

        else
            let
                tokEnd =
                    case matchTag codes absPos totalLen of
                        Just e ->
                            Just e

                        Nothing ->
                            case matchAutolink codes absPos totalLen of
                                Just ( e, _ ) ->
                                    Just e

                                Nothing ->
                                    Nothing
            in
            case tokEnd of
                Just e ->
                    let
                        len =
                            e - absPos
                    in
                    if len - (labelLen - i) > 0 then
                        True

                    else
                        isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd (i + len)

                Nothing ->
                    isLabelEndInsideTokenLoop codes totalLen labelStart labelEnd (i + 1)



-- 6. INLINE LINK


matchLinkCloseParen : Array Int -> Int -> Int -> Maybe Int
matchLinkCloseParen codes pos end =
    let
        j =
            skipSpaceTabNl codes pos end
    in
    if j - end < 0 && at j codes == 41 then
        Just (j + 1)

    else
        Nothing


{-| Matches `inline.link`'s title (`rules.ts:472`: `"(?:\\"?|[^"\\])*"`, `'(?:\\'?|[^'\\])*'`,
`\((?:\\\)?|[^)\\])*\)`) followed by `[ \t\n]*\)`, returning `Just ( titleEnd, rawEnd )`.

The optional delimiter escape (`\\"?`, `\\'?`, `\\\)?`) greedily consumes `\` + `delim` first,
then backtracks to `\` so `delim` closes the title; rightmost matching candidate wins.
-}
matchInlineLinkTitleAndEnd : Array Int -> Int -> Int -> Maybe ( Int, Int )
matchInlineLinkTitleAndEnd codes pos end =
    if pos - end >= 0 then
        Nothing

    else
        let
            b =
                at pos codes
        in
        if b == 34 then
            scanEscapedDelimAndEnd codes (pos + 1) end 34 Nothing

        else if b == 39 then
            scanEscapedDelimAndEnd codes (pos + 1) end 39 Nothing

        else if b == 40 then
            scanEscapedDelimAndEnd codes (pos + 1) end 41 Nothing

        else
            Nothing


scanEscapedDelimAndEnd : Array Int -> Int -> Int -> Int -> Maybe ( Int, Int ) -> Maybe ( Int, Int )
scanEscapedDelimAndEnd codes i end delim best =
    if i - end >= 0 then
        best

    else
        let
            b =
                at i codes
        in
        if b == 92 then
            if i + 1 - end < 0 && at (i + 1) codes - delim == 0 then
                let
                    nextBest =
                        case matchLinkCloseParen codes (i + 2) end of
                            Just rawEnd ->
                                Just ( i + 2, rawEnd )

                            Nothing ->
                                best
                in
                scanEscapedDelimAndEnd codes (i + 2) end delim nextBest

            else
                scanEscapedDelimAndEnd codes (i + 1) end delim best

        else if b - delim == 0 then
            case matchLinkCloseParen codes (i + 1) end of
                Just rawEnd ->
                    Just ( i + 1, rawEnd )

                Nothing ->
                    best

        else
            scanEscapedDelimAndEnd codes (i + 1) end delim best


skipSpaceTabNl : Array Int -> Int -> Int -> Int
skipSpaceTabNl codes i end =
    if i - end < 0 then
        let
            c =
                at i codes
        in
        if c == 32 || c == 9 || c == 10 then
            skipSpaceTabNl codes (i + 1) end

        else
            i

    else
        i


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


matchLinkTail : Array Int -> Int -> Int -> Maybe ( Int, Maybe ( Int, Int ) )
matchLinkTail codes pos end =
    let
        i0 =
            skipSpacesTabs codes pos end

        hasSpace =
            i0 - pos > 0

        ( i1, hasNl ) =
            if i0 - end < 0 && at i0 codes == 10 then
                ( skipSpacesTabs codes (i0 + 1) end, True )

            else
                ( i0, False )

        withTitle =
            if hasSpace || hasNl then
                case matchInlineLinkTitleAndEnd codes i1 end of
                    Just ( titleEnd, rawEnd ) ->
                        Just ( rawEnd, Just ( i1, titleEnd ) )

                    Nothing ->
                        Nothing

            else
                Nothing
    in
    case withTitle of
        Just res ->
            Just res

        Nothing ->
            Maybe.map (\rawEnd -> ( rawEnd, Nothing )) (matchLinkCloseParen codes pos end)


type alias LinkSpan =
    { rawEnd : Int
    , isImage : Bool
    , labelStart : Int
    , labelEnd : Int
    , hrefStart : Int
    , hrefEnd : Int
    , title : Maybe ( Int, Int )
    }


matchInlineLink : Array Int -> Int -> Int -> Bool -> Maybe LinkSpan
matchInlineLink codes pos end pedantic =
    let
        maybeOpen =
            if pos + 1 - end < 0 && at pos codes == 33 && at (pos + 1) codes == 91 then
                Just ( True, pos + 1 )

            else if pos - end < 0 && at pos codes == 91 then
                Just ( False, pos )

            else
                Nothing
    in
    case maybeOpen of
        Nothing ->
            Nothing

        Just ( isImage, openBracket ) ->
            let
                labelStart =
                    openBracket + 1

                ends =
                    findInlineLabelEnds codes labelStart end -1
            in
            tryLinkLabelEnds codes end pedantic isImage labelStart ends


tryLinkLabelEnds : Array Int -> Int -> Bool -> Bool -> Int -> List Int -> Maybe LinkSpan
tryLinkLabelEnds codes end pedantic isImage labelStart ends =
    case ends of
        [] ->
            Nothing

        closeBracket :: rest ->
            if closeBracket + 1 - end >= 0 || at (closeBracket + 1) codes /= 40 then
                tryLinkLabelEnds codes end pedantic isImage labelStart rest

            else
                let
                    parenOpen =
                        closeBracket + 1

                    attempt =
                        if pedantic then
                            matchPedanticLinkDest codes end isImage labelStart closeBracket parenOpen

                        else
                            matchNormalLinkDest codes end isImage labelStart closeBracket parenOpen
                in
                case attempt of
                    Just span ->
                        Just span

                    Nothing ->
                        tryLinkLabelEnds codes end pedantic isImage labelStart rest


matchPedanticLinkDest : Array Int -> Int -> Bool -> Int -> Int -> Int -> Maybe LinkSpan
matchPedanticLinkDest codes end isImage labelStart closeBracket parenOpen =
    let
        j =
            scanUntilRparenOrLineEnd codes (parenOpen + 1) end
    in
    if j - end < 0 && at j codes == 41 then
        Just
            { rawEnd = j + 1
            , isImage = isImage
            , labelStart = labelStart
            , labelEnd = closeBracket
            , hrefStart = parenOpen + 1
            , hrefEnd = j
            , title = Nothing
            }

    else
        Nothing


{-| Pedantic `\((.*?)\)`: the href is the shortest `.` run up to a `)`.
-}
scanUntilRparenOrLineEnd : Array Int -> Int -> Int -> Int
scanUntilRparenOrLineEnd codes j end =
    if j - end < 0 then
        let
            b =
                at j codes
        in
        if b == 41 || isLineTerminator b then
            j

        else
            scanUntilRparenOrLineEnd codes (j + 1) end

    else
        j


matchNormalLinkDest : Array Int -> Int -> Bool -> Int -> Int -> Int -> Maybe LinkSpan
matchNormalLinkDest codes end isImage labelStart closeBracket parenOpen =
    let
        i =
            skipSpaceTabNl codes (parenOpen + 1) end
    in
    if i - end >= 0 then
        Nothing

    else
        let
            angleAttempt =
                if at i codes == 60 then
                    case scanLinkAngleHref codes (i + 1) end of
                        Just gt ->
                            if gt - (i + 1) > 0 then
                                case matchLinkTail codes (gt + 1) end of
                                    Just ( rawEnd, title ) ->
                                        Just
                                            { rawEnd = rawEnd
                                            , isImage = isImage
                                            , labelStart = labelStart
                                            , labelEnd = closeBracket
                                            , hrefStart = i
                                            , hrefEnd = gt + 1
                                            , title = title
                                            }

                                    Nothing ->
                                        Nothing

                            else
                                Nothing

                        Nothing ->
                            Nothing

                else
                    Nothing

            bareAttempt =
                case angleAttempt of
                    Just span ->
                        Just span

                    Nothing ->
                        let
                            runEnd =
                                scanBareLinkHref codes i end
                        in
                        if runEnd - i <= 0 then
                            Nothing

                        else
                            case matchLinkTail codes runEnd end of
                                Just ( rawEnd, title ) ->
                                    Just
                                        { rawEnd = rawEnd
                                        , isImage = isImage
                                        , labelStart = labelStart
                                        , labelEnd = closeBracket
                                        , hrefStart = i
                                        , hrefEnd = runEnd
                                        , title = title
                                        }

                                Nothing ->
                                    backtrackBareHrefRparen codes isImage labelStart closeBracket i runEnd
        in
        case bareAttempt of
            Just span ->
                Just span

            Nothing ->
                -- Third `href` alternative `(?=\))` (`rules.ts:471`): only tried after bare href fails.
                if at i codes == 41 then
                    Just
                        { rawEnd = i + 1
                        , isImage = isImage
                        , labelStart = labelStart
                        , labelEnd = closeBracket
                        , hrefStart = i
                        , hrefEnd = i
                        , title = Nothing
                        }

                else
                    Nothing


scanLinkAngleHref : Array Int -> Int -> Int -> Maybe Int
scanLinkAngleHref codes j end =
    if j - end >= 0 then
        Nothing

    else
        let
            b =
                at j codes
        in
        if b == 62 then
            Just j

        else if b == 10 || b == 60 then
            Nothing

        else if b == 92 then
            -- `\\.`: an escape takes any character but a line terminator.
            if j + 1 - end < 0 && not (isLineTerminator (at (j + 1) codes)) then
                scanLinkAngleHref codes (j + 1 + charWidth (at (j + 1) codes)) end

            else
                Nothing

        else
            scanLinkAngleHref codes (j + 1) end


scanBareLinkHref : Array Int -> Int -> Int -> Int
scanBareLinkHref codes i end =
    if i - end < 0 && at i codes > 0x20 then
        scanBareLinkHref codes (i + 1) end

    else
        i


backtrackBareHrefRparen : Array Int -> Bool -> Int -> Int -> Int -> Int -> Maybe LinkSpan
backtrackBareHrefRparen codes isImage labelStart closeBracket hrefStart cand =
    if cand - hrefStart <= 0 then
        Nothing

    else
        let
            c =
                cand - 1
        in
        if at c codes == 41 && c - hrefStart > 0 then
            Just
                { rawEnd = c + 1
                , isImage = isImage
                , labelStart = labelStart
                , labelEnd = closeBracket
                , hrefStart = hrefStart
                , hrefEnd = c
                , title = Nothing
                }

        else
            backtrackBareHrefRparen codes isImage labelStart closeBracket hrefStart c


{-| Matches `other.pedanticHrefTitle` `^([^'"]*[^\s])\s+(['"])(.*)\2` on a pedantic href,
returning `( href, title )`.

The regex backtracks `[^'"]*` from the first quote down to the start, so the href ends at the
last split point (at most just past the first quote) where a non-space character is followed by
a whitespace run and then a quote; that non-space character may itself be the first quote. The
title then runs to the last matching quote before the end of the `.*` run.
-}
matchPedanticHrefTitle : String -> Maybe ( String, String )
matchPedanticHrefTitle href =
    let
        src =
            fromString href

        q =
            findFirstQuote src.codes 0 src.len
    in
    if q - src.len >= 0 then
        Nothing

    else
        matchPedanticHrefTitleAt href src.codes src.len (q + 1)


matchPedanticHrefTitleAt : String -> Array Int -> Int -> Int -> Maybe ( String, String )
matchPedanticHrefTitleAt href codes n hrefEnd =
    if hrefEnd <= 0 then
        Nothing

    else
        let
            ( c, w ) =
                decodeRevAt codes hrefEnd

            shorter =
                hrefEnd - w

            q =
                skipJsSpaces codes hrefEnd n

            quoteChar =
                at q codes
        in
        if isJsSpace c || q - hrefEnd == 0 || q - n >= 0 || (quoteChar /= 39 && quoteChar /= 34) then
            matchPedanticHrefTitleAt href codes n shorter

        else
            case findLastQuote codes quoteChar q (dotRunEnd codes (q + 1) n - 1) of
                Just closeQ ->
                    Just ( String.slice 0 hrefEnd href, String.slice (q + 1) closeQ href )

                Nothing ->
                    matchPedanticHrefTitleAt href codes n shorter


findFirstQuote : Array Int -> Int -> Int -> Int
findFirstQuote codes q n =
    if q - n < 0 then
        let
            b =
                at q codes
        in
        if b == 39 || b == 34 then
            q

        else
            findFirstQuote codes (q + 1) n

    else
        q


findLastQuote : Array Int -> Int -> Int -> Int -> Maybe Int
findLastQuote codes quoteChar q k =
    if k - q <= 0 then
        Nothing

    else if at k codes - quoteChar == 0 then
        Just k

    else
        findLastQuote codes quoteChar q (k - 1)


-- 7. REFLINK / NOLINK


type alias ReflinkSpan =
    { rawEnd : Int
    , isImage : Bool
    , labelStart : Int
    , labelEnd : Int
    , refSpan : Maybe ( Int, Int )
    }


matchReflinkAt : Array Int -> Int -> Int -> Bool -> Bool -> Maybe ReflinkSpan
matchReflinkAt codes pos end pedantic bounded =
    let
        maybeOpen =
            if pos + 1 - end < 0 && at pos codes == 33 && at (pos + 1) codes == 91 then
                Just ( True, pos + 1 )

            else if pos - end < 0 && at pos codes == 91 then
                Just ( False, pos )

            else
                Nothing
    in
    case maybeOpen of
        Nothing ->
            Nothing

        Just ( isImage, openBracket ) ->
            let
                labelStart =
                    openBracket + 1

                maxSp =
                    if bounded then
                        999

                    else
                        -1

                maxSpMaybe =
                    if bounded then
                        Just 999

                    else
                        Nothing

                ends =
                    findInlineLabelEnds codes labelStart end maxSp
            in
            case tryReflinkEnds codes end pedantic bounded maxSpMaybe isImage labelStart ends of
                Just span ->
                    Just span

                Nothing ->
                    -- nolink
                    case matchBlockLabel codes labelStart end maxSpMaybe of
                        Nothing ->
                            Nothing

                        Just close ->
                            let
                                withEmptyRef =
                                    close + 2 - end < 0 && at (close + 1) codes == 91 && at (close + 2) codes == 93

                                fullEnd =
                                    if withEmptyRef then
                                        close + 3

                                    else
                                        close + 1

                                -- `nolink(?!\()`: when `[]` is followed by `(`, `(?:\[\])?` backtracks
                                -- to empty and the lookahead then sees the `[`.
                                parenAfter =
                                    bounded && fullEnd - end < 0 && at fullEnd codes == 40
                            in
                            if parenAfter && not withEmptyRef then
                                Nothing

                            else
                                Just
                                    { rawEnd =
                                        if parenAfter then
                                            close + 1

                                        else
                                            fullEnd
                                    , isImage = isImage
                                    , labelStart = labelStart
                                    , labelEnd = close
                                    , refSpan = Nothing
                                    }


tryReflinkEnds : Array Int -> Int -> Bool -> Bool -> Maybe Int -> Bool -> Int -> List Int -> Maybe ReflinkSpan
tryReflinkEnds codes end pedantic bounded maxSpMaybe isImage labelStart ends =
    case ends of
        [] ->
            Nothing

        closeBracket :: rest ->
            let
                rOpen0 =
                    closeBracket + 1
            in
            if pedantic && not bounded then
                let
                    rOpen =
                        skipJsSpaces codes rOpen0 end
                in
                if rOpen - end < 0 && at rOpen codes == 91 then
                    let
                        rClose =
                            scanUntilChar codes (rOpen + 1) end 93
                    in
                    if rClose - end < 0 && at rClose codes == 93 then
                        Just
                            { rawEnd = rClose + 1
                            , isImage = isImage
                            , labelStart = labelStart
                            , labelEnd = closeBracket
                            , refSpan =
                                if rClose - (rOpen + 1) > 0 then
                                    Just ( rOpen + 1, rClose )

                                else
                                    Nothing
                            }

                    else
                        tryReflinkEnds codes end pedantic bounded maxSpMaybe isImage labelStart rest

                else
                    tryReflinkEnds codes end pedantic bounded maxSpMaybe isImage labelStart rest

            else if rOpen0 - end < 0 && at rOpen0 codes == 91 then
                case matchBlockLabel codes (rOpen0 + 1) end maxSpMaybe of
                    Just rClose ->
                        Just
                            { rawEnd = rClose + 1
                            , isImage = isImage
                            , labelStart = labelStart
                            , labelEnd = closeBracket
                            , refSpan = Just ( rOpen0 + 1, rClose )
                            }

                    Nothing ->
                        tryReflinkEnds codes end pedantic bounded maxSpMaybe isImage labelStart rest

            else
                tryReflinkEnds codes end pedantic bounded maxSpMaybe isImage labelStart rest



-- 8. PRE-SCAN DELIMITER MASKING (blockSkip, reflinkSearch, anyPunctuationOutsideCode)


matchBlockSkipCodeAt : Array Int -> Int -> Int -> Maybe ( Int, Int )
matchBlockSkipCodeAt codes pos end =
    let
        bodyStart =
            skipSameChar codes pos end 96

        k =
            bodyStart - pos
    in
    if k == 0 || bodyStart - end >= 0 || at bodyStart codes == 96 then
        Nothing

    else
        let
            i =
                scanUntilChar codes (bodyStart + 1) end 96
        in
        if i - end >= 0 then
            Nothing

        else
            let
                runEnd =
                    skipSameChar codes i end 96
            in
            if (runEnd - i) - k == 0 then
                Just ( runEnd, k )

            else
                Nothing


matchBlockSkipAt : Array Int -> Int -> Int -> Maybe Int
matchBlockSkipAt codes pos end =
    if pos - end >= 0 then
        Nothing

    else
        let
            b =
                at pos codes
        in
        if b == 91 then
            -- link
            case scanBlockSkipBracket codes (pos + 1) end of
                Nothing ->
                    Nothing

                Just cb ->
                    if cb + 1 - end >= 0 || at (cb + 1) codes /= 40 then
                        Nothing

                    else
                        case scanBlockSkipParens codes (cb + 2) end of
                            Just afterRparen ->
                                Just (afterRparen - pos)

                            Nothing ->
                                Nothing

        else if b == 96 then
            -- (?<!`)code
            if pos > 0 && at (pos - 1) codes == 96 then
                Nothing

            else
                case matchBlockSkipCodeAt codes pos end of
                    Just ( codeEnd, _ ) ->
                        Just (codeEnd - pos)

                    Nothing ->
                        Nothing

        else if b == 60 then
            -- <(?! )[^<>]*?>
            if pos + 1 - end < 0 && at (pos + 1) codes == 32 then
                Nothing

            else
                scanBlockSkipHtml codes pos (pos + 1) end

        else
            Nothing


scanBlockSkipBracket : Array Int -> Int -> Int -> Maybe Int
scanBlockSkipBracket codes i end =
    if i - end >= 0 then
        Nothing

    else
        let
            b =
                at i codes
        in
        if b == 93 then
            Just i

        else if b == 91 then
            Nothing

        else if b == 96 then
            case matchBlockSkipCodeAt codes i end of
                Just ( codeEnd, _ ) ->
                    scanBlockSkipBracket codes codeEnd end

                Nothing ->
                    Nothing

        else
            scanBlockSkipBracket codes (i + 1) end


scanBlockSkipParens : Array Int -> Int -> Int -> Maybe Int
scanBlockSkipParens codes j end =
    if j - end >= 0 then
        Nothing

    else
        let
            b =
                at j codes
        in
        if b == 41 then
            Just (j + 1)

        else if b == 92 then
            if j + 1 - end >= 0 then
                Nothing

            else
                scanBlockSkipParens codes (j + 1 + charWidth (at (j + 1) codes)) end

        else if b == 40 then
            case scanBlockSkipNestedParen codes (j + 1) end of
                Just afterInner ->
                    scanBlockSkipParens codes afterInner end

                Nothing ->
                    Nothing

        else
            scanBlockSkipParens codes (j + 1) end


scanBlockSkipNestedParen : Array Int -> Int -> Int -> Maybe Int
scanBlockSkipNestedParen codes j end =
    if j - end >= 0 then
        Nothing

    else
        let
            b =
                at j codes
        in
        if b == 41 then
            Just (j + 1)

        else if b == 40 then
            Nothing

        else if b == 92 then
            if j + 1 - end >= 0 then
                Nothing

            else
                scanBlockSkipNestedParen codes (j + 1 + charWidth (at (j + 1) codes)) end

        else
            scanBlockSkipNestedParen codes (j + 1) end


scanBlockSkipHtml : Array Int -> Int -> Int -> Int -> Maybe Int
scanBlockSkipHtml codes pos j end =
    if j - end >= 0 then
        Nothing

    else
        let
            b =
                at j codes
        in
        if b == 62 then
            Just ((j + 1) - pos)

        else if b == 60 then
            Nothing

        else
            scanBlockSkipHtml codes pos (j + 1) end


findLastCodeInSpan : Array Int -> Int -> Int -> Int -> Maybe Int
findLastCodeInSpan codes start end target =
    findLastCodeLoop codes start target (end - 1)


findLastCodeLoop : Array Int -> Int -> Int -> Int -> Maybe Int
findLastCodeLoop codes start target i =
    if i - start < 0 then
        Nothing

    else if at i codes - target == 0 then
        Just i

    else
        findLastCodeLoop codes start target (i - 1)


linkInText : Src -> Int -> Int -> Dict String LinkDef -> Bool -> Bool
linkInText src start end links pedantic =
    if not (hasCode 91 src.codes start end) then
        False

    else if linkInTextBlockSkip src.codes start end pedantic start then
        True

    else
        linkInTextReflinks src start end links pedantic start


linkInTextBlockSkip : Array Int -> Int -> Int -> Bool -> Int -> Bool
linkInTextBlockSkip codes start end pedantic i =
    if i - end >= 0 then
        False

    else
        case matchBlockSkipAt codes i end of
            Just len ->
                if
                    at i codes
                        == 91
                        && not (i - start > 0 && at (i - 1) codes == 33)
                        && isJust (matchInlineLink codes i (i + len) pedantic)
                then
                    True

                else
                    linkInTextBlockSkip codes start end pedantic (i + len)

            Nothing ->
                linkInTextBlockSkip codes start end pedantic (i + 1)


linkInTextReflinks : Src -> Int -> Int -> Dict String LinkDef -> Bool -> Int -> Bool
linkInTextReflinks src start end links pedantic j =
    if j - end >= 0 then
        False

    else
        let
            b =
                at j src.codes
        in
        if b == 33 || b == 91 then
            case matchReflinkAt src.codes j end False True of
                Just m ->
                    let
                        found =
                            if not m.isImage then
                                case findLastCodeInSpan src.codes j m.rawEnd 91 of
                                    Just refStart ->
                                        let
                                            key =
                                                normalizeLabel (String.slice (refStart + 1) (m.rawEnd - 1) src.str)
                                        in
                                        if Dict.member key links then
                                            let
                                                nested =
                                                    (refStart - j)
                                                        > 1
                                                        && linkInText src (j + 1) (refStart - 1) links pedantic
                                            in
                                            not nested

                                        else
                                            False

                                    Nothing ->
                                        False

                            else
                                False
                    in
                    if found then
                        True

                    else
                        linkInTextReflinks src start end links pedantic m.rawEnd

                Nothing ->
                    linkInTextReflinks src start end links pedantic (j + 1)

        else
            linkInTextReflinks src start end links pedantic (j + 1)


isJust : Maybe a -> Bool
isJust m =
    case m of
        Just _ ->
            True

        Nothing ->
            False


{-| Builds `maskedCodes : Array Int` (exact same UTF-16 length as `src.codes`).
-}
buildMaskedCodes : Src -> Dict String LinkDef -> Bool -> Array Int
buildMaskedCodes src links pedantic =
    let
        n =
            src.len

        arr1 =
            if not (Dict.isEmpty links) && hasCode 91 src.codes 0 n then
                replaceReflinkSearch src 0 n links pedantic []
                    |> List.reverse
                    |> Array.fromList

            else
                src.codes

        arr2 =
            if hasCode 96 arr1 0 n || hasCode 92 arr1 0 n then
                maskAnyPunctOutsideCode arr1 0 n

            else
                arr1

        arr3 =
            if hasCode 91 arr2 0 n || hasCode 96 arr2 0 n || hasCode 60 arr2 0 n then
                maskBlockSkip arr2 0 n

            else
                arr2
    in
    arr3


pushCodesRange : Array Int -> Int -> Int -> List Int -> List Int
pushCodesRange codes i end acc =
    if i - end >= 0 then
        acc

    else
        pushCodesRange codes (i + 1) end (at i codes :: acc)


pushRepeatCode : Int -> Int -> List Int -> List Int
pushRepeatCode code count acc =
    if count <= 0 then
        acc

    else
        pushRepeatCode code (count - 1) (code :: acc)


replaceReflinkSearch : Src -> Int -> Int -> Dict String LinkDef -> Bool -> List Int -> List Int
replaceReflinkSearch src start end links pedantic acc =
    replaceReflinkSearchLoop src end links pedantic start start acc


replaceReflinkSearchLoop : Src -> Int -> Dict String LinkDef -> Bool -> Int -> Int -> List Int -> List Int
replaceReflinkSearchLoop src end links pedantic last i acc =
    if i - end >= 0 then
        pushCodesRange src.codes last end acc

    else
        let
            b =
                at i src.codes
        in
        if b == 33 || b == 91 then
            case matchReflinkAt src.codes i end False True of
                Just m ->
                    let
                        acc0 =
                            pushCodesRange src.codes last i acc

                        accNext =
                            case findLastCodeInSpan src.codes i m.rawEnd 91 of
                                Just refStart ->
                                    let
                                        key =
                                            normalizeLabel (String.slice (refStart + 1) (m.rawEnd - 1) src.str)
                                    in
                                    if Dict.member key links then
                                        if (refStart - i) > 1 && not m.isImage && linkInText src (i + 1) (refStart - 1) links pedantic then
                                            let
                                                a1 =
                                                    91 :: acc0

                                                a2 =
                                                    replaceReflinkSearch src (i + 1) (refStart - 1) links pedantic a1

                                                a3 =
                                                    91 :: 93 :: a2

                                                a4 =
                                                    pushRepeatCode 97 (m.rawEnd - refStart - 2) a3
                                            in
                                            93 :: a4

                                        else
                                            let
                                                a1 =
                                                    91 :: acc0

                                                a2 =
                                                    pushRepeatCode 97 (m.rawEnd - i - 2) a1
                                            in
                                            93 :: a2

                                    else
                                        pushCodesRange src.codes i m.rawEnd acc0

                                Nothing ->
                                    pushCodesRange src.codes i m.rawEnd acc0
                    in
                    replaceReflinkSearchLoop src end links pedantic m.rawEnd m.rawEnd accNext

                Nothing ->
                    replaceReflinkSearchLoop src end links pedantic last (i + 1) acc

        else
            replaceReflinkSearchLoop src end links pedantic last (i + 1) acc


maskAnyPunctOutsideCode : Array Int -> Int -> Int -> Array Int
maskAnyPunctOutsideCode arr i n =
    if i - n >= 0 then
        arr

    else
        let
            c =
                at i arr
        in
        if c == 96 then
            case matchInlineCodeAt arr i n of
                Just ( codeEnd, k ) ->
                    let
                        arr1 =
                            replaceTicksInSpan arr (i + k) (codeEnd - k)
                    in
                    maskAnyPunctOutsideCode arr1 codeEnd n

                Nothing ->
                    maskAnyPunctOutsideCode arr (skipSameChar arr i n 96) n

        else if c == 92 && i + 1 - n < 0 then
            let
                nextCp =
                    at (i + 1) arr

                w =
                    charWidth nextCp
            in
            if isPunct nextCp then
                let
                    a1 =
                        Array.set i 43 arr
                            |> Array.set (i + 1) 43

                    a2 =
                        if w == 2 then
                            Array.set (i + 2) 43 a1

                        else
                            a1
                in
                maskAnyPunctOutsideCode a2 (i + 1 + w) n

            else
                maskAnyPunctOutsideCode arr (i + 1) n

        else
            maskAnyPunctOutsideCode arr (i + charWidth c) n


replaceTicksInSpan : Array Int -> Int -> Int -> Array Int
replaceTicksInSpan arr j end =
    if j - end >= 0 then
        arr

    else if at j arr == 96 then
        replaceTicksInSpan (Array.set j 43 arr) (j + 1) end

    else
        replaceTicksInSpan arr (j + 1) end


maskBlockSkip : Array Int -> Int -> Int -> Array Int
maskBlockSkip arr i n =
    if i - n >= 0 then
        arr

    else
        let
            b =
                at i arr
        in
        if b == 91 || b == 96 || b == 60 then
            case matchBlockSkipAt arr i n of
                Just len ->
                    let
                        a1 =
                            Array.set i 91 arr
                                |> fillRange 97 (i + 1) (i + len - 1)
                                |> Array.set (i + len - 1) 93
                    in
                    maskBlockSkip a1 (i + len) n

                Nothing ->
                    maskBlockSkip arr (i + 1) n

        else
            maskBlockSkip arr (i + 1) n


fillRange : Int -> Int -> Int -> Array Int -> Array Int
fillRange val i end arr =
    if i - end >= 0 then
        arr

    else
        fillRange val (i + 1) end (Array.set i val arr)



-- 9. EM / STRONG / DEL


type alias EmStrongLMatch =
    { delimChar : Int
    , lLen : Int
    , nextIsPunct : Bool
    , isUndNonPunct : Bool
    }


matchEmStrongL : Array Int -> Int -> Int -> Mode -> Maybe EmStrongLMatch
matchEmStrongL codes pos end mode =
    if pos - end >= 0 then
        Nothing

    else
        let
            d =
                at pos codes
        in
        if d /= 42 && d /= 95 then
            Nothing

        else
            let
                afterRun =
                    skipSameChar codes pos end d

                lLen =
                    afterRun - pos
            in
            if afterRun - end >= 0 then
                Nothing

            else
                let
                    nextCh =
                        at afterRun codes

                    nextIsPunct =
                        if isPedantic mode then
                            not (isOpenQuote nextCh) && isPunct nextCh

                        else if isGfm mode then
                            nextCh /= 126 && isPunct nextCh

                        else
                            isPunct nextCh
                in
                if nextIsPunct then
                    Just
                        { delimChar = d
                        , lLen = lLen
                        , nextIsPunct = True
                        , isUndNonPunct = False
                        }

                else if not (isJsSpace nextCh) then
                    Just
                        { delimChar = d
                        , lLen = lLen
                        , nextIsPunct = False
                        , isUndNonPunct = d == 95
                        }

                else
                    Nothing


type RDelimKind
    = RightOnly
    | LeftOnly
    | Both


type alias RDelimHit =
    { delimStart : Int
    , delimLen : Int
    , kind : RDelimKind
    , nextPos : Int
    }


skipOrphanInsideStrong : Array Int -> Int -> Int -> Int -> Int
skipOrphanInsideStrong codes base end d =
    let
        other =
            if d == 42 then
                95

            else
                42

        i1 =
            skipNotAstUnd codes base end
    in
    if i1 + 1 - end >= 0 || at i1 codes - other /= 0 || at (i1 + 1) codes - other /= 0 then
        0

    else
        let
            i2 =
                skipNotAstUnd codes (i1 + 2) end
        in
        if i2 - end >= 0 || at i2 codes - d /= 0 then
            0

        else
            let
                i3 =
                    skipNotAstUnd codes (i2 + 1) end
            in
            if i3 + 1 - end < 0 && at i3 codes - other == 0 && at (i3 + 1) codes - other == 0 then
                i3 - base

            else
                0


skipNotAstUnd : Array Int -> Int -> Int -> Int
skipNotAstUnd codes i end =
    if i - end < 0 then
        let
            b =
                at i codes
        in
        if b == 95 || b == 42 then
            i

        else
            skipNotAstUnd codes (i + 1) end

    else
        i


type alias RDelimCfg =
    { codes : Array Int
    , base : Int
    , end : Int
    , delim : Int
    , gfmAst : Bool
    , pedantic : Bool
    , isUnd : Bool
    , isDel : Bool
    }


nextRDelim : Array Int -> Int -> Int -> Int -> Int -> Mode -> Bool -> Maybe RDelimHit
nextRDelim codes base end scanPos delim mode isDel =
    let
        startPos =
            if scanPos == 0 && not isDel then
                let
                    skip =
                        skipOrphanInsideStrong codes base end delim
                in
                if skip > 0 then
                    skip

                else
                    0

            else
                scanPos

        cfg =
            { codes = codes
            , base = base
            , end = end
            , delim = delim
            , gfmAst = not isDel && delim == 42 && isGfm mode
            , pedantic = not isDel && isPedantic mode
            , isUnd = not isDel && delim == 95
            , isDel = isDel
            }
    in
    nextRDelimLoop cfg (base + startPos)


nextRDelimLoop : RDelimCfg -> Int -> Maybe RDelimHit
nextRDelimLoop cfg pos0 =
    if pos0 - cfg.end >= 0 then
        Nothing

    else
        let
            pos =
                if at pos0 cfg.codes - cfg.delim /= 0 then
                    let
                        nextD =
                            scanUntilChar cfg.codes pos0 cfg.end cfg.delim
                    in
                    if nextD - cfg.end >= 0 then
                        cfg.end

                    else
                        let
                            prevUnit =
                                at (nextD - 1) cfg.codes
                        in
                        if nextD - 1 - pos0 > 0 && prevUnit >= 0xDC00 && prevUnit <= 0xDFFF then
                            nextD - 2

                        else
                            nextD - 1

                else
                    pos0
        in
        if pos - cfg.end >= 0 then
            Nothing

        else
            let
                prevCh =
                    at pos cfg.codes

                prevEnd =
                    pos + charWidth prevCh
            in
            if prevEnd - cfg.end >= 0 || at prevEnd cfg.codes - cfg.delim /= 0 then
                nextRDelimLoop cfg prevEnd

            else
                let
                    runEnd =
                        skipSameChar cfg.codes prevEnd cfg.end cfg.delim

                    runLen =
                        runEnd - prevEnd
                in
                if cfg.isDel && runLen > 2 then
                    nextRDelimLoop cfg runEnd

                else if prevCh - cfg.delim == 0 then
                    nextRDelimLoop cfg prevEnd

                else
                    let
                        hasNext =
                            runEnd - cfg.end < 0

                        nextCh =
                            if hasNext then
                                at runEnd cfg.codes

                            else
                                -1
                    in
                    case classifyRDelim cfg prevCh hasNext nextCh of
                        Just kind ->
                            Just
                                { delimStart = prevEnd - cfg.base
                                , delimLen = runLen
                                , kind = kind
                                , nextPos = runEnd - cfg.base
                                }

                        Nothing ->
                            nextRDelimLoop cfg prevEnd


classifyRDelim : RDelimCfg -> Int -> Bool -> Int -> Maybe RDelimKind
classifyRDelim cfg prevCh hasNext nextCh =
    let
        isP ch =
            if cfg.gfmAst then
                ch /= 126 && isPunct ch

            else
                isPunct ch

        isPs ch =
            if cfg.gfmAst then
                ch /= 126 && isPunctOrSpace ch

            else
                isPunctOrSpace ch

        isNps ch =
            if cfg.gfmAst then
                ch == 126 || not (isPunctOrSpace ch)

            else
                not (isPunctOrSpace ch)

        prevP =
            isP prevCh

        prevNps =
            isNps prevCh

        prevSpace =
            isJsSpace prevCh
    in
    if prevP && (not hasNext || isJsSpace nextCh) then
        Just RightOnly

    else if prevNps && (not hasNext || isPs nextCh) then
        Just RightOnly

    else if
        hasNext
            && isNps nextCh
            && (if cfg.pedantic then
                    prevSpace

                else
                    isPs prevCh
               )
    then
        Just LeftOnly

    else if hasNext && prevSpace && isP nextCh then
        Just LeftOnly

    else if hasNext && prevP && isP nextCh then
        Just Both

    else if
        hasNext
            && isNps nextCh
            && (if cfg.pedantic then
                    prevP || prevNps

                else if cfg.isUnd then
                    False

                else
                    prevNps
               )
    then
        Just Both

    else
        Nothing


matchDelL : Array Int -> Int -> Int -> Maybe ( Int, Bool )
matchDelL codes pos end =
    if pos - end >= 0 || at pos codes /= 126 then
        Nothing

    else
        let
            lLen =
                if pos + 1 - end < 0 && at (pos + 1) codes == 126 then
                    2

                else
                    1
        in
        if pos + lLen - end >= 0 then
            Nothing

        else
            let
                nextCh =
                    at (pos + lLen) codes
            in
            if nextCh == 126 then
                Nothing

            else if isPunct nextCh then
                Just ( lLen, True )

            else if not (isJsSpace nextCh) then
                Just ( lLen, False )

            else
                Nothing



-- 10. BR


matchBr : Array Int -> Int -> Int -> Bool -> Maybe Int
matchBr codes pos end breaks =
    let
        i =
            if pos - end < 0 && at pos codes == 92 then
                pos + 1

            else
                let
                    spEnd =
                        skipSameChar codes pos end 32
                in
                if not breaks && (spEnd - pos) < 2 then
                    -1

                else
                    spEnd
    in
    if i < 0 || i - end >= 0 || at i codes /= 10 then
        Nothing

    else
        let
            afterNl =
                i + 1
        in
        if isAllJsSpace codes afterNl end then
            Nothing

        else
            Just (skipSpacesTabs codes afterNl end)



-- 11. GFM URL & EXTENDED EMAIL


isExtEmailLocalChar : Int -> Bool
isExtEmailLocalChar b =
    isAsciiAlnum b || b == 46 || b == 95 || b == 43 || b == 45


isExtEmailDomainChar : Int -> Bool
isExtEmailDomainChar b =
    isAsciiAlnum b || b == 45 || b == 95


matchExtendedEmailAt : Array Int -> Int -> Int -> Maybe Int
matchExtendedEmailAt codes pos end =
    let
        i1 =
            skipExtEmailLocal codes pos end
    in
    if i1 - pos == 0 || i1 - end >= 0 || at i1 codes /= 64 then
        Nothing

    else
        let
            d0 =
                i1 + 1

            i2 =
                skipExtEmailDomain codes d0 end
        in
        if i2 - d0 == 0 then
            Nothing

        else
            let
                validEnds =
                    scanExtEmailLabels codes i2 end []
            in
            popValidEmailEnd codes end validEnds


skipExtEmailLocal : Array Int -> Int -> Int -> Int
skipExtEmailLocal codes i end =
    if i - end < 0 && isExtEmailLocalChar (at i codes) then
        skipExtEmailLocal codes (i + 1) end

    else
        i


skipExtEmailDomain : Array Int -> Int -> Int -> Int
skipExtEmailDomain codes i end =
    if i - end < 0 && isExtEmailDomainChar (at i codes) then
        skipExtEmailDomain codes (i + 1) end

    else
        i


scanExtEmailLabels : Array Int -> Int -> Int -> List Int -> List Int
scanExtEmailLabels codes cur end validEnds =
    if cur - end < 0 && at cur codes == 46 then
        let
            ( nextCur, lastAlnum ) =
                scanExtEmailOneLabel codes (cur + 1) end -1
        in
        if lastAlnum >= 0 then
            if lastAlnum - nextCur == 0 then
                scanExtEmailLabels codes nextCur end (nextCur :: validEnds)

            else
                lastAlnum :: validEnds

        else
            validEnds

    else
        validEnds


scanExtEmailOneLabel : Array Int -> Int -> Int -> Int -> ( Int, Int )
scanExtEmailOneLabel codes cur end lastAlnum =
    if cur - end < 0 then
        let
            b =
                at cur codes
        in
        if isAsciiAlnum b then
            scanExtEmailOneLabel codes (cur + 1) end (cur + 1)

        else if b == 45 || b == 95 then
            scanExtEmailOneLabel codes (cur + 1) end lastAlnum

        else
            ( cur, lastAlnum )

    else
        ( cur, lastAlnum )


popValidEmailEnd : Array Int -> Int -> List Int -> Maybe Int
popValidEmailEnd codes end ends =
    case ends of
        [] ->
            Nothing

        e :: rest ->
            let
                blocked =
                    e - end < 0 && isExtEmailDomainChar (at e codes)
            in
            if not blocked then
                Just e

            else
                popValidEmailEnd codes end rest


matchProtocolPrefix : Array Int -> Int -> Int -> Int
matchProtocolPrefix codes pos end =
    let
        rem =
            end - pos
    in
    if
        rem
            >= 8
            && toAsciiLower (at pos codes)
            == 104
            && toAsciiLower (at (pos + 1) codes)
            == 116
            && toAsciiLower (at (pos + 2) codes)
            == 116
            && toAsciiLower (at (pos + 3) codes)
            == 112
            && toAsciiLower (at (pos + 4) codes)
            == 115
            && at (pos + 5) codes
            == 58
            && at (pos + 6) codes
            == 47
            && at (pos + 7) codes
            == 47
    then
        8

    else if
        rem
            >= 7
            && toAsciiLower (at pos codes)
            == 104
            && toAsciiLower (at (pos + 1) codes)
            == 116
            && toAsciiLower (at (pos + 2) codes)
            == 116
            && toAsciiLower (at (pos + 3) codes)
            == 112
            && at (pos + 4) codes
            == 58
            && at (pos + 5) codes
            == 47
            && at (pos + 6) codes
            == 47
    then
        7

    else if
        rem
            >= 6
            && toAsciiLower (at pos codes)
            == 102
            && toAsciiLower (at (pos + 1) codes)
            == 116
            && toAsciiLower (at (pos + 2) codes)
            == 112
            && at (pos + 3) codes
            == 58
            && at (pos + 4) codes
            == 47
            && at (pos + 5) codes
            == 47
    then
        6

    else
        0


isBackpedalPunct : Int -> Bool
isBackpedalPunct b =
    b
        == 63
        || b
        == 33
        || b
        == 46
        || b
        == 44
        || b
        == 58
        || b
        == 59
        || b
        == 42
        || b
        == 95
        || b
        == 39
        || b
        == 34
        || b
        == 126
        || b
        == 41


backpedalOnceEnd : Array Int -> Int -> Int -> Int
backpedalOnceEnd codes i end =
    if i - end >= 0 then
        i

    else
        let
            b =
                at i codes
        in
        if b == 40 then
            let
                j =
                    scanUntilChar codes (i + 1) end 41
            in
            if j - end < 0 && at j codes == 41 then
                backpedalOnceEnd codes (j + 1) end

            else
                i

        else if b == 38 then
            let
                j =
                    skipAsciiAlnum codes (i + 1) end
            in
            if j - (i + 1) > 0 && j + 1 - end == 0 && at j codes == 59 then
                i

            else
                backpedalOnceEnd codes (i + 1) end

        else if isBackpedalPunct b then
            let
                j =
                    skipBackpedalPunct codes i end
            in
            if j - end == 0 then
                if j - i >= 2 then
                    j - 1

                else
                    i

            else
                backpedalOnceEnd codes j end

        else
            backpedalOnceEnd codes (i + 1) end


skipAsciiAlnum : Array Int -> Int -> Int -> Int
skipAsciiAlnum codes i end =
    if i - end < 0 && isAsciiAlnum (at i codes) then
        skipAsciiAlnum codes (i + 1) end

    else
        i


skipBackpedalPunct : Array Int -> Int -> Int -> Int
skipBackpedalPunct codes i end =
    if i - end < 0 && isBackpedalPunct (at i codes) then
        skipBackpedalPunct codes (i + 1) end

    else
        i


backpedalEnd : Array Int -> Int -> Int -> Int
backpedalEnd codes pos end =
    let
        nextEnd =
            backpedalOnceEnd codes pos end
    in
    if nextEnd - end == 0 then
        end

    else
        backpedalEnd codes pos nextEnd


type alias GfmUrlSpan =
    { rawEnd : Int
    , isBareEmail : Bool
    , isWww : Bool
    }


startsWithMailto : Array Int -> Int -> Int -> Bool
startsWithMailto codes pos end =
    pos + 7 - end
        <= 0
        && at pos codes
        == 109
        && at (pos + 1) codes
        == 97
        && at (pos + 2) codes
        == 105
        && at (pos + 3) codes
        == 108
        && at (pos + 4) codes
        == 116
        && at (pos + 5) codes
        == 111
        && at (pos + 6) codes
        == 58


startsWithXmpp : Array Int -> Int -> Int -> Bool
startsWithXmpp codes pos end =
    pos + 5 - end
        <= 0
        && at pos codes
        == 120
        && at (pos + 1) codes
        == 109
        && at (pos + 2) codes
        == 112
        && at (pos + 3) codes
        == 112
        && at (pos + 4) codes
        == 58


startsWithWww : Array Int -> Int -> Int -> Bool
startsWithWww codes pos end =
    pos + 4 - end
        <= 0
        && at pos codes
        == 119
        && at (pos + 1) codes
        == 119
        && at (pos + 2) codes
        == 119
        && at (pos + 3) codes
        == 46


isXmppResChar : Int -> Bool
isXmppResChar b =
    isAsciiAlnum b || b == 64 || b == 46


skipXmppRes : Array Int -> Int -> Int -> Int
skipXmppRes codes i end =
    if i - end < 0 && isXmppResChar (at i codes) then
        skipXmppRes codes (i + 1) end

    else
        i


scanUrlBody : Array Int -> Int -> Int -> Int
scanUrlBody codes i end =
    if i - end >= 0 then
        i

    else
        let
            c =
                at i codes
        in
        if c == 60 || isJsSpace c then
            i

        else
            scanUrlBody codes (i + charWidth c) end


matchGfmUrl : Array Int -> Int -> Int -> Maybe GfmUrlSpan
matchGfmUrl codes pos end =
    let
        emailProtoAttempt =
            if startsWithMailto codes pos end then
                case matchExtendedEmailAt codes (pos + 7) end of
                    Just eEnd ->
                        Just
                            { rawEnd = backpedalEnd codes pos eEnd
                            , isBareEmail = False
                            , isWww = False
                            }

                    Nothing ->
                        Nothing

            else if startsWithXmpp codes pos end then
                case matchExtendedEmailAt codes (pos + 5) end of
                    Just eEnd0 ->
                        let
                            eEnd =
                                if eEnd0 + 1 - end < 0 && at eEnd0 codes == 47 && isXmppResChar (at (eEnd0 + 1) codes) then
                                    skipXmppRes codes (eEnd0 + 1) end

                                else
                                    eEnd0
                        in
                        Just
                            { rawEnd = backpedalEnd codes pos eEnd
                            , isBareEmail = False
                            , isWww = False
                            }

                    Nothing ->
                        Nothing

            else
                Nothing
    in
    case emailProtoAttempt of
        Just span ->
            Just span

        Nothing ->
            let
                pl =
                    matchProtocolPrefix codes pos end

                ( prefixLen, isWww ) =
                    if pl > 0 then
                        ( pl, False )

                    else if startsWithWww codes pos end then
                        ( 4, True )

                    else
                        ( 0, False )

                urlAttempt =
                    if prefixLen > 0 && pos + prefixLen - end < 0 then
                        let
                            b0 =
                                at (pos + prefixLen) codes
                        in
                        if isAsciiAlnum b0 || b0 == 45 then
                            let
                                uEnd =
                                    scanUrlBody codes (pos + prefixLen + 1) end
                            in
                            Just
                                { rawEnd = backpedalEnd codes pos uEnd
                                , isBareEmail = False
                                , isWww = isWww
                                }

                        else
                            Nothing

                    else
                        Nothing
            in
            case urlAttempt of
                Just span ->
                    Just span

                Nothing ->
                    case matchExtendedEmailAt codes pos end of
                        Just eEnd ->
                            Just
                                { rawEnd = eEnd
                                , isBareEmail = True
                                , isWww = False
                                }

                        Nothing ->
                            Nothing



-- 12. INLINE TEXT


isBrSpacesNl : Array Int -> Int -> Int -> Bool -> Bool
isBrSpacesNl codes pos end breaks =
    let
        i =
            skipSameChar codes pos end 32
    in
    i - end < 0 && at i codes == 10 && (breaks || (i - pos) >= 2)


isGfmTextEmailChar : Int -> Bool
isGfmTextEmailChar b =
    isAsciiAlnum b
        || b
        == 46
        || b
        == 33
        || b
        == 35
        || b
        == 36
        || b
        == 37
        || b
        == 38
        || b
        == 39
        || b
        == 42
        || b
        == 43
        || b
        == 47
        || b
        == 61
        || b
        == 63
        || b
        == 95
        || b
        == 96
        || b
        == 123
        || b
        == 124
        || b
        == 125
        || b
        == 126
        || b
        == 45


startsGfmEmailLocal : Array Int -> Int -> Int -> Bool
startsGfmEmailLocal codes pos end =
    let
        i =
            skipGfmTextEmailChars codes pos end
    in
    i - pos > 0 && i - end < 0 && at i codes == 64


skipGfmTextEmailChars : Array Int -> Int -> Int -> Int
skipGfmTextEmailChars codes i end =
    if i - end < 0 && isGfmTextEmailChar (at i codes) then
        skipGfmTextEmailChars codes (i + 1) end

    else
        i


isNormalInlineTrigger : Int -> Bool
isNormalInlineTrigger b =
    b == 92 || b == 60 || b == 33 || b == 91 || b == 96 || b == 42 || b == 95


isGfmInlineTrigger : Int -> Bool
isGfmInlineTrigger b =
    b == 92 || b == 60 || b == 33 || b == 91 || b == 96 || b == 42 || b == 126 || b == 95


matchInlineText : Array Int -> Int -> Int -> Mode -> Maybe Int
matchInlineText codes start end mode =
    if start - end >= 0 then
        Nothing

    else if not (isGfm mode) then
        let
            c0 =
                at start codes

            pos0 =
                if c0 == 96 then
                    skipSameChar codes start end 96

                else
                    start + charWidth c0
        in
        if isBrSpacesNl codes pos0 end False then
            Just pos0

        else
            Just (scanNormalInlineText codes pos0 end)

    else
        let
            breaks =
                isBreaks mode

            c0 =
                at start codes

            c0End =
                start + charWidth c0
        in
        if not (isAsciiAlnum c0) && (startsWithMailto codes c0End end || startsWithXmpp codes c0End end) then
            Just c0End

        else
            let
                pos0 =
                    if c0 == 96 then
                        skipSameChar codes start end 96

                    else if c0 == 126 then
                        skipSameChar codes start end 126

                    else
                        c0End
            in
            if
                pos0 - end
                    < 0
                    && (at pos0 codes
                            == 96
                            || at pos0 codes
                            == 126
                            || isBrSpacesNl codes pos0 end breaks
                            || startsGfmEmailLocal codes pos0 end
                       )
            then
                Just pos0

            else
                Just (scanGfmInlineText codes pos0 end breaks)


scanNormalInlineText : Array Int -> Int -> Int -> Int
scanNormalInlineText codes pos end =
    if pos - end >= 0 then
        end

    else
        let
            b =
                at pos codes
        in
        if isNormalInlineTrigger b then
            pos

        else
            let
                next =
                    pos + charWidth b
            in
            if b /= 32 && isBrSpacesNl codes next end False then
                next

            else
                scanNormalInlineText codes next end


scanGfmInlineText : Array Int -> Int -> Int -> Bool -> Int
scanGfmInlineText codes pos end breaks =
    if pos - end >= 0 then
        end

    else
        let
            b =
                at pos codes
        in
        if isGfmInlineTrigger b then
            pos

        else if breaks && isBrSpacesNl codes pos end True then
            pos

        else if (b == 104 || b == 72 || b == 102 || b == 70) && matchProtocolPrefix codes pos end > 0 then
            pos

        else if b == 119 && startsWithWww codes pos end then
            pos

        else
            let
                next =
                    pos + charWidth b
            in
            if b /= 32 && isBrSpacesNl codes next end breaks then
                next

            else if not (isAsciiAlnum b) && (startsWithMailto codes next end || startsWithXmpp codes next end) then
                next

            else if not (isGfmTextEmailChar b) && startsGfmEmailLocal codes next end then
                next

            else
                scanGfmInlineText codes next end breaks
