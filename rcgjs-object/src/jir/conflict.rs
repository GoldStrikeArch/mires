//! The duplicate autopsy: what two bodies that claim one item actually disagree about.
//!
//! [`super::summary::merge_duplicates`] and [`super::summary::merge_aliases`] answer *which* body
//! survives. This module answers *why they differed*, which is what a person needs when the answer
//! is surprising. It is deliberately separate from the rule: the rule runs on every link and must
//! be cheap, and this runs only for the pairs the rule had to choose between.
//!
//! Both consumers share it. `jsc-link` records a [`ConflictRecord`] per choice and writes them out
//! under `-Cllvm-args=js-jir-dump-conflicts=<path>`; `examples/jir-conflicts.rs` does the same
//! walk over objects read straight off disk, which is how the numbers in `JIR.md` were produced.
//!
//! The comparison is between two **normal forms** ([`NormalizedBody`]), not two encoded bodies, so
//! everything the identity rule already forgives — storage markers, block numbering, dead blocks,
//! intern ids — is gone before the first field is compared and cannot be reported as a difference.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use super::hash::Hash128;
use super::interns::StrId;
use super::origin::OriginTable;
use super::semantic::{BasicBlock, BodyKind, Statement, Terminator};
use super::summary::NormalizedBody;
use crate::lane_report::push_string;

/// The schema of the document [`write()`] produces.
pub const CONFLICT_REPORT_SCHEMA: u32 = 1;

/// How the conflicting items were grouped together.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ConflictRelation {
    /// One Rust item read from more than one object file.
    Duplicate,
    /// Two items whose printed JavaScript the text deduplicator merged.
    Alias,
}

impl ConflictRelation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Duplicate => "duplicate",
            Self::Alias => "alias",
        }
    }
}

/// As far as the objects can say, why two normal forms differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffCause {
    /// One side's origins carry an inline stack and the other's do not: MIR inlining ran on one of
    /// the two crates.
    Inlined,
    /// The CFGs are different sizes.
    Shape,
    /// Same shape, different content — most often different type arguments.
    Content,
    /// A body would not decode, so nothing could be compared.
    Undecodable,
}

impl DiffCause {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inlined => "inlined",
            Self::Shape => "shape",
            Self::Content => "content",
            Self::Undecodable => "undecodable",
        }
    }
}

/// What a group of same-named (or same-text) items turned out to be.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// The encoded bodies are the same bytes. Nothing to decide.
    Identical,
    /// The bytes differ and the normal forms do not: exactly what the normal form exists for.
    NormalizedIdentical,
    /// The normal forms differ; `difference` names the first disagreeing field.
    Different {
        cause: DiffCause,
        difference: String,
    },
    /// One JavaScript name over two Rust items. A hard link error, not a choice.
    NameCollision,
    /// Fewer than two of the members carry a body.
    NoBody,
}

impl Verdict {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Identical => "identical",
            Self::NormalizedIdentical => "normalized-identical",
            Self::Different { .. } => "different",
            Self::NameCollision => "name-collision",
            Self::NoBody => "no-body",
        }
    }

    pub const fn cause_str(&self) -> &'static str {
        match self {
            Self::Different { cause, .. } => cause.as_str(),
            _ => "-",
        }
    }

    pub fn difference(&self) -> Option<&str> {
        match self {
            Self::Different { difference, .. } => Some(difference.as_str()),
            _ => None,
        }
    }

    /// Every verdict label, in the order a histogram prints them.
    pub const ALL_LABELS: [&'static str; 5] = [
        "identical",
        "normalized-identical",
        "different",
        "name-collision",
        "no-body",
    ];
}

/// One member of a conflicting group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictEntry {
    /// Where the item came from: an object label, an rlib member, a crate name.
    pub origin: String,
    /// The item's path as its origin table records it, when there is one.
    pub path: String,
    /// The content hash of the encoded body (the S3 blob address).
    pub blob: Option<Hash128>,
    /// The hash of the body's normal form.
    pub normalized: Option<Hash128>,
}

/// One group of items that claimed the same identity, and what came of it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRecord {
    /// The JavaScript name the group is keyed on.
    pub name: String,
    pub relation: ConflictRelation,
    pub verdict: Verdict,
    pub entries: Vec<ConflictEntry>,
}

impl ConflictRecord {
    /// The record for a group whose two representatives are already normalized.
    ///
    /// `origins` are used only to tell inlining apart from the rest; pass `None` when they are not
    /// decoded, and the cause falls back to shape and content.
    pub fn of(
        name: &str,
        relation: ConflictRelation,
        left: &NormalizedBody,
        right: &NormalizedBody,
        origins: Option<(&OriginTable, &OriginTable)>,
    ) -> ConflictRecord {
        ConflictRecord {
            name: name.to_owned(),
            relation,
            verdict: Verdict::Different {
                cause: cause(left, right, origins),
                difference: first_difference(left, right)
                    .unwrap_or_else(|| "nothing (the walk found no difference)".to_owned()),
            },
            entries: Vec::new(),
        }
    }

