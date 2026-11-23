//! Versioned, owned MIR used by whole-program JavaScript optimization.
//!
//! The executable body contains stable semantic identities and no source paths or spans. Those
//! live in a separate diagnostics table, so two crates can merge the same monomorphic body even
//! when rustc describes it through different paths. Encoding is deliberately conservative: one
//! unsupported operation makes the whole item unavailable and keeps its printed JavaScript as the
//! executable fallback.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::jir::JirItem;

/// The independent schema of [`LateIrAnalysisSummary`].
///
/// The containing object format is versioned too. A clean rebuild is required when this changes.
pub const LATE_IR_ANALYSIS_VERSION: u16 = 11;

pub const DUPLICATE_CONFLICT: &str =
    "identical JavaScript definitions carried conflicting late-IR bodies";

/// The largest anonymous constant allocation decoded into an owned constant tree.
pub const MAX_CONST_MEMORY_BYTES: u64 = 65536;

/// The deepest pointer chain followed while decoding constant memory.
pub const MAX_CONST_TREE_DEPTH: u32 = 64;
pub const SYNTHESIZED_ITEM: &str = "the item is synthesized and has no typed MIR body";

/// An owned late-IR body or an explicit reason to retain the printed JavaScript body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LateIrAnalysisSummary {
    pub version: u16,
    pub availability: Availability,
    pub carrier_bridges: Vec<CarrierBridge>,
    pub diagnostics: DiagnosticInfo,
    pub allocator_provider: AllocatorProvider,
    /// The item's JIR-S body and origins (`rcgjs-object` schema), when it encoded. Independent of
    /// `availability`, whose schema-11 body remains a compatibility and report summary.
    pub jir: Option<JirItem>,
    /// Why `jir` is `None`: always an `Encode(..)` decline.
    pub jir_decline: Option<crate::jir::Decline>,
}

