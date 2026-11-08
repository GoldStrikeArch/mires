//! `jir-payload`: what an object file's bytes are spent on.
//!
//! For each `.rlib` (every `rcgjs` member) or bare object given: the text section, every section
//! of the binary member, the S3 blob table (blob count, items with a body, dedup ratio), and the
//! inflated size against the deflated one for S1/S3/S5. A format 19 member (the base64 text
//! footer) is reported the way `research/v2/object_payload_size.py` did, so the two formats can be
//! set side by side. `scripts/jir-payload.sh` drives this over the FingerprintJS boundary rlibs.
//!
//! Output is one line per fact, `key<TAB>value`, prefixed by the file and member, so a script can
//! grep a figure out; `--summary` prints one table row per object instead.

use std::collections::BTreeMap;
use std::path::Path;

use rcgjs_object::jir::summary::ItemClass;
use rcgjs_object::object::{self, SectionId};

fn main() {
    let mut summary = false;
    let mut paths = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--summary" => summary = true,
            _ => paths.push(arg),
        }
    }
    if paths.is_empty() {
        eprintln!("usage: jir-payload [--summary] <file.rlib|object>...");
        std::process::exit(2);
    }
    if summary {
        println!(
            "{:<48} {:>7} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>6} {:>6} {:>6}",
            "object",
            "format",
            "bytes",
            "text",
            "S1",
            "S2",
            "S3",
            "S4",
            "S5",
            "S6",
            "items",
            "bodies",
            "blobs"
        );
    }
    let mut failed = false;
    for path in &paths {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("jir-payload: cannot read `{path}`: {err}");
                failed = true;
                continue;
            }
        };
        let label = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        if bytes.starts_with(b"!<arch>\n") {
            match ar_members(&bytes) {
                Ok(members) => {
                    let mut seen = 0usize;
                    for (name, data) in members {
                        if report(&format!("{label}[{name}]"), data, summary) {
                            seen += 1;
                        }
                    }
                    if seen == 0 {
                        eprintln!("jir-payload: `{path}` holds no rcgjs object");
                        failed = true;
                    }
                }
                Err(err) => {
                    eprintln!("jir-payload: `{path}`: {err}");
                    failed = true;
                }
            }
        } else if !report(&label, &bytes, summary) {
            eprintln!("jir-payload: `{path}` is not an rcgjs object");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// Reports one member. Returns whether it was an rcgjs object of either format.
fn report(label: &str, bytes: &[u8], summary: bool) -> bool {
    if object::is_object(bytes) {
        match object::read(bytes) {
            Ok(Some(object)) => {
                report_20(label, bytes, &object, summary);
                true
            }
            Ok(None) => false,
            Err(err) => {
                println!("{label}\terror\t{err}");
                true
            }
        }
    } else {
        report_19(label, bytes, summary)
    }
}

fn report_20(label: &str, bytes: &[u8], object: &object::Object, summary: bool) {
    let text_len = bytes.len() as u64 - member_len(bytes);
    let mut section_len: BTreeMap<u16, u64> = BTreeMap::new();
    for entry in &object.sections {
        section_len.insert(entry.id, entry.len);
    }
    let len = |id: SectionId| section_len.get(&(id as u16)).copied().unwrap_or(0);
    let items = object.summaries.len();
    let with_body = object
        .summaries
        .iter()
        .filter(|item| item.body.is_some())
        .count();
    let blobs = object.blobs.len();
    let bodies_ulen: u64 = object.blobs.iter().map(|blob| blob.ulen).sum();
    let bodies_clen: u64 = object.blobs.iter().map(|blob| blob.clen).sum();
    let origins_ulen: u64 = object
        .summaries
        .iter()
        .filter_map(|item| item.origin)
        .map(|blob| blob.ulen)
        .sum();
    let origins_clen: u64 = object
        .summaries
        .iter()
        .filter_map(|item| item.origin)
        .map(|blob| blob.clen)
        .sum();
    if summary {
        println!(
            "{:<48} {:>7} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>6} {:>6} {:>6}",
            truncate(label, 48),
            object::FORMAT_VERSION,
            bytes.len(),
            text_len,
            len(SectionId::Interns),
            len(SectionId::Summaries),
            len(SectionId::Bodies),
            len(SectionId::Baseline),
            len(SectionId::Origins),
            len(SectionId::Legacy),
            items,
            with_body,
            blobs
        );
        return;
    }
    let p = |key: &str, value: String| println!("{label}\t{key}\t{value}");
    p("format", object::FORMAT_VERSION.to_string());
    p("crate", object.header.crate_name.clone());
    p("jir_schema", object.header.jir_schema.to_string());
    p("emit_args", object.header.emit_args.clone());
    p("bytes", bytes.len().to_string());
    p("text_bytes", text_len.to_string());
    p("member_bytes", member_len(bytes).to_string());
    for id in SectionId::ALL {
        p(
            &format!("section.{}", id.name().replace(' ', "_")),
            len(id).to_string(),
        );
    }
    p("items", items.to_string());
    p("items_with_body", with_body.to_string());
    let available = object
        .summaries
        .iter()
        .filter(|item| item.availability.is_available())
        .count();
    p("items_available", available.to_string());
    p("blobs", blobs.to_string());
    p(
        "blob_dedup_ratio",
        match blobs {
            0 => "n/a".to_owned(),
            _ => format!("{:.3}", with_body as f64 / blobs as f64),
        },
    );
    p("bodies_inflated", bodies_ulen.to_string());
    p("bodies_deflated", bodies_clen.to_string());
    p("origins_inflated", origins_ulen.to_string());
    p("origins_deflated", origins_clen.to_string());
    let interns_plain = postcard::to_allocvec(&object.interns).map_or(0, |bytes| bytes.len());
    p("interns_inflated", interns_plain.to_string());
    p("interns_deflated", len(SectionId::Interns).to_string());
    p("interns_strings", object.interns.strings.len().to_string());
    p("interns_types", object.interns.types.len().to_string());
    p(
        "interns_sigs",
        object.interns.sigs.descriptors.len().to_string(),
    );
    p(
        "interns_source_facts",
        object.interns.source_facts.len().to_string(),
    );
    let mut classes: BTreeMap<&'static str, usize> = BTreeMap::new();
    for item in &object.summaries {
        for name in item.class.names() {
            *classes.entry(name).or_default() += 1;
        }
        if item.class == ItemClass::empty() && item.body.is_some() {
            *classes.entry("(none)").or_default() += 1;
        }
    }
    for (name, count) in classes {
        p(&format!("class.{name}"), count.to_string());
    }
    let mut declines: BTreeMap<String, usize> = BTreeMap::new();
    for item in &object.summaries {
        if let rcgjs_object::jir::summary::Availability::Unavailable(decline) = &item.availability {
            *declines.entry(decline.code().to_owned()).or_default() += 1;
        }
    }
    for (code, count) in declines {
        p(&format!("decline.{code}"), count.to_string());
    }
}

/// The length of the member plus trailer: everything after the text section.
fn member_len(bytes: &[u8]) -> u64 {
    let trailer = &bytes[bytes.len() - 16..];
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&trailer[8..16]);
    bytes.len() as u64 - u64::from_le_bytes(raw)
}

