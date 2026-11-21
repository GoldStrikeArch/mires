use crate::diff::{line_tokens, myers, ws, Options, ADD, KEEP, REMOVE};
use crate::utf16::{drop_first_unit, drop_last_unit};
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

/// Hunk ranges are JavaScript numbers: parsed headers can exceed `u32` or be
/// `NaN`, exactly as `+match[1]` gives them in parse.js.
#[derive(Default)]
pub struct Hunk {
    pub old_start: f64,
    pub old_lines: f64,
    pub new_start: f64,
    pub new_lines: f64,
    pub lines: Vec<String>,
}
/// A file patch. `metadata_order` records the order in which parse.js first
/// assigns each property, which is the order of the object it returns.
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
impl Patch {
    fn record(&mut self, key: &'static str) {
        if !self.metadata_order.contains(&key) {
            self.metadata_order.push(key)
        }
    }
    fn set_old_name(&mut self, name: String) {
        self.record("oldFileName");
        self.old_name = Some(name)
    }
    fn set_new_name(&mut self, name: String) {
        self.record("newFileName");
        self.new_name = Some(name)
    }
    fn set_flag(&mut self, key: &'static str) {
        self.record(key);
        match key {
            "isGit" => self.git = true,
            "isRename" => self.rename = true,
            "isCopy" => self.copy = true,
            "isCreate" => self.create = true,
            "isDelete" => self.delete = true,
            _ => self.binary = true,
        }
    }
    fn set_old_mode(&mut self, mode: &str) {
        self.record("oldMode");
        self.old_mode = Some(mode.to_string())
    }
    fn set_new_mode(&mut self, mode: &str) {
        self.record("newMode");
        self.new_mode = Some(mode.to_string())
    }
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
            h.old_lines += 1.0
        }
        if kind != REMOVE {
            h.new_lines += 1.0
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
                pending.old_start = (old + 1 - k) as f64;
                pending.new_start = (new + 1 - k) as f64;
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
/// The start printed for an empty range is one lower (format.js); created
/// hunks hold nonnegative integers.
fn range(out: &mut String, start: f64, lines: f64) {
    number(out, (start - if lines == 0.0 { 1.0 } else { 0.0 }) as usize);
    out.push(',');
    number(out, lines as usize);
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
        range(&mut out, h.old_start, h.old_lines);
        out.push_str(" +");
        range(&mut out, h.new_start, h.new_lines);
        out.push_str(" @@\n");
        for l in &h.lines {
            out.push_str(l);
            out.push('\n')
        }
    }
    out
}