impl LateIrAnalysisSummary {
    /// A synthesized item or other item for which there is no body to encode.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            version: LATE_IR_ANALYSIS_VERSION,
            availability: Availability::Unavailable(reason.into()),
            carrier_bridges: Vec::new(),
            diagnostics: DiagnosticInfo::default(),
            allocator_provider: AllocatorProvider::Unknown,
            jir: None,
            jir_decline: None,
        }
    }

    pub fn allocator_provider(provider: AllocatorProvider) -> Self {
        Self {
            allocator_provider: provider,
            ..Self::default()
        }
    }

    /// Validates the schema version and every local, type, and CFG reference.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != LATE_IR_ANALYSIS_VERSION {
            return Err(format!(
                "late-IR analysis summary has format {}, but this backend reads format {}",
                self.version, LATE_IR_ANALYSIS_VERSION
            ));
        }
        match &self.availability {
            Availability::Available(body) => body.validate(),
            Availability::Semantic(_, reason) if reason.is_empty() => {
                Err("semantic late-IR header has no body fallback reason".to_owned())
            }
            Availability::Semantic(body, _) => body.validate_semantic_header(),
            Availability::Unavailable(reason) if reason.is_empty() => {
                Err("unavailable late-IR body has no fallback reason".to_owned())
            }
            Availability::Unavailable(_) => Ok(()),
        }?;
        if self
            .carrier_bridges
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err("late-IR carrier bridges are not strictly sorted".to_owned());
        }
        self.carrier_bridges
            .iter()
            .try_for_each(CarrierBridge::validate)
    }

    /// Serializes one validated body using the object-file codec.
    #[allow(dead_code)]
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        postcard::to_allocvec(self).map_err(|error| error.to_string())
    }

    /// Decodes and validates one body before it can reach whole-program analysis.
    #[allow(dead_code)]
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let body: Self = postcard::from_bytes(bytes).map_err(|error| error.to_string())?;
        body.validate()?;
        Ok(body)
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn status(&self) -> &'static str {
        match self.availability {
            Availability::Available(_) => "available",
            Availability::Semantic(..) => "semantic",
            Availability::Unavailable(_) => "unavailable",
        }
    }

    pub fn body_kind(&self) -> Option<&'static str> {
        self.body().map(|body| body.kind.as_str())
    }

    pub fn fallback_reason(&self) -> Option<&str> {
        match &self.availability {
            Availability::Available(_) => None,
            Availability::Semantic(_, reason) => Some(reason),
            Availability::Unavailable(reason) => Some(reason),
        }
    }

    pub fn artifact_allocator_provider(&self) -> AllocatorProvider {
        self.allocator_provider
    }

    /// Exact producer and consumer boundaries for a String carried through a fallback body.
    pub fn carrier_bridges(&self) -> &[CarrierBridge] {
        &self.carrier_bridges
    }

    /// The executable body. Consumers must still honor [`Self::validate`].
    pub fn body(&self) -> Option<&LateBody> {
        match &self.availability {
            Availability::Available(body) | Availability::Semantic(body, _) => Some(body),
            Availability::Unavailable(_) => None,
        }
    }

    /// Source metadata for warnings and reports.
    pub fn diagnostics(&self) -> &DiagnosticInfo {
        &self.diagnostics
    }

    pub fn counts(&self) -> LateIrFactCounts {
        let Some(body) = self.body() else {
            return LateIrFactCounts::default();
        };
        let mut counts = LateIrFactCounts {
            locals: body.locals.len(),
            ..Default::default()
        };
        for block in &body.blocks {
            counts.allocations += block
                .statements
                .iter()
                .filter(|statement| {
                    matches!(
                        statement.kind,
                        StatementKind::Assign {
                            value: Rvalue::Aggregate { .. },
                            ..
                        }
                    )
                })
                .count();
            match &block.terminator {
                Terminator::Call(call) => {
                    counts.calls += 1;
                    counts.direct_calls +=
                        usize::from(matches!(call.callee, Callee::Direct { .. }));
                }
                Terminator::Drop { .. } => counts.drops += 1,
                _ => {}
            }
        }
        counts
    }

    /// Conservatively combines bodies attached to identical duplicate definitions.
    ///
    /// Diagnostics never decide semantic equality. Their sorted union is retained for reporting.
    /// The operation is commutative and associative, so archive traversal order does not matter.
    pub fn merge(&mut self, other: &Self) -> bool {
        self.diagnostics.merge(&other.diagnostics);
        self.allocator_provider = self.allocator_provider.merge(other.allocator_provider);
        // The JIR-S lane: keep a body over none. Two present bodies are W4's identity rule
        // (`SemanticKey` + normalized body hash), not this merge's; the first is kept.
        if self.jir.is_none() && other.jir.is_some() {
            self.jir = other.jir.clone();
            self.jir_decline = other.jir_decline.clone();
        }
        if self.version == other.version {
            self.carrier_bridges
                .retain(|bridge| other.carrier_bridges.binary_search(bridge).is_ok());
        } else {
            self.carrier_bridges.clear();
        }
        if self.version != other.version
            || self.is_duplicate_conflict()
            || other.is_duplicate_conflict()
        {
            self.version = LATE_IR_ANALYSIS_VERSION;
            self.availability = Availability::Unavailable(DUPLICATE_CONFLICT.to_owned());
            return true;
        }
        if self.availability == other.availability {
            return false;
        }

        match (&self.availability, &other.availability) {
            (Availability::Available(_), Availability::Available(_)) => {
                self.availability = Availability::Unavailable(DUPLICATE_CONFLICT.to_owned());
                true
            }
            (Availability::Available(available), Availability::Semantic(semantic, _)) => {
                if available.has_semantic_header(semantic) {
                    false
                } else {
                    self.availability = Availability::Unavailable(DUPLICATE_CONFLICT.to_owned());
                    true
                }
            }
            (Availability::Semantic(semantic, _), Availability::Available(available)) => {
                if available.has_semantic_header(semantic) {
                    self.availability = other.availability.clone();
                    false
                } else {
                    self.availability = Availability::Unavailable(DUPLICATE_CONFLICT.to_owned());
                    true
                }
            }
            (
                Availability::Semantic(left_body, left_reason),
                Availability::Semantic(right_body, right_reason),
            ) => {
                if left_body == right_body {
                    if right_reason < left_reason {
                        self.availability = other.availability.clone();
                    }
                    false
                } else {
                    self.availability = Availability::Unavailable(DUPLICATE_CONFLICT.to_owned());
                    true
                }
            }
            (Availability::Unavailable(_), Availability::Available(_))
            | (Availability::Unavailable(_), Availability::Semantic(..)) => {
                self.availability = other.availability.clone();
                false
            }
            (Availability::Available(_), Availability::Unavailable(_))
            | (Availability::Semantic(..), Availability::Unavailable(_)) => false,
            (Availability::Unavailable(left), Availability::Unavailable(right)) => {
                if right < left {
                    self.availability = other.availability.clone();
                }
                false
            }
        }
    }

    /// v2's duplicate rule for this frozen schema-11 summary: a body difference is a
    /// deterministic **choice**, never a loss of availability.
    ///
    /// [`Self::merge`] is v1's, kept verbatim, and it degrades to
    /// [`DUPLICATE_CONFLICT`] whenever two duplicates carry different bodies. That degrade is
    /// where all 74 of the FingerprintJS conflicts came from (`JIR.md`): almost every one of them
    /// is a *text* dedup merge of two different monomorphizations, whose bodies differ for good
    /// reason. So this runs v1's merge for its diagnostics, carrier bridges and allocator
    /// provider, and then — when the only thing that conflicted was the body — replaces the
    /// degrade with the smaller of the two availabilities by encoded bytes. Smaller by bytes
    /// rather than "the first one read", so the outcome does not depend on archive order.
    ///
    /// A **version** mismatch keeps v1's degrade: two schema versions in one program is a stale
    /// build, and the reader's `validate` already reports it.
    ///
    /// Returns whether a choice had to be made.
    pub fn merge_choosing(&mut self, other: &Self) -> bool {
        let versions_agree = self.version == other.version;
        let mine = self.availability.clone();
        let theirs = other.availability.clone();
        if !self.merge(other) {
            return false;
        }
        if !versions_agree {
            return true;
        }
        self.availability = match (availability_key(&mine), availability_key(&theirs)) {
            (Some(left), Some(right)) if right < left => theirs,
            (Some(_), Some(_)) => mine,
            // Unencodable is not a state the writer can produce; keep v1's answer rather than
            // guess, so the degrade is still visible if it ever happens.
            _ => return true,
        };
        true
    }

    fn is_duplicate_conflict(&self) -> bool {
        matches!(
            &self.availability,
            Availability::Unavailable(reason) if reason == DUPLICATE_CONFLICT
        )
    }

    pub fn with_version_for_test(mut self, version: u16) -> Self {
        self.version = version;
        self
    }

    pub fn from_body_for_test(body: LateBody) -> Self {
        Self {
            version: LATE_IR_ANALYSIS_VERSION,
            availability: Availability::Available(body),
            carrier_bridges: Vec::new(),
            diagnostics: DiagnosticInfo::default(),
            allocator_provider: AllocatorProvider::Unknown,
            jir: None,
            jir_decline: None,
        }
    }

    pub fn with_carrier_bridges_for_test(mut self, bridges: Vec<CarrierBridge>) -> Self {
        self.carrier_bridges = bridges;
        self
    }
}

/// The allocator implementation selected for a final linked artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AllocatorProvider {
    /// Append-only: postcard serializes this enum by index.
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

impl Default for LateIrAnalysisSummary {
    fn default() -> Self {
        Self::unavailable(SYNTHESIZED_ITEM)
    }
}

/// Counts surfaced in the deterministic optimizer report.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LateIrFactCounts {
    pub locals: usize,
    pub allocations: usize,
    pub calls: usize,
    pub direct_calls: usize,
    pub drops: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Availability {
    /// Append-only: postcard serializes this enum by index.
    Available(LateBody),
    Unavailable(String),
    /// A standard operation with a complete typed signature and a semantic implementation, but no
    /// fully encoded MIR CFG. Printed JavaScript remains its fallback body.
    Semantic(LateBody, String),
}

/// A total, order-independent key for an availability: its postcard bytes.
fn availability_key(availability: &Availability) -> Option<Vec<u8>> {
    postcard::to_allocvec(availability).ok()
}