    /// The record for a group where at least one side could not be decoded for comparison.
    pub fn undecodable(
        name: impl Into<String>,
        relation: ConflictRelation,
        message: impl Into<String>,
        entries: Vec<ConflictEntry>,
    ) -> ConflictRecord {
        ConflictRecord {
            name: name.into(),
            relation,
            verdict: Verdict::Different {
                cause: DiffCause::Undecodable,
                difference: message.into(),
            },
            entries,
        }
    }

    pub fn with_entries(mut self, entries: Vec<ConflictEntry>) -> ConflictRecord {
        self.entries = entries;
        self
    }
}

/// Why two normal forms differ.
pub fn cause(
    left: &NormalizedBody,
    right: &NormalizedBody,
    origins: Option<(&OriginTable, &OriginTable)>,
) -> DiffCause {
    if let Some((left_origins, right_origins)) = origins {
        let inlined = |origins: &OriginTable| {
            origins
                .origins
                .iter()
                .any(|origin| !origin.inline_stack.is_empty())
        };
        if inlined(left_origins) != inlined(right_origins) {
            return DiffCause::Inlined;
        }
    }
    if left.body.blocks.len() != right.body.blocks.len()
        || left.body.statement_count() != right.body.statement_count()
        || left.body.locals.len() != right.body.locals.len()
    {
        return DiffCause::Shape;
    }
    DiffCause::Content
}

/// The first field of the body two normal forms disagree about, walked in the order the schema
/// declares it: identity, kind, signature, locals, then block by block.
///
/// `None` when they do not differ at all, which for two bodies with different normalized hashes
/// would mean the hash sees something the walk does not — worth knowing, so it is reported rather
/// than smoothed over.
pub fn first_difference(left: &NormalizedBody, right: &NormalizedBody) -> Option<String> {
    let a = &left.body;
    let b = &right.body;
    if a.item != b.item {
        return Some(field(
            "item",
            format!("{:?}", a.item),
            format!("{:?}", b.item),
        ));
    }
    if a.kind.as_str() != b.kind.as_str() {
        return Some(field(
            "kind",
            a.kind.as_str().to_owned(),
            b.kind.as_str().to_owned(),
        ));
    }
    if a.semantic != b.semantic {
        return Some(field(
            "semantic",
            format!("{:?}", a.semantic),
            format!("{:?}", b.semantic),
        ));
    }
    // A static's initializer or a vtable's slots: the whole body, for those two kinds.
    if !matches!(a.kind, BodyKind::Function) && a.kind != b.kind {
        return Some(field(
            "kind payload",
            render(left, &format!("{:?}", a.kind)),
            render(right, &format!("{:?}", b.kind)),
        ));
    }
    if a.signature != b.signature {
        return Some(field(
            "signature",
            format!("{:?}", a.signature),
            format!("{:?}", b.signature),
        ));
    }
    if a.locals.len() != b.locals.len() {
        return Some(field(
            "locals",
            format!("{} locals", a.locals.len()),
            format!("{} locals", b.locals.len()),
        ));
    }
    for (index, (one, two)) in a.locals.iter().zip(&b.locals).enumerate() {
        if one != two {
            return Some(field(
                &format!("local {index}"),
                format!("{one:?}"),
                format!("{two:?}"),
            ));
        }
    }
    if a.blocks.len() != b.blocks.len() {
        return Some(field(
            "blocks",
            format!("{} blocks", a.blocks.len()),
            format!("{} blocks", b.blocks.len()),
        ));
    }
    for (index, (one, two)) in a.blocks.iter().zip(&b.blocks).enumerate() {
        if let Some(difference) = block_difference(left, right, index, one, two) {
            return Some(difference);
        }
    }
    None
}

fn block_difference(
    left: &NormalizedBody,
    right: &NormalizedBody,
    index: usize,
    a: &BasicBlock,
    b: &BasicBlock,
) -> Option<String> {
    if a.cleanup != b.cleanup {
        return Some(field(
            &format!("block {index} cleanup"),
            a.cleanup.to_string(),
            b.cleanup.to_string(),
        ));
    }
    if a.statements.len() != b.statements.len() {
        return Some(field(
            &format!("block {index}"),
            format!("{} statements", a.statements.len()),
            format!("{} statements", b.statements.len()),
        ));
    }
    for (position, (one, two)) in a.statements.iter().zip(&b.statements).enumerate() {
        if one != two {
            return Some(field(
                &format!("block {index} statement {position}"),
                describe_statement(left, one),
                describe_statement(right, two),
            ));
        }
    }
    if a.terminator != b.terminator {
        return Some(field(
            &format!("block {index} terminator"),
            describe_terminator(left, &a.terminator),
            describe_terminator(right, &b.terminator),
        ));
    }
    None
}

fn field(name: &str, left: String, right: String) -> String {
    format!("{name}: {left}  \u{2260}  {right}")
}

pub fn describe_statement(body: &NormalizedBody, statement: &Statement) -> String {
    render(body, &format!("{:?}", statement.kind))
}

pub fn describe_terminator(body: &NormalizedBody, terminator: &Terminator) -> String {
    render(body, &format!("{terminator:?}"))
}

