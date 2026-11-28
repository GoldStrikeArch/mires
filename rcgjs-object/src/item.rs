//! The unit of linking: one named, self contained piece of JavaScript.
//!
//! Codegen no longer produces one string per codegen unit. It produces a [`JsItem`] per mono item,
//! carrying enough structure for the whole-program pass in `link.rs` to work out what is reachable
//! and what may therefore be dropped:
//!
//! * `refs` contains the item's free identifier uses, computed by
//!   [`crate::jsast::visit_free_idents`]. Lexically bound locals are excluded even when they share
//!   a name with another item. The linker follows names present in its item table and handles
//!   compiler stubs separately; ordinary JavaScript builtins do not create item edges.
//! * `zombies` are the things the backend could not lower. Recording rather than reporting them is
//!   the whole point: a zombie in an item nothing reaches costs nothing, and only a zombie the
//!   program can actually reach is an error. See [`Zombie`].
//! * `linkage` says what the item's name and visibility are: whether the name is fixed, whether the
//!   item is a root, and whether the module exports it. See [`Linkage`].
//!
//! # Object file format
//!
//! A codegen unit's "object file" is format 20 of [`crate::object`]: the concatenated
//! printed JavaScript of its items — the **text section**, byte for byte what v1 wrote — followed
//! by a binary member holding the crate-scope interns (S1), the eagerly read summary index (S2),
//! the content-addressed JIR bodies (S3), this module's baseline records (S4: source map, zombies,
//! debug path, TypeScript signature), the origin tables (S5), and the compatibility and report
//! facts the linker still consumes (S6: optimization info, late-IR summary, baseline shadow).
//! Every item's summary
//! stores the byte range of its JavaScript **into the text section** rather than a second copy of
//! the text, which keeps the member small and makes splicing at link time a slice. Spans do not
//! survive serialization (they are meaningless in another compilation session), so every zombie
//! carries a location string rendered when it was recorded.
//!
//! The reader still accepts the format 19 text footer (`//# rcgjs: <base64 postcard>`) during
//! Phase 1, so an rlib built before the member existed links — without JIR bodies, and with a note
//! saying so when the two formats meet in one program. Phase 2 deletes that path.
//!
//! The format is versioned because postcard is not self describing: a table written in an older
//! shape does not fail to parse, it parses into something else, and the program that comes out is
//! wrong rather than rejected. The version turns that into a diagnostic naming the rebuild that
//! fixes it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::jir::codec::Codec;
use crate::jir::hash::HashAlgorithm;
use serde::{Deserialize, Serialize};

use crate::jsast::{self, Loc, Mapping, Stmt};

pub use crate::jir::hash::Hash128;
pub use crate::jir::interns::Interns;
pub use crate::jir::origin::OriginTable;
pub use crate::jir::semantic::Body;
pub use crate::jir::summary::{Availability, ItemSummary, SemanticKey};
pub use crate::jir::Decline;
pub use crate::object::{
    ChunkTarget, Header, ItemInput, ItemKind, ItemMap, Linkage, Object, ObjectError, SourceOrder,
    Writer, FORMAT_VERSION,
};

/// The allocator implementation selected for a final linked artifact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum AllocatorProvider {
    #[default]
    Unknown,
    Default,
    Custom,
    Conflict,
}

impl AllocatorProvider {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Conflict, _) | (_, Self::Conflict) => Self::Conflict,
            (Self::Unknown, provider) | (provider, Self::Unknown) => provider,
            (left, right) if left == right => left,
            _ => Self::Conflict,
        }
    }
}

/// A JavaScript identifier the backend owns: the name of a top level item.
///
/// Distinct from a plain `String` so that the reachability pass cannot confuse an item name with,
/// say, a local. Construct one only through `naming.rs`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct JsName(String);

impl JsName {
    pub fn new(name: String) -> JsName {
        JsName(name)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::borrow::Borrow<str> for JsName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for JsName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JsName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a zombie came from, for grouping in reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ZombieKind {
    /// A construct the MIR walk cannot lower.
    Unsupported,
    /// A constant the backend cannot turn into a JavaScript value.
    Constant,
    /// A backend invariant that did not hold — a would-be ICE, reported instead of panicking.
    Internal,
}

impl ZombieKind {
    fn tag(self) -> u8 {
        match self {
            ZombieKind::Unsupported => 0,
            ZombieKind::Constant => 1,
            ZombieKind::Internal => 2,
        }
    }

    fn from_tag(tag: u8) -> Option<ZombieKind> {
        Some(match tag {
            0 => ZombieKind::Unsupported,
            1 => ZombieKind::Constant,
            2 => ZombieKind::Internal,
            _ => return None,
        })
    }
}

/// How the zombie of an item that owes cleanup but carries no JIR body begins: its baseline text
/// cannot run its cleanup paths. The producer records it and the linker keeps it through
/// duplicate resolution, so a reachable copy that owes cleanup fails the link rather than losing
/// its destructors to a copy lowered from MIR without cleanup.
pub const CLEANUP_LOWERING_REQUIRED: &str =
    "requires semantic cleanup lowering; baseline fallback would discard its cleanup paths";

/// Something the backend could not lower, standing in the emitted JavaScript as a runtime abort.
///
/// Nothing is reported when a zombie is recorded. `link.rs` reports the ones the program can
/// actually reach, which is why `loc` is rendered eagerly: by the time a zombie from another crate
/// is read back out of an rlib there is no source map that could render its span.
#[derive(Clone, Debug)]
pub struct Zombie {
    /// `file.rs:LINE:COL`, rendered when the zombie was recorded.
    pub loc: String,
    pub message: String,
    /// The span, kept while the item stays in this compilation session. Dropped on serialization,
    /// so it only ever distinguishes two zombies of one codegen run — which is what the baseline
    /// replay check compares (`baseline_shadow::same_zombies`).
    pub span: Option<SpanKey>,
    pub kind: ZombieKind,
}

/// A `rustc_span::SpanData`, as plain numbers: the two byte positions and the parent definition's
/// index.
///
/// The span type is rustc's and cannot cross into this crate, so the backend projects it. The
/// syntax context is left out because it is not reachable from outside `rustc_span`; two zombies
/// of one lowering that share a byte range and a parent share it too, and the only thing a span is
/// ever compared for here is telling two zombies of one codegen run apart.
pub type SpanKey = (u32, u32, Option<u32>);

/// One optimizer choice attached to an item.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptimizationSelection {
    pub pass: String,
    pub selected: String,
    pub reason: String,
}

/// One candidate an optimizer could not prove safe.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptimizationMiss {
    pub pass: String,
    pub reason: String,
    /// A rendered source location, when the candidate belongs to one.
    pub location: Option<String>,
    /// The rendered source file, classified against the workspace only by the final linker.
    pub source: Option<String>,
}

