//! Myers diagonal search with jsdiff's predecessor choice and edge pruning.
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

pub const KEEP: u8 = 0;
pub const REMOVE: u8 = 1;
pub const ADD: u8 = 2;
pub const CHARS: u8 = 0;
pub const WORDS: u8 = 1;
pub const LINES: u8 = 2;
pub const JSON: u8 = 3;

#[derive(Default)]
pub struct Options {
    pub ignore_case: bool,
    pub separate: bool,
    pub strip_cr: bool,
    pub ignore_ws: bool,
    pub newline_token: bool,
    pub ignore_eof: bool,
}
#[derive(Clone, Copy)]
pub struct Run {
    pub kind: u8,
    pub count: usize,
    previous: usize,
}
#[derive(Clone, Copy)]
struct Path {
    old: i32,
    last: usize,
}

fn append(arena: &mut Vec<Run>, path: &mut Path, kind: u8, count: usize, separate: bool) {
    if count == 0 {
        return;
    }
    if separate {
        for _ in 0..count {
            arena.push(Run {
                kind,
                count: 1,
                previous: path.last,
            });
            path.last = arena.len() - 1;
        }
    } else {
        let last = arena[path.last];
        let merge = last.kind == kind && path.last != 0;
        arena.push(Run {
            kind,
            count: count + if merge { last.count } else { 0 },
            previous: if merge { last.previous } else { path.last },
        });
        path.last = arena.len() - 1;
    }
}

fn common(
    path: &mut Path,
    k: i32,
    before: &[String],
    after: &[String],
    arena: &mut Vec<Run>,
    separate: bool,
) -> i32 {
    let start = path.old;
    let mut new = start - k;
    while path.old + 1 < before.len() as i32
        && new + 1 < after.len() as i32
        && before[(path.old + 1) as usize] == after[(new + 1) as usize]
    {
        path.old += 1;
        new += 1;
    }
    append(arena, path, KEEP, (path.old - start) as usize, separate);
    new
}

fn finish(arena: &[Run], mut index: usize) -> Vec<Run> {
    let mut out = Vec::new();
    while index > 0 {
        let r = arena[index];
        out.push(r);
        index = r.previous;
    }
    out.reverse();
    out
}

pub fn myers(before: &[String], after: &[String], separate: bool) -> Vec<Run> {
    let limit = (before.len() + after.len()) as i32;
    let base = limit + 1;
    let absent = Path { old: -2, last: 0 };
    let mut best = vec![absent; (base * 2 + 1) as usize];
    let mut arena = vec![Run {
        kind: KEEP,
        count: 0,
        previous: 0,
    }];
    let mut first = Path { old: -1, last: 0 };
    let end_old = before.len() as i32 - 1;
    let end_new = after.len() as i32 - 1;
    let new = common(&mut first, 0, before, after, &mut arena, separate);
    if first.old == end_old && new == end_new {
        return finish(&arena, first.last);
    }
    best[base as usize] = first;
    let mut low = -limit;
    let mut high = limit;
    for distance in 1..=limit {
        let mut k = low.max(-distance);
        let stop = high.min(distance);
        while k <= stop {
            let i = (base + k) as usize;
            let remove = best[i - 1];
            let add = best[i + 1];
            best[i - 1] = absent;
            let can_remove = remove.old >= -1 && remove.old < end_old;
            let can_add = add.old >= -1 && add.old - k >= 0 && add.old - k <= end_new;
            if can_remove || can_add {
                let mut path;
                if !can_remove || (can_add && remove.old < add.old) {
                    path = add;
                    append(&mut arena, &mut path, ADD, 1, separate);
                } else {
                    path = remove;
                    path.old += 1;
                    append(&mut arena, &mut path, REMOVE, 1, separate);
                }
                let new = common(&mut path, k, before, after, &mut arena, separate);
                if path.old == end_old && new == end_new {
                    return finish(&arena, path.last);
                }
                best[i] = path;
                if path.old == end_old {
                    high = high.min(k - 1)
                }
                if new == end_new {
                    low = low.max(k + 1)
                }
            } else {
                best[i] = absent
            }
            k += 2;
        }
    }
    Vec::new()
}

