//! The lane report: one line per item saying what the JIR lane could own.
//!
//! `-Cllvm-args=js-lane-report=<path>` makes the link step write this file. It answers the one
//! question the coverage gate asks — *how many bytes of the program does the JIR lane own, and what
//! is the reason for every byte it does not* — and it answers it per item, so a regression names
//! the items that caused it.
//!
//! The shape is fixed by `scripts/jir-coverage.sh`, which reads it:
//!
//! ```json
//! {"schema":1,"items":[{"name":"core$fmt$write","bytes":412,"availability":"available",
//!                       "decline":null,"class":["refs","unwind"],"lane":"jir"}]}
//! ```
//!
//! `availability` is one of `available`, `semantic`, `unavailable` — the same three words v1's
//! optimizer report used, so the script reads either file the same way. `decline` is a
//! [`Decline::code`], the stable machine key, and is `null` exactly when the item is `available`.
//! `bytes` is what a size report attributes to the item today (its printed text) so the numbers
//! add up to the program. `lane` is which lowering printed the item: `baseline` when the lane
//! driver did not run or did not offer the item to the lowerer, `jir` when the JIR lane printed
//! it, `fallback` when the lowerer declined and the baseline text shipped instead.
//!
//! The file is deterministic: items are sorted by name, the keys are in a fixed order, and nothing
//! about the writer depends on iteration order or on a hash seed. Two links of one program produce
//! two byte-identical reports.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use crate::jir::decline::Decline;
use crate::jir::summary::{Availability, ItemClass, ItemSummary};

/// The schema of the document this module writes. Bumped when a key changes meaning; a new key
/// that a reader may ignore does not bump it.
pub const LANE_REPORT_SCHEMA: u32 = 1;

/// How much of an item the JIR lane owns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneAvailability {
    /// The lane produced the item's JavaScript.
    Available,
    /// The lane understands the item's body but did not produce its JavaScript: the body is
    /// available for analysis (call summaries, escape facts, representation decisions) while the
    /// printed baseline text is still what ships. This is the state the shadow lane is in for most
    /// of Phase 1, and it is counted separately because it is the population Phase 2 converts.
    Semantic,
    /// The lane has nothing: `decline` says why.
    Unavailable,
}

impl LaneAvailability {
    /// The word the report writes and `jir-coverage.sh` matches on.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Semantic => "semantic",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Which lowering printed an item.
///
/// Distinct from [`LaneAvailability`], which says what the lane *could* own: an item can be
/// `Semantic` (it has a body) and still be printed from the baseline because its class is not
/// promoted, which is [`ItemLane::Baseline`], not [`ItemLane::Fallback`]. `Fallback` is reserved
/// for the item the lowerer was asked about and declined, which is the number a promotion round
/// drives to zero.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ItemLane {
    /// The lane driver did not lower this item: no lane ran, or its class is not promoted.
    #[default]
    Baseline,
    /// The JIR lane produced the printed text.
    Jir,
    /// The lowerer was offered the item and declined; `decline` says why.
    Fallback,
}

impl ItemLane {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Jir => "jir",
            Self::Fallback => "fallback",
        }
    }
}

/// One item's row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaneItem {
    /// The item's JavaScript identifier: [`crate::jir::summary::SemanticKey::link_name`].
    pub name: String,
    /// The bytes the program spends on the item.
    pub bytes: u64,
    pub availability: LaneAvailability,
    /// Why the lane does not own the item. `None` when it does.
    pub decline: Option<Decline>,
    pub class: ItemClass,
    /// Which lowering printed the item. See [`ItemLane`].
    pub lane: ItemLane,
}

impl LaneItem {
    /// The row for an item the link step has not decided anything further about: what the object's
    /// summary index already says.
    ///
    /// A summary that carries a body is `Semantic` rather than `Available`: having a body is not
    /// having produced JavaScript from it. A lane that lowered the item overwrites the field.
    pub fn of(summary: &ItemSummary) -> LaneItem {
        let (availability, decline) = match &summary.availability {
            Availability::Available => (LaneAvailability::Semantic, None),
            Availability::Unavailable(decline) => {
                (LaneAvailability::Unavailable, Some(decline.clone()))
            }
        };
        LaneItem {
            name: summary.key.link_name.clone(),
            bytes: u64::from(summary.estimated_bytes),
            availability,
            decline,
            class: summary.class,
            lane: ItemLane::Baseline,
        }
    }
}

/// Writes the report to `path`, creating the parent directory if it does not exist.
pub fn write(path: impl AsRef<Path>, items: &[LaneItem]) -> io::Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, render(items))
}

