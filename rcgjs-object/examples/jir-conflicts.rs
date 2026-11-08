//! `jir-conflicts`: what duplicate definitions of one item actually disagree about.
//!
//! One monomorphic item can be codegenned by several crates and several codegen units. v1 keyed
//! everything on the emitted JavaScript name, compared the printed text, and — when the late-IR
//! bodies behind two identical texts were not byte-equal — declined the item's availability with
//! `DUPLICATE_CONFLICT`. On the FingerprintJS idiomatic port that fired 74 times.
//!
//! This tool reproduces the comparison over v2 objects and says what each disagreement *is*:
//!
//! * `identical` — the two bodies are the same postcard bytes. Nothing to decide.
//! * `normalized-identical` — the bytes differ and the normal form ([`NormalizedBody`]) does not:
//!   storage markers, nops, borrowck edges, block numbering, dead blocks, dead locals, or intern
//!   ids. v1 declined these; v2 merges them.
//! * `different` — the normal forms differ. The tool names the first field that does and guesses
//!   the cause: `inlined` when one side's origins carry an inline stack and the other's do not (MIR
//!   inlining ran on one of the two crates), `shape` when the CFGs are different sizes, `content`
//!   otherwise.
//! * `name-collision` — one JavaScript name over two different Rust items. The only case that is
//!   still a hard link error.
//!
//! ```text
//! cargo run -p rcgjs-object --example jir-conflicts -- [options] <file.rlib|object>...
//!
//!   --limit N        print at most N groups per verdict (default 20; 0 for all)
//!   --expect <file>  a newline-separated list of link names to report on individually
//!   --quiet          the histogram only
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use rcgjs_object::jir::conflict::{self, Cause};
use rcgjs_object::jir::origin::OriginTable;
use rcgjs_object::jir::semantic::Body;
use rcgjs_object::jir::summary::{merge_headers, ItemSummary, NormalizedBody};
use rcgjs_object::jir::Hash128;
use rcgjs_object::object::{self, Object};

fn main() {
    let mut limit = 20usize;
    let mut quiet = false;
    let mut expect: Option<String> = None;
    let mut paths: Vec<String> = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--limit" => {
                limit = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(20)
            }
            "--expect" => expect = arguments.next(),
            "--quiet" => quiet = true,
            "--help" | "-h" => {
                eprintln!(
                    "usage: jir-conflicts [--limit N] [--expect <file>] [--quiet] \
                     <file.rlib|object>..."
                );
                return;
            }
            _ => paths.push(argument),
        }
    }
    if paths.is_empty() {
        eprintln!(
            "usage: jir-conflicts [--limit N] [--expect <file>] [--quiet] <file.rlib|object>..."
        );
        std::process::exit(2);
    }

    let mut objects: Vec<(String, Object)> = Vec::new();
    for path in &paths {
        match std::fs::read(path) {
            Ok(bytes) => load(path, &bytes, &mut objects),
            Err(err) => {
                eprintln!("jir-conflicts: cannot read `{path}`: {err}");
                std::process::exit(1);
            }
        }
    }
    if objects.is_empty() {
        eprintln!("jir-conflicts: none of the given files hold an rcgjs object");
        std::process::exit(1);
    }
    println!("objects\t{}", objects.len());
    let items: usize = objects
        .iter()
        .map(|(_, object)| object.summaries.len())
        .sum();
    println!("items\t{items}");

    // Every object in one program has to agree on the header. A disagreement is the real cause of
    // most "two different definitions" errors, so it is checked first and reported as itself.
    let (base_label, base) = &objects[0];
    let mut header_mismatch = 0usize;
    for (label, object) in objects.iter().skip(1) {
        if let Err(decline) = merge_headers(&base.header, &object.header) {
            header_mismatch += 1;
            println!("header-mismatch\t{base_label}\t{label}\t{decline}");
        }
    }
    println!("header-mismatch\t{header_mismatch}");

    let groups = group(&objects);
    let mut verdicts: BTreeMap<&'static str, Vec<Group>> = BTreeMap::new();
    let mut drift: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut duplicate_items = 0usize;
    for group in groups {
        if group.entries.len() < 2 {
            continue;
        }
        duplicate_items += 1;
        let group = judge(&objects, group);
        if group.verdict == "different" {
            *drift.entry(group.cause.as_str()).or_default() += 1;
        }
        verdicts.entry(group.verdict).or_default().push(group);
    }

    println!("duplicate-names\t{duplicate_items}");
    for verdict in [
        "identical",
        "normalized-identical",
        "different",
        "name-collision",
        "no-body",
    ] {
        let count = verdicts.get(verdict).map_or(0, Vec::len);
        println!("verdict.{verdict}\t{count}");
    }
    for (cause, count) in &drift {
        println!("different.{cause}\t{count}");
    }

    if !quiet {
        for verdict in ["name-collision", "different", "normalized-identical"] {
            let Some(rows) = verdicts.get(verdict) else {
                continue;
            };
            println!("\n== {verdict} ({}) ==", rows.len());
            for (index, row) in rows.iter().enumerate() {
                if limit != 0 && index >= limit {
                    println!("  … {} more", rows.len() - limit);
                    break;
                }
                print_group(&objects, row);
            }
        }
    }

    if let Some(path) = expect {
        let mut present: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, object) in &objects {
            for summary in &object.summaries {
                *present.entry(summary.key.link_name.as_str()).or_default() += 1;
            }
        }
        report_expected(&path, &verdicts, &present);
    }
}

