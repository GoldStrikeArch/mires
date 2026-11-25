//! The one vocabulary of reasons a body, type, operation, or optimization is not taken further.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Defines a fieldless reason enum with a stable code and message text per variant.
macro_rules! reasons {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => ($code:literal, $text:literal) ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant ),*
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),* ];

            /// The stable machine key.
            pub const fn code(self) -> &'static str {
                match self { $( $name::$variant => $code ),* }
            }

            /// The reason text.
            pub const fn text(self) -> &'static str {
                match self { $( $name::$variant => $text ),* }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.text())
            }
        }
    };
}

/// Why something was not encoded, lowered, optimized, or merged.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Decline {
    Boundary(BoundaryDecline),
    Cost(CostDecline),
    Version(VersionDecline),
    Proof(ProofDecline),
    Encode(EncodeDecline),
    Merge(MergeDecline),
    Lower(LowerDecline),
}

impl Decline {
    /// The stable machine key, unique across every variant of every inner enum.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Boundary(inner) => inner.code(),
            Self::Cost(inner) => inner.code(),
            Self::Version(inner) => inner.code(),
            Self::Proof(inner) => inner.code(),
            Self::Encode(inner) => inner.code(),
            Self::Merge(inner) => inner.code(),
            Self::Lower(inner) => inner.code(),
        }
    }

    /// The category name, for grouping in reports.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Boundary(_) => "boundary",
            Self::Cost(_) => "cost",
            Self::Version(_) => "version",
            Self::Proof(_) => "proof",
            Self::Encode(_) => "encode",
            Self::Merge(_) => "merge",
            Self::Lower(_) => "lower",
        }
    }

    /// One instance of every variant (with placeholder dynamic parts), for exhaustiveness tests.
    pub fn catalog() -> Vec<Decline> {
        let mut all = Vec::new();
        all.extend(BoundaryDecline::ALL.iter().copied().map(Decline::Boundary));
        all.extend(CostDecline::catalog().into_iter().map(Decline::Cost));
        all.extend(VersionDecline::catalog().into_iter().map(Decline::Version));
        all.extend(ProofDecline::ALL.iter().copied().map(Decline::Proof));
        all.extend(EncodeDecline::catalog().into_iter().map(Decline::Encode));
        all.extend(MergeDecline::catalog().into_iter().map(Decline::Merge));
        all.extend(LowerDecline::catalog().into_iter().map(Decline::Lower));
        all
    }
}

impl fmt::Display for Decline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boundary(inner) => inner.fmt(f),
            Self::Cost(inner) => inner.fmt(f),
            Self::Version(inner) => inner.fmt(f),
            Self::Proof(inner) => inner.fmt(f),
            Self::Encode(inner) => inner.fmt(f),
            Self::Merge(inner) => inner.fmt(f),
            Self::Lower(inner) => inner.fmt(f),
        }
    }
}

reasons! {
    /// A pinned boundary.
    pub enum BoundaryDecline {
        /// `plan::abi_flat`: the callee is a root, is exported, or has a fixed name.
        Exported => ("boundary.exported", "the item's name is part of the program's interface"),
        /// `plan::abi_flat`: something names the callee that is not a plain direct call.
        NonCallReference => ("boundary.non-call-reference", "the item is referenced other than by a direct call"),
        /// `plan::abi_flat`: no parameter survives the field-only use summary.
        ParamEscapes => ("boundary.param-escapes", "no parameter of the item is used through its fields only"),
        /// `plan::abi_flat`: spill registers do not cross a chunk boundary.
        Chunks => ("boundary.chunks", "the program is split into chunks"),
        /// `plan::abi_flat`: a spread or `#[track_caller]` ABI fixes the parameter list.
        SpreadAbi => ("boundary.spread-abi", "the item's ABI shape is not a plain parameter list"),
        /// `plan::abi_flat`: a `#[track_caller]` caller checks the callee's recorded parameter count.
        TrackedCaller => ("boundary.tracked-caller", "a `#[track_caller]` caller pins the callee's parameter count"),
        /// `plan::abi_flat`: no unused name was left for a spill register.
        GeneratedNameCollision => ("boundary.generated-name-collision", "no unused name was available for a spill register"),
        /// Mutable-reference copy-back is only implemented on normal returns.
        ExceptionalCopyBack => ("boundary.exceptional-copy-back", "mutable argument writes must remain visible when the callee throws"),
    }
}