// Line classes of parse.js. Each regex is anchored at the start of the line;
// `\s` is the ECMAScript whitespace set (`ws`) and `\w` is ASCII.
const UNDERLINE: &str = "===================================================================";
fn space_after(line: &str, start: &str) -> bool {
    line.strip_prefix(start)
        .is_some_and(|rest| rest.starts_with(ws))
}
/// `/^diff --git /`
fn is_git_header(line: &str) -> bool {
    line.starts_with("diff --git ")
}
/// `/^(---|\+\+\+)\s/`
fn is_file_header(line: &str) -> bool {
    space_after(line, "---") || space_after(line, "+++")
}
/// `/^@@\s/`
fn is_hunk_header(line: &str) -> bool {
    space_after(line, "@@")
}
fn is_word(b: &u8) -> bool {
    b.is_ascii_alphanumeric() || *b == b'_'
}
/// The end of `diff(?: -r \w+)+` in a match of `/^diff(?: -r \w+)+\s/`. The
/// repetition is greedy; when the last group is not followed by whitespace,
/// backtracking gives it up and the space that starts it matches `\s`.
fn hg_prefix(line: &str) -> Option<usize> {
    if !line.starts_with("diff") {
        return None;
    }
    let b = line.as_bytes();
    let mut ends = Vec::new();
    let mut at = 4;
    while line[at..].starts_with(" -r ") && b.get(at + 4).is_some_and(is_word) {
        at += 4;
        while b.get(at).is_some_and(is_word) {
            at += 1
        }
        ends.push(at)
    }
    match ends.last() {
        Some(&end) if line[end..].starts_with(ws) => Some(end),
        _ if ends.len() >= 2 => Some(ends[ends.len() - 2]),
        _ => None,
    }
}
/// The length of the match of `/^(?:Index:|diff(?: -r \w+)+)\s+/`.
fn index_header(line: &str) -> Option<usize> {
    let start = if line.starts_with("Index:") {
        6
    } else {
        hg_prefix(line)?
    };
    let rest = &line[start..];
    let spaces = rest.len() - rest.trim_start_matches(ws).len();
    (spaces > 0).then_some(start + spaces)
}
/// `isDiffHeader`: a Git, `Index:` or Mercurial header.
fn is_diff_header(line: &str) -> bool {
    is_git_header(line) || index_header(line).is_some()
}
/// The capture of `(.*)`: `.` stops at the line terminators \r, U+2028 and U+2029.
fn rest_of_line(s: &str) -> &str {
    match s.find(|c| matches!(c, '\r' | '\u{2028}' | '\u{2029}')) {
        Some(end) => &s[..end],
        None => s,
    }
}
/// The capture of `(\d+)`, ASCII digits only.
fn mode(s: &str) -> Option<&str> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    (n > 0).then(|| &s[..n])
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
fn octal(b: &[u8], i: usize) -> bool {
    b.get(i).is_some_and(|c| (b'0'..=b'7').contains(c))
}
/// `parseQuotedFileName`: the unescaped name and the length of the quoted
/// text, or `None` for an unterminated quote or an unknown escape.
fn quoted(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    // WORKAROUND(MIRES-019): compare a copied byte to avoid a promoted Option<&u8>.
    if b.first().copied() != Some(b'"') {
        return None;
    }
    let mut out = String::new();
    let mut j = 1;
    while j < b.len() {
        match b[j] {
            b'"' => return Some((out, j + 1)),
            b'\\' if j + 1 < b.len() => {
                j += 1;
                let escaped = match b[j] {
                    b'a' => '\x07',
                    b'b' => '\x08',
                    b'f' => '\x0c',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'v' => '\x0b',
                    b'\\' => '\\',
                    b'"' => '"',
                    b'0'..=b'7' => {
                        // A run of three-digit octal escapes holds raw bytes,
                        // decoded together as UTF-8 by TextDecoder (which also
                        // consumes a leading BOM). Uint8Array keeps the low byte.
                        let value = |at: usize| {
                            (((b[at] - b'0') as u32) << 6 | ((b[at + 1] - b'0') as u32) << 3
                                | (b[at + 2] - b'0') as u32) as u8
                        };
                        if !octal(b, j + 1) || !octal(b, j + 2) {
                            return None;
                        }
                        let mut bytes = Vec::new();
                        bytes.push(value(j));
                        j += 3;
                        while b.get(j).copied() == Some(b'\\') && octal(b, j + 1) {
                            if !octal(b, j + 2) || !octal(b, j + 3) {
                                return None;
                            }
                            bytes.push(value(j + 1));
                            j += 4;
                        }
                        let decoded = utf8_filename(&bytes);
                        out.push_str(
                            &decoded
                                .strip_prefix('\u{feff}')
                                .unwrap_or(&decoded)
                                .replace('\u{e800}', "\u{e800}\u{e800}"),
                        );
                        continue;
                    }
                    _ => return None,
                };
                out.push(escaped);
                j += 1;
            }
            _ => {
                let start = j;
                j += 1;
                while j < b.len() && b[j] != b'\\' && b[j] != b'"' {
                    j += 1;
                }
                out.push_str(&s[start..j]);
            }
        }
    }
    None
}
/// `unquoteIfQuoted`
fn unquote(s: &str) -> String {
    if s.starts_with('"') {
        if let Some((name, _)) = quoted(s) {
            return name;
        }
    }
    s.to_string()
}
/// `parseGitDiffHeader`: the names after `diff --git `, when they can be
/// split. Both quoted forms skip the one UTF-16 unit assumed to be a space.
fn git_names(rest: &str) -> Option<(String, String)> {
    if rest.starts_with('"') {
        let (old, length) = quoted(rest)?;
        let after = drop_first_unit(&rest[length..]);
        let new = if after.starts_with('"') {
            quoted(&after)?.0
        } else {
            after
        };
        return Some((old, new));
    }
    if let Some(at) = rest.find('"').filter(|&at| at > 0) {
        let (new, _) = quoted(&rest[at..])?;
        return Some((drop_last_unit(&rest[..at]), new));
    }
    if rest.starts_with("a/") {
        // Split on the middle ` b/`, which is right when both names agree.
        let splits: Vec<usize> = rest.match_indices(" b/").map(|(at, _)| at).collect();
        let mid = *splits.get(splits.len() / 2)?;
        return Some((rest[..mid].to_string(), rest[mid + 1..].to_string()));
    }
    None
}
/// One Git extended header line. At most one of parse.js's regexes matches.
fn extended_header(p: &mut Patch, line: &str) {
    if let Some(v) = line.strip_prefix("rename from ") {
        p.set_old_name(String::from("a/") + &unquote(rest_of_line(v)));
        p.set_flag("isRename")
    } else if let Some(v) = line.strip_prefix("rename to ") {
        p.set_new_name(String::from("b/") + &unquote(rest_of_line(v)));
        p.set_flag("isRename")
    } else if let Some(v) = line.strip_prefix("copy from ") {
        p.set_old_name(String::from("a/") + &unquote(rest_of_line(v)));
        p.set_flag("isCopy")
    } else if let Some(v) = line.strip_prefix("copy to ") {
        p.set_new_name(String::from("b/") + &unquote(rest_of_line(v)));
        p.set_flag("isCopy")
    } else if let Some(m) = line.strip_prefix("new file mode ").and_then(mode) {
        p.set_flag("isCreate");
        p.set_new_mode(m)
    } else if let Some(m) = line.strip_prefix("deleted file mode ").and_then(mode) {
        p.set_flag("isDelete");
        p.set_old_mode(m)
    } else if let Some(m) = line.strip_prefix("old mode ").and_then(mode) {
        p.set_old_mode(m)
    } else if let Some(m) = line.strip_prefix("new mode ").and_then(mode) {
        p.set_new_mode(m)
    } else if line.starts_with("Binary files ") {
        p.set_flag("isBinary")
    }
}
/// `parseFileHeader`: a `---` or `+++` line, if there is one.
fn parse_file_header(lines: &[&str], i: &mut usize, p: &mut Patch) {
    let Some(&line) = lines.get(*i) else {
        return;
    };
    if !is_file_header(line) {
        return;
    }
    let mut data = line[3..].trim_matches(ws).split('\t');
    let name = data.next().unwrap_or("");
    let header = data.next().unwrap_or("").trim_matches(ws).to_string();
    let name = if name.starts_with('"') {
        unquote(name)
    } else {
        name.replace("\\\\", "\\")
    };
    if line.starts_with("---") {
        p.set_old_name(name);
        p.record("oldHeader");
        p.old_header = Some(header)
    } else {
        p.set_new_name(name);
        p.record("newHeader");
        p.new_header = Some(header)
    }
    *i += 1
}
fn digits(s: &str) -> Option<(f64, &str)> {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    // A run of ASCII digits always parses; huge runs round to infinity as `+s` does.
    (n > 0).then(|| (s[..n].parse::<f64>().unwrap_or(f64::NAN), &s[n..]))
}
fn optional_count(s: &str) -> (Option<f64>, &str) {
    match s.strip_prefix(',').and_then(digits) {
        Some((n, rest)) => (Some(n), rest),
        None => (None, s),
    }
}
type Range = (f64, Option<f64>, f64, Option<f64>);
/// The captures of `/@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/` at the
/// start of `s`; the leading `@@ -` is already consumed.
fn range_at(s: &str) -> Option<Range> {
    let (old_start, s) = digits(s)?;
    let (old_lines, s) = optional_count(s);
    let (new_start, s) = digits(s.strip_prefix(" +")?)?;
    let (new_lines, s) = optional_count(s);
    s.starts_with(" @@")
        .then_some((old_start, old_lines, new_start, new_lines))
}
/// The leftmost match of the range regex, which `split` finds anywhere in the line.
fn hunk_range(line: &str) -> Option<Range> {
    let mut from = 0;
    while let Some(at) = line[from..].find("@@ -") {
        let start = from + at;
        if let Some(range) = range_at(&line[start + 4..]) {
            return Some(range);
        }
        from = start + 1
    }
    None
}
/// `parseHunk`: an unmatched header gives NaN starts and one line per side.
fn parse_hunk(lines: &[&str], i: &mut usize) -> Result<Hunk, ()> {
    let (old_start, old_lines, new_start, new_lines) =
        hunk_range(lines[*i]).unwrap_or((f64::NAN, None, f64::NAN, None));
    *i += 1;
    let mut h = Hunk {
        old_start,
        old_lines: old_lines.unwrap_or(1.0),
        new_start,
        new_lines: new_lines.unwrap_or(1.0),
        lines: Vec::new(),
    };
    if h.old_lines == 0.0 {
        h.old_start += 1.0
    }
    if h.new_lines == 0.0 {
        h.new_start += 1.0
    }
    let mut adds = 0usize;
    let mut removes = 0usize;
    while let Some(&line) = lines.get(*i) {
        if !((removes as f64) < h.old_lines || (adds as f64) < h.new_lines || line.starts_with('\\'))
        {
            break;
        }
        // An empty line is context, except the last one: `''[0]` is undefined.
        let operation = if line.is_empty() && *i + 1 != lines.len() {
            Some(' ')
        } else {
            line.chars().next()
        };
        match operation {
            Some('+') => adds += 1,
            Some('-') => removes += 1,
            Some(' ') => {
                adds += 1;
                removes += 1
            }
            Some('\\') => (),
            _ => return Err(()),
        }
        h.lines.push(line.to_string());
        *i += 1
    }
    if adds == 0 && h.new_lines == 1.0 {
        h.new_lines = 0.0
    }
    if removes == 0 && h.old_lines == 1.0 {
        h.old_lines = 0.0
    }
    if adds as f64 != h.new_lines || removes as f64 != h.old_lines {
        return Err(());
    }
    if let Some(&next) = lines.get(*i) {
        if next.starts_with(['+', ' ', '-']) && !is_file_header(next) {
            return Err(());
        }
    }
    Ok(h)
}
/// `parseIndex`: one file's metadata, file headers and hunks.
fn parse_index(lines: &[&str], i: &mut usize) -> Result<Patch, ()> {
    let mut p = Patch::default();
    let mut seen_diff_header = false;
    while let Some(&line) = lines.get(*i) {
        if is_file_header(line) || is_hunk_header(line) {
            break;
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // A second diff header belongs to the next file; this one ends here.
            if seen_diff_header {
                return Ok(p);
            }
            seen_diff_header = true;
            p.set_flag("isGit");
            if let Some((old, new)) = git_names(rest) {
                p.set_old_name(old);
                p.set_new_name(new)
            }
            *i += 1;
            while let Some(&ext) = lines.get(*i) {
                if is_file_header(ext) || is_hunk_header(ext) || is_diff_header(ext) {
                    break;
                }
                extended_header(&mut p, ext);
                *i += 1
            }
            continue;
        }
        if let Some(length) = index_header(line) {
            if seen_diff_header {
                return Ok(p);
            }
            seen_diff_header = true;
            p.record("index");
            p.index = Some(line[length..].trim_matches(ws).to_string())
        }
        *i += 1
    }
    parse_file_header(lines, i, &mut p);
    parse_file_header(lines, i, &mut p);
    if p.old_name.is_some() != p.new_name.is_some() {
        return Err(());
    }
    while let Some(&line) = lines.get(*i) {
        if is_diff_header(line) || is_file_header(line) || line.starts_with(UNDERLINE) {
            break;
        }
        if is_hunk_header(line) {
            p.hunks.push(parse_hunk(lines, i)?)
        } else {
            *i += 1
        }
    }
    Ok(p)
}
pub fn parse(s: &str) -> Result<Vec<Patch>, ()> {
    let lines: Vec<&str> = s.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        out.push(parse_index(&lines, &mut i)?)
    }
    Ok(out)
}