/// Reads one file into `objects`: an `ar` archive contributes each of its rcgjs members.
fn load(path: &str, bytes: &[u8], objects: &mut Vec<(String, Object)>) {
    let label = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    if bytes.starts_with(b"!<arch>\n") {
        match ar_members(bytes) {
            Ok(members) => {
                for (name, data) in members {
                    read_into(&format!("{label}[{name}]"), data, objects);
                }
            }
            Err(err) => eprintln!("jir-conflicts: `{path}`: {err}"),
        }
        return;
    }
    read_into(&label, bytes, objects);
}

fn read_into(label: &str, bytes: &[u8], objects: &mut Vec<(String, Object)>) {
    if !object::is_object(bytes) {
        return;
    }
    match object::read(bytes) {
        Ok(Some(object)) => objects.push((label.to_owned(), object)),
        Ok(None) => {}
        Err(err) => eprintln!("jir-conflicts: `{label}`: {err}"),
    }
}

/// One item in one object.
#[derive(Clone, Copy)]
struct Entry {
    object: usize,
    item: usize,
}

struct Group {
    name: String,
    entries: Vec<Entry>,
    verdict: &'static str,
    /// For `different`: `inlined`, `shape`, or `content`.
    cause: Cause,
    /// The first field the two representative normal forms disagree about.
    difference: Option<String>,
    blob_hashes: Vec<Hash128>,
    normalized_hashes: Vec<Hash128>,
    paths: Vec<String>,
}

/// Every item, by JavaScript name, in object then item order.
fn group(objects: &[(String, Object)]) -> Vec<Group> {
    let mut table: BTreeMap<&str, Vec<Entry>> = BTreeMap::new();
    for (object_index, (_, object)) in objects.iter().enumerate() {
        for (item_index, summary) in object.summaries.iter().enumerate() {
            table
                .entry(summary.key.link_name.as_str())
                .or_default()
                .push(Entry {
                    object: object_index,
                    item: item_index,
                });
        }
    }
    table
        .into_iter()
        .map(|(name, entries)| Group {
            name: name.to_owned(),
            entries,
            verdict: "identical",
            cause: Cause::None,
            difference: None,
            blob_hashes: Vec::new(),
            normalized_hashes: Vec::new(),
            paths: Vec::new(),
        })
        .collect()
}

fn summary_of(objects: &[(String, Object)], entry: Entry) -> &ItemSummary {
    &objects[entry.object].1.summaries[entry.item]
}

