//! The object file: format 27.
//!
//! An object is what one codegen unit becomes and what the link step reads back out of an rlib. It
//! has two halves. The **text section** comes first: the codegen unit's prologue and every item's
//! printed baseline JavaScript, concatenated, exactly as v1 wrote them — so an item's `js` is a
//! byte-identical slice whether the object was written before or after this format. The **binary
//! member** follows, and a fixed-size **trailer** closes the file so a reader finds the member
//! without scanning the text:
//!
//! ```text
//! text section        UTF-8: prologue + item JavaScript, back to back
//! member              RCGJS\0 · u16 format · u32 header len · postcard(Header)
//!                     · u32 table len · postcard(Vec<SectionEntry>) · sections
//! trailer (16 bytes)  RCGJS\0 · u16 format · u64 member offset (from the start of the file)
//! ```
//!
//! Every section is a byte range of the member (`SectionEntry.offset` is member-relative) with a
//! [`Hash128`] checksum the reader verifies before decoding anything out of it:
//!
//! | id | section | contents |
//! | --- | --- | --- |
//! | S1 | interns | `postcard(Packed { ulen, deflate(postcard(Interns)) })`: strings, bytes, signatures, source facts, the crate-scope `TypeTable`. |
//! | S2 | summaries | `postcard(Vec<ItemSummary>)`, uncompressed, decoded eagerly. |
//! | S3 | bodies | `u32 table len · postcard(Vec<BlobEntry>) · blobs`: content-addressed, independently deflated `postcard(Body)` blobs; two items with one body share one blob. |
//! | S4 | baseline | Opaque to this crate: the producer's per-item baseline record (source map, zombies, debug path, TypeScript signature). |
//! | S5 | origins | Independently deflated `postcard(OriginTable)` blobs, one per item that has a body. |
//! | S7 | sources | Independently deflated source text, retained for later source-map emission. |
//! | S6 | legacy | Compatibility and report facts still consumed by the linker: analysis summaries, shadow checks, and optimization info. |
//!
//! [`ItemSummary::fallback`] addresses the text section (`offset` from the start of the file),
//! [`ItemSummary::body`] a blob in S3 and [`ItemSummary::origin`] a blob in S5, both relative to
//! their section's start. A reader inflates a body only when it asks for it, which is what makes
//! "decode only the reachable bodies" a property of the reader rather than of the writer.
//!
//! [`FORMAT_VERSION`] is bumped whenever any of this changes shape; postcard is not
//! self-describing, so an object written in another shape would parse into something else rather
//! than fail. The header additionally pins the JIR schema, the rustc that produced the object, its
//! data layout, and the emit-affecting options, so that a mismatch is a diagnostic naming the
//! rebuild that fixes it instead of a miscompilation.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::jir::codec::{self, Codec, CodecError};
use crate::jir::hash::{Hash128, HashAlgorithm};
use crate::jir::interns::Interns;
use crate::jir::origin::OriginTable;
use crate::jir::semantic::{Body, BodyKind};
use crate::jir::summary::{self, Availability, ItemClass, ItemSummary, SemanticKey};
use crate::jir::JIR_SCHEMA;

/// The object format. Every object in a program has to carry this one.
pub const FORMAT_VERSION: u16 = 27;

/// The marker that opens the member and the trailer.
pub const MAGIC: [u8; 6] = *b"RCGJS\0";

/// The trailer: magic, format, member offset.
const TRAILER_LEN: usize = 6 + 2 + 8;

// -------------------------------------------------------------------------------------------
// Item metadata shared by the summary index and the producer
// -------------------------------------------------------------------------------------------

/// What an item is, which decides how it is ordered in the output.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ItemKind {
    /// Append-only: postcard serializes this enum by index, and the index is the format 19 tag.
    ///
    /// A function declaration. Hoisted by JavaScript, so order does not matter.
    Fn,
    /// A module level `let`, from a Rust `static`. Must precede its users.
    Static,
    /// An interned vtable, emitted as a module level array.
    Vtable,
    /// A piece of the allocator shim.
    Alloc,
    /// A hoisted shared constant — a `#[track_caller]` location, a long string literal — emitted
    /// as a module level `const`.
    Const,
    /// An `import` declaration, in ES module output. Printed before everything else.
    ///
    /// One item binds one name, and the item's name *is* that binding: an import is then reachable
    /// exactly when some other item mentions what it binds, and two crates that import the same
    /// name from the same module produce one item with one text, which the link step collapses
    /// like any other duplicate. The item's `debug_path` is the module specifier it imports from.
    Import,
}

impl ItemKind {
    /// Whether the item is a module level binding, which has to be initialized in order.
    pub fn is_binding(self) -> bool {
        matches!(self, ItemKind::Static | ItemKind::Vtable | ItemKind::Const)
    }

    /// The format 19 tag. Append only: a value already written into an object file keeps it.
    pub fn tag(self) -> u8 {
        match self {
            ItemKind::Fn => 0,
            ItemKind::Static => 1,
            ItemKind::Vtable => 2,
            ItemKind::Alloc => 3,
            ItemKind::Const => 4,
            ItemKind::Import => 5,
        }
    }

    pub fn from_tag(tag: u8) -> Option<ItemKind> {
        Some(match tag {
            0 => ItemKind::Fn,
            1 => ItemKind::Static,
            2 => ItemKind::Vtable,
            3 => ItemKind::Alloc,
            4 => ItemKind::Const,
            5 => ItemKind::Import,
            _ => return None,
        })
    }
}