fn report_19(label: &str, bytes: &[u8], summary: bool) -> bool {
    const FOOTER: &[u8] = b"//# rcgjs: ";
    let Some(at) = rfind(bytes, FOOTER) else {
        return false;
    };
    if at != 0 && bytes[at - 1] != b'\n' {
        return false;
    }
    let text = &bytes[..at];
    let encoded = &bytes[at + FOOTER.len()..];
    let encoded_len = encoded
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |end| end + 1);
    // base64 without the padding accounted exactly: 3 bytes per 4 characters, minus padding.
    let padding = encoded[..encoded_len]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    let payload = encoded_len / 4 * 3 - padding;
    if summary {
        println!(
            "{:<48} {:>7} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>6} {:>6} {:>6}",
            truncate(label, 48),
            19,
            bytes.len(),
            text.len(),
            "-",
            "-",
            "-",
            payload,
            "-",
            "-",
            "-",
            "-",
            "-"
        );
        return true;
    }
    println!("{label}\tformat\t19");
    println!("{label}\tbytes\t{}", bytes.len());
    println!("{label}\ttext_bytes\t{}", text.len());
    println!("{label}\tbase64_bytes\t{encoded_len}");
    println!("{label}\tpayload_bytes\t{payload}");
    true
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

fn truncate(label: &str, width: usize) -> String {
    if label.len() <= width {
        return label.to_owned();
    }
    let keep = width - 1;
    let start = label.len() - keep;
    format!("…{}", &label[start..])
}

/// The members of a GNU/BSD `ar` archive: `(name, data)`.
///
/// Hand-rolled because rustc names our member through the long-name table (`/0`), which common
/// `ar` front ends render unhelpfully, and because this tool must not depend on rustc's crates.
fn ar_members(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    let mut members = Vec::new();
    let mut names: Option<&[u8]> = None;
    let mut at = 8;
    while at + 60 <= bytes.len() {
        let header = &bytes[at..at + 60];
        at += 60;
        let raw_name = std::str::from_utf8(&header[..16])
            .map_err(|_| "member name is not ASCII")?
            .trim_end()
            .to_owned();
        let size: usize = std::str::from_utf8(&header[48..58])
            .map_err(|_| "member size is not ASCII")?
            .trim()
            .parse()
            .map_err(|_| "member size is not a number")?;
        let end = at
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or("member runs past the archive")?;
        let data = &bytes[at..end];
        at = end + (size & 1);
        if raw_name == "//" {
            names = Some(data);
            continue;
        }
        if raw_name == "/" || raw_name == "/SYM64/" {
            continue;
        }
        let name = match raw_name.strip_prefix('/') {
            Some(index) if !index.is_empty() => {
                let index: usize = index.parse().map_err(|_| "bad long-name index")?;
                let table = names.ok_or("long name without a name table")?;
                let entry = &table[index.min(table.len())..];
                let end = entry
                    .iter()
                    .position(|byte| *byte == b'/' || *byte == b'\n')
                    .unwrap_or(entry.len());
                String::from_utf8_lossy(&entry[..end]).into_owned()
            }
            _ => match raw_name.strip_prefix("#1/") {
                // BSD: the name is the first N bytes of the data.
                Some(len) => {
                    let len: usize = len.parse().map_err(|_| "bad BSD name length")?;
                    let (name, rest) = data.split_at(len.min(data.len()));
                    let name = String::from_utf8_lossy(name)
                        .trim_end_matches('\0')
                        .to_owned();
                    members.push((name, rest));
                    continue;
                }
                None => raw_name.trim_end_matches('/').to_owned(),
            },
        };
        members.push((name, data));
    }
    Ok(members)
}