/// Stable identity of a monomorphic Rust item.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ItemId {
    pub definition: DefinitionId,
    /// Stable hash of the monomorphic item type, including generic arguments.
    pub instantiation: u128,
}

/// An exact direct Rust item together with the JavaScript identifier emitted for it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DirectItemRef {
    pub item: ItemId,
    pub link_name: String,
}

impl DirectItemRef {
    pub fn validate(&self) -> Result<(), String> {
        if self.link_name.is_empty() {
            Err("late-IR direct item reference has an empty link name".to_owned())
        } else {
            Ok(())
        }
    }
}

/// A String producer whose only consumers are the exact shared-string adapter and drop glue.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct CarrierBridge {
    pub producer: DirectItemRef,
    pub as_str: DirectItemRef,
    pub drop: DirectItemRef,
}

impl CarrierBridge {
    pub fn validate(&self) -> Result<(), String> {
        self.producer.validate()?;
        self.as_str.validate()?;
        self.drop.validate()
    }
}

/// Stable identity of a Rust definition across crate and compilation-session boundaries.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DefinitionId {
    pub crate_id: u64,
    pub local_hash: u64,
}

/// Stable identity of a monomorphic Rust type.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TypeId(pub u128);

/// An executable function CFG or a typed static boundary.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LateBody {
    pub item: ItemId,
    pub kind: BodyKind,
    pub semantic: BodySemantic,
    pub signature: Signature,
    pub locals: Vec<Local>,
    /// Sorted by stable type identity.
    pub types: Vec<TypeDefinition>,
    /// Indexed by [`BlockId`]. Empty for statics.
    pub blocks: Vec<BasicBlock>,
}

impl LateBody {
    fn has_semantic_header(&self, header: &Self) -> bool {
        header.blocks.is_empty()
            && self.item == header.item
            && self.kind == header.kind
            && self.semantic == header.semantic
            && self.signature == header.signature
            && self.locals == header.locals
            && header.types.iter().all(|definition| {
                self.types
                    .binary_search_by_key(&definition.id, |candidate| candidate.id)
                    .is_ok_and(|index| self.types[index] == *definition)
            })
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_inner(false)
    }

    pub fn validate_semantic_header(&self) -> Result<(), String> {
        if !matches!(self.semantic, BodySemantic::Standard(_)) {
            return Err("semantic late-IR header is not a canonical standard operation".to_owned());
        }
        self.validate_inner(true)
    }

    fn validate_inner(&self, semantic_header: bool) -> Result<(), String> {
        let mut previous = None;
        let mut types = BTreeSet::new();
        for definition in &self.types {
            if previous.is_some_and(|id| id >= definition.id) {
                return Err("late-IR type definitions are not strictly sorted".to_owned());
            }
            previous = Some(definition.id);
            types.insert(definition.id);
        }
        require_type(&types, self.signature.result, "function result")?;
        for argument in &self.signature.arguments {
            require_type(&types, *argument, "function argument")?;
        }
        for definition in &self.types {
            definition.kind.validate(&types)?;
        }

        for (index, local) in self.locals.iter().enumerate() {
            if local.id.0 as usize != index {
                return Err(format!(
                    "late-IR local {} is stored at index {index}",
                    local.id.0
                ));
            }
            require_type(&types, local.ty, "local")?;
        }

        match self.kind {
            BodyKind::Function => {
                if self.locals.first().map(|local| local.role) != Some(LocalRole::Return) {
                    return Err("late-IR function local 0 is not the return place".to_owned());
                }
                if self.blocks.is_empty() && !semantic_header {
                    return Err("late-IR function has no entry block".to_owned());
                }
                let arguments: Vec<_> = self
                    .locals
                    .iter()
                    .filter(|local| local.role == LocalRole::Argument)
                    .map(|local| local.ty)
                    .collect();
                if arguments != self.signature.arguments {
                    return Err("late-IR signature does not match argument locals".to_owned());
                }
                if self.locals[0].ty != self.signature.result {
                    return Err("late-IR signature does not match the return local".to_owned());
                }
            }
            BodyKind::Static => {
                if !self.locals.is_empty() || !self.blocks.is_empty() {
                    return Err("late-IR static contains a function CFG".to_owned());
                }
            }
        }

        let context = ValidationContext {
            types: &types,
            locals: self.locals.len(),
            blocks: self.blocks.len(),
        };
        for (index, block) in self.blocks.iter().enumerate() {
            if block.id.0 as usize != index {
                return Err(format!(
                    "late-IR block {} is stored at index {index}",
                    block.id.0
                ));
            }
            let mut previous_statement = None;
            for statement in &block.statements {
                if previous_statement.is_some_and(|old| old >= statement.index) {
                    return Err(format!(
                        "late-IR block {index} statement indices are not strictly sorted"
                    ));
                }
                previous_statement = Some(statement.index);
                statement.kind.validate(&context)?;
            }
            block.terminator.validate(&context)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BodyKind {
    /// Append-only: postcard serializes this enum by index.
    Function,
    Static,
}

impl BodyKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Static => "static",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    pub arguments: Vec<TypeId>,
    pub result: TypeId,
    pub abi: AbiSignature,
}

/// The JavaScript parameter shape current MIR lowering gives this Rust body.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbiSignature {
    pub parameters: u16,
    pub spread: Option<SpreadArgument>,
    pub caller_location: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SpreadArgument {
    pub local: LocalId,
    pub arity: u16,
}

/// A body identity established from rustc definitions rather than paths or JavaScript names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BodySemantic {
    /// Append-only: postcard serializes this enum by index.
    Ordinary,
    Standard(StandardOperation),
    DropGlue(TypeId),
}

/// Standard-library operation families eligible for native representation analysis.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum StandardOperation {
    /// Append-only: postcard serializes this enum by index.
    VecNew,
    VecWithCapacity,
    VecLen,
    VecCapacity,
    VecPush,
    VecClear,
    VecReserve,
    VecAsSlice,
    VecAsMutSlice,
    VecPop,
    VecIsEmpty,
    VecExtend,
    VecRemove,
    VecSwap,
    VecIndex,
    VecIndexMut,
    VecDrop,
    StringNew,
    StringWithCapacity,
    StringLen,
    StringCapacity,
    StringPush,
    StringPushStr,
    StringClear,
    StringAsStr,
    StringDrop,
    SliceSwap,
    StringFmtWriteStr,
    StringFmtWriteChar,
    StringFmtWriteFmt,
    StringFmtSpecWriteFmt,
    F64SpecToString,
    IntToString,
    BoolToString,
    CharToString,
    /// `Argument::new_display` for a proven default-Display primitive, `&str`, `String`, or `f64`.
    FormatArgumentNewDisplay,
    /// `Arguments::new` for a compiler-generated `format_args!` template.
    FormatArgsNew,
    /// The exact `fn(Arguments) -> String` that owns a formatted result.
    FormatArgsToString,
    /// `Argument::new_lower_hex` for a proven integer type.
    FormatArgumentNewLowerHex,
    /// `Argument::new_upper_hex` for a proven integer type.
    FormatArgumentNewUpperHex,
    /// `<f64 as core::str::FromStr>::from_str`.
    F64FromStr,
    /// `<f32 as core::str::FromStr>::from_str`.
    F32FromStr,
    VecPushMut,
    VecReserveExact,
    VecShrinkToFit,
    VecTruncate,
    VecResize,
    VecSwapRemove,
    VecFromElem,
    VecExtendFromSlice,
    VecClone,
    SliceToVec,
    VecRangeIndex,
    VecRangeIndexMut,
    VecFromBoxArray,
    VecIter,
    VecIterMut,
    VecReplace,
    VecMemSwap,
    VecInsert,
    VecInsertMut,
    BTreeMapNew,
    BTreeMapLen,
    BTreeMapIsEmpty,
    BTreeMapClear,
    BTreeMapContainsKey,
    BTreeMapGet,
    BTreeMapGetMut,
    BTreeMapInsert,
    BTreeMapRemove,
    BTreeMapDrop,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeDefinition {
    pub id: TypeId,
    pub kind: TypeKind,
    pub metadata: TypeMetadata,
}

/// Semantic facts representation analysis needs without consulting diagnostic paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TypeMetadata {
    pub candidate: RepresentationCandidate,
    pub abi: NominalAbi,
    pub boundary: PinnedBoundary,
    pub sized: bool,
    pub zero_sized: bool,
    pub needs_drop: bool,
    pub allocator: AllocatorIdentity,
    pub size_bytes: Option<u64>,
    pub align_bytes: Option<u64>,
    /// Whether this ADT directly defines a synchronous or asynchronous destructor.
    ///
    /// `needs_drop` also includes destruction inherited through fields. Together the two facts
    /// distinguish compiler structural drop from an own destructor without naming source items.
    pub has_own_drop: bool,
}