#[allow(
    dead_code,
    reason = "constructor reserved for optimizer report emitters"
)]
impl OptimizationMiss {
    pub fn new(pass: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            pass: pass.into(),
            reason: reason.into(),
            location: None,
            source: None,
        }
    }

    pub fn at_source(mut self, source: impl Into<String>, location: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self.location = Some(location.into());
        self
    }
}

/// Generic metadata shared by optimizer passes and serialized in object files.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OptimizationInfo {
    pub selections: Vec<OptimizationSelection>,
    pub misses: Vec<OptimizationMiss>,
    /// Passes with proof that their rewrite survived into this item's final JavaScript AST.
    pub applied: BTreeSet<String>,
}

impl OptimizationInfo {
    pub fn select(
        &mut self,
        pass: impl Into<String>,
        selected: impl Into<String>,
        reason: impl Into<String>,
    ) {
        self.selections.push(OptimizationSelection {
            pass: pass.into(),
            selected: selected.into(),
            reason: reason.into(),
        });
    }

    /// Records a rewrite that survived all item-local JavaScript passes.
    pub fn apply(&mut self, pass: impl Into<String>) {
        self.applied.insert(pass.into());
    }

    /// Unions optimizer facts from another copy of the same monomorphization.
    ///
    /// rustc may codegen one instantiation in more than one crate. The JavaScript bodies can be
    /// identical while the analyses that ran in those compilations found different candidates.
    /// Keep all of those facts, with a canonical order so archive traversal order cannot change
    /// the final report.
    pub fn merge(&mut self, other: &Self) {
        self.selections.extend(other.selections.iter().cloned());
        self.selections.sort_by(|a, b| {
            (&a.pass, &a.selected, &a.reason).cmp(&(&b.pass, &b.selected, &b.reason))
        });
        self.selections.dedup();

        self.misses.extend(other.misses.iter().cloned());
        self.misses.sort_by(|a, b| {
            (&a.pass, &a.source, &a.location, &a.reason).cmp(&(
                &b.pass,
                &b.source,
                &b.location,
                &b.reason,
            ))
        });
        self.misses.dedup();

        self.applied.extend(other.applied.iter().cloned());
    }
}

/// The JIR-S payload attached to a [`JsItem`] during codegen.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum JsItemJir {
    /// The item's encoded JIR-S body and origins, written to S3/S5.
    Encoded(Box<JirItem>),
    /// The encoder declined this item; `item` keeps its stable identity when known so direct
    /// calls and function pointers still resolve to it.
    Declined {
        item: Option<crate::jir::semantic::ItemId>,
        decline: Decline,
    },
    /// A synthesized item (allocator shim, import, hoisted constant) with no Rust identity.
    #[default]
    Synthesized,
}

impl From<JirItem> for JsItemJir {
    fn from(jir: JirItem) -> Self {
        Self::Encoded(Box::new(jir))
    }
}

impl JsItemJir {
    pub fn encoded(&self) -> Option<&JirItem> {
        match self {
            Self::Encoded(jir) => Some(jir),
            Self::Declined { .. } | Self::Synthesized => None,
        }
    }

    pub fn item_id(&self) -> Option<crate::jir::semantic::ItemId> {
        match self {
            Self::Encoded(jir) => Some(jir.body.item),
            Self::Declined { item, .. } => *item,
            Self::Synthesized => None,
        }
    }

    pub fn availability(&self) -> Availability {
        match self {
            Self::Encoded(_) => Availability::Available,
            Self::Declined { decline, .. } => Availability::Unavailable(decline.clone()),
            Self::Synthesized => Availability::Unavailable(Decline::Encode(
                crate::jir::decline::EncodeDecline::Synthesized,
            )),
        }
    }
}

/// One named top level declaration, plus everything the link step needs to know about it.
pub struct JsItem {
    pub name: JsName,
    pub kind: ItemKind,
    pub decl: Stmt,
    /// Every identifier the declaration mentions. Over-approximate; see the module docs.
    pub refs: BTreeSet<JsName>,
    pub zombies: Vec<Zombie>,
    pub linkage: Linkage,
    /// The Rust def path, for comments and for `required by` chains.
    pub debug_path: String,
    /// Where this item has to be declared, for the kinds whose order is observable. `None` for
    /// everything else, which is ordered by dependency and then by name. See [`SourceOrder`].
    pub order: Option<SourceOrder>,
    /// Where the item was declared, which is the location its header line maps to. `None` for a
    /// synthesized item (a vtable, an allocator shim) and whenever source maps are off.
    pub loc: Option<Loc>,
    /// Emitted JavaScript name to the Rust name behind it, for the map's `names` table.
    pub names: Vec<(String, String)>,
    /// The item's TypeScript signature, for `js-dts=on`. `Some` only for a function whose Rust
    /// signature and emitted parameter list agree; see the backend's `dts::signature`.
    pub dts: Option<crate::dts::Dts>,
    /// Decisions and rejected candidates collected by representation and lowering passes.
    pub optimization: OptimizationInfo,
    /// Allocator provider metadata carried on the allocator shim marker item.
    pub allocator_provider: AllocatorProvider,
    /// The item's JIR-S state (encoded body, encoder decline with optional identity, or synthesized).
    pub jir: JsItemJir,
}

