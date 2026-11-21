module Renderer exposing (renderBlocks, renderInline)

{-| Parser, Renderer, and TextRenderer mirroring `src/Parser.ts`, `src/Renderer.ts`,
and `src/TextRenderer.ts` of marked 18.1.1.
-}

import Array exposing (Array)
import Helpers exposing (cleanUrl, pushEscapedHtml)
import Tokens
    exposing
        ( Align(..)
        , BlockToken(..)
        , InlineSlot
        , InlineToken(..)
        , ListItem
        , TableCell
        )
import Unicode exposing (at, charWidth, fromString, isJsSpace)


{-| Renders block tokens into an HTML string (`Parser.parse` + `Renderer`).
-}
renderBlocks : List BlockToken -> Array InlineSlot -> String
renderBlocks tokens slots =
    renderBlocksAcc tokens slots []
        |> List.reverse
        |> String.concat


{-| Renders inline tokens into an HTML string (`Parser.parseInline` + `Renderer`).
-}
renderInline : List InlineToken -> String
renderInline tokens =
    renderInlineAcc tokens []
        |> List.reverse
        |> String.concat


slotTokens : Int -> Array InlineSlot -> List InlineToken
slotTokens id slots =
    case Array.get id slots of
        Just slot ->
            slot.tokens

        Nothing ->
            []


renderBlocksAcc : List BlockToken -> Array InlineSlot -> List String -> List String
renderBlocksAcc tokens slots acc =
    case tokens of
        [] ->
            acc

        tok :: rest ->
            let
                acc1 =
                    case tok of
                        Space _ ->
                            acc

                        Def _ ->
                            acc

                        Hr _ ->
                            "<hr>\n" :: acc

                        Heading h ->
                            let
                                d =
                                    String.fromInt h.depth

                                a1 =
                                    ("<h" ++ d ++ ">") :: acc

                                a2 =
                                    renderInlineAcc (slotTokens h.inlineId slots) a1
                            in
                            ("</h" ++ d ++ ">\n") :: a2

                        Code c ->
                            let
                                langStr =
                                    case c.lang of
                                        Just l ->
                                            firstNonSpaceRun l

                                        Nothing ->
                                            ""

                                a1 =
                                    if String.isEmpty langStr then
                                        "<pre><code>" :: acc

                                    else
                                        "\">" :: pushEscapedHtml False langStr ("<pre><code class=\"language-" :: acc)

                                a2 =
                                    if String.isEmpty c.text then
                                        a1

                                    else
                                        let
                                            trimmed =
                                                if String.endsWith "\n" c.text then
                                                    String.dropRight 1 c.text

                                                else
                                                    c.text
                                        in
                                        "\n" :: pushEscapedHtml True trimmed a1
                            in
                            "</code></pre>\n" :: a2

                        Blockquote bq ->
                            let
                                a1 =
                                    "<blockquote>\n" :: acc

                                a2 =
                                    renderBlocksAcc bq.tokens slots a1
                            in
                            "</blockquote>\n" :: a2

                        Html h ->
                            h.text :: acc

                        ListBlock lb ->
                            let
                                a1 =
                                    if lb.ordered then
                                        if lb.start /= 1 then
                                            ("<ol start=\"" ++ String.fromInt lb.start ++ "\">\n") :: acc

                                        else
                                            "<ol>\n" :: acc

                                    else
                                        "<ul>\n" :: acc

                                a2 =
                                    renderListItemsAcc lb.items slots a1
                            in
                            if lb.ordered then
                                "</ol>\n" :: a2

                            else
                                "</ul>\n" :: a2

                        BlockCheckbox cb ->
                            renderCheckboxAcc cb.checked acc

                        Paragraph p ->
                            let
                                a1 =
                                    "<p>" :: acc

                                a2 =
                                    renderInlineAcc (slotTokens p.inlineId slots) a1
                            in
                            "</p>\n" :: a2

                        BlockText t ->
                            renderInlineAcc (slotTokens t.inlineId slots) acc

                        Table tbl ->
                            let
                                a1 =
                                    "<table>\n<thead>\n<tr>\n" :: acc

                                a2 =
                                    renderTableCellsAcc tbl.header slots a1

                                a3 =
                                    "</tr>\n</thead>\n" :: a2

                                a4 =
                                    if List.isEmpty tbl.rows then
                                        a3

                                    else
                                        "</tbody>" :: renderTableRowsAcc tbl.rows slots ("<tbody>" :: a3)
                            in
                            "</table>\n" :: a4
            in
            renderBlocksAcc rest slots acc1


renderListItemsAcc : List ListItem -> Array InlineSlot -> List String -> List String
renderListItemsAcc items slots acc =
    case items of
        [] ->
            acc

        item :: rest ->
            let
                a1 =
                    "<li>" :: acc

                a2 =
                    renderBlocksAcc item.tokens slots a1
            in
            renderListItemsAcc rest slots ("</li>\n" :: a2)


renderTableRowsAcc : List (List TableCell) -> Array InlineSlot -> List String -> List String
renderTableRowsAcc rows slots acc =
    case rows of
        [] ->
            acc

        row :: rest ->
            let
                a1 =
                    "<tr>\n" :: acc

                a2 =
                    renderTableCellsAcc row slots a1
            in
            renderTableRowsAcc rest slots ("</tr>\n" :: a2)