/// A signature slot of a standard operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SignatureSlot {
    Result,
    Argument(u16),
}

/// A cost gate declined.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CostDecline {
    RegionNoShrink,
    RegionNoShrinkBytes {
        members: Vec<String>,
        old_bytes: u64,
        new_bytes: u64,
    },
    NoImprovement {
        before: u64,
        after: u64,
    },
    Budget {
        budget: String,
    },
}

impl CostDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RegionNoShrink => "cost.region-no-shrink",
            Self::RegionNoShrinkBytes { .. } => "cost.region-no-shrink-bytes",
            Self::NoImprovement { .. } => "cost.no-improvement",
            Self::Budget { .. } => "cost.budget",
        }
    }

    pub fn catalog() -> Vec<CostDecline> {
        vec![
            CostDecline::RegionNoShrink,
            CostDecline::RegionNoShrinkBytes {
                members: vec!["a".to_owned(), "b".to_owned()],
                old_bytes: 1,
                new_bytes: 1,
            },
            CostDecline::NoImprovement {
                before: 1,
                after: 1,
            },
            CostDecline::Budget {
                budget: "inline-bytes".to_owned(),
            },
        ]
    }
}

impl fmt::Display for CostDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegionNoShrink => {
                f.write_str("native region does not shrink the complete reachable item graph")
            }
            Self::RegionNoShrinkBytes {
                members,
                old_bytes,
                new_bytes,
            } => write!(
                f,
                "native region [{}] does not shrink the complete reachable item graph ({old_bytes} -> {new_bytes} bytes)",
                members.join(", ")
            ),
            Self::NoImprovement { before, after } => {
                write!(f, "does not improve ({before} -> {after})")
            }
            Self::Budget { budget } => write!(f, "the `{budget}` budget is exhausted"),
        }
    }
}

/// Clone and ABI versioning declines.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VersionDecline {
    NativeCloneCollision,
    AbiVersions { versions: u32 },
    CarrierCommitRollback { reason: String },
}

impl VersionDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NativeCloneCollision => "version.native-clone-collision",
            Self::AbiVersions { .. } => "version.abi-versions",
            Self::CarrierCommitRollback { .. } => "version.carrier-commit-rollback",
        }
    }

    pub fn catalog() -> Vec<VersionDecline> {
        vec![
            VersionDecline::NativeCloneCollision,
            VersionDecline::AbiVersions { versions: 1 },
            VersionDecline::CarrierCommitRollback {
                reason: "a".to_owned(),
            },
        ]
    }
}

impl fmt::Display for VersionDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeCloneCollision => {
                f.write_str("native clone name collides with an existing identifier")
            }
            Self::AbiVersions { versions } => {
                write!(f, "the item already has {versions} ABI version(s)")
            }
            Self::CarrierCommitRollback { reason } => {
                write!(f, "the representation commit did not verify: {reason}")
            }
        }
    }
}

reasons! {
    /// A required proof is missing.
    pub enum ProofDecline {
        Witness => ("proof.witness", "the optimization witness failed its independent check"),
    }
}