/// A JIR-S body with its origin table, as the encoder produces it and the object stores it.
pub use crate::jir::JirItem;

impl JsItem {
    /// Builds an item, computing `refs` from the declaration.
    pub fn new(
        name: JsName,
        kind: ItemKind,
        decl: Stmt,
        zombies: Vec<Zombie>,
        linkage: Linkage,
        debug_path: String,
    ) -> JsItem {
        let mut refs = BTreeSet::new();
        jsast::visit_free_idents(&decl, &mut |ident| {
            if ident != name.as_str() {
                refs.insert(JsName::new(ident.to_owned()));
            }
        });
        JsItem {
            name,
            kind,
            decl,
            refs,
            zombies,
            linkage,
            debug_path,
            order: None,
            loc: None,
            names: Vec::new(),
            dts: None,
            optimization: OptimizationInfo::default(),
            allocator_provider: AllocatorProvider::Unknown,
            jir: JsItemJir::Synthesized,
        }
    }

    /// Merges link-level references discovered during JIR encoding into `self.refs`.
    pub fn with_extra_refs(mut self, extra: impl IntoIterator<Item = JsName>) -> Self {
        for dep in extra {
            if dep != self.name {
                self.refs.insert(dep);
            }
        }
        self
    }

    /// Attaches the item's JIR-S payload (encoded body, decline, or synthesized marker).
    pub fn with_jir(mut self, jir: JsItemJir) -> Self {
        self.jir = jir;
        self
    }

    /// Attaches the TypeScript signature `js-dts=on` writes into the `.d.ts`.
    pub fn with_dts(mut self, dts: Option<crate::dts::Dts>) -> JsItem {
        self.dts = dts;
        self
    }

    /// Fixes where this item is declared relative to the others that carry one. See
    /// [`SourceOrder`].
    pub fn with_order(mut self, order: SourceOrder) -> JsItem {
        self.order = Some(order);
        self
    }

    /// Attaches what the source map needs: where the item was declared and what its names mean.
    pub fn with_source(mut self, loc: Option<Loc>, names: Vec<(String, String)>) -> JsItem {
        if let Some(loc) = &loc {
            let location = format!("{}:{}:{}", loc.file, loc.line, loc.col + 1);
            for miss in &mut self.optimization.misses {
                if miss.location.is_none() {
                    *miss = miss.clone().at_source(loc.file.clone(), location.clone());
                }
            }
        }
        if crate::opts::get().source_map {
            self.loc = loc;
            self.names = names;
        }
        self
    }

    /// Attaches allocator provider identity (set on `__rust_no_alloc_shim_is_unstable`).
    pub fn with_allocator_provider(mut self, provider: AllocatorProvider) -> Self {
        self.allocator_provider = provider;
        self
    }

    /// The item's JavaScript and the source map entries for it, relative to its own first line.
    ///
    /// The mappings are always computed the same way; with source maps off the item carries no
    /// [`Stmt::Loc`] markers and no names, so the list comes out empty and the text is exactly what
    /// [`jsast::program_to_string`] would have printed.
    ///
    /// A hoisted constant gets no header comment even with comments on: its debug path *is* its
    /// value spelled again (`caller location file.rs:19:15` above `{ file: "file.rs", line: 19,
    /// column: 15 }`), so the comment would be pure duplication of the one thing this milestone
    /// exists to stop repeating. `emit-test.sh`'s filter already handles an item with no comment.
    pub fn to_js_mapped(&self, comments: bool) -> (String, Vec<Mapping>) {
        let comments = comments && self.kind != ItemKind::Const;
        let names: std::collections::BTreeMap<String, String> =
            self.names.iter().cloned().collect();
        let style = match crate::opts::get().minify.is_enabled() {
            true => jsast::Style::Compact,
            false => jsast::Style::Readable,
        };
        let target = crate::opts::get().ecma_target;
        if comments {
            let header = jsast::comment(self.debug_path.clone());
            jsast::program_refs_to_string_mapped_styled_for_target(
                &[&header, &self.decl],
                names,
                self.loc.clone(),
                style,
                target,
            )
        } else {
            jsast::program_to_string_mapped_styled_for_target(
                std::slice::from_ref(&self.decl),
                names,
                self.loc.clone(),
                style,
                target,
            )
        }
    }
}