/// Where a declaration whose *order* is part of what it means came from.
///
/// A `view!` template cloner is the case, and the only one so far. The reference compiler declares
/// one per distinct template in the order the module first uses them, and a trace numbers them in
/// declaration order, so the order is observable. The backend names a cloner after a hash of its
/// HTML, which orders it arbitrarily, so it carries the source position of the `view!` that built
/// it instead and the link step sorts on that.
///
/// The position is the *expansion's*, not the generated item's, so every template one `view!` built
/// shares it; [`SourceOrder::within`] is what then keeps them in the order the expansion declared
/// them, which is the order the view mentions them in.
///
/// A template several views use takes the *earliest* position of any of them, which is what "first
/// use" means. It cannot be the first one codegenned: items are codegenned in a symbol-hash order
/// that has nothing to do with the source.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceOrder {
    pub file: String,
    pub line: u32,
    pub col: u32,
    /// Which declaration of that one expansion this is, for two that share the position above.
    pub within: u32,
}

/// How an item joins the program: its name, its rooting and its visibility.
///
/// The three are separate facts and only look like one because a `#[no_mangle]` item in the crate
/// being linked has all three. The same item read back out of a dependency's rlib keeps its exact
/// name, because something outside the program may still call it by that name, but is neither a
/// root nor part of this program's interface: a static archive hands out members that are asked
/// for, it does not push its own exports into the program.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Linkage {
    /// The exact name the item must be emitted under. Minification never renames it.
    pub fixed_name: Option<String>,
    /// Whether the item is a dead-code-elimination root.
    pub root: bool,
    /// Whether the module's export clause names it. Only ES module output has one.
    pub exported: bool,
}

impl Linkage {
    /// An item only this program refers to, under a name only this program sees.
    pub fn internal() -> Linkage {
        Linkage::default()
    }

    /// An item the program exposes to its host under `name`: `#[no_mangle]`, `#[export_name]`.
    pub fn exported(name: String) -> Linkage {
        Linkage {
            fixed_name: Some(name),
            root: true,
            exported: true,
        }
    }

    /// An item that keeps `name` and is a root, but is no part of the program's interface.
    pub fn rooted(name: String) -> Linkage {
        Linkage {
            fixed_name: Some(name),
            root: true,
            exported: false,
        }
    }

    /// An item that must be emitted under `name` and is nothing else: not a root, not exported.
    ///
    /// The allocator shim is the case. Its name is fixed because the calls to it are compiled into
    /// `library/alloc` under exactly that symbol, and it is not a root because a program that never
    /// allocates should not carry one.
    pub fn fixed(name: String) -> Linkage {
        Linkage {
            fixed_name: Some(name),
            root: false,
            exported: false,
        }
    }

    /// Turns this into the linkage of the same item seen from a dependency.
    ///
    /// The name survives and everything else goes. Keeping the name is what stops minification
    /// from renaming a dependency's `#[no_mangle]` item that external JavaScript calls; dropping
    /// the rest is what keeps `compiler_builtins`' few hundred exports from rooting all of `core`.
    pub fn demote(&mut self) {
        self.root = false;
        self.exported = false;
    }
}

/// The source map entries for one item, with generated lines relative to its own first line.
///
/// Relative, because an item's place in the finished program is not known until the link step has
/// decided what is reachable, and an item from an rlib was printed in another compilation entirely.
/// An item's JavaScript always starts at the beginning of a line, so only the line has to be
/// rebased; the columns are already right.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItemMap {
    pub sources: Vec<String>,
    pub names: Vec<String>,
    /// `(gen_line, gen_col, source, src_line, src_col, name)`. `name` is `-1` for "no name", and
    /// `source`/`name` index the two lists above.
    pub segments: Vec<(u32, u32, u32, u32, u32, i32)>,
}

// -------------------------------------------------------------------------------------------
// The container
// -------------------------------------------------------------------------------------------

/// What every object in a program has to agree on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// [`JIR_SCHEMA`] at write time.
    pub jir_schema: u16,
    /// A digest of the producing rustc's version string (`sess.cfg_version`, the `rustc -vV`
    /// release line), so an object from another toolchain is caught before its type ids are
    /// compared with this one's.
    pub rustc_version_hash: u64,
    /// The target's data layout string.
    pub data_layout: String,
    /// The emit-affecting backend options, normalized (`key=value` pairs sorted by key, last
    /// value per key), which every object in a program must have been compiled with.
    pub emit_args: String,
    /// The Rust crate the object's items belong to.
    pub crate_name: String,
    pub codec: Codec,
    pub hash: HashAlgorithm,
}

/// The sections, by id.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionId {
    Interns = 1,
    Summaries = 2,
    Bodies = 3,
    Baseline = 4,
    Origins = 5,
    Legacy = 6,
    Sources = 7,
}

impl SectionId {
    pub const ALL: [SectionId; 7] = [
        SectionId::Interns,
        SectionId::Summaries,
        SectionId::Bodies,
        SectionId::Baseline,
        SectionId::Origins,
        SectionId::Legacy,
        SectionId::Sources,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SectionId::Interns => "S1 interns",
            SectionId::Summaries => "S2 summaries",
            SectionId::Bodies => "S3 bodies",
            SectionId::Baseline => "S4 baseline",
            SectionId::Origins => "S5 origins",
            SectionId::Legacy => "S6 legacy",
            SectionId::Sources => "S7 sources",
        }
    }

    fn from_id(id: u16) -> Option<SectionId> {
        SectionId::ALL
            .into_iter()
            .find(|section| *section as u16 == id)
    }
}

/// One row of the section table.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SectionEntry {
    pub id: u16,
    /// From the start of the member.
    pub offset: u64,
    pub len: u64,
    pub checksum: Hash128,
}