renderTableCellsAcc : List TableCell -> Array InlineSlot -> List String -> List String
renderTableCellsAcc cells slots acc =
    case cells of
        [] ->
            acc

        cell :: rest ->
            let
                openTag =
                    if cell.header then
                        case cell.align of
                            AlignNone ->
                                "<th>"

                            AlignLeft ->
                                "<th align=\"left\">"

                            AlignCenter ->
                                "<th align=\"center\">"

                            AlignRight ->
                                "<th align=\"right\">"

                    else
                        case cell.align of
                            AlignNone ->
                                "<td>"

                            AlignLeft ->
                                "<td align=\"left\">"

                            AlignCenter ->
                                "<td align=\"center\">"

                            AlignRight ->
                                "<td align=\"right\">"

                closeTag =
                    if cell.header then
                        "</th>\n"

                    else
                        "</td>\n"

                a1 =
                    renderInlineAcc (slotTokens cell.inlineId slots) (openTag :: acc)
            in
            renderTableCellsAcc rest slots (closeTag :: a1)


renderCheckboxAcc : Bool -> List String -> List String
renderCheckboxAcc checked acc =
    if checked then
        "<input checked=\"\" disabled=\"\" type=\"checkbox\"> " :: acc

    else
        "<input disabled=\"\" type=\"checkbox\"> " :: acc


renderInlineAcc : List InlineToken -> List String -> List String
renderInlineAcc tokens acc =
    case tokens of
        [] ->
            acc

        tok :: rest ->
            let
                acc1 =
                    case tok of
                        Escape e ->
                            pushEscapedHtml False e.text acc

                        InlineHtml h ->
                            h.text :: acc

                        InlineCheckbox cb ->
                            renderCheckboxAcc cb.checked acc

                        Strong s ->
                            "</strong>" :: renderInlineAcc s.tokens ("<strong>" :: acc)

                        Em e ->
                            "</em>" :: renderInlineAcc e.tokens ("<em>" :: acc)

                        Codespan c ->
                            "</code>" :: pushEscapedHtml True c.text ("<code>" :: acc)

                        Br ->
                            "<br>" :: acc

                        Del d ->
                            "</del>" :: renderInlineAcc d.tokens ("<del>" :: acc)

                        Link l ->
                            case Maybe.andThen cleanUrl l.href of
                                Nothing ->
                                    if l.autolink then
                                        pushEscapedHtml True l.text acc

                                    else
                                        renderInlineAcc l.tokens acc

                                Just clean ->
                                    let
                                        a1 =
                                            "\"" :: pushEscapedHtml l.autolink clean ("<a href=\"" :: acc)

                                        a2 =
                                            case l.title of
                                                Just t ->
                                                    if String.isEmpty t then
                                                        a1

                                                    else
                                                        "\"" :: pushEscapedHtml False t (" title=\"" :: a1)

                                                Nothing ->
                                                    a1

                                        a3 =
                                            ">" :: a2

                                        a4 =
                                            if l.autolink then
                                                pushEscapedHtml True l.text a3

                                            else
                                                renderInlineAcc l.tokens a3
                                    in
                                    "</a>" :: a4

                        Image img ->
                            let
                                alt =
                                    renderTextInline img.tokens
                            in
                            case Maybe.andThen cleanUrl img.href of
                                Nothing ->
                                    pushEscapedHtml False alt acc

                                Just clean ->
                                    let
                                        a1 =
                                            "\" alt=\"" :: pushEscapedHtml False clean ("<img src=\"" :: acc)

                                        a2 =
                                            "\"" :: pushEscapedHtml False alt a1

                                        a3 =
                                            case img.title of
                                                Just t ->
                                                    if String.isEmpty t then
                                                        a2

                                                    else
                                                        "\"" :: pushEscapedHtml False t (" title=\"" :: a2)

                                                Nothing ->
                                                    a2
                                    in
                                    ">" :: a3

                        InlineText t ->
                            if t.escaped then
                                t.text :: acc

                            else
                                pushEscapedHtml False t.text acc
            in
            renderInlineAcc rest acc1


{-| Exact port of `TextRenderer` (`src/TextRenderer.ts`) used by `Renderer.image` for `alt` text.
-}
renderTextInline : List InlineToken -> String
renderTextInline tokens =
    renderTextInlineAcc tokens []
        |> List.reverse
        |> String.concat


renderTextInlineAcc : List InlineToken -> List String -> List String
renderTextInlineAcc tokens acc =
    case tokens of
        [] ->
            acc

        tok :: rest ->
            let
                acc1 =
                    case tok of
                        Strong r ->
                            r.text :: acc

                        Em r ->
                            r.text :: acc

                        Codespan r ->
                            r.text :: acc

                        Del r ->
                            r.text :: acc

                        InlineHtml r ->
                            r.text :: acc

                        InlineText r ->
                            r.text :: acc

                        Escape r ->
                            r.text :: acc

                        Link r ->
                            r.text :: acc

                        Image r ->
                            r.text :: acc

                        Br ->
                            acc

                        InlineCheckbox r ->
                            r.raw :: acc
            in
            renderTextInlineAcc rest acc1


{-| Matches `other.notSpaceStart` (`^\S*`).
-}
firstNonSpaceRun : String -> String
firstNonSpaceRun s =
    let
        src =
            fromString s

        endIdx =
            scanNonSpace src.codes 0 src.len
    in
    String.slice 0 endIdx s


scanNonSpace : Array Int -> Int -> Int -> Int
scanNonSpace codes i len =
    if i - len >= 0 then
        len

    else
        let
            c =
                at i codes
        in
        if isJsSpace c then
            i

        else
            scanNonSpace codes (i + charWidth c) len