/// Interns the sources and names of `mappings` into the compact [`ItemMap`] the object stores.
///
/// Public because the JIR lane builds a [`LinkItem`] out of a freshly lowered declaration at link
/// time and has to intern its mappings by exactly the rule the object writer used, or an item's
/// map would mean one thing on one lane and another on the other.
pub fn item_map(mappings: Vec<Mapping>) -> ItemMap {
    let mut map = ItemMap::default();
    for mapping in mappings {
        let source = intern(&mut map.sources, &mapping.loc.file);
        let name = match &mapping.name {
            Some(name) => intern(&mut map.names, name) as i32,
            None => -1,
        };
        map.segments.push((
            mapping.gen_line,
            mapping.gen_col,
            source,
            mapping.loc.line,
            mapping.loc.col,
            name,
        ));
    }
    map
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

// -------------------------------------------------------------------------------------------
// Object files
// -------------------------------------------------------------------------------------------

/// What every object of one compilation carries in its header, beyond the crate name.
///
/// Computed once from the session by the producer's `object_identity` and stored on the module so
/// that writing an object needs no `Session`.
#[derive(Clone, Debug, Default)]
pub struct ObjectIdentity {
    /// A digest of `sess.cfg_version`, the release line `rustc -vV` prints, which is what the
    /// sysroot cache keys toolchains by.
    pub rustc_version_hash: u64,
    pub data_layout: String,
    /// The emit-affecting `-Cllvm-args=js-*` options, normalized. See [`normalized_emit_args`].
    pub emit_args: String,
}

/// The emit-affecting options in canonical spelling: `key=value` pairs sorted by key, the last
/// value given for a key winning, everything else dropped.
///
/// The key list is `scripts/sysroot-cache.sh`'s `jsc_normalize_emit_args` allow-list; the two have
/// to agree, because this string is what the link step will hold every object in a program to and
/// that script is what decides which sysroot a set of options builds against.
pub fn normalized_emit_args(args: &[String]) -> String {
    const EMIT_AFFECTING: [&str; 13] = [
        "js-names",
        "js-structure",
        "js-queue",
        "js-switch",
        "js-scoped-lets",
        "js-minify",
        "js-ecma-target",
        "js-hoist-consts",
        "js-line-comments",
        "js-opt",
        "js-opt-passes",
        "js-jir",
        "js-cleanup",
    ];
    let mut by_key: BTreeMap<&str, &str> = BTreeMap::new();

    // Profiles are aliases, not independent emit options. Apply every profile first, exactly as
    // the option parser does, so an explicit option wins wherever it appeared in the argument
    // list. Unknown profiles remain in the key rather than sharing the default sysroot.
    for arg in args {
        let Some(value) = arg.strip_prefix("js-profile=") else {
            continue;
        };
        match crate::opts::Profile::parse(value) {
            Some(profile) => {
                for expanded in profile.emit_args() {
                    let (key, value) = expanded
                        .split_once('=')
                        .expect("profile emit arguments are key=value pairs");
                    by_key.insert(key, value);
                }
            }
            None => {
                by_key.insert("js-profile", value);
            }
        }
    }
    for arg in args {
        let (key, value) = match arg.split_once('=') {
            Some((key, value)) => (key, value),
            None => (arg.as_str(), ""),
        };
        if EMIT_AFFECTING.contains(&key) {
            by_key.insert(key, value);
        }
    }
    by_key
        .into_iter()
        .map(|(key, value)| match value.is_empty() {
            true => key.to_owned(),
            false => format!("{key}={value}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An item read back out of an object file: the same shape as a [`JsItem`], with the declaration
/// as already-printed text and the zombie spans flattened to strings.
#[derive(Clone, Debug)]
pub struct LinkItem {
    pub name: JsName,
    pub kind: ItemKind,
    /// The printed declaration, spliced verbatim into the output. Never re-parsed.
    pub js: String,
    pub refs: BTreeSet<JsName>,
    pub zombies: Vec<FlatZombie>,
    pub linkage: Linkage,
    pub debug_path: String,
    /// The Rust crate that emitted this item.
    pub crate_name: String,
    /// Where this item has to be declared, for the kinds whose order is observable. See
    /// [`SourceOrder`].
    pub order: Option<SourceOrder>,
    /// The source map entries for `js`, relative to its first line. Empty unless source maps were
    /// on when the object was written.
    pub map: ItemMap,
    /// The item's TypeScript signature, or `None` for one that has none.
    pub dts: Option<crate::dts::Dts>,
    /// Optimization metadata carried through rlibs into the final link.
    pub optimization: OptimizationInfo,
    /// Allocator provider metadata carried on the allocator shim marker item.
    pub allocator_provider: AllocatorProvider,
    /// The JIR half: the item's summary, and its body once the link step has decided the item is
    /// reachable and decoded it. See [`LinkJir`].
    pub jir: LinkJir,
}

/// The JIR side of a [`LinkItem`]: what the summary index said about the item, a handle on the
/// object it came from, and the body once decoded.
///
/// The body is **not** decoded when the object is read. `link.rs` calls [`LinkJir::decode`] for
/// the reachable items only, after the graph walk, so an unreachable body is never inflated. Until
/// then `body`/`origins` are `None` whether or not the item has one; [`LinkJir::has_body`] is what
/// says which.
#[derive(Clone, Debug, Default)]
pub struct LinkJir {
    /// The item's summary index entry. `None` for an item from a format 19 object or one
    /// synthesized by the link step.
    pub summary: Option<ItemSummary>,
    /// The object the item was read from, shared by every item of that object.
    object: Option<Arc<Object>>,
    linked_interns: Option<Arc<Interns>>,
    pub body: Option<Body>,
    pub origins: Option<OriginTable>,
}

impl LinkJir {
    /// A summary with no object behind it: what a test builds, and what an item synthesized by
    /// the link step would carry if it had a summary at all. Nothing can be decoded from it.
    pub fn from_summary(summary: ItemSummary) -> LinkJir {
        LinkJir {
            summary: Some(summary),
            object: None,
            linked_interns: None,
            body: None,
            origins: None,
        }
    }

    /// Whether the object holds a body for this item, decoded or not.
    pub fn has_body(&self) -> bool {
        self.summary
            .as_ref()
            .is_some_and(|summary| summary.body.is_some())
    }

    /// The interns the body refers into.
    pub fn interns(&self) -> Option<&Interns> {
        self.linked_interns
            .as_deref()
            .or_else(|| self.object.as_ref().map(|object| &object.interns))
    }

    /// The source object's compatibility identity, retained after link-time imports.
    pub fn header(&self) -> Option<&crate::object::Header> {
        self.object.as_ref().map(|object| &object.header)
    }

    /// Appended metadata for a link-time body rewrite. Original object IDs stay valid.
    pub fn use_interns(&mut self, interns: Interns) {
        self.linked_interns = Some(Arc::new(interns));
    }

    /// Shares an already-built link-time interns table across multiple items from the same object.
    pub fn use_shared_interns(&mut self, interns: Arc<Interns>) {
        self.linked_interns = Some(interns);
    }

    /// Inflates, decodes and verifies the body and origins. Returns whether a body was decoded:
    /// `Ok(false)` for an item that has none. Idempotent.
    pub fn decode(&mut self) -> Result<bool, String> {
        if self.body.is_some() {
            return Ok(true);
        }
        let (Some(summary), Some(object)) = (&self.summary, &self.object) else {
            return Ok(false);
        };
        match object.decode_verified(summary) {
            Ok(Some((mut body, origins))) => {
                if crate::opts::get().str_repr != crate::opts::StrRepr::Bytes {
                    body.forget_str_calls();
                }
                self.body = Some(body);
                self.origins = Some(origins);
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(err) => Err(err.to_string()),
        }
    }
}

impl LinkItem {
    /// Canonically merges physical summaries for two definitions with one semantic body.
    ///
    /// Equal summaries survive. Any disagreement or missing side becomes unavailable, so archive
    /// order cannot select an overclaim and the final planner declines until the inputs are
    /// rebuilt.
    pub fn merge_physical_summary(&mut self, duplicate: &Self) {
        let merged = match (
            self.jir
                .summary
                .as_ref()
                .and_then(|summary| summary.physical.as_ref()),
            duplicate
                .jir
                .summary
                .as_ref()
                .and_then(|summary| summary.physical.as_ref()),
        ) {
            (Some(left), Some(right)) if left == right => Some(left.clone()),
            _ => None,
        };
        if let Some(summary) = self.jir.summary.as_mut() {
            summary.physical = merged;
        }
    }

    /// The item's JavaScript with its header comment stripped.
    ///
    /// This is the form two definitions of the same instantiation are compared in. The comment
    /// holds a def path, and a def path is spelled *relative to the crate that printed it*: `core`
    /// codegens `<u32 as ops::bit::Shl>::shl` and a downstream crate codegens the very same body as
    /// `<u32 as core::ops::Shl>::shl`. That disagreement is about naming, not about code, and it
    /// would otherwise be reported as a miscompilation on every link against real `core`.
    pub fn code(&self) -> &str {
        let mut rest = self.js.as_str();
        loop {
            let trimmed = rest.trim_start_matches(['\n', '\r']);
            if !trimmed.starts_with("//") {
                return trimmed;
            }
            rest = match trimmed.find('\n') {
                Some(end) => &trimmed[end..],
                None => return "",
            };
        }
    }

    /// Merges analysis metadata from a duplicate definition (W4).
    ///
    /// The two items are one item, or two items the text optimizer proved interchangeable. Either
    /// way the *analyses* that produced their metadata ran in two compilation contexts and may
    /// have found different facts, so the union is what is kept. What is **not** kept is v1's
    /// degrade: v1 turned any disagreement between two bodies into
    /// `Unavailable(DUPLICATE_CONFLICT)` and an optimizer miss against the item, which is where
    /// every one of the FingerprintJS conflicts came from. v2 makes a deterministic choice
    /// instead (`merge_choosing`), and records it — where it records anything — as a
    /// `Decline::Merge(BodyChoice)` in the lane report rather than as a lost lane.
    ///
    /// Origins are unioned in the only sense they can be: an origin table is **positional**, one
    /// entry per statement of *its own* body, so two of them cannot be interleaved. The surviving
    /// body keeps its own table, and the diagnostics that are not positional — zombies, misses,
    /// selections, carrier bridges — are unioned by the merges above. That is what "origins take
    /// no part in equality" means on this side of the rule.
    ///
    /// The caller decides which side's JIR body survives; see
    /// [`crate::jir::summary::merge_duplicates`]. This only fills a gap: an item encoded in one
    /// crate and not in the other keeps the body that exists.
    pub fn merge_duplicate_metadata(&mut self, duplicate: &Self) {
        self.optimization.merge(&duplicate.optimization);
        self.allocator_provider = self.allocator_provider.merge(duplicate.allocator_provider);
        if !self.jir.has_body() && duplicate.jir.has_body() {
            self.jir = duplicate.jir.clone();
        }
    }
}

/// A zombie without its span.
#[derive(Clone, Debug)]
pub struct FlatZombie {
    pub loc: String,
    pub message: String,
    /// Round trips through the object file so that a report can group by cause, which the
    /// zombie-report golden test in a later milestone needs.
    #[allow(dead_code, reason = "retained for serialized zombie diagnostics")]
    pub kind: ZombieKind,
}

/// The banner every object file and every finished program starts with.
pub fn prologue(label: &str) -> String {
    format!("// rustc_codegen_js — {label}\n")
}

#[derive(Serialize, Deserialize)]
struct SerZombie {
    loc: String,
    message: String,
    kind: u8,
}

/// The per-item S4 record: metadata beyond the summary index and the text itself.
#[derive(Serialize, Deserialize)]
struct SerBaseline {
    zombies: Vec<SerZombie>,
    debug_path: String,
    map: ItemMap,
    dts: Option<crate::dts::Dts>,
    optimization: OptimizationInfo,
    allocator_provider: AllocatorProvider,
}

/// Renders a codegen unit's items as an object file: the printed JavaScript, then the member.
pub fn write_object(
    prologue: &str,
    items: &[JsItem],
    comments: bool,
    crate_name: &str,
    identity: &ObjectIdentity,
    interns: Interns,
    sources: &BTreeMap<String, String>,
) -> Vec<u8> {
    let header = crate::object::Header {
        jir_schema: crate::jir::JIR_SCHEMA,
        rustc_version_hash: identity.rustc_version_hash,
        data_layout: identity.data_layout.clone(),
        emit_args: identity.emit_args.clone(),
        crate_name: crate_name.to_owned(),
        codec: Codec::Deflate,
        hash: HashAlgorithm::SipHash13x128,
    };
    let mut writer = Writer::new(header, prologue, interns);
    let mut baseline = Vec::with_capacity(items.len());
    for item in items {
        let (js, mappings) = item.to_js_mapped(comments);
        let deps: Vec<String> = item.refs.iter().map(|r| r.as_str().to_owned()).collect();
        debug_assert!(deps.windows(2).all(|pair| pair[0] < pair[1]));
        let availability = item.jir.availability();
        writer.push_item(
            &js,
            ItemInput {
                key: SemanticKey {
                    link_name: item.name.as_str().to_owned(),
                    item: item.jir.item_id(),
                },
                availability,
                deps,
                kind: item.kind,
                linkage: item.linkage.clone(),
                order: item.order.clone(),
                dts: item.dts.is_some(),
                body: item.jir.encoded().map(|jir| (&jir.body, &jir.origins)),
            },
        );
        baseline.push(SerBaseline {
            zombies: item
                .zombies
                .iter()
                .map(|z| SerZombie {
                    loc: z.loc.clone(),
                    message: z.message.clone(),
                    kind: z.kind.tag(),
                })
                .collect(),
            debug_path: item.debug_path.clone(),
            map: item_map(mappings),
            dts: item.dts.clone(),
            optimization: item.optimization.clone(),
            allocator_provider: item.allocator_provider,
        });
    }
    let baseline = postcard::to_allocvec(&baseline).expect("S4 records are always serializable");
    writer.finish(&baseline, sources)
}

/// Renders items as a finished program: runnable JavaScript, no item table.
///
/// Returns the text and the line each item starts on, which is what rebases the item-relative
/// source map entries onto the program that came out.
pub fn write_program(prologue: &str, items: &[&LinkItem]) -> (String, Vec<u32>) {
    let mut out = String::from(prologue);
    let mut line = prologue.matches('\n').count() as u32;
    let mut starts = Vec::with_capacity(items.len());
    for item in items {
        starts.push(line);
        out.push_str(&item.js);
        line += item.js.matches('\n').count() as u32;
    }
    (out, starts)
}

/// What [`read_object`] found.
#[derive(Debug)]
pub struct ReadObject {
    pub items: Vec<LinkItem>,
    /// What every object in one program has to agree on, for
    /// [`crate::jir::summary::merge_headers`].
    pub header: Header,
    object: Arc<Object>,
}

impl ReadObject {
    /// Source contents retained independently of reachable JIR bodies.
    pub fn sources(&self) -> Result<BTreeMap<String, String>, String> {
        self.object.sources().map_err(|error| error.to_string())
    }

    /// Object metadata survives even when a generic-only crate has no emitted items.
    pub fn interns(&self) -> &Interns {
        &self.object.interns
    }
}

/// Reads an object file's items back, slicing each declaration out of the text section.
///
/// Returns `Ok(None)` for bytes with neither a format 20 trailer nor a format 19 footer — an
/// object some other backend wrote, rustc metadata, or one of ours that was already pruned down to
/// a finished program. An object this backend cannot read is an error rather than a `None`:
/// dropping every item of a stale rlib on the floor would report itself as a program missing half
/// of `core`.
///
/// Bodies are **not** decoded here; see [`LinkJir`].
pub fn read_object(bytes: &[u8]) -> Result<Option<ReadObject>, String> {
    if !crate::object::is_object(bytes) {
        return Ok(None);
    }
    let object = crate::object::read(bytes)
        .map_err(|err| err.to_string())?
        .expect("a trailer was found");
    read_object_20(object).map(Some)
}

fn read_object_20(mut object: Object) -> Result<ReadObject, String> {
    let baseline: Vec<SerBaseline> = postcard::from_bytes(&std::mem::take(&mut object.baseline))
        .map_err(|err| format!("corrupt rcgjs baseline section (postcard: {err})"))?;
    if baseline.len() != object.summaries.len() {
        return Err(format!(
            "corrupt rcgjs object: {} summaries, {} baseline records",
            object.summaries.len(),
            baseline.len(),
        ));
    }

    let mut texts = Vec::with_capacity(object.summaries.len());
    for summary in &object.summaries {
        texts.push(
            object
                .text_of(summary)
                .map_err(|err| err.to_string())?
                .to_owned(),
        );
    }
    let _ = object.take_text();
    let crate_name = object.header.crate_name.clone();
    let header = object.header.clone();
    let summaries = std::mem::take(&mut object.summaries);
    let object = Arc::new(object);

    let mut items = Vec::with_capacity(summaries.len());
    for ((summary, js), record) in summaries.into_iter().zip(texts).zip(baseline) {
        let name = summary.key.link_name.clone();
        let zombies = flat_zombies(&name, record.zombies)?;
        items.push(LinkItem {
            name: JsName::new(name),
            kind: summary.kind,
            js,
            refs: summary.deps.iter().cloned().map(JsName::new).collect(),
            zombies,
            linkage: summary.linkage.clone(),
            debug_path: record.debug_path,
            crate_name: crate_name.clone(),
            order: summary.order.clone(),
            map: record.map,
            dts: record.dts,
            optimization: record.optimization,
            allocator_provider: record.allocator_provider,
            jir: LinkJir {
                summary: Some(summary),
                object: Some(Arc::clone(&object)),
                linked_interns: None,
                body: None,
                origins: None,
            },
        });
    }
    Ok(ReadObject {
        items,
        header,
        object,
    })
}

fn flat_zombies(name: &str, zombies: Vec<SerZombie>) -> Result<Vec<FlatZombie>, String> {
    let mut out = Vec::with_capacity(zombies.len());
    for z in zombies {
        let kind = ZombieKind::from_tag(z.kind)
            .ok_or_else(|| format!("rcgjs item `{name}` has an unknown zombie kind"))?;
        out.push(FlatZombie {
            loc: z.loc,
            message: z.message,
            kind,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link_item(pass: &str) -> LinkItem {
        let mut optimization = OptimizationInfo::default();
        optimization.select(pass, "selected", "because");
        LinkItem {
            name: JsName::new("f".to_owned()),
            kind: ItemKind::Fn,
            js: "function f() {}\n".to_owned(),
            refs: BTreeSet::new(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: "crate::f".to_owned(),
            crate_name: "crate".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization,
            allocator_provider: AllocatorProvider::Unknown,
            jir: Default::default(),
        }
    }

    fn write(items: &[JsItem]) -> Vec<u8> {
        write_object(
            "",
            items,
            false,
            "crate",
            &ObjectIdentity::default(),
            Interns::new(),
            &BTreeMap::new(),
        )
    }

    fn read(bytes: &[u8]) -> Result<Option<Vec<LinkItem>>, String> {
        read_object(bytes).map(|read| read.map(|read| read.items))
    }

    fn object(provider: AllocatorProvider) -> Vec<u8> {
        let item = JsItem::new(
            JsName::new("f".to_owned()),
            ItemKind::Fn,
            jsast::function("f", Vec::new(), Vec::new()),
            Vec::new(),
            Linkage::internal(),
            "crate::f".to_owned(),
        )
        .with_allocator_provider(provider);
        write(&[item])
    }

    #[test]
    fn allocator_provider_round_trips_without_changing_program_bytes() {
        let first = object(AllocatorProvider::Default);
        let second = object(AllocatorProvider::Custom);
        assert_ne!(first, second);

        let first = read(&first).unwrap().unwrap();
        let second = read(&second).unwrap().unwrap();
        assert_eq!(first[0].allocator_provider, AllocatorProvider::Default);
        assert_eq!(second[0].allocator_provider, AllocatorProvider::Custom);
        let first_refs = first.iter().collect::<Vec<_>>();
        let second_refs = second.iter().collect::<Vec<_>>();
        assert_eq!(
            write_program("", &first_refs).0,
            write_program("", &second_refs).0
        );
    }

    #[test]
    fn object_reader_rejects_a_stale_format_with_the_rebuild_note() {
        let mut object = object(AllocatorProvider::Unknown);
        let at = object.len() - 10;
        object[at..at + 2].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        let error = read_object(&object).unwrap_err();
        assert!(
            error.contains(&format!("format {}", FORMAT_VERSION + 1)),
            "{error}"
        );
        assert!(error.contains("build_sysroot.sh --clean"), "{error}");
    }

    #[test]
    fn text_that_is_neither_format_is_not_an_object() {
        assert!(read(b"function f() {}\n").unwrap().is_none());
        assert!(read(&[0xff, 0xfe, 0x00, 0x01]).unwrap().is_none());
    }

    #[test]
    fn the_object_starts_with_the_printed_text_and_ends_with_the_trailer() {
        let object = object(AllocatorProvider::Unknown);
        assert!(object.starts_with(b"function f() {"), "{:?}", &object[..20]);
        assert!(crate::object::is_object(&object));
        let items = read(&object).unwrap().unwrap();
        assert_eq!(items[0].js, "function f() {}\n");
        assert_eq!(items[0].crate_name, "crate");
        let summary = items[0].jir.summary.as_ref().unwrap();
        assert_eq!(summary.key.link_name, "f");
        assert_eq!(summary.key.item, None);
        assert!(!summary.availability.is_available());
        assert_eq!(summary.estimated_bytes, 16);
        assert_eq!(summary.deps, Vec::<String>::new());
    }

    #[test]
    fn a_jir_body_round_trips_and_is_decoded_lazily() {
        let mut interns = Interns::new();
        let (body, origins) = crate::item::tests::jir::sample(&mut interns);
        let item = JsItem::new(
            JsName::new("f".to_owned()),
            ItemKind::Fn,
            jsast::function("f", Vec::new(), Vec::new()),
            Vec::new(),
            Linkage::internal(),
            "crate::f".to_owned(),
        )
        .with_jir(JsItemJir::from(JirItem {
            body: body.clone(),
            origins: origins.clone(),
        }));
        let bytes = write_object(
            "",
            &[item],
            false,
            "crate",
            &ObjectIdentity::default(),
            interns,
            &BTreeMap::new(),
        );
        let mut items = read(&bytes).unwrap().unwrap();
        let item = &mut items[0];
        let summary = item.jir.summary.as_ref().unwrap();
        assert!(summary.availability.is_available());
        assert_eq!(summary.key.item, Some(body.item));
        assert!(summary.body_hash.is_some());
        assert!(summary.physical.is_some());
        assert!(item.jir.has_body());
        assert!(item.jir.body.is_none(), "not decoded until asked");
        assert_eq!(item.jir.decode(), Ok(true));
        assert_eq!(item.jir.body.as_ref(), Some(&body));
        assert_eq!(item.jir.origins.as_ref(), Some(&origins));
        assert_eq!(item.jir.decode(), Ok(true), "idempotent");

        let mut duplicate = item.clone();
        let physical = item
            .jir
            .summary
            .as_ref()
            .and_then(|summary| summary.physical.clone())
            .expect("a physical summary");
        item.merge_physical_summary(&duplicate);
        assert_eq!(
            item.jir
                .summary
                .as_ref()
                .and_then(|summary| summary.physical.as_ref()),
            Some(&physical),
            "equal duplicate summaries survive"
        );
        duplicate
            .jir
            .summary
            .as_mut()
            .and_then(|summary| summary.physical.as_mut())
            .expect("a physical summary")
            .format += 1;
        item.merge_physical_summary(&duplicate);
        assert!(
            item.jir
                .summary
                .as_ref()
                .is_some_and(|summary| summary.physical.is_none()),
            "disagreement declines canonically"
        );
    }

    #[test]
    fn emit_args_are_normalized_by_key_with_the_last_value_winning() {
        let args: Vec<String> = [
            "js-minify=on",
            "js-root=main",
            "js-opt=report",
            "js-minify=locals",
            "js-scoped-lets",
            "js-chunk=a:b",
            "js-cleanup=off",
            "js-cleanup=on",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(
            normalized_emit_args(&args),
            "js-cleanup=on js-minify=locals js-opt=report js-scoped-lets"
        );
        assert_eq!(normalized_emit_args(&[]), "");
    }

    #[test]
    fn emit_args_expand_profiles_before_explicit_overrides() {
        let args = [
            "js-minify=off".to_owned(),
            "js-profile=release-speed".to_owned(),
            "js-opt-passes=sroa".to_owned(),
        ];
        assert_eq!(
            normalized_emit_args(&args),
            "js-cleanup=on js-ecma-target=2018 js-minify=off js-opt=on js-opt-passes=sroa"
        );

        let unknown = ["js-profile=future".to_owned()];
        assert_eq!(normalized_emit_args(&unknown), "js-profile=future");
    }

    #[test]
    fn identical_duplicates_union_optimizer_metadata_canonically() {
        let mut first = link_item("z-pass");
        first.optimization.apply("future-poller");
        let mut forward = first.clone();
        let mut second = link_item("a-pass");
        second.optimization.apply("ast-dce");
        let mut reverse = second.clone();

        forward.merge_duplicate_metadata(&second);
        reverse.merge_duplicate_metadata(&first);

        assert_eq!(forward.optimization, reverse.optimization);
        assert_eq!(forward.optimization.selections.len(), 2);
        assert_eq!(forward.optimization.selections[0].pass, "a-pass");
        assert_eq!(forward.optimization.selections[1].pass, "z-pass");
        assert_eq!(
            forward.optimization.applied,
            BTreeSet::from(["ast-dce".to_owned(), "future-poller".to_owned()])
        );
    }

    #[test]
    fn identical_duplicates_retain_and_deduplicate_rejected_source_candidates() {
        let mut first = link_item("shared-pass");
        first.optimization.misses.push(
            OptimizationMiss::new("z-pass", "z reason").at_source("src/lib.rs", "src/lib.rs:9:5"),
        );
        let mut second = link_item("shared-pass");
        second.optimization.misses.push(
            OptimizationMiss::new("a-pass", "a reason").at_source("src/lib.rs", "src/lib.rs:9:5"),
        );

        let mut forward = first.clone();
        forward.merge_duplicate_metadata(&second);
        forward.merge_duplicate_metadata(&second);
        let mut reverse = second.clone();
        reverse.merge_duplicate_metadata(&first);

        assert_eq!(forward.optimization, reverse.optimization);
        assert_eq!(forward.optimization.selections.len(), 1);
        assert_eq!(forward.optimization.misses.len(), 2);
        assert_eq!(forward.optimization.misses[0].pass, "a-pass");
        assert_eq!(forward.optimization.misses[1].pass, "z-pass");
    }

    #[test]
    fn js_name_borrows_str_and_to_js_mapped_prints_comment_without_cloning_decl() {
        let mut map = BTreeMap::new();
        map.insert(JsName::new("alpha".to_owned()), 1u32);
        assert_eq!(map.get("alpha"), Some(&1));
        let name = JsName::new("beta".to_owned());
        let s: &str = name.as_ref();
        assert_eq!(s, "beta");

        let item = JsItem::new(
            JsName::new("f".to_owned()),
            ItemKind::Fn,
            jsast::function("f", Vec::new(), vec![jsast::ret(jsast::num(1))]),
            Vec::new(),
            Linkage::internal(),
            "crate::f".to_owned(),
        );
        let (with_comment, _) = item.to_js_mapped(true);
        assert_eq!(
            with_comment,
            "// crate::f\nfunction f() {\n  return 1;\n}\n"
        );
        let (without_comment, _) = item.to_js_mapped(false);
        assert_eq!(without_comment, "function f() {\n  return 1;\n}\n");
    }

    /// A hand-built JIR body for the object tests: `fn f(x: i32) -> i32 { x + 1 }`.
    pub mod jir {
        use crate::jir::interns::Interns;
        use crate::jir::origin::{Origin, OriginId, OriginKind, OriginTable};
        use crate::jir::semantic::*;
        use crate::jir::types::*;

        const I32: TypeId = TypeId(3);

        fn i32_descriptor() -> TypeDescriptor {
            TypeDescriptor {
                id: I32,
                name: None,
                kind: TypeKind::Integer {
                    signed: true,
                    bits: 32,
                },
                layout: LayoutFacts {
                    size: Some(4),
                    unsized_prefix: None,
                    align: Some(4),
                    sized: true,
                    zst: false,
                    one_zst: false,
                    needs_drop: false,
                    has_own_drop: false,
                },
                meta: TypeMeta::default(),
                js: JsFacts {
                    int: Some(IntFacts {
                        signed: true,
                        bits: 32,
                        repr: IntRepr::Number,
                    }),
                    is_indirect: false,
                    needs_clone: false,
                    crosses_transparent_union: false,
                    transparent: None,
                    peeled: I32,
                    unsized_tail: None,
                    enum_repr: None,
                    is_primitive_enum: false,
                    pointer_form: None,
                    byte_element: false,
                    fields: Vec::new(),
                },
            }
        }

        fn place(local: u32) -> Place {
            Place {
                local: LocalId(local),
                projection: Vec::new(),
                ty: I32,
            }
        }

        pub fn sample(interns: &mut Interns) -> (Body, OriginTable) {
            interns.intern_type(i32_descriptor());
            let body = Body {
                item: ItemId {
                    definition: DefinitionId {
                        crate_id: 1,
                        local_hash: 7,
                    },
                    instantiation: 7,
                },
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
                locals: vec![
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
                ],
                blocks: vec![BasicBlock {
                    id: BlockId(0),
                    cleanup: false,
                    statements: vec![Statement {
                        kind: StatementKind::Assign {
                            destination: place(0),
                            value: Rvalue::Binary {
                                op: BinaryOp::Add,
                                left: Operand {
                                    ty: I32,
                                    kind: OperandKind::Copy(place(1)),
                                },
                                right: Operand {
                                    ty: I32,
                                    kind: OperandKind::Constant(Constant::Scalar {
                                        bits: 1,
                                        bytes: 4,
                                    }),
                                },
                            },
                        },
                    }],
                    terminator: Terminator::Return,
                }],
            };
            let origins = OriginTable {
                origins: vec![Origin {
                    kind: OriginKind::Statement,
                    source: None,
                    local_name: None,
                    inline_stack: Vec::new(),
                }],
                statements: vec![OriginId(0)],
                terminators: vec![OriginId(0)],
                locals: vec![OriginId(0), OriginId(0)],
                item_path: interns.intern_str("crate::f"),
                root: None,
            };
            (body, origins)
        }
    }
}
