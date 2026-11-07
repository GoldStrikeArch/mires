//! Source maps, version 3: the `.map` file that lets a browser step through the Rust.
//!
//! The map is assembled at link time, from the per item entries `item.rs` stores in each object
//! file. That is the only place it can be assembled: an item's line number in the finished program
//! is not known until the reachability walk has decided what is in it, and an item that came out of
//! an rlib was printed in a different compilation altogether. Rebasing is a single addition —
//! every item's JavaScript starts at the beginning of a line — so the entries an object carries are
//! line-relative and the columns are already final.
//!
//! # What the map says
//!
//! * `sources` are file names as rustc spells them in a diagnostic, so `--remap-path-prefix`
//!   applies. `sourcesContent` carries the text of every one this compilation still has in its
//!   source map — which is this crate's own files; a file that came from a dependency's rlib is
//!   `null`, and the browser is left to find it.
//! * `names` is the Rust name behind an emitted one: the function's def path, and every local whose
//!   spelling had to change (`self$1` for the second inlined `self`). Chrome reads the name off the
//!   mapping covering the identifier, which is why `jsast.rs` opens a fresh mapping right after one.
//! * every generated line gets a mapping at column 0. Firefox stops a mapping at the end of its
//!   line, so a run of lines from one Rust statement needs the mapping repeated (js_of_ocaml does
//!   the same, `js_output.ml`); and a line with nothing to say maps to
//!   [`GENERATED`], a source that exists only to be named in `ignoreList`. That is the blackboxing
//!   trick: the prologue, the dispatch scaffolding and the stubs are code no Rust programmer wrote,
//!   and a debugger that is told so steps over them instead of into them.

use std::collections::BTreeMap;

use rustc_session::Session;

use crate::item::LinkItem;

/// The stand-in source for generated glue: mapped so that no line is unattributed, and named in
/// `ignoreList` so that a debugger steps over it.
const GENERATED: &str = "<rustc_codegen_mires generated>";

/// One resolved entry, in the order the map wants them.
#[derive(Clone, Copy)]
struct Segment {
    line: u32,
    col: u32,
    source: u32,
    src_line: u32,
    src_col: u32,
    name: i32,
}

/// Builds the JSON of the map for a finished program.
///
/// `starts` is the line each item begins on, as [`crate::item::write_program`] reported it, and
/// `file` is the name of the JavaScript the map describes.
pub(crate) fn build(
    sess: &Session,
    program: &str,
    items: &[&LinkItem],
    starts: &[u32],
    file: &str,
) -> String {
    let mut sources: Vec<String> = vec![GENERATED.to_owned()];
    let mut names: Vec<String> = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();

    for (item, start) in items.iter().zip(starts) {
        let map = &item.map;
        // Both tables are per item and tiny; the global index of each entry is looked up once.
        let source_index: Vec<u32> =
            map.sources.iter().map(|source| intern(&mut sources, source)).collect();
        let name_index: Vec<u32> = map.names.iter().map(|name| intern(&mut names, name)).collect();

        for &(line, col, source, src_line, src_col, name) in &map.segments {
            // An entry pointing outside its own item's tables is a corrupt object; skipping it
            // costs a stepping stop, where trusting it would put the whole map out of step.
            let Some(&source) = source_index.get(source as usize) else { continue };
            let name = match usize::try_from(name) {
                Ok(index) => match name_index.get(index) {
                    Some(&index) => index as i32,
                    None => continue,
                },
                Err(_) => -1,
            };
            segments.push(Segment {
                line: line + start,
                col,
                source,
                // A `Loc`'s line is 1 based, as a diagnostic prints it; a map's is 0 based.
                src_line: src_line.saturating_sub(1),
                src_col,
                name,
            });
        }
    }

    // The entries of one item are in printing order and the items are in program order, so this
    // sort has nothing to do; it is what makes that an observation rather than an assumption.
    segments.sort_by_key(|segment| (segment.line, segment.col));

    let mappings = encode(program, &segments, starts);
    let contents = source_contents(sess, &sources);

    let mut json = String::with_capacity(mappings.len() + 4096);
    json.push_str("{\"version\":3,\"file\":");
    write_json_string(&mut json, file);
    json.push_str(",\"sources\":[");
    for (i, source) in sources.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        write_json_string(&mut json, source);
    }
    json.push_str("],\"sourcesContent\":[");
    for (i, content) in contents.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        match content {
            Some(text) => write_json_string(&mut json, text),
            None => json.push_str("null"),
        }
    }
    json.push_str("],\"names\":[");
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        write_json_string(&mut json, name);
    }
    // The generated source is always index 0, and always the one to step over.
    json.push_str("],\"ignoreList\":[0],\"mappings\":");
    write_json_string(&mut json, &mappings);
    json.push_str("}\n");
    json
}