/// A byte range inside a section (or, for a fallback, inside the file), with the length it
/// inflates to. `clen == ulen` for an uncompressed range.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct BlobRef {
    pub offset: u64,
    pub clen: u64,
    pub ulen: u64,
}

/// One row of the S3 blob table.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlobEntry {
    /// The content hash of the inflated postcard bytes.
    pub hash: Hash128,
    /// From the start of S3.
    pub offset: u64,
    pub clen: u64,
    pub ulen: u64,
}

impl BlobEntry {
    pub fn blob_ref(&self) -> BlobRef {
        BlobRef {
            offset: self.offset,
            clen: self.clen,
            ulen: self.ulen,
        }
    }
}

/// A single deflated payload with its inflated length (S1).
#[derive(Serialize, Deserialize)]
struct Packed {
    ulen: u64,
    data: Vec<u8>,
}

/// A failure to read an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjectError {
    /// The trailer names another format.
    Format { found: u16 },
    /// The header's JIR schema is not this crate's.
    Schema { found: u16 },
    /// Anything structurally wrong: a section checksum that does not match, an offset out of
    /// range, bytes that do not decode.
    Corrupt(String),
    /// A per-item decode failure (a body or origin blob).
    Item { name: String, message: String },
}

impl fmt::Display for ObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectError::Format { found } => write!(
                f,
                "this object holds an rcgjs item table in format {found}, and this backend reads \
                 format {FORMAT_VERSION}: it was written by another build of the backend. Rebuild \
                 everything this program links against, the sysroot included \
                 (`scripts/build_sysroot.sh --clean`)"
            ),
            ObjectError::Schema { found } => write!(
                f,
                "this object carries JIR schema {found}, and this backend reads schema \
                 {JIR_SCHEMA}: it was written by another build of the backend. Rebuild everything \
                 this program links against, the sysroot included \
                 (`scripts/build_sysroot.sh --clean`)"
            ),
            ObjectError::Corrupt(message) => write!(f, "corrupt rcgjs object: {message}"),
            ObjectError::Item { name, message } => {
                write!(f, "rcgjs item `{name}`: {message}")
            }
        }
    }
}

impl std::error::Error for ObjectError {}

impl From<CodecError> for ObjectError {
    fn from(err: CodecError) -> ObjectError {
        ObjectError::Corrupt(err.0)
    }
}

// -------------------------------------------------------------------------------------------
// Writing
// -------------------------------------------------------------------------------------------

/// What the producer knows about one item, before the writer places it.
pub struct ItemInput<'a> {
    pub key: SemanticKey,
    pub availability: Availability,
    /// The item's `refs`, sorted, its own name excluded.
    pub deps: Vec<String>,
    pub kind: ItemKind,
    pub linkage: Linkage,
    pub order: Option<SourceOrder>,
    pub dts: bool,
    /// The JIR body and its origins, when the item has them.
    pub body: Option<(&'a Body, &'a OriginTable)>,
}

/// Builds one object: text section first, then the member, then the trailer.
///
/// Items are pushed in output order; the index [`Writer::push_item`] returns is the item's position
/// in S2, which is also the position the producer writes its S4 and S6 records at.
pub struct Writer {
    header: Header,
    text: String,
    interns: Interns,
    summaries: Vec<ItemSummary>,
    blobs: Vec<BlobEntry>,
    blob_index: BTreeMap<Hash128, usize>,
    /// The S3 blob bytes, before the table is prepended.
    body_bytes: Vec<u8>,
    origin_bytes: Vec<u8>,
}

impl Writer {
    /// Starts an object whose text section begins with `prologue`. `interns` are the crate-scope
    /// tables every body pushed later refers into.
    pub fn new(header: Header, prologue: &str, interns: Interns) -> Writer {
        Writer {
            header,
            text: prologue.to_owned(),
            interns,
            summaries: Vec::new(),
            blobs: Vec::new(),
            blob_index: BTreeMap::new(),
            body_bytes: Vec::new(),
            origin_bytes: Vec::new(),
        }
    }

    /// The bytes written to the text section so far: the offset the next item's JavaScript lands
    /// at.
    pub fn text_len(&self) -> usize {
        self.text.len()
    }

    /// Appends the item's JavaScript to the text section and its summary to the index, storing its
    /// body (content-addressed) and origins. Returns the item's index.
    pub fn push_item(&mut self, js: &str, item: ItemInput<'_>) -> usize {
        let start = self.text.len();
        self.text.push_str(js);
        let fallback = BlobRef {
            offset: start as u64,
            clen: js.len() as u64,
            ulen: js.len() as u64,
        };

        let mut body_hash = None;
        let mut semantic_hash = None;
        let mut body_ref = None;
        let mut origin_ref = None;
        let mut abi = None;
        let mut physical = None;
        let mut class = ItemClass::empty();
        if let Some((body, origins)) = item.body {
            let bytes = codec::encode_body(body);
            let hash = Hash128::of(&bytes);
            let index = match self.blob_index.get(&hash) {
                Some(index) => *index,
                None => {
                    let packed = self.header.codec.compress(&bytes);
                    let entry = BlobEntry {
                        hash,
                        offset: self.body_bytes.len() as u64,
                        clen: packed.len() as u64,
                        ulen: bytes.len() as u64,
                    };
                    self.body_bytes.extend_from_slice(&packed);
                    self.blobs.push(entry);
                    self.blob_index.insert(hash, self.blobs.len() - 1);
                    self.blobs.len() - 1
                }
            };
            body_hash = Some(hash);
            let normalized = summary::semantic_hash(body, &self.interns);
            semantic_hash = Some(normalized);
            physical = summary::PhysicalSummary::of(body, &self.interns.types, normalized);
            body_ref = Some(self.blobs[index].blob_ref());

            let origin_plain = codec::encode_origins(origins);
            let origin_packed = self.header.codec.compress(&origin_plain);
            origin_ref = Some(BlobRef {
                offset: self.origin_bytes.len() as u64,
                clen: origin_packed.len() as u64,
                ulen: origin_plain.len() as u64,
            });
            self.origin_bytes.extend_from_slice(&origin_packed);

            if matches!(body.kind, BodyKind::Function) {
                abi = Some(body.signature.abi);
            }
            class = ItemClass::of(body, &self.interns.types);
        }

        self.summaries.push(ItemSummary {
            key: item.key,
            body_hash,
            semantic_hash,
            availability: item.availability,
            deps: item.deps,
            abi,
            physical,
            kind: item.kind,
            linkage: item.linkage,
            order: item.order,
            dts: item.dts,
            fallback,
            body: body_ref,
            origin: origin_ref,
            estimated_bytes: js.len().min(u32::MAX as usize) as u32,
            class,
        });
        self.summaries.len() - 1
    }