/// The producer could not encode the item, or the encoding failed to verify.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EncodeDecline {
    Synthesized,
    Invalid {
        message: String,
    },
    Codec {
        message: String,
    },
    AtStatement {
        block: u32,
        index: u32,
        reason: Box<EncodeDecline>,
    },
    AtTerminator {
        block: u32,
        reason: Box<EncodeDecline>,
    },
    SpreadNotTuple,
    SpreadTooLarge,
    TooManyParameters,
    RepeatCount,
    ThreadLocal,
    UnsupportedTerminator {
        name: String,
    },
    ConstantEvaluation {
        ty: String,
    },
    PointerConstantNotReference {
        ty: String,
    },
    StringConstantNoMemory,
    StringConstantInvalid,
    SliceConstantNoMemory,
    SliceConstantNonSlicePointee {
        ty: String,
    },
    IndirectConstantNoMemory,
    MutableConstantMemory,
    ConstantMemoryTooLarge,
    StrConstantNoLength,
    SliceConstantNoLength {
        pointee: String,
    },
    SliceConstantTooLong,
    SliceElementNoLayout {
        element: String,
    },
    SliceConstantLengthOverflow,
    SliceConstantOverrun {
        element: String,
    },
    ConstantTooDeep,
    ConstantNoLayout {
        ty: String,
    },
    ConstantUnreadable {
        ty: String,
    },
    ConstantVariantUnreadable {
        ty: String,
    },
    ArrayConstantTooLong,
    ConstantUnencodable {
        ty: String,
    },
    ConstantAddressNoProvenance {
        ty: String,
    },
    ConstantUninitialized {
        ty: String,
    },
    VtablePointerConstant,
    WideGlobalPointerConstant {
        ty: String,
    },
    AnonymousPointerConstant,
    VtablePointerConstantYet,
    IntegerWidth {
        ty: String,
    },
    ArrayLength {
        ty: String,
    },
    VariantDiscriminant {
        variant: u32,
        ty: String,
    },
    EnumTagWidth {
        ty: String,
    },
    Disabled,
    StaticSubPointerConstant {
        ty: String,
    },
}