pub fn ws(c: char) -> bool {
    matches!(c,'\u{9}'..='\u{d}'|' '|'\u{a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')
}
fn word(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || c == '_'
        || matches!(c, '\u{ad}'|'\u{c0}'..='\u{d6}'|'\u{d8}'..='\u{f6}'|'\u{f8}'..='\u{2c6}'|'\u{2c8}'..='\u{2d7}'|'\u{2de}'..='\u{2ff}'|'\u{1e00}'..='\u{1eff}')
}
fn word_tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut class = 3;
    let mut i = 0;
    while i < s.len() {
        let (c, width) = crate::utf16::point(&s[i..]);
        let next = if ws(c) {
            0
        } else if word(c) {
            1
        } else {
            2
        };
        if next != class || next == 2 {
            if i > start {
                parts.push(&s[start..i]);
            }
            start = i;
        }
        class = next;
        i += width;
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    let mut previous = "";
    for part in parts {
        if ws(part.chars().next().unwrap()) {
            if let Some(token) = out.last_mut() {
                token.push_str(part)
            } else {
                out.push(part.to_string())
            }
        } else if previous.chars().next().is_some_and(ws) {
            if out.len() == 1 && out[0] == previous {
                out[0].push_str(part)
            } else {
                let mut token = previous.to_string();
                token.push_str(part);
                out.push(token)
            }
        } else {
            out.push(part.to_string())
        }
        previous = part;
    }
    out
}

pub fn line_tokens(s: &str, o: &Options) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..bytes.len() {
        if bytes[i] != 10 {
            continue;
        }
        let end = if i > start && bytes[i - 1] == 13 {
            i - 1
        } else {
            i
        };
        if o.newline_token {
            if start < end {
                out.push(s[start..end].to_string())
            }
            out.push(if o.strip_cr { "\n" } else { &s[end..i + 1] }.to_string());
        } else if o.strip_cr && end < i {
            let mut line = s[start..end].to_string();
            line.push('\n');
            out.push(line);
        } else {
            out.push(s[start..i + 1].to_string())
        }
        start = i + 1;
    }
    if start < s.len() {
        out.push(s[start..].to_string())
    }
    out
}

pub fn tokens(mode: u8, s: &str, o: &Options) -> Vec<String> {
    match mode {
        WORDS => word_tokens(s),
        LINES | JSON => line_tokens(s, o),
        _ => {
            let mut out = Vec::new();
            let mut i = 0;
            while i < s.len() {
                let (_, width) = crate::utf16::point(&s[i..]);
                out.push(s[i..i + width].to_string());
                i += width;
            }
            out
        }
    }
}
fn keys(mode: u8, tokens: &[String], o: &Options) -> Vec<String> {
    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        let mut s = token.as_str();
        if mode == WORDS
            || (mode == LINES && o.ignore_ws && (!o.newline_token || !s.contains('\n')))
        {
            s = s.trim_matches(ws)
        } else if mode == LINES && o.ignore_eof && !o.newline_token {
            s = s.strip_suffix('\n').unwrap_or(s)
        }
        let key = if mode == JSON {
            s.replace(",\r", "\r").replace(",\n", "\n")
        } else {
            s.to_string()
        };
        out.push(if o.ignore_case {
            key.to_lowercase()
        } else {
            key
        })
    }
    out
}
pub struct Change {
    pub kind: u8,
    pub count: usize,
    pub value: String,
}
pub fn changes(mode: u8, before: &[String], after: &[String], o: &Options) -> Vec<Change> {
    let runs = if mode == LINES && !o.ignore_case && !o.ignore_eof && !o.ignore_ws {
        myers(before, after, o.separate)
    } else {
        myers(&keys(mode, before, o), &keys(mode, after, o), o.separate)
    };
    let mut old = 0;
    let mut new = 0;
    let mut out = Vec::new();
    for r in runs {
        let mut value = String::new();
        for j in 0..r.count {
            // diffJson keeps the longer of two equal lines, by UTF-16 length.
            let token = if r.kind == REMOVE
                || (mode == JSON
                    && r.kind == KEEP
                    && crate::utf16::units(&before[old + j]) > crate::utf16::units(&after[new + j]))
            {
                &before[old + j]
            } else {
                &after[new + j]
            };
            value.push_str(if mode == WORDS && j > 0 {
                token.trim_start_matches(ws)
            } else {
                token
            });
        }
        if r.kind != ADD {
            old += r.count
        }
        if r.kind != REMOVE {
            new += r.count
        }
        out.push(Change {
            kind: r.kind,
            count: r.count,
            value,
        });
    }
    if mode == WORDS && !o.separate {
        crate::words::dedupe(&mut out)
    }
    out
}