    /// The summaries pushed so far, for a producer that wants to check them.
    pub fn summaries(&self) -> &[ItemSummary] {
        &self.summaries
    }

    /// Finishes the object. `baseline` and `legacy` are the producer's S4 and S6 payloads.
    pub fn finish(
        self,
        baseline: &[u8],
        legacy: &[u8],
        sources: &BTreeMap<String, String>,
    ) -> Vec<u8> {
        let codec = self.header.codec;

        let interns_plain = codec::encode(&self.interns);
        let s1 = codec::encode(&Packed {
            ulen: interns_plain.len() as u64,
            data: codec.compress(&interns_plain),
        });

        // S3 is the blob table followed by the blobs. A body's `BlobRef` counts from the section's
        // start, so the table's own span is added now that it is known.
        let blob_table = codec::encode(&self.blobs);
        let table_span = 4 + blob_table.len() as u64;
        let mut s3 = Vec::with_capacity(table_span as usize + self.body_bytes.len());
        s3.extend_from_slice(&(blob_table.len() as u32).to_le_bytes());
        s3.extend_from_slice(&blob_table);
        s3.extend_from_slice(&self.body_bytes);
        let mut summaries = self.summaries;
        for summary in &mut summaries {
            if let Some(body) = &mut summary.body {
                body.offset += table_span;
            }
        }
        let s2 = codec::encode(&summaries);
        let s5 = self.origin_bytes;

        let source_bytes = codec::encode(sources);
        let s7 = codec::encode(&Packed {
            ulen: source_bytes.len() as u64,
            data: codec.compress(&source_bytes),
        });
        let sections: [(SectionId, &[u8]); 7] = [
            (SectionId::Interns, &s1),
            (SectionId::Summaries, &s2),
            (SectionId::Bodies, &s3),
            (SectionId::Baseline, baseline),
            (SectionId::Origins, &s5),
            (SectionId::Legacy, legacy),
            (SectionId::Sources, &s7),
        ];

        let header = codec::encode(&self.header);
        // The table's own size is fixed once its row count and field widths are known; offsets are
        // therefore computed with a table of the final size.
        let mut entries: Vec<SectionEntry> = sections
            .iter()
            .map(|(id, bytes)| SectionEntry {
                id: *id as u16,
                offset: 0,
                len: bytes.len() as u64,
                checksum: Hash128::of(bytes),
            })
            .collect();
        // postcard varints: an offset's encoded width depends on its value, so iterate until the
        // layout is a fixed point (it converges in one or two steps).
        let mut table = codec::encode(&entries);
        loop {
            let mut offset = (MAGIC.len() + 2 + 4 + header.len() + 4 + table.len()) as u64;
            for (entry, (_, bytes)) in entries.iter_mut().zip(sections.iter()) {
                entry.offset = offset;
                offset += bytes.len() as u64;
            }
            let next = codec::encode(&entries);
            if next.len() == table.len() {
                table = next;
                break;
            }
            table = next;
        }

        let member_offset = self.text.len() as u64;
        let mut out = self.text.into_bytes();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&(header.len() as u32).to_le_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(&(table.len() as u32).to_le_bytes());
        out.extend_from_slice(&table);
        for (_, bytes) in &sections {
            out.extend_from_slice(bytes);
        }
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&member_offset.to_le_bytes());
        out
    }
}

// -------------------------------------------------------------------------------------------
// Reading
// -------------------------------------------------------------------------------------------

/// A read object: the header, the eagerly decoded tables, and the compressed blob sections kept
/// for lazy decoding.
#[derive(Clone, Debug)]
pub struct Object {
    pub header: Header,
    pub interns: Interns,
    pub summaries: Vec<ItemSummary>,
    /// The S3 blob table.
    pub blobs: Vec<BlobEntry>,
    /// The text section. Empty once the reader has [`Object::take_text`]n it.
    pub text: String,
    /// The producer's S4 payload.
    pub baseline: Vec<u8>,
    /// The producer's S6 payload.
    pub legacy: Vec<u8>,
    /// The section table, for size reports.
    pub sections: Vec<SectionEntry>,
    bodies: Vec<u8>,
    origins: Vec<u8>,
    sources: Vec<u8>,
}

/// Whether `bytes` end in a format 20 trailer. Cheap; does not validate anything else.
pub fn is_object(bytes: &[u8]) -> bool {
    bytes.len() >= TRAILER_LEN && bytes[bytes.len() - TRAILER_LEN..].starts_with(&MAGIC)
}