impl EncodeDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Synthesized => "encode.synthesized",
            Self::Invalid { .. } => "encode.invalid",
            Self::Codec { .. } => "encode.codec",
            Self::Disabled => "encode.disabled",
            Self::AtStatement { reason, .. } | Self::AtTerminator { reason, .. } => reason.code(),
            Self::SpreadNotTuple => "encode.spread-not-tuple",
            Self::SpreadTooLarge => "encode.spread-too-large",
            Self::TooManyParameters => "encode.too-many-parameters",
            Self::RepeatCount => "encode.repeat-count",
            Self::ThreadLocal => "encode.thread-local",
            Self::UnsupportedTerminator { .. } => "encode.unsupported-terminator",
            Self::ConstantEvaluation { .. } => "encode.constant-evaluation",
            Self::PointerConstantNotReference { .. } => "encode.pointer-constant-not-reference",
            Self::StringConstantNoMemory => "encode.string-constant-no-memory",
            Self::StringConstantInvalid => "encode.string-constant-invalid",
            Self::SliceConstantNoMemory => "encode.slice-constant-no-memory",
            Self::SliceConstantNonSlicePointee { .. } => "encode.slice-constant-non-slice-pointee",
            Self::IndirectConstantNoMemory => "encode.indirect-constant-no-memory",
            Self::MutableConstantMemory => "encode.mutable-constant-memory",
            Self::ConstantMemoryTooLarge => "encode.constant-memory-too-large",
            Self::StrConstantNoLength => "encode.str-constant-no-length",
            Self::SliceConstantNoLength { .. } => "encode.slice-constant-no-length",
            Self::SliceConstantTooLong => "encode.slice-constant-too-long",
            Self::SliceElementNoLayout { .. } => "encode.slice-element-no-layout",
            Self::SliceConstantLengthOverflow => "encode.slice-constant-length-overflow",
            Self::SliceConstantOverrun { .. } => "encode.slice-constant-overrun",
            Self::ConstantTooDeep => "encode.constant-too-deep",
            Self::ConstantNoLayout { .. } => "encode.constant-no-layout",
            Self::ConstantUnreadable { .. } => "encode.constant-unreadable",
            Self::ConstantVariantUnreadable { .. } => "encode.constant-variant-unreadable",
            Self::ArrayConstantTooLong => "encode.array-constant-too-long",
            Self::ConstantUnencodable { .. } => "encode.constant-unencodable",
            Self::ConstantAddressNoProvenance { .. } => "encode.constant-address-no-provenance",
            Self::ConstantUninitialized { .. } => "encode.constant-uninitialized",
            Self::VtablePointerConstant => "encode.vtable-pointer-constant",
            Self::WideGlobalPointerConstant { .. } => "encode.wide-global-pointer-constant",
            Self::AnonymousPointerConstant => "encode.anonymous-pointer-constant",
            Self::VtablePointerConstantYet => "encode.vtable-pointer-constant-yet",
            Self::IntegerWidth { .. } => "encode.integer-width",
            Self::ArrayLength { .. } => "encode.array-length",
            Self::VariantDiscriminant { .. } => "encode.variant-discriminant",
            Self::EnumTagWidth { .. } => "encode.enum-tag-width",
            Self::StaticSubPointerConstant { .. } => "encode.static-sub-pointer-constant",
        }
    }

    pub fn catalog() -> Vec<EncodeDecline> {
        let ty = || "T".to_owned();
        vec![
            EncodeDecline::Synthesized,
            EncodeDecline::Invalid {
                message: "JIR function has no entry block".to_owned(),
            },
            EncodeDecline::Codec {
                message: "Hit the end of buffer, expected more data".to_owned(),
            },
            EncodeDecline::AtStatement {
                block: 0,
                index: 0,
                reason: Box::new(EncodeDecline::ThreadLocal),
            },
            EncodeDecline::AtTerminator {
                block: 0,
                reason: Box::new(EncodeDecline::UnsupportedTerminator {
                    name: "Yield".to_owned(),
                }),
            },
            EncodeDecline::SpreadNotTuple,
            EncodeDecline::SpreadTooLarge,
            EncodeDecline::TooManyParameters,
            EncodeDecline::RepeatCount,
            EncodeDecline::ThreadLocal,
            EncodeDecline::UnsupportedTerminator {
                name: "Yield".to_owned(),
            },
            EncodeDecline::ConstantEvaluation { ty: ty() },
            EncodeDecline::PointerConstantNotReference { ty: ty() },
            EncodeDecline::StringConstantNoMemory,
            EncodeDecline::StringConstantInvalid,
            EncodeDecline::SliceConstantNoMemory,
            EncodeDecline::SliceConstantNonSlicePointee { ty: ty() },
            EncodeDecline::IndirectConstantNoMemory,
            EncodeDecline::MutableConstantMemory,
            EncodeDecline::ConstantMemoryTooLarge,
            EncodeDecline::StrConstantNoLength,
            EncodeDecline::SliceConstantNoLength { pointee: ty() },
            EncodeDecline::SliceConstantTooLong,
            EncodeDecline::SliceElementNoLayout { element: ty() },
            EncodeDecline::SliceConstantLengthOverflow,
            EncodeDecline::SliceConstantOverrun { element: ty() },
            EncodeDecline::ConstantTooDeep,
            EncodeDecline::ConstantNoLayout { ty: ty() },
            EncodeDecline::ConstantUnreadable { ty: ty() },
            EncodeDecline::ConstantVariantUnreadable { ty: ty() },
            EncodeDecline::ArrayConstantTooLong,
            EncodeDecline::ConstantUnencodable { ty: ty() },
            EncodeDecline::ConstantAddressNoProvenance { ty: ty() },
            EncodeDecline::ConstantUninitialized { ty: ty() },
            EncodeDecline::VtablePointerConstant,
            EncodeDecline::WideGlobalPointerConstant { ty: ty() },
            EncodeDecline::AnonymousPointerConstant,
            EncodeDecline::VtablePointerConstantYet,
            EncodeDecline::IntegerWidth { ty: ty() },
            EncodeDecline::ArrayLength { ty: ty() },
            EncodeDecline::VariantDiscriminant {
                variant: 0,
                ty: ty(),
            },
            EncodeDecline::EnumTagWidth { ty: ty() },
            EncodeDecline::Disabled,
            EncodeDecline::StaticSubPointerConstant { ty: ty() },
        ]
    }
}

