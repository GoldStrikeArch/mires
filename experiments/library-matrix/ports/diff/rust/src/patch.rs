use crate::diff::{line_tokens, myers, Options, ADD, KEEP, REMOVE};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

#[derive(Default)]
pub struct Hunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<String>,
}
#[derive(Default)]
pub struct Patch {
    pub metadata_order: Vec<&'static str>,
    pub old_name: Option<String>,
    pub new_name: Option<String>,
    pub old_header: Option<String>,
    pub new_header: Option<String>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub index: Option<String>,
    pub git: bool,
    pub rename: bool,
    pub copy: bool,
    pub binary: bool,
    pub create: bool,
    pub delete: bool,
    pub hunks: Vec<Hunk>,
}
fn prefix(mark: char, s: &str) -> String {
    let mut out = String::new();
    out.push(mark);
    out.push_str(s);
    out
}
fn emit(h: &mut Hunk, kind: u8, lines: &[String]) {
    for line in lines {
        let c = if kind == ADD {
            '+'
        } else if kind == REMOVE {
            '-'
        } else {
            ' '
        };
        if let Some(l) = line.strip_suffix('\n') {
            h.lines.push(prefix(c, l))
        } else {
            h.lines.push(prefix(c, line));
            h.lines.push("\\ No newline at end of file".to_string())
        }
        if kind != ADD {
            h.old_lines += 1
        }
        if kind != REMOVE {
            h.new_lines += 1
        }
    }
}
pub fn create(before: &str, after: &str, context: usize, mut patch: Patch) -> Patch {
    let before = line_tokens(before, &Options::default());
    let after = line_tokens(after, &Options::default());
    let runs = myers(&before, &after, false);
    let mut old = 0;
    let mut new = 0;
    let mut pending = Hunk::default();
    let mut active = false;
    let mut last_keep = 0;
    for (i, r) in runs.iter().enumerate() {
        if r.kind == KEEP {
            if active {
                let close = i + 1 == runs.len() || r.count > context.saturating_mul(2);
                let count = if close { r.count.min(context) } else { r.count };
                emit(&mut pending, KEEP, &after[new..new + count]);
                if close {
                    patch.hunks.push(core::mem::take(&mut pending));
                    active = false
                }
            }
            last_keep = r.count;
        } else {
            if !active {
                let k = context.min(last_keep);
                pending.old_start = old + 1 - k;
                pending.new_start = new + 1 - k;
                emit(&mut pending, KEEP, &after[new - k..new]);
                active = true;
            }
            if r.kind == ADD {
                emit(&mut pending, ADD, &after[new..new + r.count])
            } else {
                emit(&mut pending, REMOVE, &before[old..old + r.count])
            }
        }
        if r.kind != ADD {
            old += r.count
        }
        if r.kind != REMOVE {
            new += r.count
        }
    }
    if active {
        patch.hunks.push(pending)
    }
    patch
}
fn number(out: &mut String, mut n: usize) {
    let mut bytes = [0u8; 24];
    let mut i = 24;
    loop {
        i -= 1;
        bytes[i] = (n % 10) as u8 + b'0';
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for b in &bytes[i..] {
        out.push(*b as char)
    }
}
fn filename(s: &str) -> String {
    if !s.bytes().any(|b| b < 32 || b > 126 || b == 34 || b == 92) {
        return s.to_string();
    }
    let mut out = String::from("\"");
    let clean = crate::utf16::scalar_text(s);
    for b in clean.bytes() {
        let escaped = match b {
            7 => "\\a",
            8 => "\\b",
            9 => "\\t",
            10 => "\\n",
            11 => "\\v",
            12 => "\\f",
            13 => "\\r",
            34 => "\\\"",
            92 => "\\\\",
            _ => "",
        };
        if !escaped.is_empty() {
            out.push_str(escaped)
        } else if b < 32 || b > 126 {
            out.push('\\');
            out.push((48 + (b >> 6)) as char);
            out.push((48 + ((b >> 3) & 7)) as char);
            out.push((48 + (b & 7)) as char)
        } else {
            out.push(b as char)
        }
    }
    out.push('"');
    out
}
fn file_header(out: &mut String, tag: &str, name: &Option<String>, header: &Option<String>) {
    if let Some(name) = name {
        out.push_str(tag);
        out.push_str(&filename(name));
        if let Some(h) = header {
            if !h.is_empty() {
                out.push('\t');
                out.push_str(h)
            }
        }
        out.push('\n')
    }
}
pub fn format(p: &Patch) -> String {
    let mut out = String::new();
    if p.old_name == p.new_name {
        if let Some(n) = &p.old_name {
            out.push_str("Index: ");
            out.push_str(n);
            out.push('\n')
        }
    }
    out.push_str("===================================================================\n");
    file_header(&mut out, "--- ", &p.old_name, &p.old_header);
    file_header(&mut out, "+++ ", &p.new_name, &p.new_header);
    for h in &p.hunks {
        out.push_str("@@ -");
        number(&mut out, h.old_start - usize::from(h.old_lines == 0));
        out.push(',');
        number(&mut out, h.old_lines);
        out.push_str(" +");
        number(&mut out, h.new_start - usize::from(h.new_lines == 0));
        out.push(',');
        number(&mut out, h.new_lines);
        out.push_str(" @@\n");
        for l in &h.lines {
            out.push_str(l);
            out.push('\n')
        }
    }
    out
}
// WORKAROUND(MIRES-017): Utf8Chunks in String::from_utf8_lossy falls through
// the final successful state into the generated "bad block" exception.
fn utf8_filename(mut bytes: &[u8]) -> String {
    let mut name = String::new();
    loop {
        match core::str::from_utf8(bytes) {
            Ok(rest) => {
                name.push_str(rest);
                return name;
            }
            Err(error) => {
                let end = error.valid_up_to();
                name.push_str(core::str::from_utf8(&bytes[..end]).unwrap());
                name.push('\u{fffd}');
                if let Some(skip) = error.error_len() {
                    bytes = &bytes[end + skip..];
                } else {
                    return name;
                }
            }
        }
    }
}

fn quoted(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    // WORKAROUND(MIRES-019): compare a copied byte to avoid a promoted Option<&u8>.
    if b.first().copied() != Some(b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'"' => return Some((out, i + 1)),
            b'\\' if i + 1 < b.len() => {
                if (b'0'..=b'7').contains(&b[i + 1]) {
                    let mut bytes = Vec::new();
                    while i + 1 < b.len() && b[i] == b'\\' && (b'0'..=b'7').contains(&b[i + 1]) {
                        if i + 3 >= b.len()
                            || !b[i + 2..=i + 3].iter().all(|c| (b'0'..=b'7').contains(c))
                        {
                            return None;
                        }
                        bytes.push(
                            ((b[i + 1] - 48) << 6) | ((b[i + 2] - 48) << 3) | (b[i + 3] - 48),
                        );
                        i += 4;
                    }
                    let decoded = utf8_filename(&bytes);
                    out.push_str(
                        &decoded
                            .strip_prefix('\u{feff}')
                            .unwrap_or(&decoded)
                            .replace('\u{e800}', "\u{e800}\u{e800}"),
                    );
                } else {
                    out.push(match b[i + 1] {
                        b'a' => '\x07',
                        b'b' => '\x08',
                        b't' => '\t',
                        b'n' => '\n',
                        b'v' => '\x0b',
                        b'f' => '\x0c',
                        b'r' => '\r',
                        b'"' => '"',
                        b'\\' => '\\',
                        _ => return None,
                    });
                    i += 2;
                }
            }
            _ => {
                let start = i;
                while i < b.len() && b[i] != b'\\' && b[i] != b'"' {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                out.push_str(&s[start..i]);
            }
        }
    }
    None
}
fn unquote(s: &str) -> String {
    match quoted(s) {
        Some((n, _)) => n,
        None => s.to_string(),
    }
}
fn git_names(rest: &str, p: &mut Patch) {
    if let Some((s, len)) = quoted(rest) {
        let next = rest[len..].trim_start_matches(crate::diff::ws);
        if !next.is_empty() {
            p.old_name = Some(s);
            p.new_name = Some(unquote(next))
        }
    } else if let Some(q) = rest.find('"') {
        if q > 0 {
            if let Some((s, _)) = quoted(&rest[q..]) {
                p.old_name = Some(rest[..q].trim_end().to_string());
                p.new_name = Some(s)
            }
        }
    } else if rest.starts_with("a/") {
        let mut splits = Vec::new();
        for (at, _) in rest.match_indices(" b/") {
            splits.push(at)
        }
        if !splits.is_empty() {
            let at = splits[splits.len() / 2];
            p.old_name = Some(rest[..at].to_string());
            p.new_name = Some(rest[at + 1..].to_string())
        }
    }
}
fn note(p: &mut Patch, name: &'static str) {
    if !p.metadata_order.contains(&name) {
        p.metadata_order.push(name)
    }
}
fn ext(p: &mut Patch, line: &str) {
    for (i, tag) in [
        "rename from ",
        "rename to ",
        "copy from ",
        "copy to ",
        "new file mode ",
        "deleted file mode ",
        "old mode ",
        "new mode ",
    ]
    .iter()
    .enumerate()
    {
        if let Some(v) = line.strip_prefix(tag) {
            match i {
                0..=3 => {
                    note(
                        p,
                        if i % 2 == 0 {
                            "oldFileName"
                        } else {
                            "newFileName"
                        },
                    );
                    note(p, if i < 2 { "isRename" } else { "isCopy" });
                }
                4 => {
                    note(p, "isCreate");
                    note(p, "newMode")
                }
                5 => {
                    note(p, "isDelete");
                    note(p, "oldMode")
                }
                6 => note(p, "oldMode"),
                _ => note(p, "newMode"),
            }
            match i {
                0 | 2 => {
                    p.old_name = Some(String::from("a/") + &unquote(v));
                    if i == 0 {
                        p.rename = true
                    } else {
                        p.copy = true
                    }
                }
                1 | 3 => {
                    p.new_name = Some(String::from("b/") + &unquote(v));
                    if i == 1 {
                        p.rename = true
                    } else {
                        p.copy = true
                    }
                }
                4 => {
                    p.create = true;
                    p.new_mode = Some(v.to_string())
                }
                5 => {
                    p.delete = true;
                    p.old_mode = Some(v.to_string())
                }
                6 => p.old_mode = Some(v.to_string()),
                _ => p.new_mode = Some(v.to_string()),
            }
            return;
        }
    }
    if line.starts_with("Binary files ") {
        note(p, "isBinary");
        p.binary = true
    }
}
fn file(line: &str) -> bool {
    line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("---\t")
        || line.starts_with("+++\t")
}
fn header(line: &str) -> bool {
    line.starts_with("diff --git ") || line.starts_with("Index: ") || line.starts_with("diff -r ")
}
fn parse_file(line: &str, p: &mut Patch) {
    let mut parts = line[3..].trim_matches(crate::diff::ws).split('\t');
    let n = parts.next().unwrap_or("");
    let name = if n.starts_with('"') {
        unquote(n)
    } else {
        n.replace("\\\\", "\\")
    };
    let h = parts
        .next()
        .unwrap_or("")
        .trim_matches(crate::diff::ws)
        .to_string();
    if line.starts_with("---") {
        note(p, "oldFileName");
        note(p, "oldHeader");
        p.old_name = Some(name);
        p.old_header = Some(h)
    } else {
        note(p, "newFileName");
        note(p, "newHeader");
        p.new_name = Some(name);
        p.new_header = Some(h)
    }
}
fn range(s: &str) -> Result<(usize, usize), ()> {
    let (a, b) = s.split_once(',').unwrap_or((s, "1"));
    let start = a.parse::<usize>().map_err(|_| ())?;
    let n = b.parse::<usize>().map_err(|_| ())?;
    Ok((start + usize::from(n == 0), n))
}
fn parse_hunk(lines: &[&str], i: &mut usize) -> Result<Hunk, ()> {
    let line = lines[*i];
    *i += 1;
    let (head, _) = line
        .strip_prefix("@@ -")
        .ok_or(())?
        .split_once(" @@")
        .ok_or(())?;
    let (a, b) = head.split_once(" +").ok_or(())?;
    let (old_start, old_lines) = range(a)?;
    let (new_start, new_lines) = range(b)?;
    let mut h = Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        lines: Vec::new(),
    };
    let mut old = 0;
    let mut new = 0;
    while *i < lines.len()
        && (old < h.old_lines || new < h.new_lines || lines[*i].starts_with('\\'))
    {
        let l = lines[*i];
        let op = l
            .as_bytes()
            .first()
            .copied()
            .unwrap_or(if *i + 1 < lines.len() { 32 } else { 0 });
        match op {
            32 => {
                new += 1;
                old += 1
            }
            43 => new += 1,
            45 => old += 1,
            92 => (),
            _ => return Err(()),
        }
        h.lines.push(l.to_string());
        *i += 1;
    }
    if old == 0 && h.old_lines == 1 {
        h.old_lines = 0
    }
    if new == 0 && h.new_lines == 1 {
        h.new_lines = 0
    }
    if old != h.old_lines || new != h.new_lines {
        return Err(());
    }
    if *i < lines.len()
        && matches!(lines[*i].as_bytes().first(), Some(32 | 43 | 45))
        && !file(lines[*i])
    {
        return Err(());
    }
    Ok(h)
}
pub fn parse(s: &str) -> Result<Vec<Patch>, ()> {
    let mut lines = Vec::new();
    for l in s.split('\n') {
        lines.push(l)
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let mut p = Patch::default();
        let mut seen = false;
        while i < lines.len() && !file(lines[i]) && !lines[i].starts_with("@@ ") {
            let l = lines[i];
            if header(l) {
                if seen {
                    break;
                }
                seen = true;
                if let Some(rest) = l.strip_prefix("diff --git ") {
                    note(&mut p, "isGit");
                    p.git = true;
                    git_names(rest, &mut p);
                    if p.old_name.is_some() {
                        note(&mut p, "oldFileName");
                        note(&mut p, "newFileName");
                    }
                } else if let Some(rest) = l.strip_prefix("Index: ") {
                    note(&mut p, "index");
                    p.index = Some(rest.trim_matches(crate::diff::ws).to_string())
                } else if let Some(last) = l.rsplit_once(' ') {
                    note(&mut p, "index");
                    p.index = Some(last.1.to_string())
                }
            } else if p.git {
                ext(&mut p, l)
            }
            i += 1
        }
        for _ in 0..2 {
            if i < lines.len() && file(lines[i]) {
                parse_file(lines[i], &mut p);
                i += 1
            }
        }
        if p.old_name.is_some() != p.new_name.is_some() {
            return Err(());
        }
        while i < lines.len() {
            let l = lines[i];
            if file(l)
                || header(l)
                || l.starts_with(
                    "===================================================================",
                )
            {
                break;
            }
            if l.starts_with("@@ ") {
                p.hunks.push(parse_hunk(&lines, &mut i)?)
            } else {
                i += 1
            }
        }
        out.push(p);
    }
    Ok(out)
}
fn swap(n: &mut Option<String>) {
    if let Some(s) = n {
        if s.starts_with("a/") {
            *s = String::from("b/") + &s[2..]
        } else if s.starts_with("b/") {
            *s = String::from("a/") + &s[2..]
        }
    }
}
pub fn reverse(p: &mut Patch) {
    core::mem::swap(&mut p.old_name, &mut p.new_name);
    core::mem::swap(&mut p.old_header, &mut p.new_header);
    core::mem::swap(&mut p.old_mode, &mut p.new_mode);
    core::mem::swap(&mut p.create, &mut p.delete);
    if p.git {
        swap(&mut p.old_name);
        swap(&mut p.new_name)
    }
    if p.copy {
        p.copy = false;
        p.rename = false;
        p.delete = true;
        p.create = false;
        p.new_name = Some("/dev/null".to_string());
        p.new_header = None;
        p.hunks.clear();
        return;
    }
    for h in &mut p.hunks {
        core::mem::swap(&mut h.old_start, &mut h.new_start);
        core::mem::swap(&mut h.old_lines, &mut h.new_lines);
        for l in &mut h.lines {
            if l.starts_with('+') {
                *l = prefix('-', &l[1..])
            } else if l.starts_with('-') {
                *l = prefix('+', &l[1..])
            }
        }
    }
}
fn convert(source: &str, p: &mut Patch) {
    let mut unix = false;
    let mut win = false;
    for l in source.split_inclusive('\n') {
        if l.ends_with("\r\n") {
            win = true
        } else if l.ends_with('\n') {
            unix = true
        }
    }
    if unix == win {
        return;
    }
    let mut patch_win = false;
    let mut patch_unix = false;
    for h in &p.hunks {
        for (i, l) in h.lines.iter().enumerate() {
            if l.starts_with('\\') {
                continue;
            }
            if l.ends_with('\r') {
                patch_win = true
            } else if !h.lines.get(i + 1).is_some_and(|s| s.starts_with('\\')) {
                patch_unix = true
            }
        }
    }
    if win && !patch_win {
        for h in &mut p.hunks {
            for i in 0..h.lines.len() {
                if !h.lines[i].starts_with('\\')
                    && !h.lines.get(i + 1).is_some_and(|s| s.starts_with('\\'))
                {
                    h.lines[i].push('\r')
                }
            }
        }
    }
    if unix && patch_win && !patch_unix {
        for h in &mut p.hunks {
            for l in &mut h.lines {
                if l.ends_with('\r') {
                    l.pop();
                }
            }
        }
    }
}
fn matches_at(h: &Hunk, lines: &[&str], mut i: usize) -> bool {
    for l in &h.lines {
        if l.starts_with('+') || l.starts_with('\\') {
            continue;
        }
        let content = if l.is_empty() { "" } else { &l[1..] };
        if i >= lines.len() || lines[i] != content {
            return false;
        }
        i += 1
    }
    true
}
pub fn apply(source: &str, p: &mut Patch) -> Option<String> {
    if p.hunks.is_empty() {
        return Some(source.to_string());
    }
    convert(source, p);
    let mut lines = Vec::new();
    for l in source.split('\n') {
        lines.push(l)
    }
    let mut add_eof = false;
    let mut remove_eof = false;
    let mut prev = 0;
    for l in &p.hunks.last()?.lines {
        if l.starts_with('\\') {
            if prev == 43 {
                remove_eof = true
            }
            if prev == 45 {
                add_eof = true
            }
        }
        prev = l.as_bytes().first().copied().unwrap_or(32)
    }
    // WORKAROUND(MIRES-019): querying the referenced line avoids a promoted Some(&"").
    let eof = lines.last().is_some_and(|line| line.is_empty());
    match (remove_eof, add_eof) {
        (true, true) if eof => return None,
        (true, false) => {
            if !eof {
                return None;
            }
            lines.pop();
        }
        (false, true) => {
            if eof {
                return None;
            }
            lines.push("")
        }
        _ => (),
    }
    let mut min = 0;
    let mut offset = 0isize;
    let mut result = Vec::new();
    for h in &p.hunks {
        let desired = h.old_start as isize - 1 + offset;
        let max = lines.len().checked_sub(h.old_lines)?;
        let mut pos = None;
        // Increasing distance from the declared line, with the later candidate first.
        let first = desired.clamp(min as isize, max.max(min) as isize);
        if min <= max {
            let extent = (first - min as isize).max(max as isize - first);
            for d in 0..=extent {
                let hi = first + d;
                let lo = first - d;
                if hi <= max as isize && matches_at(h, &lines, hi as usize) {
                    pos = Some(hi as usize);
                    break;
                }
                if lo >= min as isize && lo < hi && matches_at(h, &lines, lo as usize) {
                    pos = Some(lo as usize);
                    break;
                }
            }
        }
        let at = pos?;
        for l in &lines[min..at] {
            result.push(*l)
        }
        let mut cursor = at;
        let mut context = 0;
        for l in &h.lines {
            let op = l.as_bytes().first().copied().unwrap_or(32);
            if op == 43 {
                result.push(&l[1..]);
                context = 0
            }
            if op == 45 {
                cursor += 1;
                context = 0
            }
            if op == 32 {
                result.push(lines[cursor]);
                cursor += 1;
                context += 1
            }
        }
        result.truncate(result.len() - context);
        min = cursor - context;
        offset = at as isize + 1 - h.old_start as isize;
    }
    for l in &lines[min..] {
        result.push(*l)
    }
    Some(result.join("\n"))
}