// Line-ending conversion of apply.js and line-endings.js.
fn only_win_line_endings(s: &str) -> bool {
    s.contains("\r\n")
        && !s.starts_with('\n')
        && !s.as_bytes().windows(2).any(|w| w[1] == b'\n' && w[0] != b'\r')
}
fn only_unix_line_endings(s: &str) -> bool {
    !s.contains("\r\n") && s.contains('\n')
}
fn before_marker(lines: &[String], i: usize) -> bool {
    lines.get(i + 1).is_some_and(|next| next.starts_with('\\'))
}
fn is_unix(p: &Patch) -> bool {
    !p.hunks.iter().any(|h| {
        h.lines
            .iter()
            .any(|l| !l.starts_with('\\') && l.ends_with('\r'))
    })
}
fn is_win(p: &Patch) -> bool {
    p.hunks
        .iter()
        .any(|h| h.lines.iter().any(|l| l.ends_with('\r')))
        && p.hunks.iter().all(|h| {
            h.lines.iter().enumerate().all(|(i, l)| {
                l.starts_with('\\') || l.ends_with('\r') || before_marker(&h.lines, i)
            })
        })
}
fn unix_to_win(p: &mut Patch) {
    for h in &mut p.hunks {
        for i in 0..h.lines.len() {
            let l = &h.lines[i];
            if !(l.starts_with('\\') || l.ends_with('\r') || before_marker(&h.lines, i)) {
                h.lines[i].push('\r')
            }
        }
    }
}
fn win_to_unix(p: &mut Patch) {
    for h in &mut p.hunks {
        for l in &mut h.lines {
            if l.ends_with('\r') {
                l.pop();
            }
        }
    }
}
/// `distanceIterator`: the positions after `start`, alternating forward and
/// backward by increasing distance, within `[min, max]`. Positions are
/// JavaScript numbers, so a NaN start yields nothing, as in npm.
struct Distance {
    start: f64,
    min: f64,
    max: f64,
    want_forward: bool,
    backward_exhausted: bool,
    forward_exhausted: bool,
    offset: f64,
}
impl Distance {
    fn new(start: f64, min: f64, max: f64) -> Self {
        Distance {
            start,
            min,
            max,
            want_forward: true,
            backward_exhausted: false,
            forward_exhausted: false,
            offset: 1.0,
        }
    }
}
impl Iterator for Distance {
    type Item = f64;
    fn next(&mut self) -> Option<f64> {
        if self.want_forward && !self.forward_exhausted {
            if self.backward_exhausted {
                self.offset += 1.0
            } else {
                self.want_forward = false
            }
            if self.start + self.offset <= self.max {
                return Some(self.start + self.offset);
            }
            self.forward_exhausted = true
        }
        if !self.backward_exhausted {
            if !self.forward_exhausted {
                self.want_forward = true
            }
            if self.min <= self.start - self.offset {
                let position = self.start - self.offset;
                self.offset += 1.0;
                return Some(position);
            }
            self.backward_exhausted = true;
            return self.next();
        }
        None
    }
}
/// `lines[i]`, where a position outside the array (or NaN) reads as undefined.
fn source_line<'a>(lines: &[&'a str], i: f64) -> Option<&'a str> {
    if i >= 0.0 && i < lines.len() as f64 {
        lines.get(i as usize).copied()
    } else {
        None
    }
}
/// The source lines from `from` while below `until`; a position outside the
/// source reads as undefined, which joins as an empty line.
fn copy_lines<'a>(result: &mut Vec<&'a str>, lines: &[&'a str], from: f64, until: f64) {
    let mut i = from;
    while i < until {
        result.push(source_line(lines, i).unwrap_or(""));
        i += 1.0
    }
}
struct Fitted<'a> {
    patched: Vec<&'a str>,
    last_old_line: f64,
}
/// `applyHunk` with no errors allowed: every removed and context line must
/// equal the source line at its position. Trailing context is given back.
fn fit<'a>(hunk: &'a [String], lines: &[&'a str], mut to_pos: f64) -> Option<Fitted<'a>> {
    let mut patched = Vec::new();
    let mut context = 0;
    for l in hunk {
        let (operation, content) = match l.chars().next() {
            Some(c) => (c, &l[c.len_utf8()..]),
            None => (' ', ""),
        };
        match operation {
            '-' => {
                if source_line(lines, to_pos) != Some(content) {
                    return None;
                }
                to_pos += 1.0;
                context = 0
            }
            '+' => {
                patched.push(content);
                context = 0
            }
            ' ' => {
                context += 1;
                let line = source_line(lines, to_pos).filter(|&line| line == content)?;
                patched.push(line);
                to_pos += 1.0
            }
            _ => (),
        }
    }
    patched.truncate(patched.len() - context);
    Some(Fitted {
        patched,
        last_old_line: to_pos - context as f64 - 1.0,
    })
}
pub fn apply(source: &str, mut p: Patch) -> Option<String> {
    if only_win_line_endings(source) && is_unix(&p) {
        unix_to_win(&mut p)
    } else if only_unix_line_endings(source) && is_win(&p) {
        win_to_unix(&mut p)
    }
    let mut lines: Vec<&str> = source.split('\n').collect();
    let Some(last) = p.hunks.last() else {
        return Some(source.to_string());
    };
    let mut remove_eofnl = false;
    let mut add_eofnl = false;
    let mut previous = "";
    for line in &last.lines {
        if line.starts_with('\\') {
            if previous.starts_with('+') {
                remove_eofnl = true
            } else if previous.starts_with('-') {
                add_eofnl = true
            }
        }
        previous = line
    }
    // WORKAROUND(MIRES-019): querying the referenced line avoids a promoted Some(&"").
    let eof = lines.last().is_some_and(|line| line.is_empty());
    if remove_eofnl {
        if add_eofnl {
            if eof {
                return None;
            }
        } else if eof {
            lines.pop();
        } else {
            return None;
        }
    } else if add_eofnl {
        if eof {
            return None;
        }
        lines.push("")
    }
    let mut result: Vec<&str> = Vec::new();
    // Positions are JavaScript numbers: a NaN start fits only a hunk with no
    // removed or context line, and then every later position is NaN too.
    let mut min_line = 0.0;
    let mut offset = 0.0;
    for h in &p.hunks {
        let max_line = lines.len() as f64 - h.old_lines;
        // The declared position first, then the distance iterator around it.
        let mut to_pos = h.old_start + offset - 1.0;
        let mut candidates = Distance::new(to_pos, min_line, max_line);
        let fitted = loop {
            if let Some(fitted) = fit(&h.lines, &lines, to_pos) {
                break fitted;
            }
            to_pos = candidates.next()?
        };
        copy_lines(&mut result, &lines, min_line, to_pos);
        result.extend(fitted.patched);
        min_line = fitted.last_old_line + 1.0;
        offset = to_pos + 1.0 - h.old_start
    }
    copy_lines(&mut result, &lines, min_line, lines.len() as f64);
    Some(result.join("\n"))
}