/// Decides what a group of same-named items is, decoding only what it has to.
fn judge(objects: &[(String, Object)], mut group: Group) -> Group {
    let summaries: Vec<&ItemSummary> = group
        .entries
        .iter()
        .map(|entry| summary_of(objects, *entry))
        .collect();

    // One name over two Rust items: still a hard error, and the only one left.
    let mut keys: Vec<_> = summaries.iter().map(|summary| summary.key.item).collect();
    keys.sort();
    keys.dedup();
    if keys.len() > 1 {
        group.verdict = "name-collision";
        group.paths = group
            .entries
            .iter()
            .map(|entry| item_path(objects, *entry))
            .collect();
        return group;
    }

    group.blob_hashes = summaries
        .iter()
        .filter_map(|summary| summary.body_hash)
        .collect();
    if group.blob_hashes.len() < 2 {
        group.verdict = "no-body";
        return group;
    }
    let mut distinct = group.blob_hashes.clone();
    distinct.sort();
    distinct.dedup();
    if distinct.len() == 1 {
        group.verdict = "identical";
        return group;
    }

    // The bytes differ. Decode and normalize; only then is there something to say.
    let mut bodies: Vec<(Entry, Body, OriginTable)> = Vec::new();
    for entry in &group.entries {
        let (_, object) = &objects[entry.object];
        let summary = &object.summaries[entry.item];
        match object.decode_verified(summary) {
            Ok(Some((body, origins))) => bodies.push((*entry, body, origins)),
            Ok(None) => {}
            Err(err) => {
                group.verdict = "different";
                group.cause = Cause::Undecodable;
                group.difference = Some(format!("body does not decode: {err}"));
                return group;
            }
        }
    }
    let normalized: Vec<NormalizedBody> = bodies
        .iter()
        .map(|(entry, body, _)| NormalizedBody::of(body, &objects[entry.object].1.interns))
        .collect();
    group.normalized_hashes = normalized.iter().map(NormalizedBody::hash).collect();
    group.paths = group
        .entries
        .iter()
        .map(|entry| item_path(objects, *entry))
        .collect();

    let mut distinct = group.normalized_hashes.clone();
    distinct.sort();
    distinct.dedup();
    if distinct.len() == 1 {
        group.verdict = "normalized-identical";
        // The writer must have reached the same conclusion, or the index lies about the body.
        for (index, entry) in group.entries.iter().enumerate() {
            let stored = summary_of(objects, *entry).semantic_hash;
            if stored.is_some() && stored != Some(group.normalized_hashes[index]) {
                group.difference = Some(format!(
                    "summary says {} and the body normalizes to {}",
                    stored.unwrap_or_default(),
                    group.normalized_hashes[index]
                ));
            }
        }
        return group;
    }

    // Two representatives: the one the merge rule would keep, and the first that differs from it.
    let keep = group
        .normalized_hashes
        .iter()
        .enumerate()
        .min_by_key(|(_, hash)| **hash)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let other = group
        .normalized_hashes
        .iter()
        .position(|hash| *hash != group.normalized_hashes[keep])
        .unwrap_or(0);
    group.verdict = "different";
    group.difference = Some(
        conflict::first_difference(&normalized[keep], &normalized[other])
            .unwrap_or_else(|| "nothing (the walk found no difference)".to_owned()),
    );
    group.cause = conflict::cause(
        &normalized[keep],
        &normalized[other],
        Some((&bodies[keep].2, &bodies[other].2)),
    );
    group
}

/// The item's path as its origin table records it, prefixed by the object it came from.
fn item_path(objects: &[(String, Object)], entry: Entry) -> String {
    let (label, object) = &objects[entry.object];
    let summary = &object.summaries[entry.item];
    let path = object
        .origins(summary)
        .ok()
        .flatten()
        .and_then(|origins| object.interns.str(origins.item_path).map(str::to_owned))
        .unwrap_or_else(|| "?".to_owned());
    format!("{label} :: {path}")
}

fn print_group(objects: &[(String, Object)], group: &Group) {
    println!("  {}", group.name);
    for (index, entry) in group.entries.iter().enumerate() {
        let summary = summary_of(objects, *entry);
        let blob = summary
            .body_hash
            .map(|hash| hash.to_string())
            .unwrap_or_else(|| "-".to_owned());
        let normalized = group
            .normalized_hashes
            .get(index)
            .map(Hash128::to_string)
            .unwrap_or_else(|| "-".to_owned());
        let path = group
            .paths
            .get(index)
            .cloned()
            .unwrap_or_else(|| "?".to_owned());
        println!("    blob {blob}  normalized {normalized}  {path}");
    }
    if let Some(difference) = &group.difference {
        println!("    cause {}: {difference}", group.cause.as_str());
    }
}

/// Reports on a list of names the caller already believes are conflicts: v1's report names them,
/// and this says what v2 makes of each.
fn report_expected(
    path: &str,
    verdicts: &BTreeMap<&'static str, Vec<Group>>,
    present: &BTreeMap<&str, usize>,
) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("jir-conflicts: cannot read `{path}`: {err}");
            return;
        }
    };
    let mut where_is: BTreeMap<&str, (&'static str, Cause)> = BTreeMap::new();
    for (verdict, groups) in verdicts {
        for group in groups {
            where_is.insert(group.name.as_str(), (verdict, group.cause));
        }
    }
    let names: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    println!("\n== expected ({}) ==", names.len());
    for name in &names {
        // A name that is not a duplicate group is either in the program exactly once — so v1's
        // conflict cannot have come from a cross-object duplicate — or not in it at all.
        let (verdict, cause) = where_is.get(name).copied().unwrap_or_else(|| {
            match present.get(name).copied().unwrap_or(0) {
                0 => ("absent", Cause::None),
                _ => ("single", Cause::None),
            }
        });
        *counts.entry(verdict).or_default() += 1;
        println!("  {name}\t{verdict}\t{}", cause.as_str());
    }
    for (verdict, count) in counts {
        println!("expected.{verdict}\t{count}");
    }
}

/// The members of a GNU/BSD `ar` archive: `(name, data)`. The same hand-rolled reader
/// `jir-payload` uses, and for the same reason: rustc names our member through the long-name
/// table, and a tool in this crate must not depend on rustc's crates.
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