/// The report's bytes: items sorted by name, then by everything else, so that two items sharing a
/// name (which the dedup rule should have prevented) still order deterministically.
pub fn render(items: &[LaneItem]) -> String {
    let mut sorted: Vec<&LaneItem> = items.iter().collect();
    sorted.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.bytes.cmp(&b.bytes))
            .then_with(|| a.availability.as_str().cmp(b.availability.as_str()))
            .then_with(|| {
                a.decline
                    .as_ref()
                    .map(Decline::code)
                    .cmp(&b.decline.as_ref().map(Decline::code))
            })
            .then_with(|| a.class.0.cmp(&b.class.0))
            .then_with(|| a.lane.as_str().cmp(b.lane.as_str()))
    });

    let mut out = String::with_capacity(64 * sorted.len() + 32);
    let _ = write!(out, "{{\"schema\":{LANE_REPORT_SCHEMA},\"items\":[");
    for (index, item) in sorted.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        push_string(&mut out, &item.name);
        let _ = write!(out, ",\"bytes\":{}", item.bytes);
        out.push_str(",\"availability\":");
        push_string(&mut out, item.availability.as_str());
        out.push_str(",\"decline\":");
        match &item.decline {
            Some(decline) => push_string(&mut out, decline.code()),
            None => out.push_str("null"),
        }
        out.push_str(",\"class\":[");
        for (position, name) in item.class.names().iter().enumerate() {
            if position > 0 {
                out.push(',');
            }
            push_string(&mut out, name);
        }
        out.push_str("],\"lane\":");
        push_string(&mut out, item.lane.as_str());
        out.push('}');
    }
    out.push_str("]}\n");
    out
}

/// A JSON string literal. Control characters are escaped the short way where JSON has one and as
/// `\u00XX` otherwise; everything else, non-ASCII included, is written through as UTF-8.
///
/// Shared across `lane_report`, `jir::conflict`, `sourcemap`, and `jsc-link` so every hand-written
/// JSON document escapes strings identically.
pub fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if control < ' ' => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::decline::{EncodeDecline, MergeDecline};

    fn item(name: &str, bytes: u64, availability: LaneAvailability) -> LaneItem {
        LaneItem {
            name: name.to_owned(),
            bytes,
            availability,
            decline: None,
            class: ItemClass::empty(),
            lane: ItemLane::Baseline,
        }
    }

    #[test]
    fn renders_the_shape_jir_coverage_reads() {
        let mut declined = item("b_item", 20, LaneAvailability::Unavailable);
        declined.decline = Some(Decline::Encode(EncodeDecline::Disabled));
        declined.class = ItemClass::REFS.union(ItemClass::UNWIND);
        declined.lane = ItemLane::Fallback;
        let mut available = item("a_item", 10, LaneAvailability::Available);
        available.lane = ItemLane::Jir;
        let items = vec![
            declined,
            available,
            item("c_item", 30, LaneAvailability::Semantic),
        ];
        let rendered = render(&items);
        assert_eq!(
            rendered,
            "{\"schema\":1,\"items\":[\
             {\"name\":\"a_item\",\"bytes\":10,\"availability\":\"available\",\"decline\":null,\"class\":[],\"lane\":\"jir\"},\
             {\"name\":\"b_item\",\"bytes\":20,\"availability\":\"unavailable\",\"decline\":\"encode.disabled\",\"class\":[\"refs\",\"unwind\"],\"lane\":\"fallback\"},\
             {\"name\":\"c_item\",\"bytes\":30,\"availability\":\"semantic\",\"decline\":null,\"class\":[],\"lane\":\"baseline\"}\
             ]}\n"
        );
    }

    #[test]
    fn is_deterministic_under_input_order() {
        let mut first = item("z", 1, LaneAvailability::Unavailable);
        first.decline = Some(Decline::Merge(MergeDecline::BodyChoice {
            chosen: 1,
            rejected: 2,
        }));
        let second = item("a", 2, LaneAvailability::Available);
        let third = item("m", 3, LaneAvailability::Semantic);
        let forward = render(&[first.clone(), second.clone(), third.clone()]);
        let backward = render(&[third, second, first]);
        assert_eq!(forward, backward);
    }

    #[test]
    fn escapes_a_name_that_is_not_json_safe() {
        let name = "a\"b\\c\nd\u{1}e";
        let rendered = render(&[item(name, 0, LaneAvailability::Available)]);
        assert!(
            rendered.contains("\"name\":\"a\\\"b\\\\c\\nd\\u0001e\""),
            "{rendered}"
        );
    }

    #[test]
    fn push_string_matches_serde_json_for_control_and_unicode_chars() {
        let mut sample = String::new();
        for code in 0u32..0x20 {
            sample.push(char::from_u32(code).expect("valid scalar"));
        }
        sample.push_str("\"\\hello caf\u{e9} \u{2260} \u{1f600}");
        let mut out = String::new();
        push_string(&mut out, &sample);
        assert_eq!(out, serde_json::to_string(&sample).expect("json string"));
    }

    #[test]
    fn writes_the_file_and_creates_its_directory() {
        let base = std::env::temp_dir().join(format!(
            "rcgjs-lane-report-{}-{}",
            std::process::id(),
            LANE_REPORT_SCHEMA
        ));
        let path = base.join("nested").join("lane.json");
        let _ = fs::remove_dir_all(&base);
        write(&path, &[item("only", 7, LaneAvailability::Available)]).expect("write");
        let text = fs::read_to_string(&path).expect("read");
        assert!(
            text.starts_with("{\"schema\":1,\"items\":[{\"name\":\"only\""),
            "{text}"
        );
        let _ = fs::remove_dir_all(&base);
    }
}