/// The debug rendering with every `StrId(n)` replaced by the text it means in *that* body's
/// normalized string table: the two sides intern independently, so the raw ids are noise.
pub fn render(body: &NormalizedBody, debug: &str) -> String {
    const NEEDLE: &str = "StrId(";
    let mut out = String::with_capacity(debug.len());
    let mut rest = debug;
    while let Some(at) = rest.find(NEEDLE) {
        out.push_str(&rest[..at]);
        let after = &rest[at + NEEDLE.len()..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        match (
            digits.parse::<u32>(),
            after[digits.len()..].starts_with(')'),
        ) {
            (Ok(id), true) => {
                let text = body.str(StrId(id)).unwrap_or("?");
                let _ = write!(out, "`{text}`");
                rest = &after[digits.len() + 1..];
            }
            _ => {
                out.push_str(NEEDLE);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    truncate(&out, 240)
}

fn truncate(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    let kept: String = value.chars().take(width).collect();
    format!("{kept}\u{2026}")
}

/// Writes the autopsy to `path`, creating the parent directory if it does not exist.
///
/// Deterministic: records are sorted by name and then by relation, and every key is written in a
/// fixed order.
pub fn write(path: impl AsRef<Path>, records: &[ConflictRecord]) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, render_report(records))
}

/// The autopsy document's bytes.
pub fn render_report(records: &[ConflictRecord]) -> String {
    let mut sorted: Vec<&ConflictRecord> = records.iter().collect();
    sorted.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.relation.as_str().cmp(b.relation.as_str()))
            .then_with(|| a.verdict.as_str().cmp(b.verdict.as_str()))
    });

    let mut counts: Vec<(&'static str, usize)> = Verdict::ALL_LABELS
        .iter()
        .map(|label| {
            (
                *label,
                sorted
                    .iter()
                    .filter(|row| row.verdict.as_str() == *label)
                    .count(),
            )
        })
        .collect();
    counts.retain(|(_, count)| *count > 0);

    let mut out = String::with_capacity(128 * sorted.len() + 64);
    let _ = write!(
        out,
        "{{\"schema\":{CONFLICT_REPORT_SCHEMA},\"histogram\":{{"
    );
    for (index, (verdict, count)) in counts.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(&mut out, verdict);
        let _ = write!(out, ":{count}");
    }
    out.push_str("},\"conflicts\":[");
    for (index, record) in sorted.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        push_string(&mut out, &record.name);
        out.push_str(",\"relation\":");
        push_string(&mut out, record.relation.as_str());
        out.push_str(",\"verdict\":");
        push_string(&mut out, record.verdict.as_str());
        out.push_str(",\"cause\":");
        push_string(&mut out, record.verdict.cause_str());
        out.push_str(",\"difference\":");
        match record.verdict.difference() {
            Some(difference) => push_string(&mut out, difference),
            None => out.push_str("null"),
        }
        out.push_str(",\"entries\":[");
        for (position, entry) in record.entries.iter().enumerate() {
            if position > 0 {
                out.push(',');
            }
            out.push_str("{\"origin\":");
            push_string(&mut out, &entry.origin);
            out.push_str(",\"path\":");
            push_string(&mut out, &entry.path);
            out.push_str(",\"blob\":");
            push_hash(&mut out, entry.blob);
            out.push_str(",\"normalized\":");
            push_hash(&mut out, entry.normalized);
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push_str("]}\n");
    out
}

fn push_hash(out: &mut String, hash: Option<Hash128>) {
    match hash {
        Some(hash) => push_string(out, &hash.to_string()),
        None => out.push_str("null"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_is_deterministic_and_shaped_as_documented() {
        let record = |name: &str, verdict: Verdict| ConflictRecord {
            name: name.to_owned(),
            relation: ConflictRelation::Alias,
            verdict,
            entries: vec![ConflictEntry {
                origin: "libcore".to_owned(),
                path: "core::ptr::drop_glue".to_owned(),
                blob: Some(Hash128 { lo: 1, hi: 0 }),
                normalized: Some(Hash128 { lo: 2, hi: 0 }),
            }],
        };
        let different = || Verdict::Different {
            cause: DiffCause::Shape,
            difference: "blocks: 3 blocks  \u{2260}  4 blocks".to_owned(),
        };
        let rows = vec![record("z", different()), record("a", different())];
        let forward = render_report(&rows);
        let backward = render_report(&[rows[1].clone(), rows[0].clone()]);
        assert_eq!(forward, backward);
        assert!(
            forward.starts_with("{\"schema\":1,\"histogram\":{\"different\":2},\"conflicts\":[")
        );
        assert!(forward.contains("\"name\":\"a\""));
        assert!(forward
            .contains("\"relation\":\"alias\",\"verdict\":\"different\",\"cause\":\"shape\""));
        assert!(forward.contains("\"blob\":\"00000000000000000000000000000001\""));
        assert!(forward.ends_with("]}\n"));
    }

    #[test]
    fn an_empty_autopsy_still_writes_a_document() {
        assert_eq!(
            render_report(&[]),
            "{\"schema\":1,\"histogram\":{},\"conflicts\":[]}\n"
        );
    }
}