/// Reads an object. `Ok(None)` for bytes with no trailer: something another producer wrote, or a
/// finished program. A trailer in another format is an error.
pub fn read(bytes: &[u8]) -> Result<Option<Object>, ObjectError> {
    if !is_object(bytes) {
        return Ok(None);
    }
    let trailer = &bytes[bytes.len() - TRAILER_LEN..];
    let format = u16::from_le_bytes([trailer[6], trailer[7]]);
    if format != FORMAT_VERSION {
        return Err(ObjectError::Format { found: format });
    }
    let mut offset_bytes = [0u8; 8];
    offset_bytes.copy_from_slice(&trailer[8..16]);
    let member_offset = u64::from_le_bytes(offset_bytes) as usize;
    if member_offset > bytes.len() - TRAILER_LEN {
        return Err(ObjectError::Corrupt(
            "trailer points past the end of the object".to_owned(),
        ));
    }
    let text = std::str::from_utf8(&bytes[..member_offset])
        .map_err(|err| ObjectError::Corrupt(format!("text section is not UTF-8: {err}")))?
        .to_owned();
    let member = &bytes[member_offset..bytes.len() - TRAILER_LEN];

    let mut cursor = Cursor {
        bytes: member,
        at: 0,
    };
    if cursor.take(MAGIC.len())? != MAGIC {
        return Err(ObjectError::Corrupt(
            "member does not start with the magic".to_owned(),
        ));
    }
    let member_format = cursor.u16()?;
    if member_format != FORMAT_VERSION {
        return Err(ObjectError::Format {
            found: member_format,
        });
    }
    let header_len = cursor.u32()? as usize;
    let header: Header = codec::decode(cursor.take(header_len)?, "object header")?;
    if header.jir_schema != JIR_SCHEMA {
        return Err(ObjectError::Schema {
            found: header.jir_schema,
        });
    }
    let table_len = cursor.u32()? as usize;
    let sections: Vec<SectionEntry> = codec::decode(cursor.take(table_len)?, "section table")?;

    let mut found: BTreeMap<u16, &[u8]> = BTreeMap::new();
    for entry in &sections {
        let start = entry.offset as usize;
        let end = start
            .checked_add(entry.len as usize)
            .filter(|end| *end <= member.len())
            .ok_or_else(|| {
                ObjectError::Corrupt(format!("section {} lies outside the member", entry.id))
            })?;
        let payload = &member[start..end];
        if Hash128::of(payload) != entry.checksum {
            let name = SectionId::from_id(entry.id).map_or("unknown section", SectionId::name);
            return Err(ObjectError::Corrupt(format!(
                "{name} (id {}) fails its checksum",
                entry.id
            )));
        }
        found.insert(entry.id, payload);
    }
    let section = |id: SectionId| -> Result<&[u8], ObjectError> {
        found
            .get(&(id as u16))
            .copied()
            .ok_or_else(|| ObjectError::Corrupt(format!("{} is missing", id.name())))
    };

    let packed: Packed = codec::decode(section(SectionId::Interns)?, "interns section")?;
    let interns_plain = header
        .codec
        .decompress(&packed.data, packed.ulen as usize)?;
    let mut interns: Interns = codec::decode(&interns_plain, "interns")?;
    interns.rebuild_indices();
    if !interns
        .coroutine_sources
        .values()
        .all(|source| source.valid(&interns))
    {
        return Err(ObjectError::Corrupt(
            "invalid coroutine source facts".to_owned(),
        ));
    }

    let summaries: Vec<ItemSummary> =
        codec::decode(section(SectionId::Summaries)?, "summary index")?;

    let s3 = section(SectionId::Bodies)?;
    let mut s3_cursor = Cursor { bytes: s3, at: 0 };
    let blob_table_len = s3_cursor.u32()? as usize;
    let blobs: Vec<BlobEntry> = codec::decode(s3_cursor.take(blob_table_len)?, "blob table")?;

    Ok(Some(Object {
        header,
        interns,
        summaries,
        blobs,
        text,
        baseline: section(SectionId::Baseline)?.to_vec(),
        legacy: section(SectionId::Legacy)?.to_vec(),
        sections,
        bodies: s3.to_vec(),
        origins: section(SectionId::Origins)?.to_vec(),
        sources: section(SectionId::Sources)?.to_vec(),
    }))
}

impl Object {
    /// Source text captured when this object was compiled, decoded only for map emission.
    pub fn sources(&self) -> Result<BTreeMap<String, String>, ObjectError> {
        let packed: Packed = codec::decode(&self.sources, "source text section")?;
        let bytes = self
            .header
            .codec
            .decompress(&packed.data, packed.ulen as usize)?;
        Ok(codec::decode(&bytes, "source texts")?)
    }

    /// The item's baseline JavaScript.
    pub fn text_of(&self, item: &ItemSummary) -> Result<&str, ObjectError> {
        let start = item.fallback.offset as usize;
        let end = start + item.fallback.clen as usize;
        self.text.get(start..end).ok_or_else(|| ObjectError::Item {
            name: item.key.link_name.clone(),
            message: "points outside the object body".to_owned(),
        })
    }

    /// Takes the text section out, for a reader that has sliced every item's JavaScript and does
    /// not want to keep a second copy alive beside them.
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    /// Inflates and decodes the item's body. Verified against the interns by the caller (or use
    /// [`Object::decode_verified`]).
    pub fn body(&self, item: &ItemSummary) -> Result<Option<Body>, ObjectError> {
        let Some(blob) = &item.body else {
            return Ok(None);
        };
        let bytes = self
            .inflate(&self.bodies, blob)
            .map_err(|err| ObjectError::Item {
                name: item.key.link_name.clone(),
                message: format!("body blob: {err}"),
            })?;
        if let Some(expected) = item.body_hash {
            let actual = Hash128::of(&bytes);
            if actual != expected {
                return Err(ObjectError::Item {
                    name: item.key.link_name.clone(),
                    message: format!("body blob hashes to {actual}, summary says {expected}"),
                });
            }
        }
        codec::decode_body(&bytes)
            .map(Some)
            .map_err(|err| ObjectError::Item {
                name: item.key.link_name.clone(),
                message: err.0,
            })
    }