impl fmt::Display for EncodeDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Synthesized => f.write_str("the item is synthesized and has no typed MIR body"),
            Self::Disabled => f.write_str("`-Cllvm-args=js-jir=off`: no JIR body was encoded"),
            Self::Invalid { message } | Self::Codec { message } => f.write_str(message),
            Self::AtStatement {
                block,
                index,
                reason,
            } => write!(f, "block {block} statement {index}: {reason}"),
            Self::AtTerminator { block, reason } => {
                write!(f, "block {block} terminator: {reason}")
            }
            Self::SpreadNotTuple => f.write_str("rust-call spread argument is not a tuple"),
            Self::SpreadTooLarge => f.write_str("rust-call spread argument has too many elements"),
            Self::TooManyParameters => f.write_str("function has too many JavaScript parameters"),
            Self::RepeatCount => f.write_str("array repeat count is not a target usize"),
            Self::ThreadLocal => f.write_str("thread-local references are not representable"),
            Self::UnsupportedTerminator { name } => {
                write!(f, "unsupported `{name}` terminator")
            }
            Self::ConstantEvaluation { ty } => {
                write!(f, "could not evaluate constant of type `{ty}`")
            }
            Self::PointerConstantNotReference { ty } => {
                write!(f, "the pointer constant `{ty}` is not a reference")
            }
            Self::StringConstantNoMemory => f.write_str("string constant is not backed by memory"),
            Self::StringConstantInvalid => {
                f.write_str("string constant contains invalid UTF-8 or exceeds its allocation")
            }
            Self::SliceConstantNoMemory => f.write_str("slice constant is not backed by memory"),
            Self::SliceConstantNonSlicePointee { ty } => {
                write!(f, "slice constant `{ty}` has a non-slice pointee")
            }
            Self::IndirectConstantNoMemory => {
                f.write_str("indirect constant is not backed by memory")
            }
            Self::MutableConstantMemory => {
                f.write_str("mutable constant memory cannot be decoded into a value")
            }
            Self::ConstantMemoryTooLarge => {
                f.write_str("constant memory exceeds the encoded-constant size limit")
            }
            Self::StrConstantNoLength => f.write_str("a `&str` constant has no length"),
            Self::SliceConstantNoLength { pointee } => {
                write!(f, "a `&{pointee}` constant has no length")
            }
            Self::SliceConstantTooLong => {
                f.write_str("slice constant exceeds the encoded-constant length limit")
            }
            Self::SliceElementNoLayout { element } => {
                write!(f, "no layout for `{element}` in a slice constant")
            }
            Self::SliceConstantLengthOverflow => {
                f.write_str("slice constant length does not fit the host")
            }
            Self::SliceConstantOverrun { element } => write!(
                f,
                "a `&[{element}]` constant runs past the end of its allocation"
            ),
            Self::ConstantTooDeep => {
                f.write_str("constant memory nests deeper than the encoded-constant limit")
            }
            Self::ConstantNoLayout { ty } => write!(f, "no layout for `{ty}` in a constant"),
            Self::ConstantUnreadable { ty } => {
                write!(f, "could not read a `{ty}` out of a constant")
            }
            Self::ConstantVariantUnreadable { ty } => {
                write!(f, "could not read the variant of `{ty}` out of a constant")
            }
            Self::ArrayConstantTooLong => {
                f.write_str("array constant exceeds the encoded-constant length limit")
            }
            Self::ConstantUnencodable { ty } => {
                write!(
                    f,
                    "constants of type `{ty}` cannot be encoded from an allocation"
                )
            }
            Self::ConstantAddressNoProvenance { ty } => {
                write!(f, "a `{ty}` in a constant is an address with no provenance")
            }
            Self::ConstantUninitialized { ty } => {
                write!(f, "a `{ty}` in a constant is uninitialized")
            }
            Self::VtablePointerConstant => {
                f.write_str("pointer to a vtable constant is not encoded")
            }
            Self::WideGlobalPointerConstant { ty } => write!(
                f,
                "a wide `{ty}` to a named global is not encoded in a constant"
            ),
            Self::AnonymousPointerConstant => {
                f.write_str("pointer to anonymous constant memory is not encoded yet")
            }
            Self::VtablePointerConstantYet => {
                f.write_str("pointer to a vtable constant is not encoded yet")
            }
            Self::IntegerWidth { ty } => {
                write!(f, "could not determine integer width for `{ty}`")
            }
            Self::ArrayLength { ty } => {
                write!(f, "array length for `{ty}` is not a target usize")
            }
            Self::VariantDiscriminant { variant, ty } => write!(
                f,
                "could not determine discriminant for variant {variant} of `{ty}`"
            ),
            Self::EnumTagWidth { ty } => {
                write!(f, "physical enum tag for `{ty}` has an unsupported width")
            }
            Self::StaticSubPointerConstant { ty } => write!(
                f,
                "a pointer into part of a `static` is not encoded in a constant (type `{ty}`)"
            ),
        }
    }
}

