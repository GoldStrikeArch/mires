//! Shared `#[js_export]` and `#[js_opaque]` wire formats between proc macros, `rustc_codegen_mires`,
//! and `jsc-link`.

/// Default `link_section` emitted by `#[js_export]` to request automatic JS↔Rust boundary marshaling.
pub const EXPORT_MARSHAL_SECTION: &str = "rcgjs.export.v1";

/// Prefix for marshaled `#[js_export]` items that also declare an automatic code-split chunk.
pub const EXPORT_CHUNK_PREFIX: &str = "rcgjs.export.v1.chunk.";

/// Prefix for raw (`#[js_export(raw)]`) items that declare an automatic code-split chunk.
pub const RAW_CHUNK_PREFIX: &str = "rcgjs.chunk.v1.";

/// Internal `link_section` emitted for the return-marshaling helper of `#[js_export] async fn`.
pub const EXPORT_OUT_SECTION: &str = "rcgjs.export.out.v1";

/// Hidden associated constant name emitted by `#[js_opaque]` to convey a TypeScript type spelling
/// across crate boundaries to `backend/src/dts.rs`.
pub const OPAQUE_TS_CONST_NAME: &str = "__MIRES_TS_TYPE";

/// Deployment role declared on an attribute-discovered chunk entry point.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum ChunkRole {
    #[default]
    Main,
    Island,
    Worker,
    Procedure,
}

impl ChunkRole {
    /// Canonical lowercase wire spelling matching `rcgjs_object::artifact::EntryRole`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Island => "island",
            Self::Worker => "worker",
            Self::Procedure => "procedure",
        }
    }

    /// Parses a chunk role from its wire or attribute spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "main" => Some(Self::Main),
            "island" => Some(Self::Island),
            "worker" => Some(Self::Worker),
            "procedure" => Some(Self::Procedure),
            _ => None,
        }
    }
}

/// Chunk metadata carried in a `#[js_export]` `link_section`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ChunkSpec {
    pub name: String,
    pub role: ChunkRole,
}

/// Decoded meaning of a `#[js_export]` `link_section`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportSection {
    /// Whether JS↔Rust boundary marshaling should wrap this export.
    pub marshal: bool,
    /// Optional attribute-declared code-split chunk entry.
    pub chunk: Option<ChunkSpec>,
}

/// Whether `name` is valid as both a chunk identifier and an emitted JS filename stem.
#[must_use]
pub fn is_valid_chunk_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '$'))
}

/// Encodes export marshaling and optional chunk metadata into a single `link_section` literal.
///
/// Returns `None` only when `marshal` is `false` and `chunk` is `None` (no `link_section` needed).
#[must_use]
pub fn encode_export_section(marshal: bool, chunk: Option<(&str, ChunkRole)>) -> Option<String> {
    match (marshal, chunk) {
        (true, None) => Some(EXPORT_MARSHAL_SECTION.to_owned()),
        (true, Some((name, role))) => {
            debug_assert!(is_valid_chunk_name(name), "invalid chunk name `{name}`");
            Some(format!("{EXPORT_CHUNK_PREFIX}{}.{name}", role.as_str()))
        }
        (false, Some((name, role))) => {
            debug_assert!(is_valid_chunk_name(name), "invalid chunk name `{name}`");
            Some(format!("{RAW_CHUNK_PREFIX}{}.{name}", role.as_str()))
        }
        (false, None) => None,
    }
}

/// Decodes a `#[js_export]` `link_section` emitted by [`encode_export_section`].
#[must_use]
pub fn parse_export_section(section: &str) -> Option<ExportSection> {
    if section == EXPORT_MARSHAL_SECTION {
        return Some(ExportSection {
            marshal: true,
            chunk: None,
        });
    }
    if let Some(rest) = section.strip_prefix(EXPORT_CHUNK_PREFIX) {
        let chunk = parse_role_and_name(rest)?;
        return Some(ExportSection {
            marshal: true,
            chunk: Some(chunk),
        });
    }
    if let Some(rest) = section.strip_prefix(RAW_CHUNK_PREFIX) {
        let chunk = parse_role_and_name(rest)?;
        return Some(ExportSection {
            marshal: false,
            chunk: Some(chunk),
        });
    }
    None
}

fn parse_role_and_name(rest: &str) -> Option<ChunkSpec> {
    let (role_raw, name) = rest.split_once('.')?;
    let role = ChunkRole::parse(role_raw)?;
    if !is_valid_chunk_name(name) {
        return None;
    }
    Some(ChunkSpec {
        name: name.to_owned(),
        role,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_all_export_section_shapes() {
        assert_eq!(encode_export_section(false, None), None);
        assert_eq!(
            encode_export_section(true, None).as_deref(),
            Some(EXPORT_MARSHAL_SECTION)
        );
        assert_eq!(
            parse_export_section(EXPORT_MARSHAL_SECTION),
            Some(ExportSection {
                marshal: true,
                chunk: None,
            })
        );

        for role in [
            ChunkRole::Main,
            ChunkRole::Island,
            ChunkRole::Worker,
            ChunkRole::Procedure,
        ] {
            for marshal in [true, false] {
                let encoded = encode_export_section(marshal, Some(("counter-1", role)))
                    .expect("chunked export encodes");
                assert_eq!(
                    parse_export_section(&encoded),
                    Some(ExportSection {
                        marshal,
                        chunk: Some(ChunkSpec {
                            name: "counter-1".to_owned(),
                            role,
                        }),
                    })
                );
            }
        }
    }

    #[test]
    fn rejects_malformed_chunk_sections_and_names() {
        assert!(!is_valid_chunk_name(""));
        assert!(!is_valid_chunk_name("a/b"));
        assert!(!is_valid_chunk_name("a:b"));
        assert!(!is_valid_chunk_name("a.b"));
        assert!(!is_valid_chunk_name("a b"));
        assert!(is_valid_chunk_name("counter_1-a$b"));

        assert_eq!(
            parse_export_section("rcgjs.export.v1.chunk.bogus.foo"),
            None
        );
        assert_eq!(parse_export_section("rcgjs.export.v1.chunk.island."), None);
        assert_eq!(
            parse_export_section("rcgjs.export.v1.chunk.island.a.b"),
            None
        );
        assert_eq!(parse_export_section("rcgjs.chunk.v1.shared.foo"), None);
        assert_eq!(parse_export_section("rcgjs.export.out.v1"), None);
    }
}