    /// Inflates and decodes the item's origin table.
    pub fn origins(&self, item: &ItemSummary) -> Result<Option<OriginTable>, ObjectError> {
        let Some(blob) = &item.origin else {
            return Ok(None);
        };
        let bytes = self
            .inflate(&self.origins, blob)
            .map_err(|err| ObjectError::Item {
                name: item.key.link_name.clone(),
                message: format!("origin blob: {err}"),
            })?;
        codec::decode_origins(&bytes)
            .map(Some)
            .map_err(|err| ObjectError::Item {
                name: item.key.link_name.clone(),
                message: err.0,
            })
    }

    /// Decodes the item's body and origins and verifies both against the interns: the one entry
    /// point the link step uses, so that no decoded body escapes unverified.
    pub fn decode_verified(
        &self,
        item: &ItemSummary,
    ) -> Result<Option<(Body, OriginTable)>, ObjectError> {
        let Some(body) = self.body(item)? else {
            return Ok(None);
        };
        body.verify(&self.interns.types, &self.interns)
            .map_err(|err| ObjectError::Item {
                name: item.key.link_name.clone(),
                message: format!("body does not verify: {err}"),
            })?;
        let origins = self.origins(item)?.ok_or_else(|| ObjectError::Item {
            name: item.key.link_name.clone(),
            message: "has a body but no origin table".to_owned(),
        })?;
        body.verify_origins(&origins)
            .map_err(|err| ObjectError::Item {
                name: item.key.link_name.clone(),
                message: format!("origin table does not verify: {err}"),
            })?;
        Ok(Some((body, origins)))
    }

    fn inflate(&self, section: &[u8], blob: &BlobRef) -> Result<Vec<u8>, CodecError> {
        let start = blob.offset as usize;
        let end = start
            .checked_add(blob.clen as usize)
            .filter(|end| *end <= section.len())
            .ok_or_else(|| CodecError("blob reference lies outside its section".to_owned()))?;
        self.header
            .codec
            .decompress(&section[start..end], blob.ulen as usize)
    }

    /// The byte length of a section, `0` if absent.
    pub fn section_len(&self, id: SectionId) -> u64 {
        self.sections
            .iter()
            .find(|entry| entry.id == id as u16)
            .map_or(0, |entry| entry.len)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], ObjectError> {
        let end = self
            .at
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| ObjectError::Corrupt("member is truncated".to_owned()))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, ObjectError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, ObjectError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// A header for tests and tools that have no rustc session.
pub fn test_header(crate_name: &str) -> Header {
    Header {
        jir_schema: JIR_SCHEMA,
        rustc_version_hash: 0,
        data_layout: String::new(),
        emit_args: String::new(),
        crate_name: crate_name.to_owned(),
        codec: Codec::Deflate,
        hash: HashAlgorithm::SipHash13x128,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jir::origin::{Origin, OriginId, OriginKind};
    use crate::jir::semantic::*;
    use crate::jir::types::*;

    const UNIT: TypeId = TypeId(1);
    const I32: TypeId = TypeId(3);
    const REF_I32: TypeId = TypeId(6);

    fn scalar(id: u128, kind: TypeKind, size: u64) -> TypeDescriptor {
        let int = match kind {
            TypeKind::Integer { signed, bits } => Some(IntFacts {
                signed,
                bits,
                repr: IntRepr::Number,
            }),
            _ => None,
        };
        TypeDescriptor {
            id: TypeId(id),
            name: None,
            kind,
            layout: LayoutFacts {
                size: Some(size),
                unsized_prefix: None,
                align: Some(size.max(1)),
                sized: true,
                zst: size == 0,
                one_zst: size == 0,
                needs_drop: false,
                has_own_drop: false,
            },
            meta: TypeMeta::default(),
            js: JsFacts {
                int,
                is_indirect: size == 0,
                needs_clone: false,
                crosses_transparent_union: false,
                transparent: None,
                peeled: TypeId(id),
                unsized_tail: None,
                enum_repr: None,
                is_primitive_enum: false,
                pointer_form: None,
                byte_element: false,
                fields: Vec::new(),
            },
        }
    }

    fn interns() -> Interns {
        let mut interns = Interns::new();
        interns.intern_type(scalar(1, TypeKind::Tuple(Vec::new()), 0));
        interns.intern_type(scalar(
            3,
            TypeKind::Integer {
                signed: true,
                bits: 32,
            },
            4,
        ));
        let mut reference = scalar(6, TypeKind::Opaque, 4);
        reference.kind = TypeKind::Reference {
            pointee: I32,
            mutable: false,
        };
        interns.intern_type(reference);
        interns
    }

    fn item(n: u64) -> ItemId {
        ItemId {
            definition: DefinitionId {
                crate_id: 1,
                local_hash: n,
            },
            instantiation: n as u128,
        }
    }

    fn place(local: u32, ty: TypeId) -> Place {
        Place {
            local: LocalId(local),
            projection: Vec::new(),
            ty,
        }
    }

    /// `fn f(x: i32) -> i32 { x + 1 }`, with `n` as the item identity.
    fn scalar_body(n: u64, take_ref: bool) -> Body {
        let mut locals = vec![
            Local {
                id: LocalId(0),
                role: LocalRole::Return,
                ty: I32,
            },
            Local {
                id: LocalId(1),
                role: LocalRole::Argument,
                ty: I32,
            },
        ];
        let mut statements = vec![Statement {
            kind: StatementKind::Assign {
                destination: place(0, I32),
                value: Rvalue::Binary {
                    op: BinaryOp::Add,
                    left: Operand {
                        ty: I32,
                        kind: OperandKind::Copy(place(1, I32)),
                    },
                    right: Operand {
                        ty: I32,
                        kind: OperandKind::Constant(Constant::Scalar { bits: 1, bytes: 4 }),
                    },
                },
            },
        }];
        if take_ref {
            locals.push(Local {
                id: LocalId(2),
                role: LocalRole::Temporary,
                ty: REF_I32,
            });
            statements.push(Statement {
                kind: StatementKind::Assign {
                    destination: place(2, REF_I32),
                    value: Rvalue::Reference {
                        kind: BorrowKind::Shared,
                        place: place(1, I32),
                    },
                },
            });
        }
        Body {
            item: item(n),
            kind: BodyKind::Function,
            semantic: BodySemantic::Ordinary,
            signature: Signature {
                arguments: vec![I32],
                result: I32,
                abi: AbiSignature {
                    parameters: 1,
                    spread: None,
                    caller_location: false,
                    inline_never: false,
                    ..AbiSignature::default()
                },
            },
            locals,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                cleanup: false,
                statements,
                terminator: Terminator::Return,
            }],
        }
    }

