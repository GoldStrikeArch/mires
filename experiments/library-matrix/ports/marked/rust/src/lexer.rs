//! Block and inline Lexer + Tokenizer mirroring `src/Lexer.ts` and `src/Tokenizer.ts` of marked 18.1.1.
//!
//! Uses a single-pass `(src: &str, bytes: &[u8], pos: usize)` cursor in both `block_tokens` and
//! `inline_tokens_inner` so `as_bytes()` is called once per buffer instead of re-materializing
//! suffix strings on every token (`PERF-003`).

use alloc::string::String;
use alloc::vec::Vec;
use mires_js::collections::HashMap;
use mires_js::{throw_value, JsValue};

use crate::block_rules::{
    find_list_task_checkbox, has_any_line, has_double_blank_line_end, indent_code_compensation,
    is_blockquote_at, is_blockquote_begin, is_fences_begin, is_heading_begin, is_html_interrupt,
    is_list_task, is_next_bullet, join_lines, line_end, match_block_code, match_blockquote,
    match_def, match_fences, match_heading, match_hr_at, match_html_normal, match_html_pedantic,
    match_lheading_at, match_list_item, match_list_start, match_newline, match_paragraph_at,
    match_table_at, parse_table_align, preprocess_pedantic_block, remove_code_indent,
    replace_pedantic_list_nesting, strip_blockquote_markers, strip_list_task,
    strip_one_blockquote_prefix, trim_end_raw, trim_table_align_chars, trim_table_row_blank_line,
    ListMarkerSpec,
};
use crate::helpers::{
    collapse_spaces, decode_numeric_character_references_into, expand_tabs, find_closing_paren,
    is_blank_line, normalize_label, normalize_newlines, rtrim_byte, split_cells,
    trim_trailing_blank_lines, unescape_any_punctuation, unescape_link_brackets,
};
use crate::inline_rules::{
    build_masked_bytes, is_end_a_tag, is_end_pre_script_tag, is_label_end_inside_token,
    is_start_a_tag, is_start_pre_script_tag, match_autolink, match_br, match_del_l,
    match_em_strong_l, match_escape, match_gfm_url, match_inline_code_at, match_inline_link,
    match_inline_text, match_pedantic_href_title, match_reflink_at, match_tag, next_rdelim,
    RDelimKind,
};
use crate::tokens::{
    Align, BlockToken, InlineSlot, InlineToken, LinkDef, ListItem, Mode, Options, TableCell,
};
use crate::unicode::{trim_js, trim_start_js, PrevChar};

struct LexerState {
    in_link: bool,
    in_raw_block: bool,
    link_emitted: bool,
    link_paren_possible: bool,
    top: bool,
}

pub struct Lexer {
    pub options: Options,
    pub mode: Mode,
    pub links: HashMap<String, LinkDef>,
    pub inline_slots: Vec<InlineSlot>,
    state: LexerState,
}

impl Lexer {
    pub fn new(options: Options) -> Self {
        Self {
            options,
            mode: options.mode(),
            links: HashMap::new(),
            inline_slots: Vec::new(),
            state: LexerState {
                in_link: false,
                in_raw_block: false,
                link_emitted: false,
                link_paren_possible: true,
                top: true,
            },
        }
    }

    pub fn lex(&mut self, src: &str) -> Vec<BlockToken> {
        let normalized = normalize_newlines(src);
        let mut tokens = Vec::new();
        self.block_tokens(&normalized, &mut tokens, false);

        let count = self.inline_slots.len();
        let mut i = 0;
        while i < count {
            let slot_src = core::mem::take(&mut self.inline_slots[i].src);
            let mut slot_tokens = core::mem::take(&mut self.inline_slots[i].tokens);
            self.inline_tokens(&slot_src, &mut slot_tokens);
            self.inline_slots[i].tokens = slot_tokens;
            i += 1;
        }

        tokens
    }

    pub fn lex_inline(&mut self, src: &str) -> Vec<InlineToken> {
        let mut tokens = Vec::new();
        self.inline_tokens(src, &mut tokens);
        tokens
    }

    #[inline]
    fn queue_inline(&mut self, src: String) -> usize {
        let id = self.inline_slots.len();
        self.inline_slots.push(InlineSlot {
            src,
            tokens: Vec::new(),
        });
        id
    }