/// The index of `value` in `list`, appending it if it is not there yet.
fn intern(list: &mut Vec<String>, value: &str) -> u32 {
    match list.iter().position(|entry| entry == value) {
        Some(index) => index as u32,
        None => {
            list.push(value.to_owned());
            (list.len() - 1) as u32
        }
    }
}

/// The text of each source, for the ones this compilation still holds.
///
/// The lookup is by the name the entry was recorded under, which is the name rustc would print in a
/// diagnostic — the same spelling on both sides, remapping included. Embedding the text is what
/// makes stepping work from a `file://` page and from a build directory that is nowhere near the
/// sources; a source that is not here (one of `core`'s, read back out of an rlib) is `null` and the
/// browser resolves it against the map's own URL.
fn source_contents(sess: &Session, sources: &[String]) -> Vec<Option<String>> {
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();
    for file in sess.source_map().files().iter() {
        let name = file.name.prefer_remapped_unconditionally().to_string();
        if let Some(text) = &file.src {
            by_name.insert(name, text.to_string());
        }
    }
    sources
        .iter()
        .map(|source| {
            if source == GENERATED {
                Some("// generated by rustc_codegen_mires\n".to_owned())
            } else {
                by_name.get(source).cloned()
            }
        })
        .collect()
}

/// VLQ-encodes the segments, giving every generated line a mapping at column 0.
fn encode(program: &str, segments: &[Segment], starts: &[u32]) -> String {
    let total = line_count(program);
    let boundaries: std::collections::HashSet<u32> = starts.iter().copied().collect();

    let generated = Segment { line: 0, col: 0, source: 0, src_line: 0, src_col: 0, name: -1 };

    let mut out = String::new();
    let mut next = 0usize;
    // The deltas of every field but the column carry across lines.
    let mut prev_source = 0i64;
    let mut prev_src_line = 0i64;
    let mut prev_src_col = 0i64;
    let mut prev_name = 0i64;
    // The mapping in effect at the end of the line before: what a line with nothing of its own
    // repeats, which is the Firefox workaround and the blackboxing rule in one.
    let mut carry = generated;

    for line in 0..total {
        if line > 0 {
            out.push(';');
        }
        // An item's first line starts afresh: the blank line and the header comment above it are
        // not a continuation of the item before, they are glue.
        if boundaries.contains(&line) {
            carry = generated;
        }

        let first = next;
        while next < segments.len() && segments[next].line == line {
            next += 1;
        }
        let on_line = &segments[first..next];

        let mut prev_col = 0i64;
        let mut wrote = false;
        // A line with nothing of its own repeats what was in effect, at column 0. A line that has
        // a mapping needs no filler: everything before its first column already resolves to the
        // mapping before it, and the indentation is not somewhere a debugger stops.
        if on_line.is_empty() {
            let filler = Segment { line, col: 0, ..carry };
            write_segment(
                &mut out,
                &filler,
                &mut prev_col,
                &mut prev_source,
                &mut prev_src_line,
                &mut prev_src_col,
                &mut prev_name,
            );
            wrote = true;
        }
        for segment in on_line {
            if wrote {
                out.push(',');
            }
            write_segment(
                &mut out,
                segment,
                &mut prev_col,
                &mut prev_source,
                &mut prev_src_line,
                &mut prev_src_col,
                &mut prev_name,
            );
            wrote = true;
        }
        if let Some(last) = on_line.last() {
            carry = *last;
        }
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn write_segment(
    out: &mut String,
    segment: &Segment,
    prev_col: &mut i64,
    prev_source: &mut i64,
    prev_src_line: &mut i64,
    prev_src_col: &mut i64,
    prev_name: &mut i64,
) {
    let col = segment.col as i64;
    let source = segment.source as i64;
    let src_line = segment.src_line as i64;
    let src_col = segment.src_col as i64;
    vlq(out, col - *prev_col);
    vlq(out, source - *prev_source);
    vlq(out, src_line - *prev_src_line);
    vlq(out, src_col - *prev_src_col);
    *prev_col = col;
    *prev_source = source;
    *prev_src_line = src_line;
    *prev_src_col = src_col;
    if segment.name >= 0 {
        let name = segment.name as i64;
        vlq(out, name - *prev_name);
        *prev_name = name;
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Appends a base 64 VLQ: the sign in the low bit, six bits per digit, the top one a continuation.
fn vlq(out: &mut String, value: i64) {
    let mut bits = if value < 0 { ((-value) as u64) << 1 | 1 } else { (value as u64) << 1 };
    loop {
        let mut digit = (bits & 0x1f) as usize;
        bits >>= 5;
        if bits > 0 {
            digit |= 0x20;
        }
        out.push(BASE64[digit] as char);
        if bits == 0 {
            return;
        }
    }
}

/// How many lines the program has, counting a trailing partial one.
fn line_count(program: &str) -> u32 {
    let newlines = program.matches('\n').count() as u32;
    if program.is_empty() || program.ends_with('\n') { newlines } else { newlines + 1 }
}

/// Writes a JSON string literal. Everything below a space is escaped, as JSON requires.
fn write_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_vlq(text: &str) -> Vec<i64> {
        let mut out = Vec::new();
        let mut value = 0u64;
        let mut shift = 0;
        for ch in text.chars() {
            let digit = BASE64.iter().position(|c| *c as char == ch).expect("base 64") as u64;
            value |= (digit & 0x1f) << shift;
            shift += 5;
            if digit & 0x20 == 0 {
                let signed =
                    if value & 1 == 1 { -((value >> 1) as i64) } else { (value >> 1) as i64 };
                out.push(signed);
                value = 0;
                shift = 0;
            }
        }
        out
    }

    #[test]
    fn vlq_round_trips_the_values_a_map_holds() {
        for value in [0i64, 1, -1, 15, 16, -16, 17, 511, -511, 1_000_000, -1_000_000] {
            let mut text = String::new();
            vlq(&mut text, value);
            assert_eq!(decode_vlq(&text), vec![value], "for {value}");
        }
        // The spelling the format is usually shown with.
        let mut text = String::new();
        vlq(&mut text, 0);
        assert_eq!(text, "A");
        text.clear();
        vlq(&mut text, 1);
        assert_eq!(text, "C");
        text.clear();
        vlq(&mut text, -1);
        assert_eq!(text, "D");
        text.clear();
        vlq(&mut text, 16);
        assert_eq!(text, "gB");
    }

    #[test]
    fn every_generated_line_gets_a_mapping() {
        let program = "a\nb\nc\n";
        let segments = vec![Segment {
            line: 1,
            col: 0,
            source: 1,
            src_line: 4,
            src_col: 2,
            name: -1,
        }];
        let mappings = encode(program, &segments, &[]);
        let lines: Vec<&str> = mappings.split(';').collect();
        assert_eq!(lines.len(), 3, "{mappings}");
        // Line 0 has nothing of its own, so it is the generated source at 0:0.
        assert_eq!(decode_vlq(lines[0]), vec![0, 0, 0, 0]);
        // Line 1 is the real entry, and line 2 repeats it: Firefox stops a mapping at end of line.
        assert_eq!(decode_vlq(lines[1]), vec![0, 1, 4, 2]);
        assert_eq!(decode_vlq(lines[2]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn an_item_boundary_resets_to_the_generated_source() {
        let program = "a\nb\nc\nd\n";
        let segments = vec![Segment {
            line: 0,
            col: 0,
            source: 1,
            src_line: 9,
            src_col: 0,
            name: -1,
        }];
        // The item starting at line 2 does not inherit the line above it.
        let mappings = encode(program, &segments, &[0, 2]);
        let lines: Vec<&str> = mappings.split(';').collect();
        assert_eq!(decode_vlq(lines[0]), vec![0, 1, 9, 0]);
        // Still inside the first item: the mapping repeats.
        assert_eq!(decode_vlq(lines[1]), vec![0, 0, 0, 0]);
        // The boundary: back to the generated source, which is index 0 and line 0.
        assert_eq!(decode_vlq(lines[2]), vec![0, -1, -9, 0]);
    }

    #[test]
    fn json_strings_escape_what_json_requires() {
        let mut out = String::new();
        write_json_string(&mut out, "a\"b\\c\nd\u{1}e");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001e\"");
    }

    #[test]
    fn lines_are_counted_with_a_trailing_partial_one() {
        assert_eq!(line_count(""), 0);
        assert_eq!(line_count("a\n"), 1);
        assert_eq!(line_count("a\nb"), 2);
        assert_eq!(line_count("a\nb\n"), 2);
    }
}