/// Merging duplicate definitions across objects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MergeDecline {
    Conflict,
    BodyChoice {
        chosen: u128,
        rejected: u128,
    },
    Header {
        field: String,
        left: String,
        right: String,
    },
}

impl MergeDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Conflict => "merge.conflict",
            Self::BodyChoice { .. } => "merge.body-choice",
            Self::Header { .. } => "merge.header",
        }
    }

    pub fn catalog() -> Vec<MergeDecline> {
        vec![
            MergeDecline::Conflict,
            MergeDecline::BodyChoice {
                chosen: 1,
                rejected: 2,
            },
            MergeDecline::Header {
                field: "jir schema".to_owned(),
                left: "1".to_owned(),
                right: "2".to_owned(),
            },
        ]
    }
}

impl fmt::Display for MergeDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict => {
                f.write_str("identical JavaScript definitions carried conflicting JIR bodies")
            }
            Self::BodyChoice { chosen, rejected } => write!(
                f,
                "duplicate JIR bodies differ; kept {chosen:032x} over {rejected:032x}"
            ),
            Self::Header { field, left, right } => write!(
                f,
                "objects in one program disagree on {field} (`{left}` and `{right}`); every object \
                 in a program must be compiled with the same toolchain, target, and \
                 `-Cllvm-args=js-*` options"
            ),
        }
    }
}

/// The class of operation a lowering decline is about (`LowerDecline::Other`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OpClass {
    Statement,
    Rvalue,
    Operand,
    Constant,
    Place,
    Terminator,
    Call,
    Intrinsic,
    Drop,
    Cast,
    Host,
    Template,
    Unsize,
    Vtable,
    Static,
    Region,
    Other,
}