    fn block_tokens(
        &mut self,
        input_src: &str,
        tokens: &mut Vec<BlockToken>,
        mut last_paragraph_clipped: bool,
    ) {
        let owned_pedantic;
        let src = if self.options.pedantic {
            owned_pedantic = preprocess_pedantic_block(input_src);
            owned_pedantic.as_str()
        } else {
            input_src
        };

        let bytes = src.as_bytes();
        let n = bytes.len();
        let mut pos = 0usize;
        let mut prev_pos = usize::MAX;

        while pos < n {
            if pos == prev_pos {
                throw_value(JsValue::erase("marked: infinite loop in blockTokens"));
            }
            prev_pos = pos;

            // 1. space
            if let Some(len) = match_newline(bytes, pos) {
                let raw = &src[pos..pos + len];
                pos += len;
                if len == 1 && !tokens.is_empty() {
                    if let Some(last) = tokens.last_mut() {
                        last.append_newline_to_raw();
                    }
                } else {
                    tokens.push(BlockToken::Space {
                        raw: String::from(raw),
                    });
                }
                continue;
            }

            // 2. code (indented)
            if let Some(tok) = self.tok_code(src, bytes, pos) {
                let consume = tok.raw_len().min(n - pos);
                pos += consume;
                let merged = if let Some(last) = tokens.last_mut() {
                    match last {
                        BlockToken::Paragraph { raw_len, text, .. }
                        | BlockToken::Text { raw_len, text, .. } => {
                            if let BlockToken::Code {
                                raw_len: c_len,
                                text: c_text,
                                ..
                            } = &tok
                            {
                                *raw_len += 1 + c_len;
                                text.push('\n');
                                text.push_str(c_text);
                                if let Some(slot) = self.inline_slots.last_mut() {
                                    slot.src = text.clone();
                                }
                            }
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                if !merged {
                    tokens.push(tok);
                }
                continue;
            }

            // 3. fences (disabled in pedantic)
            if !self.mode.is_pedantic() {
                if let Some(tok) = self.tok_fences(src, bytes, pos) {
                    let consume = tok.raw_len().min(n - pos);
                    pos += consume;
                    tokens.push(tok);
                    continue;
                }
            }

            // 4. heading (ATX)
            if let Some(tok) = self.tok_heading(src, bytes, pos) {
                let consume = tok.raw_len().min(n - pos);
                pos += consume;
                tokens.push(tok);
                continue;
            }

            // 5. hr
            if let Some(end) = match_hr_at(bytes, pos, 3) {
                let raw_len = end - pos;
                pos = end;
                tokens.push(BlockToken::Hr { raw_len });
                continue;
            }

            // 6. blockquote
            if let Some(tok) = self.tok_blockquote(src, bytes, pos) {
                let consume = tok.raw_len().min(n - pos);
                pos += consume;
                tokens.push(tok);
                continue;
            }

            // 7. list
            if let Some(tok) = self.tok_list(src, bytes, pos) {
                let consume = tok.raw_len().min(n - pos);
                pos += consume;
                tokens.push(tok);
                continue;
            }

            // 8. html
            if let Some(tok) = self.tok_html(src, bytes, pos) {
                let consume = tok.raw_len().min(n - pos);
                pos += consume;
                tokens.push(tok);
                continue;
            }

            // 9. def
            if let Some((def_raw, tag, def)) = self.tok_def(src, bytes, pos) {
                let consume = def_raw.len().min(n - pos);
                pos += consume;
                let merged = if let Some(last) = tokens.last_mut() {
                    match last {
                        BlockToken::Paragraph { raw_len, text, .. }
                        | BlockToken::Text { raw_len, text, .. } => {
                            *raw_len += 1 + def_raw.len();
                            text.push('\n');
                            text.push_str(def_raw);
                            if let Some(slot) = self.inline_slots.last_mut() {
                                slot.src = text.clone();
                            }
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                if !merged && !self.links.contains_key(tag.as_str()) {
                    self.links.insert(tag, def);
                    tokens.push(BlockToken::Def {
                        raw_len: def_raw.len(),
                    });
                }
                continue;
            }

            // 10. table (GFM only)
            if self.mode.is_gfm() {
                if let Some(tok) = self.tok_table(src, bytes, pos) {
                    let consume = tok.raw_len().min(n - pos);
                    pos += consume;
                    tokens.push(tok);
                    continue;
                }
            }

            // 11. lheading
            if let Some(tok) = self.tok_lheading(src, bytes, pos) {
                let consume = tok.raw_len().min(n - pos);
                pos += consume;
                tokens.push(tok);
                continue;
            }

            // 12. top-level paragraph
            if self.state.top {
                if let Some(tok) = self.tok_paragraph(src, bytes, pos) {
                    let consume = tok.raw_len().min(n - pos);
                    let merged = if last_paragraph_clipped {
                        if let Some(BlockToken::Paragraph { raw_len, text, .. }) = tokens.last_mut() {
                            if let BlockToken::Paragraph {
                                raw_len: p_len,
                                text: p_text,
                                ..
                            } = &tok
                            {
                                *raw_len += 1 + p_len;
                                text.push('\n');
                                text.push_str(p_text);
                                self.inline_slots.pop();
                                if let Some(slot) = self.inline_slots.last_mut() {
                                    slot.src = text.clone();
                                }
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !merged {
                        tokens.push(tok);
                    }
                    last_paragraph_clipped = false;
                    pos += consume;
                    continue;
                }
            }

            // 13. text (fallback when !state.top)
            let le = line_end(bytes, pos);
            if le > pos {
                let slice = &src[pos..le];
                pos = le;
                let merged = if let Some(BlockToken::Text { raw_len, text, .. }) = tokens.last_mut() {
                    *raw_len += 1 + slice.len();
                    text.push('\n');
                    text.push_str(slice);
                    if let Some(slot) = self.inline_slots.last_mut() {
                        slot.src = text.clone();
                    }
                    true
                } else {
                    false
                };
                if !merged {
                    let s = String::from(slice);
                    let inline_id = self.queue_inline(s.clone());
                    tokens.push(BlockToken::Text {
                        raw_len: slice.len(),
                        text: s,
                        inline_id,
                    });
                }
                continue;
            }

            throw_value(JsValue::erase("marked: infinite loop in blockTokens"));
        }

        self.state.top = true;
    }

    fn tok_code(&self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let len = match_block_code(bytes, pos)?;
        let cap0 = &src[pos..pos + len];
        let raw_slice = if self.options.pedantic {
            cap0
        } else {
            trim_trailing_blank_lines(cap0)
        };
        let text = remove_code_indent(raw_slice);
        Some(BlockToken::Code {
            raw_len: raw_slice.len(),
            lang: None,
            text,
        })
    }

    fn tok_fences(&self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let m = match_fences(bytes, pos)?;
        let raw_slice = &src[pos..m.raw_end];
        let body = &src[m.body_start..m.body_end];
        let info = &src[m.info_start..m.info_end];
        let text = indent_code_compensation(raw_slice, body);
        let lang = if !info.is_empty() {
            Some(unescape_any_punctuation(trim_js(info)))
        } else {
            None
        };
        Some(BlockToken::Code {
            raw_len: raw_slice.len(),
            lang,
            text,
        })
    }

    fn tok_heading(&mut self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let m = match_heading(bytes, pos, self.mode.is_pedantic())?;
        let mut text = trim_js(&src[m.text_start..m.text_end]);
        // WORKAROUND(MIRES-019): promoted constant `Some(&b'#')` / `Some(&b' ')` / `Some(&b'\t')`
        // silently decode to `{TAG:"None"}`; compare by copied byte.
        if text.as_bytes().last().copied() == Some(b'#') {
            let trimmed = rtrim_byte(text, b'#');
            let last = trimmed.as_bytes().last().copied();
            if self.options.pedantic
                || trimmed.is_empty()
                || last == Some(b' ')
                || last == Some(b'\t')
            {
                text = trim_js(trimmed);
            }
        }
        let inline_id = self.queue_inline(String::from(text));
        Some(BlockToken::Heading {
            raw_len: m.raw_end - pos,
            depth: m.depth,
            inline_id,
        })
    }

    fn tok_blockquote(&mut self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let cap_end = match_blockquote(bytes, pos)?;
        Some(self.parse_blockquote_lines(rtrim_byte(&src[pos..cap_end], b'\n')))
    }

    // WORKAROUND(MIRES-017): keeping the continuation loop in a non-Option helper with a `done`
    // flag avoids the multi-exit dispatcher `switch (d)` whose last `case 1:` omits `break;`
    // before `default: throw new Error("rustc_codegen_js: bad block " + d)`.
    fn parse_blockquote_lines(&mut self, trimmed: &str) -> BlockToken {
        let mut lines: Vec<String> = trimmed.split('\n').map(String::from).collect();
        let mut raw = String::new();
        let mut tokens: Vec<BlockToken> = Vec::new();
        let mut done = false;

        while !done && !lines.is_empty() {
            let mut in_blockquote = false;
            let mut split_at = 0usize;
            while split_at < lines.len() {
                if is_blockquote_at(lines[split_at].as_bytes(), 0) {
                    in_blockquote = true;
                    split_at += 1;
                } else if !in_blockquote {
                    split_at += 1;
                } else {
                    break;
                }
            }
            let remaining = lines.split_off(split_at);
            let current_raw = join_lines(&lines);
            let current_text = strip_blockquote_markers(&current_raw);

            if raw.is_empty() {
                raw.push_str(&current_raw);
            } else {
                raw.push('\n');
                raw.push_str(&current_raw);
            }

            let saved_top = self.state.top;
            self.state.top = true;
            self.block_tokens(&current_text, &mut tokens, true);
            self.state.top = saved_top;

            lines = remaining;
            if lines.is_empty() {
                done = true;
            } else {
                let last_kind = match tokens.last() {
                    Some(BlockToken::Code { .. })
                    | Some(BlockToken::Heading { .. })
                    | Some(BlockToken::Hr { .. })
                    | Some(BlockToken::Html { .. }) => 1,
                    Some(BlockToken::Blockquote { .. }) => 2,
                    Some(BlockToken::List { .. }) => 3,
                    _ => 0,
                };

                if last_kind == 1 {
                    done = true;
                } else if last_kind == 2 {
                    self.merge_nested_blockquote(&lines, &mut raw, &mut tokens);
                    done = true;
                } else if last_kind == 3 {
                    if let Some(next_lines) =
                        self.merge_nested_list(&lines, &mut raw, &mut tokens)
                    {
                        lines = next_lines;
                    } else {
                        done = true;
                    }
                }
            }
        }

        BlockToken::Blockquote { raw, tokens }
    }

    fn merge_nested_blockquote(
        &mut self,
        lines: &[String],
        raw: &mut String,
        tokens: &mut Vec<BlockToken>,
    ) {
        let old_raw = match tokens.last() {
            Some(BlockToken::Blockquote { raw: r, .. }) => r.clone(),
            _ => return,
        };
        let continuation = join_lines(lines);
        let mut new_text = String::with_capacity(old_raw.len() + 1 + continuation.len());
        new_text.push_str(&old_raw);
        new_text.push('\n');
        new_text.push_str(&strip_one_blockquote_prefix(&continuation));

        let new_bytes = new_text.as_bytes();
        if let Some(new_token) = self.tok_blockquote(&new_text, new_bytes, 0) {
            let mut new_raw_len = new_token.raw_len().min(new_text.len());
            while new_raw_len > 0 && !new_text.is_char_boundary(new_raw_len) {
                new_raw_len -= 1;
            }
            let leftover = &new_text[new_raw_len..];
            let leftover = leftover.strip_prefix('\n').unwrap_or(leftover);
            let leftover_count = if leftover.is_empty() {
                0
            } else {
                leftover.split('\n').count()
            };
            let consumed_count = lines.len().saturating_sub(leftover_count);
            if consumed_count > 0 {
                raw.push('\n');
                raw.push_str(&join_lines(&lines[..consumed_count]));
            }
            if let Some(slot) = tokens.last_mut() {
                *slot = new_token;
            }
        }
    }

    fn merge_nested_list(
        &mut self,
        lines: &[String],
        raw: &mut String,
        tokens: &mut Vec<BlockToken>,
    ) -> Option<Vec<String>> {
        let old_raw = match tokens.last() {
            Some(BlockToken::List { raw: r, .. }) => r.clone(),
            _ => return None,
        };
        let joined_lines = join_lines(lines);
        let mut new_text = String::with_capacity(old_raw.len() + 1 + joined_lines.len());
        new_text.push_str(&old_raw);
        new_text.push('\n');
        new_text.push_str(&joined_lines);

        let new_bytes = new_text.as_bytes();
        let new_token = self.tok_list(&new_text, new_bytes, 0)?;
        let new_raw = match &new_token {
            BlockToken::List { raw: r, .. } => r.clone(),
            _ => String::new(),
        };
        let r_keep = raw.len().saturating_sub(old_raw.len());
        if raw.is_char_boundary(r_keep) {
            raw.truncate(r_keep);
            raw.push_str(&new_raw);
        } else if new_raw.len() >= old_raw.len() && new_raw.is_char_boundary(old_raw.len()) {
            // Preserve the exact consumed UTF-8 byte count without splitting a
            // multi-byte scalar value when `> ` prefix stripping shifts `r_keep`.
            raw.push_str(&new_raw[old_raw.len()..]);
        } else {
            let mut k = r_keep.min(raw.len());
            while k > 0 && !raw.is_char_boundary(k) {
                k -= 1;
            }
            raw.truncate(k);
            raw.push_str(&new_raw);
        }
        let mut rem_start = new_raw.len().min(new_text.len());
        while rem_start > 0 && !new_text.is_char_boundary(rem_start) {
            rem_start -= 1;
        }
        let next_lines: Vec<String> = new_text[rem_start..].split('\n').map(String::from).collect();
        if let Some(slot) = tokens.last_mut() {
            *slot = new_token;
        }
        Some(next_lines)
    }

    fn tok_list(&mut self, full_src: &str, bytes: &[u8], start_pos: usize) -> Option<BlockToken> {
        let first_cap = match_list_start(bytes, start_pos)?;
        let bull = trim_js(&full_src[start_pos..first_cap.bullet_end]);
        let is_ordered = bull.len() > 1;
        let start = if is_ordered {
            parse_decimal_u64(&bull[..bull.len() - 1])
        } else {
            0
        };
        let marker_spec = if is_ordered {
            ListMarkerSpec::Ordered(bull.as_bytes()[bull.len() - 1])
        } else if self.options.pedantic {
            ListMarkerSpec::PedanticAnyUnordered
        } else {
            ListMarkerSpec::Unordered(bull.as_bytes()[0])
        };

        let n = bytes.len();
        let mut pos = start_pos;
        let mut loose = false;
        let mut ends_with_blank_line = false;
        let mut items: Vec<ListItem> = Vec::new();

        while pos < n {
            let Some(cap) = match_list_item(bytes, pos, marker_spec) else {
                break;
            };
            if match_hr_at(bytes, pos, 3).is_some() {
                break;
            }
            let item_start = pos;
            let bullet_indent = cap.bullet_end - pos;
            let first_line = &full_src[cap.rest_start..cap.rest_end];
            pos = cap.raw_end;

            let mut line = if self.options.pedantic {
                expand_tabs(first_line, bullet_indent)
            } else {
                expand_leading_space_tab(first_line, bullet_indent)
            };
            let next_line_end = line_end(bytes, pos);
            let next_line_peek = &full_src[pos..next_line_end];
            let mut blank_line = trim_js(&line).is_empty();

            let mut item_contents = String::new();
            let indent: usize;
            if self.options.pedantic {
                indent = 2;
                item_contents.push_str(trim_start_js(&line));
            } else if blank_line {
                indent = bullet_indent + 1;
            } else {
                let first_ns = first_non_space_index(&line).unwrap_or(0);
                let rel_indent = if first_ns > 4 { 1 } else { first_ns };
                item_contents.push_str(slice_utf16_from(&line, rel_indent));
                indent = rel_indent + bullet_indent;
            }

            let mut end_early = false;
            if blank_line && is_blank_line(next_line_peek) {
                pos = (next_line_end + 1).min(n);
                end_early = true;
            }

            if !end_early {
                let max_ind = indent.saturating_sub(1).min(3);
                while pos < n {
                    let raw_line_end = line_end(bytes, pos);
                    let raw_line = &full_src[pos..raw_line_end];
                    let pedantic_line;
                    let next_line_str: &str;
                    let next_line_bytes: &[u8];
                    let next_line_no_tabs: String;
                    if self.options.pedantic {
                        pedantic_line = replace_pedantic_list_nesting(raw_line);
                        next_line_str = pedantic_line.as_str();
                        next_line_bytes = pedantic_line.as_bytes();
                        next_line_no_tabs = pedantic_line.clone();
                    } else {
                        next_line_str = raw_line;
                        next_line_bytes = &bytes[pos..raw_line_end];
                        next_line_no_tabs = expand_leading_space_tab(raw_line, 0);
                    }

                    if is_fences_begin(next_line_bytes, 0, max_ind)
                        || is_heading_begin(next_line_bytes, 0, max_ind)
                        || is_html_interrupt(next_line_bytes, 0, max_ind, true)
                        || is_blockquote_begin(next_line_bytes, 0, max_ind)
                        || is_next_bullet(next_line_bytes, 0, max_ind)
                        || match_hr_at(next_line_bytes, 0, max_ind).is_some()
                    {
                        break;
                    }

                    let dedentable = first_non_space_index(&next_line_no_tabs)
                        .is_some_and(|idx| idx >= indent)
                        || trim_js(next_line_str).is_empty();
                    if dedentable {
                        item_contents.push('\n');
                        item_contents.push_str(slice_utf16_from(&next_line_no_tabs, indent));
                    } else {
                        if blank_line {
                            break;
                        }
                        let lb = line.as_bytes();
                        if is_line_indented_4_with_tabs(lb)
                            || is_fences_begin(lb, 0, max_ind)
                            || is_heading_begin(lb, 0, max_ind)
                            || match_hr_at(lb, 0, max_ind).is_some()
                        {
                            break;
                        }
                        item_contents.push('\n');
                        item_contents.push_str(next_line_str);
                    }

                    blank_line = trim_js(next_line_str).is_empty();
                    pos = (raw_line_end + 1).min(n);
                    line = String::from(slice_utf16_from(&next_line_no_tabs, indent));
                }
            }

            let item_raw = &full_src[item_start..pos];
            if !loose {
                if ends_with_blank_line {
                    loose = true;
                } else if has_double_blank_line_end(&bytes[item_start..pos]) {
                    ends_with_blank_line = true;
                }
            }

            let is_task = self.options.gfm && is_list_task(&item_contents);
            items.push(ListItem {
                raw: String::from(item_raw),
                task: is_task,
                checked: false,
                loose: false,
                text: item_contents,
                tokens: Vec::new(),
            });
        }

        if let Some(last_item) = items.last_mut() {
            let r_trim = trim_end_raw(&last_item.raw).len();
            last_item.raw.truncate(r_trim);
            let t_trim = trim_end_raw(&last_item.text).len();
            last_item.text.truncate(t_trim);
        } else {
            return None;
        }

        let list_raw = String::from(trim_end_raw(&full_src[start_pos..pos]));

        // Pass 1: tokenize items and finalize `loose` from spacers
        let mut idx = 0;
        while idx < items.len() {
            self.state.top = false;
            let item_text = items[idx].text.clone();
            let mut item_tokens = Vec::new();
            self.block_tokens(&item_text, &mut item_tokens, false);
            if !loose {
                for tok in &item_tokens {
                    if let BlockToken::Space { raw } = tok {
                        if has_any_line(raw) {
                            loose = true;
                            break;
                        }
                    }
                }
            }
            items[idx].tokens = item_tokens;
            idx += 1;
        }

        // Pass 2: place task checkboxes using final `loose`
        let mut idx = 0;
        while idx < items.len() {
            if items[idx].task {
                let first_is_text_or_para = matches!(
                    items[idx].tokens.first(),
                    Some(BlockToken::Text { .. }) | Some(BlockToken::Paragraph { .. })
                );
                if first_is_text_or_para {
                    items[idx].text = String::from(strip_list_task(&items[idx].text));
                    let first_slot_id = match items[idx].tokens.first_mut() {
                        Some(BlockToken::Text {
                            text, inline_id, ..
                        })
                        | Some(BlockToken::Paragraph {
                            text, inline_id, ..
                        }) => {
                            *text = String::from(strip_list_task(text));
                            Some(*inline_id)
                        }
                        _ => None,
                    };
                    let mut q = self.inline_slots.len();
                    while q > 0 {
                        q -= 1;
                        if is_list_task(&self.inline_slots[q].src) {
                            let stripped = String::from(strip_list_task(&self.inline_slots[q].src));
                            self.inline_slots[q].src = stripped;
                            break;
                        }
                    }
                    if let Some(cb_slice) = find_list_task_checkbox(&items[idx].raw) {
                        let checked = cb_slice != "[ ]";
                        let mut cb_raw = String::with_capacity(4);
                        cb_raw.push_str(cb_slice);
                        cb_raw.push(' ');
                        items[idx].checked = checked;
                        if loose {
                            if let Some(slot_id) = first_slot_id {
                                if let Some(
                                    BlockToken::Paragraph { text, .. }
                                    | BlockToken::Text { text, .. },
                                ) = items[idx].tokens.first_mut()
                                {
                                    let mut new_text = String::with_capacity(cb_raw.len() + text.len());
                                    new_text.push_str(&cb_raw);
                                    new_text.push_str(text);
                                    *text = new_text;
                                }
                                self.inline_slots[slot_id].tokens.insert(
                                    0,
                                    InlineToken::Checkbox {
                                        raw: cb_raw,
                                        checked,
                                    },
                                );
                            }
                        } else {
                            items[idx]
                                .tokens
                                .insert(0, BlockToken::Checkbox { checked });
                        }
                    }
                } else {
                    items[idx].task = false;
                }
            }
            idx += 1;
        }

        // Pass 3: promote `Text` to `Paragraph` when `loose`
        if loose {
            for item in &mut items {
                item.loose = true;
                for tok in &mut item.tokens {
                    if let BlockToken::Text {
                        raw_len,
                        text,
                        inline_id,
                    } = tok
                    {
                        let rl = *raw_len;
                        let t = core::mem::take(text);
                        let id = *inline_id;
                        *tok = BlockToken::Paragraph {
                            raw_len: rl,
                            text: t,
                            inline_id: id,
                        };
                    }
                }
            }
        }

        Some(BlockToken::List {
            raw: list_raw,
            ordered: is_ordered,
            start,
            loose,
            items,
        })
    }

    fn tok_html(&self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let end = if self.mode.is_pedantic() {
            match_html_pedantic(bytes, pos)?
        } else {
            match_html_normal(bytes, pos)?
        };
        let raw_slice = trim_trailing_blank_lines(&src[pos..end]);
        Some(BlockToken::Html {
            raw_len: raw_slice.len(),
            text: String::from(raw_slice),
        })
    }

    fn tok_def<'a>(&self, src: &'a str, bytes: &[u8], pos: usize) -> Option<(&'a str, String, LinkDef)> {
        let m = match_def(bytes, pos, self.mode.is_pedantic())?;
        let href_raw = &src[m.href_start..m.href_end];
        if !href_raw.starts_with('<') && find_closing_paren(href_raw) != -1 {
            return None;
        }
        let tag = collapse_spaces(&normalize_label(&src[m.label_start..m.label_end]));
        let href_inner =
            if href_raw.len() >= 2 && href_raw.starts_with('<') && href_raw.ends_with('>') {
                &href_raw[1..href_raw.len() - 1]
            } else {
                href_raw
            };
        let href = unescape_any_punctuation(href_inner);
        let title = m.title.map(|(ts, te)| unescape_any_punctuation(&src[ts + 1..te - 1]));
        Some((&src[pos..m.raw_end], tag, LinkDef { href, title }))
    }

    fn tok_table(&mut self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let m = match_table_at(bytes, pos)?;
        let align_slice = &src[m.align_start..m.align_end];
        if !bytes[m.align_start..m.align_end]
            .iter()
            .any(|&b| b == b':' || b == b'|')
        {
            return None;
        }
        let headers = split_cells(&src[m.header_start..m.header_end], None);
        let align_parts: Vec<&str> = trim_table_align_chars(align_slice).split('|').collect();
        if headers.len() != align_parts.len() {
            return None;
        }
        let mut aligns: Vec<Align> = Vec::with_capacity(align_parts.len());
        for a in align_parts {
            aligns.push(parse_table_align(a));
        }

        let mut header_cells: Vec<TableCell> = Vec::with_capacity(headers.len());
        for (i, h) in headers.into_iter().enumerate() {
            let inline_id = self.queue_inline(h);
            header_cells.push(TableCell {
                header: true,
                align: aligns[i],
                inline_id,
            });
        }

        let mut rows: Vec<Vec<TableCell>> = Vec::new();
        if let Some((bs, be)) = m.body {
            let body = &src[bs..be];
            if !trim_js(body).is_empty() {
                let trimmed = trim_table_row_blank_line(body);
                for row_str in trimmed.split('\n') {
                    let cells = split_cells(row_str, Some(header_cells.len()));
                    let mut row_cells: Vec<TableCell> = Vec::with_capacity(cells.len());
                    for (c, cell) in cells.into_iter().enumerate() {
                        let inline_id = self.queue_inline(cell);
                        row_cells.push(TableCell {
                            header: false,
                            align: aligns[c],
                            inline_id,
                        });
                    }
                    rows.push(row_cells);
                }
            }
        }

        Some(BlockToken::Table {
            raw_len: m.raw_end - pos,
            header: header_cells,
            rows,
        })
    }

    fn tok_lheading(&mut self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let m = match_lheading_at(bytes, pos, self.mode)?;
        let text = String::from(trim_js(&src[m.text_start..m.text_end]));
        let inline_id = self.queue_inline(text);
        Some(BlockToken::Heading {
            raw_len: m.raw_end - pos,
            depth: m.depth,
            inline_id,
        })
    }

    fn tok_paragraph(&mut self, src: &str, bytes: &[u8], pos: usize) -> Option<BlockToken> {
        let end = match_paragraph_at(bytes, pos, self.mode, false)?;
        let raw_slice = &src[pos..end];
        let text_slice = raw_slice.strip_suffix('\n').unwrap_or(raw_slice);
        let text = String::from(text_slice);
        let inline_id = self.queue_inline(text.clone());
        Some(BlockToken::Paragraph {
            raw_len: raw_slice.len(),
            text,
            inline_id,
        })
    }

    // -----------------------------------------------------------------------------------------
    // Inline Lexer + Tokenizer

    fn inline_tokens(&mut self, src: &str, tokens: &mut Vec<InlineToken>) {
        let bytes = src.as_bytes();
        let prev_link_paren = self.state.link_paren_possible;
        self.state.link_paren_possible = prev_link_paren && bytes.contains(&b')');
        self.inline_tokens_inner(src, bytes, tokens);
        self.state.link_paren_possible = prev_link_paren;
    }

    fn inline_tokens_inner(&mut self, full_src: &str, bytes: &[u8], tokens: &mut Vec<InlineToken>) {
        let n = bytes.len();
        // Lazily compute `masked_bytes` within this single call only if `*`, `_`, or `~` is
        // actually encountered (skips 3 masking passes on plain spans with zero cross-call state).
        let mut masked_cache: Option<Vec<u8>> = None;
        let mut pos = 0usize;
        let mut keep_prev_char = false;
        let mut prev_char = PrevChar::None;
        let mut prev_pos = usize::MAX;

        while pos < n {
            if pos == prev_pos {
                throw_value(JsValue::erase("marked: infinite loop in inlineTokens"));
            }
            prev_pos = pos;

            if !keep_prev_char {
                prev_char = PrevChar::None;
            }
            keep_prev_char = false;

            let b0 = bytes[pos];

            // 1. escape
            if b0 == b'\\' {
                if let Some(ch_byte) = match_escape(bytes, pos) {
                    pos += 2;
                    let mut t = String::with_capacity(1);
                    t.push(ch_byte as char);
                    tokens.push(InlineToken::Escape { text: t });
                    continue;
                }
            }

            // 2. tag
            if b0 == b'<' {
                if let Some(end) = match_tag(bytes, pos) {
                    let raw_b = &bytes[pos..end];
                    if !self.state.in_link && is_start_a_tag(raw_b) {
                        self.state.in_link = true;
                    } else if self.state.in_link && is_end_a_tag(raw_b) {
                        self.state.in_link = false;
                    }
                    if !self.state.in_raw_block && is_start_pre_script_tag(raw_b) {
                        self.state.in_raw_block = true;
                    } else if self.state.in_raw_block && is_end_pre_script_tag(raw_b) {
                        self.state.in_raw_block = false;
                    }
                    let raw = &full_src[pos..end];
                    pos = end;
                    tokens.push(InlineToken::Html {
                        text: String::from(raw),
                    });
                    continue;
                }
            }

            // 3. link
            if (b0 == b'[' || b0 == b'!') && self.state.link_paren_possible {
                if let Some((tok, end)) = self.tok_link(full_src, bytes, pos) {
                    pos = end;
                    tokens.push(tok);
                    continue;
                }
            }

            // 4. reflink / nolink
            if b0 == b'[' || b0 == b'!' {
                if let Some((tok, end)) = self.tok_reflink(full_src, bytes, pos) {
                    pos = end;
                    if let InlineToken::Text { text: t_text, .. } = &tok {
                        if let Some(InlineToken::Text {
                            text: last_text, ..
                        }) = tokens.last_mut()
                        {
                            last_text.push_str(t_text);
                            continue;
                        }
                    }
                    tokens.push(tok);
                    continue;
                }
            }

            // 5. em & strong
            if b0 == b'*' || b0 == b'_' {
                let masked_ref = masked_cache.get_or_insert_with(|| {
                    build_masked_bytes(full_src, bytes, &self.links, self.mode.is_pedantic())
                });
                let masked_slice: Vec<u8> = core::mem::take(masked_ref);
                let res = self.tok_em_strong(full_src, bytes, pos, &masked_slice, prev_char);
                *masked_ref = masked_slice;
                if let Some((tok, end)) = res {
                    pos = end;
                    tokens.push(tok);
                    continue;
                }
            }

            // 6. codespan
            if b0 == b'`' {
                if let Some((end, k)) = match_inline_code_at(bytes, pos) {
                    let raw_inner = &full_src[pos + k..end - k];
                    let rb = &bytes[pos + k..end - k];
                    let mut text = String::with_capacity(rb.len());
                    let mut has_non_space = false;
                    let mut last = 0usize;
                    let mut i = 0usize;
                    while i < rb.len() {
                        let b = rb[i];
                        if b == b'\n' {
                            if i > last {
                                text.push_str(&raw_inner[last..i]);
                            }
                            text.push(' ');
                            last = i + 1;
                        } else if b != b' ' {
                            has_non_space = true;
                        }
                        i += 1;
                    }
                    if last < rb.len() {
                        text.push_str(&raw_inner[last..]);
                    }
                    if has_non_space
                        && text.starts_with(' ')
                        && text.ends_with(' ')
                        && text.len() >= 2
                    {
                        text = String::from(&text[1..text.len() - 1]);
                    }
                    pos = end;
                    tokens.push(InlineToken::Codespan { text });
                    continue;
                }
            }

            // 7. br
            if b0 == b' ' || b0 == b'\\' || (b0 == b'\n' && self.mode.is_breaks()) {
                if let Some(end) = match_br(bytes, pos, self.mode.is_breaks()) {
                    pos = end;
                    tokens.push(InlineToken::Br);
                    continue;
                }
            }

            // 8. del (GFM / breaks only)
            if b0 == b'~' && self.mode.is_gfm() {
                let masked_ref = masked_cache.get_or_insert_with(|| {
                    build_masked_bytes(full_src, bytes, &self.links, self.mode.is_pedantic())
                });
                let masked_slice: Vec<u8> = core::mem::take(masked_ref);
                let res = self.tok_del(full_src, bytes, pos, &masked_slice, prev_char);
                *masked_ref = masked_slice;
                if let Some((tok, end)) = res {
                    pos = end;
                    tokens.push(tok);
                    continue;
                }
            }

            // 9. autolink
            if b0 == b'<' {
                if let Some((end, is_email)) = match_autolink(bytes, pos) {
                    let inner = &full_src[pos + 1..end - 1];
                    let text = String::from(inner);
                    let href = if is_email {
                        let mut h = String::with_capacity(7 + text.len());
                        h.push_str("mailto:");
                        h.push_str(&text);
                        h
                    } else {
                        text.clone()
                    };
                    pos = end;
                    tokens.push(InlineToken::Link {
                        href,
                        title: None,
                        text,
                        autolink: true,
                        tokens: Vec::new(),
                    });
                    continue;
                }
            }

            // 10. url (GFM / breaks only, and !state.in_link)
            if self.mode.is_gfm() && !self.state.in_link {
                if let Some(m) = match_gfm_url(bytes, pos) {
                    let raw = &full_src[pos..m.raw_end];
                    let text = String::from(raw);
                    let href = if m.is_bare_email {
                        let mut h = String::with_capacity(7 + text.len());
                        h.push_str("mailto:");
                        h.push_str(&text);
                        h
                    } else if m.is_www {
                        let mut h = String::with_capacity(7 + text.len());
                        h.push_str("http://");
                        h.push_str(&text);
                        h
                    } else {
                        text.clone()
                    };
                    pos = m.raw_end;
                    tokens.push(InlineToken::Link {
                        href,
                        title: None,
                        text,
                        autolink: true,
                        tokens: Vec::new(),
                    });
                    continue;
                }
            }

            // 11. inlineText
            if let Some(end) = match_inline_text(bytes, pos, self.mode) {
                let raw_slice = &full_src[pos..end];
                let last_pc = PrevChar::from_last_byte_slice(&bytes[pos..end]);
                if !last_pc.is_char('_') {
                    prev_char = last_pc;
                }
                keep_prev_char = true;
                pos = end;
                let escaped = self.state.in_raw_block;
                if let Some(InlineToken::Text {
                    text: last_text, ..
                }) = tokens.last_mut()
                {
                    if escaped {
                        last_text.push_str(raw_slice);
                    } else {
                        decode_numeric_character_references_into(raw_slice, last_text);
                    }
                } else {
                    let mut text = String::with_capacity(raw_slice.len());
                    if escaped {
                        text.push_str(raw_slice);
                    } else {
                        decode_numeric_character_references_into(raw_slice, &mut text);
                    }
                    tokens.push(InlineToken::Text { text, escaped });
                }
                continue;
            }

            throw_value(JsValue::erase("marked: infinite loop in inlineTokens"));
        }
    }

    fn output_link(
        &mut self,
        is_image: bool,
        raw_label: &str,
        href: String,
        title: Option<String>,
    ) -> Option<InlineToken> {
        let text = unescape_link_brackets(raw_label);
        let outer_in_link = self.state.in_link;
        let outer_link_emitted = self.state.link_emitted;
        let outer_in_raw_block = self.state.in_raw_block;
        self.state.in_link = true;
        self.state.link_emitted = false;
        let mut child_tokens = Vec::new();
        self.inline_tokens(&text, &mut child_tokens);
        let text_has_link = self.state.link_emitted;
        self.state.link_emitted = outer_link_emitted;
        self.state.in_link = outer_in_link;

        let title = title.filter(|t| !t.is_empty());

        if !is_image {
            if text_has_link {
                self.state.in_raw_block = outer_in_raw_block;
                return None;
            }
            self.state.link_emitted = true;
            Some(InlineToken::Link {
                href,
                title,
                text,
                autolink: false,
                tokens: child_tokens,
            })
        } else {
            Some(InlineToken::Image {
                href,
                title,
                text,
                tokens: child_tokens,
            })
        }
    }

    fn tok_link(
        &mut self,
        full_src: &str,
        bytes: &[u8],
        pos: usize,
    ) -> Option<(InlineToken, usize)> {
        let m = match_inline_link(bytes, pos, self.mode.is_pedantic())?;
        if !self.options.pedantic && is_label_end_inside_token(bytes, m.label_start, m.label_end) {
            return None;
        }

        let mut raw_slice = &full_src[pos..m.raw_end];
        let label_str = &full_src[m.label_start..m.label_end];
        let mut cap_href = &full_src[m.href_start..m.href_end];
        let mut cap_title = m.title.map(|(ts, te)| &full_src[ts..te]);
        let trimmed_url = trim_js(cap_href);

        if !self.options.pedantic && trimmed_url.starts_with('<') {
            if !trimmed_url.ends_with('>') {
                return None;
            }
            let without_gt = &trimmed_url[..trimmed_url.len() - 1];
            let rtrim_slash = rtrim_byte(without_gt, b'\\');
            if (trimmed_url.len() - rtrim_slash.len()) % 2 == 0 {
                return None;
            }
        } else {
            let last_paren = find_closing_paren(cap_href);
            if last_paren == -2 {
                return None;
            }
            if last_paren > -1 {
                let lp = last_paren as usize;
                let start_u16 = if m.is_image { 5 } else { 4 };
                // Match `Tokenizer.ts:783`: `linkLen = start + cap[1].length + lastParenIndex`
                // in UTF-16 code units, then map back to the UTF-8 byte boundary of `raw_slice`.
                let link_len_u16 =
                    start_u16 + utf16_len(label_str) + utf16_len(&cap_href[..lp]);
                let link_byte_len = utf16_offset_to_utf8_byte(raw_slice, link_len_u16);
                cap_href = &cap_href[..lp];
                raw_slice = trim_js(&raw_slice[..link_byte_len]);
                cap_title = None;
            }
        }

        let mut href_s = cap_href;
        let title_s: &str;
        if self.options.pedantic {
            if let Some((h, t)) = match_pedantic_href_title(href_s) {
                href_s = h;
                title_s = t;
            } else {
                title_s = "";
            }
        } else {
            title_s = match cap_title {
                Some(t) if t.len() >= 2 => &t[1..t.len() - 1],
                _ => "",
            };
        }

        href_s = trim_js(href_s);
        if href_s.starts_with('<') {
            if self.options.pedantic && !trimmed_url.ends_with('>') {
                href_s = &href_s[1..];
            } else if href_s.len() >= 2 {
                href_s = &href_s[1..href_s.len() - 1];
            } else {
                href_s = "";
            }
        }

        let final_href = if href_s.is_empty() {
            String::new()
        } else {
            unescape_any_punctuation(href_s)
        };
        let final_title = if title_s.is_empty() {
            None
        } else {
            Some(unescape_any_punctuation(title_s))
        };

        let end = pos + raw_slice.len();
        let tok = self.output_link(m.is_image, label_str, final_href, final_title)?;
        Some((tok, end))
    }

    fn tok_reflink(
        &mut self,
        full_src: &str,
        bytes: &[u8],
        pos: usize,
    ) -> Option<(InlineToken, usize)> {
        let m = match_reflink_at(bytes, pos, self.mode.is_pedantic(), false)?;
        if !self.options.pedantic && is_label_end_inside_token(bytes, m.label_start, m.label_end) {
            return None;
        }
        let label_str = &full_src[m.label_start..m.label_end];
        let lookup_raw = match m.ref_span {
            Some((rs, re)) => &full_src[rs..re],
            None => label_str,
        };
        let key = normalize_label(&collapse_spaces(lookup_raw));
        let found = self.links.get(key.as_str()).cloned();
        match found {
            None => {
                let text = String::from(&full_src[pos..pos + 1]);
                Some((
                    InlineToken::Text {
                        text,
                        escaped: false,
                    },
                    pos + 1,
                ))
            }
            Some(def) => {
                let tok = self.output_link(m.is_image, label_str, def.href, def.title)?;
                Some((tok, m.raw_end))
            }
        }
    }

    fn tok_em_strong(
        &mut self,
        full_src: &str,
        bytes: &[u8],
        pos: usize,
        full_masked: &[u8],
        prev_char: PrevChar,
    ) -> Option<(InlineToken, usize)> {
        let lm = match_em_strong_l(bytes, pos, self.mode)?;
        if lm.is_und_non_punct && prev_char.is_unicode_alnum() {
            return None;
        }
        if !lm.next_is_punct || prev_char.is_empty() || prev_char.is_inline_punctuation() {
            let l_len = lm.l_len;
            let mut delim_total = l_len as isize;
            let mut mid_delim_total = 0isize;
            let mid_run = prev_char.is_char(lm.delim_char as char);

            let clipped_masked = &full_masked[pos + l_len..];
            let mut scan_pos = 0usize;

            while let Some(hit) =
                next_rdelim(clipped_masked, scan_pos, lm.delim_char, self.mode, false)
            {
                scan_pos = hit.next_pos;
                let r_len = hit.delim_len;

                match hit.kind {
                    RDelimKind::LeftOnly => {
                        delim_total += r_len as isize;
                        continue;
                    }
                    RDelimKind::Both => {
                        if (l_len % 3 != 0) && ((l_len + r_len) % 3 == 0) {
                            mid_delim_total += r_len as isize;
                            continue;
                        }
                        if mid_run {
                            break;
                        }
                    }
                    RDelimKind::RightOnly => {}
                }

                delim_total -= r_len as isize;
                if delim_total > 0 {
                    continue;
                }

                let eff_r_len = (r_len as isize)
                    .min((r_len as isize) + delim_total + mid_delim_total)
                    .max(0) as usize;
                let raw_end = pos + l_len + hit.delim_start + eff_r_len;

                if l_len.min(eff_r_len) % 2 == 1 {
                    let inner = &full_src[pos + 1..raw_end - 1];
                    let mut child_tokens = Vec::new();
                    self.inline_tokens(inner, &mut child_tokens);
                    return Some((
                        InlineToken::Em {
                            text: String::from(inner),
                            tokens: child_tokens,
                        },
                        raw_end,
                    ));
                } else {
                    let inner = &full_src[pos + 2..raw_end - 2];
                    let mut child_tokens = Vec::new();
                    self.inline_tokens(inner, &mut child_tokens);
                    return Some((
                        InlineToken::Strong {
                            text: String::from(inner),
                            tokens: child_tokens,
                        },
                        raw_end,
                    ));
                }
            }
        }
        None
    }

    fn tok_del(
        &mut self,
        full_src: &str,
        bytes: &[u8],
        pos: usize,
        full_masked: &[u8],
        prev_char: PrevChar,
    ) -> Option<(InlineToken, usize)> {
        let (l_len, next_is_punct) = match_del_l(bytes, pos)?;
        if !next_is_punct || prev_char.is_empty() || prev_char.is_inline_punctuation() {
            let mut delim_total = l_len as isize;
            let clipped_masked = &full_masked[pos + l_len..];
            let mut scan_pos = 0usize;

            while let Some(hit) = next_rdelim(clipped_masked, scan_pos, b'~', self.mode, true) {
                scan_pos = hit.next_pos;
                let r_len = hit.delim_len;
                if r_len != l_len {
                    continue;
                }
                if matches!(hit.kind, RDelimKind::LeftOnly) {
                    delim_total += r_len as isize;
                    continue;
                }
                delim_total -= r_len as isize;
                if delim_total > 0 {
                    continue;
                }
                let eff_r_len = (r_len as isize).min((r_len as isize) + delim_total).max(0) as usize;
                let raw_end = pos + l_len + hit.delim_start + eff_r_len;
                let inner = &full_src[pos + l_len..raw_end - l_len];
                let mut child_tokens = Vec::new();
                self.inline_tokens(inner, &mut child_tokens);
                return Some((
                    InlineToken::Del {
                        text: String::from(inner),
                        tokens: child_tokens,
                    },
                    raw_end,
                ));
            }
        }
        None
    }
}

fn parse_decimal_u64(s: &str) -> u64 {
    let mut val = 0u64;
    for &b in s.as_bytes() {
        val = val * 10 + (b - b'0') as u64;
    }
    val
}

fn expand_leading_space_tab(line: &str, indent: usize) -> String {
    let bytes = line.as_bytes();
    let mut end = 0;
    while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
        end += 1;
    }
    if end == 0 || !bytes[..end].contains(&b'\t') {
        return String::from(line);
    }
    let mut out = expand_tabs(&line[..end], indent);
    out.push_str(&line[end..]);
    out
}

fn first_non_space_index(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b' ' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// JS `s.slice(units)`: the suffix after `units` UTF-16 code units, or `""` past the end.
///
/// `Tokenizer.list` slices `line` by `indent` code units. When that cut falls inside an astral
/// character, JS keeps the character's lone low surrogate and this keeps the whole character.
/// The only readers of such a line test how it starts (indented code, fence, heading, hr), and a
/// lone surrogate and an astral character both start it with a non-space, non-marker unit, so
/// the two answer those tests alike. The other slices cut a run of leading spaces or a
/// whitespace-only line, which hold BMP characters only, so they are exact.
fn slice_utf16_from(s: &str, units: usize) -> &str {
    &s[utf16_offset_to_utf8_byte(s, units)..]
}

fn is_line_indented_4_with_tabs(bytes: &[u8]) -> bool {
    let mut spaces = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' {
            spaces += 1;
        } else if b == b'\t' {
            spaces += 4;
        } else {
            return spaces >= 4;
        }
        i += 1;
    }
    false
}

fn utf16_len(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut units = 0usize;
    while i < bytes.len() {
        let (_, clen) = crate::unicode::decode_utf8_at(bytes, i);
        units += if clen == 4 { 2 } else { 1 };
        i += clen;
    }
    units
}

fn utf16_offset_to_utf8_byte(s: &str, target_u16: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut units = 0usize;
    while i < bytes.len() && units < target_u16 {
        let (_, clen) = crate::unicode::decode_utf8_at(bytes, i);
        let step = if clen == 4 { 2 } else { 1 };
        if units + step > target_u16 {
            // `target_u16` falls inside an astral surrogate pair: stop at that character's start.
            return i;
        }
        units += step;
        i += clen;
    }
    i
}