/// The allocator carried by a heap-owning standard-library type.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AllocatorIdentity {
    /// Append-only: postcard serializes this enum by index.
    NotApplicable,
    Global,
    Custom,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RepresentationCandidate {
    /// Append-only: postcard serializes this enum by index.
    None,
    Box,
    Vec,
    String,
    BTreeMap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NominalAbi {
    /// Append-only: postcard serializes this enum by index.
    Rust,
    C,
    Transparent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PinnedBoundary {
    /// Append-only: postcard serializes this enum by index.
    None,
    Host,
    ViewAbi,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TypeKind {
    /// Append-only: postcard serializes this enum by index.
    Bool,
    Char,
    Integer {
        signed: bool,
        bits: u32,
    },
    Float {
        bits: u32,
    },
    Str,
    Never,
    Tuple(Vec<TypeId>),
    Array {
        element: TypeId,
        length: u64,
    },
    Slice(TypeId),
    Reference {
        pointee: TypeId,
        mutable: bool,
    },
    RawPointer {
        pointee: TypeId,
        mutable: bool,
    },
    Nominal {
        kind: NominalKind,
        definition: DefinitionId,
        arguments: Vec<TypeId>,
        form: NominalForm,
        variants: Vec<NominalVariant>,
    },
    Pattern(TypeId),
    Opaque,
}

impl TypeKind {
    pub fn validate(&self, types: &BTreeSet<TypeId>) -> Result<(), String> {
        let check = |id| require_type(types, id, "type definition");
        match self {
            Self::Tuple(elements) => elements.iter().try_for_each(|id| check(*id)),
            Self::Array { element, .. }
            | Self::Slice(element)
            | Self::Reference {
                pointee: element, ..
            }
            | Self::RawPointer {
                pointee: element, ..
            }
            | Self::Pattern(element) => check(*element),
            Self::Nominal {
                arguments,
                form,
                variants,
                ..
            } => {
                arguments.iter().try_for_each(|id| check(*id))?;
                variants
                    .iter()
                    .flat_map(|variant| &variant.fields)
                    .try_for_each(|id| check(*id))?;
                let tagged = variants
                    .iter()
                    .filter(|variant| variant.discriminant.is_some())
                    .count();
                let result_ok = variants.iter().filter(|variant| variant.result_ok).count();
                if *form == NominalForm::Enum {
                    if tagged != 0 && tagged != variants.len() {
                        return Err(
                            "late-IR enum variants have mixed physical direct-tag facts".to_owned()
                        );
                    }
                    if variants
                        .iter()
                        .filter_map(|variant| variant.discriminant)
                        .any(|discriminant| !discriminant.is_valid())
                    {
                        return Err(
                            "late-IR enum variant has an invalid physical direct tag".to_owned()
                        );
                    }
                    if result_ok > 1 {
                        return Err(
                            "late-IR enum has more than one canonical Result::Ok variant"
                                .to_owned(),
                        );
                    }
                } else if tagged != 0 || result_ok != 0 {
                    return Err(
                        "late-IR non-enum variant has enum-only semantic metadata".to_owned()
                    );
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

impl NominalDiscriminant {
    pub fn is_valid(self) -> bool {
        match self.bytes {
            1..=15 => self.bits < (1_u128 << (u32::from(self.bytes) * 8)),
            16 => true,
            _ => false,
        }
    }
}

/// The Rust nominal form whose field graph is encoded below.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NominalForm {
    /// Append-only: postcard serializes this enum by index.
    Opaque,
    Struct,
    Enum,
    Union,
}

/// Canonically ordered field types for one nominal variant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NominalVariant {
    pub fields: Vec<TypeId>,
    /// The exact bit pattern in this variant's physical direct tag, when it has one.
    ///
    /// All variants are `None` for niche, single-variant, and uninhabited layouts.
    pub discriminant: Option<NominalDiscriminant>,
    /// Whether rustc identifies this variant as the canonical `Result::Ok` language item.
    pub result_ok: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NominalDiscriminant {
    pub bits: u128,
    pub bytes: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NominalKind {
    /// Append-only: postcard serializes this enum by index.
    Adt,
    Foreign,
    Function,
    Closure,
    Coroutine,
    CoroutineClosure,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct LocalId(pub u32);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Local {
    pub id: LocalId,
    pub role: LocalRole,
    pub ty: TypeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LocalRole {
    /// Append-only: postcard serializes this enum by index.
    Return,
    Argument,
    Temporary,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BlockId(pub u32);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BasicBlock {
    pub id: BlockId,
    pub cleanup: bool,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Statement {
    /// Original MIR statement index. Nops keep source-map positions stable.
    pub index: u32,
    pub kind: StatementKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StatementKind {
    /// Append-only: postcard serializes this enum by index.
    Assign {
        destination: Place,
        value: Rvalue,
    },
    SetDiscriminant {
        place: Place,
        variant: u32,
    },
    StorageLive(LocalId),
    StorageDead(LocalId),
    Assume(Operand),
    CopyNonOverlapping {
        source: Operand,
        destination: Operand,
        count: Operand,
    },
    Nop,
}

impl StatementKind {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Assign { destination, value } => {
                destination.validate(context)?;
                value.validate(context)
            }
            Self::SetDiscriminant { place, .. } => place.validate(context),
            Self::StorageLive(local) | Self::StorageDead(local) => context.local(*local),
            Self::Assume(operand) => operand.validate(context),
            Self::CopyNonOverlapping {
                source,
                destination,
                count,
            } => {
                source.validate(context)?;
                destination.validate(context)?;
                count.validate(context)
            }
            Self::Nop => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Rvalue {
    /// Append-only: postcard serializes this enum by index.
    Use(Operand),
    Repeat {
        value: Operand,
        count: u64,
    },
    Reference {
        kind: BorrowKind,
        place: Place,
    },
    RawPointer {
        kind: RawPointerKind,
        place: Place,
    },
    Cast {
        kind: CastKind,
        value: Operand,
        result: TypeId,
    },
    Binary {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    Unary {
        op: UnaryOp,
        value: Operand,
    },
    Discriminant(Place),
    Aggregate {
        kind: AggregateKind,
        result: TypeId,
        operands: Vec<Operand>,
    },
    CopyForDeref(Place),
    WrapUnsafeBinder(Operand),
    Reborrow {
        mutable: bool,
        place: Place,
    },
}

impl Rvalue {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Use(value) | Self::WrapUnsafeBinder(value) => value.validate(context),
            Self::Repeat { value, .. } | Self::Unary { value, .. } => value.validate(context),
            Self::Reference { place, .. }
            | Self::RawPointer { place, .. }
            | Self::Discriminant(place)
            | Self::CopyForDeref(place)
            | Self::Reborrow { place, .. } => place.validate(context),
            Self::Cast { value, result, .. } => {
                context.ty(*result)?;
                value.validate(context)
            }
            Self::Binary { left, right, .. } => {
                left.validate(context)?;
                right.validate(context)
            }
            Self::Aggregate {
                kind,
                result,
                operands,
            } => {
                context.ty(*result)?;
                kind.validate(context)?;
                operands
                    .iter()
                    .try_for_each(|operand| operand.validate(context))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AggregateKind {
    /// Append-only: postcard serializes this enum by index.
    Array(TypeId),
    Tuple,
    Adt {
        definition: DefinitionId,
        variant: u32,
        active_field: Option<u32>,
    },
    Closure(DefinitionId),
    Coroutine(DefinitionId),
    CoroutineClosure(DefinitionId),
    RawPointer {
        pointee: TypeId,
        mutable: bool,
    },
}

impl AggregateKind {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Array(ty) | Self::RawPointer { pointee: ty, .. } => context.ty(*ty),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BorrowKind {
    /// Append-only: postcard serializes this enum by index.
    Shared,
    FakeShallow,
    FakeDeep,
    Mutable,
    TwoPhase,
    ClosureCapture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RawPointerKind {
    /// Append-only: postcard serializes this enum by index.
    Const,
    Mutable,
    FakeForMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CastKind {
    /// Append-only: postcard serializes this enum by index.
    PointerExposeProvenance,
    PointerWithExposedProvenance,
    PointerCoercion,
    IntToInt,
    FloatToInt,
    FloatToFloat,
    IntToFloat,
    PtrToPtr,
    FnPtrToPtr,
    Transmute,
    Subtype,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum UnaryOp {
    /// Append-only: postcard serializes this enum by index.
    Not,
    Neg,
    PtrMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BinaryOp {
    /// Append-only: postcard serializes this enum by index.
    Add,
    AddUnchecked,
    AddWithOverflow,
    Sub,
    SubUnchecked,
    SubWithOverflow,
    Mul,
    MulUnchecked,
    MulWithOverflow,
    Div,
    Rem,
    BitXor,
    BitAnd,
    BitOr,
    Shl,
    ShlUnchecked,
    Shr,
    ShrUnchecked,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    Cmp,
    Offset,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Operand {
    pub ty: TypeId,
    pub kind: OperandKind,
}

impl Operand {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        context.ty(self.ty)?;
        match &self.kind {
            OperandKind::Copy(place) | OperandKind::Move(place) => place.validate(context),
            OperandKind::Constant(constant) => constant.validate(context),
            OperandKind::RuntimeCheck(_) => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OperandKind {
    /// Append-only: postcard serializes this enum by index.
    Copy(Place),
    Move(Place),
    Constant(Constant),
    RuntimeCheck(bool),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Constant {
    /// Append-only: postcard serializes this enum by index.
    ZeroSized,
    Scalar {
        bits: u128,
        bytes: u8,
    },
    String(String),
    Pointer {
        target: GlobalReference,
        offset: u64,
    },
    Memory(ConstTree),
}

impl Constant {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Pointer {
                target: GlobalReference::Type(ty),
                ..
            } => context.ty(*ty),
            Self::Memory(tree) => tree.validate(context),
            _ => Ok(()),
        }
    }
}

/// One constant value decoded out of anonymous immutable memory.
///
/// The tree mirrors the printed JavaScript value model: a struct, tuple, or array is its field
/// values in declaration order, a transparent wrapper is its one non-zero-sized field, an enum is
/// a variant index with that variant's field values, and a reference into anonymous memory is the
/// decoded pointee value. Scalar bits are interpreted through the Rust type at each position.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ConstTree {
    /// Append-only: postcard serializes this enum by index.
    Unit,
    Scalar {
        bits: u128,
        bytes: u8,
    },
    Str(String),
    Aggregate(Vec<ConstTree>),
    Variant {
        variant: u32,
        fields: Vec<ConstTree>,
    },
    Reference(Box<ConstTree>),
    Slice(Vec<ConstTree>),
    Global {
        target: GlobalReference,
        offset: u64,
    },
    Address(u64),
    Dyn {
        data: Box<ConstTree>,
        concrete: TypeId,
        vtable: String,
    },
    SliceAddress {
        address: u64,
        length: u64,
    },
    StaticField {
        target: GlobalReference,
        path: Vec<StaticStep>,
    },
    StaticSlice {
        target: GlobalReference,
        path: Vec<StaticStep>,
        start: u32,
        length: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StaticStep {
    /// Append-only: postcard serializes this enum by index.
    Field(String),
    Index(u32),
}

impl ConstTree {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Aggregate(fields) | Self::Variant { fields, .. } | Self::Slice(fields) => {
                fields.iter().try_for_each(|field| field.validate(context))
            }
            Self::Reference(value) => value.validate(context),
            Self::Dyn {
                data,
                concrete,
                vtable,
            } => {
                if vtable.is_empty() {
                    return Err("late-IR dyn constant has an empty vtable name".to_owned());
                }
                data.validate(context)?;
                context.ty(*concrete)
            }
            Self::Global {
                target: GlobalReference::Type(ty),
                ..
            } => context.ty(*ty),
            Self::StaticField { target, path } => {
                if !matches!(target, GlobalReference::Static(_)) || path.is_empty() {
                    return Err(
                        "late-IR static field constant must name a static with a non-empty path"
                            .to_owned(),
                    );
                }
                for step in path {
                    if let StaticStep::Field(name) = step {
                        if name.is_empty() {
                            return Err(
                                "late-IR static field constant has an empty field name".to_owned()
                            );
                        }
                    }
                }
                Ok(())
            }
            Self::StaticSlice { target, path, .. } => {
                if !matches!(target, GlobalReference::Static(_)) {
                    return Err("late-IR static slice constant must name a static".to_owned());
                }
                for step in path {
                    if let StaticStep::Field(name) = step {
                        if name.is_empty() {
                            return Err(
                                "late-IR static slice constant has an empty field name".to_owned()
                            );
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum GlobalReference {
    /// Append-only: postcard serializes this enum by index.
    Function {
        item: ItemId,
        link_name: String,
    },
    Static(DefinitionId),
    Type(TypeId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub local: LocalId,
    pub projection: Vec<Projection>,
    pub ty: TypeId,
}

impl Place {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        context.local(self.local)?;
        context.ty(self.ty)?;
        for projection in &self.projection {
            match projection {
                Projection::Field { ty, .. }
                | Projection::OpaqueCast(ty)
                | Projection::UnwrapUnsafeBinder(ty) => context.ty(*ty)?,
                Projection::Index(local) => context.local(*local)?,
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Projection {
    /// Append-only: postcard serializes this enum by index.
    Deref,
    Field {
        field: u32,
        ty: TypeId,
    },
    Index(LocalId),
    ConstantIndex {
        offset: u64,
        min_length: u64,
        from_end: bool,
    },
    Subslice {
        from: u64,
        to: u64,
        from_end: bool,
    },
    Downcast {
        variant: u32,
    },
    OpaqueCast(TypeId),
    UnwrapUnsafeBinder(TypeId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Terminator {
    /// Append-only: postcard serializes this enum by index.
    Goto(BlockId),
    Switch {
        discriminant: Operand,
        targets: Vec<(u128, BlockId)>,
        otherwise: BlockId,
    },
    Return,
    Unreachable,
    Call(Call),
    Drop {
        place: Place,
        target: BlockId,
        unwind: Unwind,
        replace: bool,
        async_drop: Option<BlockId>,
        glue: DropGlue,
    },
    Assert {
        condition: Operand,
        expected: bool,
        message: AssertMessage,
        target: BlockId,
        unwind: Unwind,
    },
    UnwindResume,
    UnwindTerminate(TerminateReason),
    FalseEdge {
        real: BlockId,
        imaginary: BlockId,
    },
    FalseUnwind {
        real: BlockId,
        unwind: Unwind,
    },
}

impl Terminator {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Goto(target) => context.block(*target),
            Self::Switch {
                discriminant,
                targets,
                otherwise,
            } => {
                discriminant.validate(context)?;
                for (_, target) in targets {
                    context.block(*target)?;
                }
                context.block(*otherwise)
            }
            Self::Return | Self::Unreachable | Self::UnwindResume | Self::UnwindTerminate(_) => {
                Ok(())
            }
            Self::Call(call) => call.validate(context),
            Self::Drop {
                place,
                target,
                unwind,
                async_drop,
                ..
            } => {
                place.validate(context)?;
                context.block(*target)?;
                unwind.validate(context)?;
                if let Some(target) = async_drop {
                    context.block(*target)?;
                }
                Ok(())
            }
            Self::Assert {
                condition,
                message,
                target,
                unwind,
                ..
            } => {
                condition.validate(context)?;
                message.validate(context)?;
                context.block(*target)?;
                unwind.validate(context)
            }
            Self::FalseEdge { real, imaginary } => {
                context.block(*real)?;
                context.block(*imaginary)
            }
            Self::FalseUnwind { real, unwind } => {
                context.block(*real)?;
                unwind.validate(context)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Call {
    pub function: Operand,
    pub callee: Callee,
    pub semantic: CallSemantic,
    pub arguments: Vec<Operand>,
    pub destination: Place,
    pub target: Option<BlockId>,
    pub unwind: Unwind,
}

/// A compiler-recognized call boundary or operation family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallSemantic {
    /// Append-only: postcard serializes this enum by index.
    Ordinary,
    GlobalAllocator,
    RawParts,
    Host,
    ViewAbi,
    RawJavaScript,
    Standard(StandardOperation),
}

/// How a declared host-binding parameter observes its argument.
///
/// Computed from the encoded type; it is not a late-IR schema field. A sink parameter's
/// JavaScript value is identical under the baseline and native lanes, so it does not pin a
/// String region: scalars pass by value, and a shared `&str` is a JavaScript string in both
/// lanes. Everything else pins. Mutable references would lose host writes under a native
/// carrier; owned `String`/`Vec` values and slices differ between the lanes (the native `Vec`
/// backing array is capacity-length, not len-length); and host handles stay opaque.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostParameterClass {
    Sink,
    Pin,
}

impl HostParameterClass {
    /// Classifies one declared host parameter or result type.
    pub fn of(ty: TypeId, types: &BTreeMap<TypeId, &TypeDefinition>) -> Self {
        classify_host_parameter(ty, types, 0)
    }
}

fn classify_host_parameter(
    ty: TypeId,
    types: &BTreeMap<TypeId, &TypeDefinition>,
    depth: u8,
) -> HostParameterClass {
    if depth > 8 {
        return HostParameterClass::Pin;
    }
    let Some(definition) = types.get(&ty).copied() else {
        return HostParameterClass::Pin;
    };
    if let TypeKind::Reference { pointee, mutable } = definition.kind {
        // The host can write back through a mutable reference; a native carrier would lose or
        // desynchronize those writes, so only shared references can reach a sink shape.
        if mutable {
            return HostParameterClass::Pin;
        }
        return classify_host_parameter(pointee, types, depth + 1);
    }
    match &definition.kind {
        TypeKind::Str | TypeKind::Bool | TypeKind::Char | TypeKind::Never => {
            HostParameterClass::Sink
        }
        TypeKind::Integer { .. } | TypeKind::Float { .. } => HostParameterClass::Sink,
        TypeKind::Tuple(fields) if fields.is_empty() => HostParameterClass::Sink,
        _ => HostParameterClass::Pin,
    }
}

impl Call {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        self.function.validate(context)?;
        self.callee.validate(context)?;
        self.arguments
            .iter()
            .try_for_each(|argument| argument.validate(context))?;
        self.destination.validate(context)?;
        if let Some(target) = self.target {
            context.block(target)?;
        }
        self.unwind.validate(context)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Callee {
    /// Append-only: postcard serializes this enum by index.
    Direct {
        item: ItemId,
        link_name: String,
    },
    Foreign {
        definition: DefinitionId,
        symbol: String,
    },
    Indirect {
        signature: TypeId,
    },
}

impl Callee {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Indirect { signature } => context.ty(*signature),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DropGlue {
    /// Append-only: postcard serializes this enum by index.
    None,
    Direct {
        item: ItemId,
        link_name: String,
    },
    Dynamic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Unwind {
    /// Append-only: postcard serializes this enum by index.
    Continue,
    Unreachable,
    Terminate(TerminateReason),
    Cleanup(BlockId),
}

impl Unwind {
    fn validate(self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::Cleanup(block) => context.block(block),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TerminateReason {
    /// Append-only: postcard serializes this enum by index.
    Abi,
    InCleanup,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AssertMessage {
    /// Append-only: postcard serializes this enum by index.
    BoundsCheck {
        length: Operand,
        index: Operand,
    },
    Overflow {
        op: BinaryOp,
        left: Operand,
        right: Operand,
    },
    OverflowNeg(Operand),
    DivisionByZero(Operand),
    RemainderByZero(Operand),
    ResumedAfterReturn(CoroutineKind),
    ResumedAfterPanic(CoroutineKind),
    ResumedAfterDrop(CoroutineKind),
    MisalignedPointer {
        required: Operand,
        found: Operand,
    },
    NullPointer,
    NullReference,
    InvalidEnum(Operand),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoroutineKind {
    /// Append-only: postcard serializes this enum by index.
    Desugared {
        kind: CoroutineDesugaring,
        source: CoroutineSource,
    },
    Coroutine {
        movable: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoroutineDesugaring {
    /// Append-only: postcard serializes this enum by index.
    Async,
    Gen,
    AsyncGen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CoroutineSource {
    /// Append-only: postcard serializes this enum by index.
    Block,
    Closure,
    Function,
}

impl AssertMessage {
    fn validate(&self, context: &ValidationContext<'_>) -> Result<(), String> {
        match self {
            Self::BoundsCheck { length, index }
            | Self::Overflow {
                left: length,
                right: index,
                ..
            }
            | Self::MisalignedPointer {
                required: length,
                found: index,
            } => {
                length.validate(context)?;
                index.validate(context)
            }
            Self::OverflowNeg(value)
            | Self::DivisionByZero(value)
            | Self::RemainderByZero(value)
            | Self::InvalidEnum(value) => value.validate(context),
            _ => Ok(()),
        }
    }
}

struct ValidationContext<'a> {
    types: &'a BTreeSet<TypeId>,
    locals: usize,
    blocks: usize,
}

impl ValidationContext<'_> {
    fn ty(&self, ty: TypeId) -> Result<(), String> {
        require_type(self.types, ty, "value")
    }

    fn local(&self, local: LocalId) -> Result<(), String> {
        if (local.0 as usize) < self.locals {
            Ok(())
        } else {
            Err(format!("late-IR references missing local {}", local.0))
        }
    }

    fn block(&self, block: BlockId) -> Result<(), String> {
        if (block.0 as usize) < self.blocks {
            Ok(())
        } else {
            Err(format!("late-IR references missing block {}", block.0))
        }
    }
}

fn require_type(types: &BTreeSet<TypeId>, ty: TypeId, context: &str) -> Result<(), String> {
    if types.contains(&ty) {
        Ok(())
    } else {
        Err(format!(
            "late-IR {context} references missing type {:032x}",
            ty.0
        ))
    }
}

/// Source metadata deliberately excluded from semantic body equality.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticInfo {
    pub item_path: Option<String>,
    pub root: Option<SourceFact>,
    pub type_names: Vec<TypeDiagnostic>,
    pub locations: Vec<LocationDiagnostic>,
}

impl DiagnosticInfo {
    pub fn normalize(&mut self) {
        self.type_names.sort();
        self.type_names.dedup();
        self.locations.sort();
        self.locations.dedup();
    }

    fn merge(&mut self, other: &Self) {
        self.item_path = match (&self.item_path, &other.item_path) {
            (Some(left), Some(right)) => Some(left.min(right).clone()),
            (None, path) | (path, None) => path.clone(),
        };
        self.root = match (&self.root, &other.root) {
            (Some(left), Some(right)) => Some(left.min(right).clone()),
            (None, source) | (source, None) => source.clone(),
        };
        self.type_names.extend(other.type_names.iter().cloned());
        self.locations.extend(other.locations.iter().cloned());
        self.normalize();
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceFact {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

impl SourceFact {
    /// Attaches this already-rendered source location to an optimizer miss.
    pub fn optimization_miss(
        &self,
        pass: impl Into<String>,
        reason: impl Into<String>,
    ) -> crate::item::OptimizationMiss {
        crate::item::OptimizationMiss::new(pass, reason).at_source(
            self.file.clone(),
            format!("{}:{}:{}", self.file, self.line, self.column + 1),
        )
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TypeDiagnostic {
    pub id: TypeId,
    pub name: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct LocationDiagnostic {
    pub site: DiagnosticSite,
    pub source: SourceFact,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum DiagnosticSite {
    /// Append-only: postcard serializes this enum by index.
    Local(LocalId),
    Statement {
        block: BlockId,
        statement: u32,
    },
    Terminator(BlockId),
}

#[derive(Clone, Copy)]
pub enum CarrierFlowEvent {
    Producer,
    SharedBorrow,
    Drop,
    AliasDefinition,
    AsStrArgument,
    OtherCarrierUse,
    ProjectedCarrierUse,
    OtherAliasUse,
    CleanupUse,
}

/// Literal pieces of a default-Display `format_args!` template, including empty
/// pieces around placeholders.
///
/// A successful decode is the native `Arguments` encoding: static pieces plus
/// one already-lowered argument text per placeholder. Width, fill, precision,
/// Debug, and explicit argument selection stay on baseline `core::fmt`.
pub fn decode_default_format_template(bytes: &[u8]) -> Result<Vec<String>, &'static str> {
    let mut pieces = vec![String::new()];
    let mut cursor = 0;
    loop {
        let Some(&header) = bytes.get(cursor) else {
            return Err("format template byte sequence has no terminator");
        };
        cursor += 1;
        match header {
            0 => {
                return if cursor == bytes.len() {
                    Ok(pieces)
                } else {
                    Err("format template has bytes after its terminator")
                };
            }
            1..=0x7f => {
                let end = cursor
                    .checked_add(usize::from(header))
                    .filter(|end| *end <= bytes.len())
                    .ok_or("format literal piece runs past the template")?;
                let text = std::str::from_utf8(&bytes[cursor..end])
                    .map_err(|_| "format literal piece is not UTF-8")?;
                pieces
                    .last_mut()
                    .expect("template starts with a piece")
                    .push_str(text);
                cursor = end;
            }
            0x80 => {
                let length = bytes
                    .get(cursor..cursor + 2)
                    .map(|length| u16::from_le_bytes([length[0], length[1]]) as usize)
                    .ok_or("long format literal has no encoded length")?;
                cursor += 2;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())
                    .ok_or("format literal piece runs past the template")?;
                let text = std::str::from_utf8(&bytes[cursor..end])
                    .map_err(|_| "format literal piece is not UTF-8")?;
                pieces
                    .last_mut()
                    .expect("template starts with a piece")
                    .push_str(text);
                cursor = end;
            }
            0xc0 => pieces.push(String::new()),
            0x81..=0xbf | 0xc1..=0xff => {
                return Err(
                    "format template uses flags, width, precision, or explicit argument selection",
                );
            }
        }
    }
}

/// Bytes of a decoded late-IR array or slice of `u8`, following one reference.
pub fn const_tree_bytes(tree: &ConstTree) -> Option<Vec<u8>> {
    match tree {
        ConstTree::Reference(inner) => const_tree_bytes(inner),
        ConstTree::Aggregate(fields) | ConstTree::Slice(fields) => fields
            .iter()
            .map(|field| match field {
                ConstTree::Scalar { bits, bytes: 1 } => u8::try_from(*bits).ok(),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

pub fn fmt_write_operation(
    receiver: Option<RepresentationCandidate>,
    exact_trait: bool,
    name: &str,
) -> Option<StandardOperation> {
    if receiver != Some(RepresentationCandidate::String) || !exact_trait {
        return None;
    }
    match name {
        "write_str" => Some(StandardOperation::StringFmtWriteStr),
        "write_char" => Some(StandardOperation::StringFmtWriteChar),
        _ => None,
    }
}

pub fn fmt_adapter_operation(
    receiver: Option<RepresentationCandidate>,
    exact_write_fmt: bool,
    exact_spec_write_fmt: bool,
) -> Option<StandardOperation> {
    if receiver != Some(RepresentationCandidate::String) {
        return None;
    }
    match (exact_write_fmt, exact_spec_write_fmt) {
        (true, false) => Some(StandardOperation::StringFmtWriteFmt),
        (false, true) => Some(StandardOperation::StringFmtSpecWriteFmt),
        _ => None,
    }
}