impl OpClass {
    /// Every class, in declaration order.
    pub const ALL: &'static [OpClass] = &[
        Self::Statement,
        Self::Rvalue,
        Self::Operand,
        Self::Constant,
        Self::Place,
        Self::Terminator,
        Self::Call,
        Self::Intrinsic,
        Self::Drop,
        Self::Cast,
        Self::Host,
        Self::Template,
        Self::Unsize,
        Self::Vtable,
        Self::Static,
        Self::Region,
        Self::Other,
    ];

    pub const fn lower_code(self) -> &'static str {
        match self {
            Self::Statement => "lower.jir-statement",
            Self::Rvalue => "lower.jir-rvalue",
            Self::Operand => "lower.jir-operand",
            Self::Constant => "lower.jir-constant",
            Self::Place => "lower.jir-place",
            Self::Terminator => "lower.jir-terminator",
            Self::Call => "lower.jir-call",
            Self::Intrinsic => "lower.jir-intrinsic",
            Self::Drop => "lower.jir-drop",
            Self::Cast => "lower.jir-cast",
            Self::Host => "lower.jir-host",
            Self::Template => "lower.jir-template",
            Self::Unsize => "lower.jir-unsize",
            Self::Vtable => "lower.jir-vtable",
            Self::Static => "lower.jir-static",
            Self::Region => "lower.jir-region",
            Self::Other => "lower.other",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::Rvalue => "rvalue",
            Self::Operand => "operand",
            Self::Constant => "constant",
            Self::Place => "place",
            Self::Terminator => "terminator",
            Self::Call => "call",
            Self::Intrinsic => "intrinsic",
            Self::Drop => "drop",
            Self::Cast => "cast",
            Self::Host => "host",
            Self::Template => "template",
            Self::Unsize => "unsize",
            Self::Vtable => "vtable",
            Self::Static => "static",
            Self::Region => "region",
            Self::Other => "other",
        }
    }
}

/// Lowering decline reasons.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LowerDecline {
    Other { op: OpClass, detail: String },
}

impl LowerDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Other { op, .. } => op.lower_code(),
        }
    }

    pub fn catalog() -> Vec<LowerDecline> {
        OpClass::ALL
            .iter()
            .copied()
            .map(|op| LowerDecline::Other {
                op,
                detail: "unsupported".to_owned(),
            })
            .collect()
    }
}

impl fmt::Display for LowerDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other { op, detail } => write!(f, "{}: {detail}", op.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn codes_are_unique() {
        let catalog = Decline::catalog();
        // Wrappers (`AtStatement`, `AtTerminator`) report their inner code by design.
        let mut codes = BTreeSet::new();
        for decline in &catalog {
            let wrapper = matches!(
                decline,
                Decline::Encode(EncodeDecline::AtStatement { .. })
                    | Decline::Encode(EncodeDecline::AtTerminator { .. })
            );
            if wrapper {
                continue;
            }
            assert!(
                codes.insert(decline.code()),
                "duplicate decline code {}",
                decline.code()
            );
        }
        assert!(catalog.len() >= 70, "catalog has {} entries", catalog.len());
    }

    #[test]
    fn display_formats_declines() {
        assert_eq!(
            Decline::Encode(EncodeDecline::Synthesized).to_string(),
            "the item is synthesized and has no typed MIR body"
        );
        assert_eq!(
            Decline::Encode(EncodeDecline::AtStatement {
                block: 3,
                index: 7,
                reason: Box::new(EncodeDecline::ThreadLocal),
            })
            .to_string(),
            "block 3 statement 7: thread-local references are not representable"
        );
        assert_eq!(
            Decline::Encode(EncodeDecline::AtTerminator {
                block: 1,
                reason: Box::new(EncodeDecline::UnsupportedTerminator {
                    name: "Yield".to_owned()
                }),
            })
            .to_string(),
            "block 1 terminator: unsupported `Yield` terminator"
        );
        assert_eq!(
            Decline::Cost(CostDecline::RegionNoShrinkBytes {
                members: vec!["a".to_owned(), "b".to_owned()],
                old_bytes: 966890,
                new_bytes: 966890,
            })
            .to_string(),
            "native region [a, b] does not shrink the complete reachable item graph (966890 -> 966890 bytes)"
        );
    }

    #[test]
    fn declines_round_trip_through_postcard() {
        for decline in Decline::catalog() {
            let bytes = postcard::to_allocvec(&decline).expect("serialize");
            let back: Decline = postcard::from_bytes(&bytes).expect("deserialize");
            assert_eq!(back, decline);
        }
    }
}