    fn origins_for(body: &Body, interns: &mut Interns) -> OriginTable {
        let path = interns.intern_str("crate::f");
        let origin = Origin {
            kind: OriginKind::Statement,
            source: None,
            local_name: None,
            inline_stack: Vec::new(),
        };
        OriginTable {
            origins: vec![origin],
            statements: vec![OriginId(0); body.statement_count()],
            terminators: vec![OriginId(0); body.blocks.len()],
            locals: vec![OriginId(0); body.locals.len()],
            item_path: path,
            root: None,
        }
    }

    fn input<'a>(
        name: &str,
        deps: &[&str],
        body: Option<(&'a Body, &'a OriginTable)>,
    ) -> ItemInput<'a> {
        ItemInput {
            key: SemanticKey {
                link_name: name.to_owned(),
                item: body.map(|(body, _)| body.item),
            },
            availability: match body {
                Some(_) => Availability::Available,
                None => Availability::Unavailable(crate::jir::Decline::Encode(
                    crate::jir::decline::EncodeDecline::Synthesized,
                )),
            },
            deps: deps.iter().map(|dep| (*dep).to_owned()).collect(),
            kind: ItemKind::Fn,
            linkage: Linkage::internal(),
            order: None,
            dts: false,
            body,
        }
    }

    #[test]
    fn source_texts_survive_empty_objects() {
        let sources = BTreeMap::from([(
            "src/lib.rs".to_owned(),
            "pub fn generic<T>(x: T) -> T { x }\n".to_owned(),
        )]);
        let build =
            || Writer::new(test_header("generic"), "", Interns::new()).finish(b"", b"", &sources);
        let bytes = build();
        let object = read(&bytes).unwrap().unwrap();
        assert!(object.summaries.is_empty());
        assert_eq!(object.sources().unwrap(), sources);
        assert_eq!(bytes, build());
        let section = object
            .sections
            .iter()
            .find(|section| section.id == SectionId::Sources as u16)
            .unwrap();
        let mut corrupt = bytes.clone();
        corrupt[section.offset as usize] ^= 1;
        assert!(read(&corrupt)
            .unwrap_err()
            .to_string()
            .contains("S7 sources"));
    }

    fn sample() -> (Vec<u8>, Vec<Body>) {
        let mut interns = interns();
        let a = scalar_body(1, false);
        let b = scalar_body(2, true);
        // Same body as `a` under another name: one blob, two items.
        let c = scalar_body(1, false);
        let origins_a = origins_for(&a, &mut interns);
        let origins_b = origins_for(&b, &mut interns);
        let origins_c = origins_for(&c, &mut interns);
        let mut writer = Writer::new(test_header("crate"), "// prologue\n", interns);
        writer.push_item(
            "function a(x) {\n  return x + 1;\n}\n",
            input("a", &["x"], Some((&a, &origins_a))),
        );
        writer.push_item(
            "function b(x) {\n  return x + 1;\n}\n",
            input("b", &["a", "x"], Some((&b, &origins_b))),
        );
        writer.push_item(
            "function c(x) {\n  return x + 1;\n}\n",
            input("c", &["b"], Some((&c, &origins_c))),
        );
        writer.push_item("function d() {}\n", input("d", &[], None));
        (writer.finish(b"S4", b"S6", &BTreeMap::new()), vec![a, b, c])
    }

    #[test]
    fn round_trips_bodies_text_and_payloads() {
        let (bytes, bodies) = sample();
        assert!(is_object(&bytes));
        assert!(bytes.starts_with(b"// prologue\nfunction a(x)"));
        let object = read(&bytes).unwrap().unwrap();
        assert_eq!(object.header, test_header("crate"));
        assert_eq!(object.baseline, b"S4");
        assert_eq!(object.legacy, b"S6");
        assert_eq!(object.summaries.len(), 4);
        assert_eq!(object.blobs.len(), 2, "identical bodies share one blob");

        let names: Vec<&str> = object
            .summaries
            .iter()
            .map(|summary| summary.key.link_name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert_eq!(
            object.text_of(&object.summaries[1]).unwrap(),
            "function b(x) {\n  return x + 1;\n}\n"
        );
        assert_eq!(object.summaries[1].deps, ["a", "x"]);
        assert_eq!(object.summaries[1].estimated_bytes, 34);
        assert_eq!(object.summaries[0].body_hash, object.summaries[2].body_hash);
        assert_ne!(object.summaries[0].body_hash, object.summaries[1].body_hash);
        assert!(object.summaries[3].body.is_none());
        assert!(object.summaries[3].physical.is_none());
        assert!(!object.summaries[3].availability.is_available());
        assert_eq!(object.summaries[0].class, ItemClass::SCALAR_LEAF);
        assert_eq!(object.summaries[1].class, ItemClass::REFS);
        assert_eq!(
            object.summaries[0].abi,
            Some(AbiSignature {
                parameters: 1,
                spread: None,
                caller_location: false,
                inline_never: false,
                ..AbiSignature::default()
            })
        );

        for (summary, expected) in object.summaries.iter().zip(&bodies) {
            let (body, origins) = object.decode_verified(summary).unwrap().unwrap();
            assert_eq!(&body, expected);
            assert_eq!(origins.locals.len(), body.locals.len());
            let physical = summary.physical.as_ref().expect("a function summary");
            assert_eq!(
                physical.validate(
                    &body,
                    &object.interns.types,
                    summary::semantic_hash(&body, &object.interns)
                ),
                Ok(())
            );
        }
        assert!(object
            .decode_verified(&object.summaries[3])
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_corrupted_section_fails_its_checksum() {
        let (mut bytes, _) = sample();
        // Flip a byte inside the S3 blob bytes: past the text and well inside the member.
        let object = read(&bytes).unwrap().unwrap();
        let member_offset = bytes.len() - TRAILER_LEN;
        let member_offset = {
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&bytes[member_offset + 8..member_offset + 16]);
            u64::from_le_bytes(raw) as usize
        };
        let s3 = object
            .sections
            .iter()
            .find(|entry| entry.id == SectionId::Bodies as u16)
            .unwrap();
        let at = member_offset + s3.offset as usize + s3.len as usize - 1;
        bytes[at] ^= 0x55;
        let err = read(&bytes).unwrap_err();
        assert!(
            matches!(&err, ObjectError::Corrupt(message) if message.contains("S3 bodies")),
            "{err}"
        );
    }

    #[test]
    fn a_body_whose_hash_disagrees_with_its_summary_is_rejected() {
        let (bytes, _) = sample();
        let mut object = read(&bytes).unwrap().unwrap();
        object.summaries[0].body_hash = Some(Hash128::of(b"nope"));
        let err = object.decode_verified(&object.summaries[0]).unwrap_err();
        assert!(
            matches!(&err, ObjectError::Item { name, message } if name == "a" && message.contains("hashes to")),
            "{err}"
        );
    }

    #[test]
    fn another_format_is_a_rebuild_diagnostic() {
        let (mut bytes, _) = sample();
        let at = bytes.len() - TRAILER_LEN + 6;
        bytes[at..at + 2].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        let err = read(&bytes).unwrap_err();
        assert_eq!(
            err,
            ObjectError::Format {
                found: FORMAT_VERSION + 1
            }
        );
        let message = err.to_string();
        assert!(
            message.contains(&format!("format {}", FORMAT_VERSION + 1)),
            "{message}"
        );
        assert!(
            message.contains(&format!("reads format {FORMAT_VERSION}")),
            "{message}"
        );
        assert!(message.contains("build_sysroot.sh --clean"), "{message}");
    }

    #[test]
    fn text_without_a_trailer_is_not_an_object() {
        assert!(read(b"function f() {}\n").unwrap().is_none());
        assert!(read(b"").unwrap().is_none());
    }

    #[test]
    fn a_truncated_member_is_corrupt() {
        let (bytes, _) = sample();
        // Keep the trailer, drop the middle: the trailer's offset then points into a member that
        // no longer holds the sections it names.
        let mut cut = bytes[..bytes.len() - TRAILER_LEN - 40].to_vec();
        cut.extend_from_slice(&bytes[bytes.len() - TRAILER_LEN..]);
        assert!(matches!(read(&cut), Err(ObjectError::Corrupt(_))));
    }

    #[test]
    fn classifier_flags() {
        let interns = interns();
        let plain = scalar_body(1, false);
        let with_ref = scalar_body(2, true);
        assert_eq!(
            ItemClass::of(&plain, &interns.types),
            ItemClass::SCALAR_LEAF
        );
        assert_eq!(ItemClass::of(&with_ref, &interns.types), ItemClass::REFS);
        assert_eq!(ItemClass::SCALAR_LEAF.to_string(), "scalar-leaf");
        assert_eq!(
            ItemClass::REFS.union(ItemClass::DROPS).to_string(),
            "refs+drops"
        );
        assert_eq!(ItemClass::empty().to_string(), "-");

        let mut with_call = plain.clone();
        with_call.blocks[0].terminator = Terminator::Call(Call {
            function: Operand {
                ty: UNIT,
                kind: OperandKind::Constant(Constant::ZeroSized),
            },
            callee: Callee::Runtime {
                member: crate::jir::StrId(0),
                sysroot: true,
            },
            semantic: CallSemantic::Ordinary,
            arguments: Vec::new(),
            destination: place(0, I32),
            target: None,
            unwind: Unwind::Cleanup(BlockId(0)),
            track_caller: None,
            host: None,
        });
        assert_eq!(
            ItemClass::of(&with_call, &interns.types),
            ItemClass::HOST.union(ItemClass::UNWIND)
        );

        let mut static_body = plain.clone();
        static_body.kind = BodyKind::Static {
            init: ConstTree::Unit,
            mutable: false,
        };
        static_body.locals.clear();
        static_body.blocks.clear();
        assert_eq!(
            ItemClass::of(&static_body, &interns.types),
            ItemClass::STATIC
        );
    }
}
