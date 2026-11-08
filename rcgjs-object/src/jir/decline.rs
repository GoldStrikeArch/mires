//! The one vocabulary of reasons a body, type, operation, or optimization is not taken further.
//!
//! v1 grew five string vocabularies (`late_ir.rs` encoder reasons, `representation_plan.rs::
//! BaselineReason`, `late_lower.rs` consts, `native_kernel.rs`/`native_std_lower.rs`/
//! `native_structural_drop.rs` declines, `async_analysis.rs::Failure`) plus a handful of
//! optimizer misses. This enum replaces all of them. Every reason that exists today is a variant
//! (or a variant with a field for its dynamic part), and [`Decline`]'s `Display` reproduces the
//! exact v1 text so reports and JSON stay stable across the migration. [`Decline::code`] is the
//! stable machine key a report or a test matches on; codes are unique across the whole enum.
//!
//! The top-level categories are the plan's: `Encode` (the producer could not encode), `Body`,
//! `Type`, `Observe` (something can observe the representation), `Boundary` (a pinned boundary),
//! `Op` (an unsupported semantic operation — the native kernel, std, structural drop, format and
//! iterator vocabularies live here as inner enums), `Cost`, `Version` (schema and ABI versioning),
//! `Proof`, `SourceMap`, `Merge`, `Lower` (the v1 `late_lower` transaction vocabulary and the
//! Phase 2 lowerer's), and `Async` (the native async lane). Dynamic parts are `String`s rather
//! than interned ids so a decline displays without an intern table and serializes on its own.
//!
//! Adding a reason: add a variant at the end of the relevant enum (append-only, like every enum in
//! this crate), give it a code no other variant uses, and add it to [`Decline::catalog`] so the
//! uniqueness test covers it.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::semantic::StandardOperation;
use super::types::TypeId;

/// Defines a fieldless reason enum with a stable code and the exact v1 text per variant.
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

            /// The exact v1 reason text.
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
    /// Append-only: postcard serializes this enum by index.
    Body(BodyDecline),
    Type(TypeDecline),
    Observe(ObserveDecline),
    Boundary(BoundaryDecline),
    Op(OpDecline),
    Cost(CostDecline),
    Version(VersionDecline),
    Proof(ProofDecline),
    SourceMap(SourceMapDecline),
    Encode(EncodeDecline),
    Merge(MergeDecline),
    Lower(LowerDecline),
    Async(AsyncDecline),
}

impl Decline {
    /// The stable machine key, unique across every variant of every inner enum.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Body(inner) => inner.code(),
            Self::Type(inner) => inner.code(),
            Self::Observe(inner) => inner.code(),
            Self::Boundary(inner) => inner.code(),
            Self::Op(inner) => inner.code(),
            Self::Cost(inner) => inner.code(),
            Self::Version(inner) => inner.code(),
            Self::Proof(inner) => inner.code(),
            Self::SourceMap(inner) => inner.code(),
            Self::Encode(inner) => inner.code(),
            Self::Merge(inner) => inner.code(),
            Self::Lower(inner) => inner.code(),
            Self::Async(inner) => inner.code(),
        }
    }

    /// The category name, for grouping in reports.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Body(_) => "body",
            Self::Type(_) => "type",
            Self::Observe(_) => "observe",
            Self::Boundary(_) => "boundary",
            Self::Op(_) => "op",
            Self::Cost(_) => "cost",
            Self::Version(_) => "version",
            Self::Proof(_) => "proof",
            Self::SourceMap(_) => "source-map",
            Self::Encode(_) => "encode",
            Self::Merge(_) => "merge",
            Self::Lower(_) => "lower",
            Self::Async(_) => "async",
        }
    }

    /// One instance of every variant (with placeholder dynamic parts), for exhaustiveness tests
    /// and report goldens.
    pub fn catalog() -> Vec<Decline> {
        let mut all = Vec::new();
        all.extend(BodyDecline::ALL.iter().copied().map(Decline::Body));
        all.extend(TypeDecline::ALL.iter().copied().map(Decline::Type));
        all.extend(ObserveDecline::ALL.iter().copied().map(Decline::Observe));
        all.extend(BoundaryDecline::ALL.iter().copied().map(Decline::Boundary));
        all.extend(OpDecline::catalog().into_iter().map(Decline::Op));
        all.extend(CostDecline::catalog().into_iter().map(Decline::Cost));
        all.extend(VersionDecline::catalog().into_iter().map(Decline::Version));
        all.extend(ProofDecline::ALL.iter().copied().map(Decline::Proof));
        all.extend(
            SourceMapDecline::ALL
                .iter()
                .copied()
                .map(Decline::SourceMap),
        );
        all.extend(EncodeDecline::catalog().into_iter().map(Decline::Encode));
        all.extend(MergeDecline::catalog().into_iter().map(Decline::Merge));
        all.extend(LowerDecline::catalog().into_iter().map(Decline::Lower));
        all.extend(AsyncDecline::ALL.iter().copied().map(Decline::Async));
        all
    }
}

impl fmt::Display for Decline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Body(inner) => inner.fmt(f),
            Self::Type(inner) => inner.fmt(f),
            Self::Observe(inner) => inner.fmt(f),
            Self::Boundary(inner) => inner.fmt(f),
            Self::Op(inner) => inner.fmt(f),
            Self::Cost(inner) => inner.fmt(f),
            Self::Version(inner) => inner.fmt(f),
            Self::Proof(inner) => inner.fmt(f),
            Self::SourceMap(inner) => inner.fmt(f),
            Self::Encode(inner) => inner.fmt(f),
            Self::Merge(inner) => inner.fmt(f),
            Self::Lower(inner) => inner.fmt(f),
            Self::Async(inner) => inner.fmt(f),
        }
    }
}

reasons! {
    /// The body as a whole is not usable for the analysis that asked.
    pub enum BodyDecline {
        /// `BaselineReason::BodyUnavailable`.
        Unavailable => ("body.unavailable", "the complete typed body is unavailable"),
        /// `late_lower.rs`.
        ExecutableUnavailable => ("body.executable-unavailable", "the complete executable late-IR body is unavailable"),
        /// `NativeStdDecline::NotAFunction`.
        NotAFunction => ("body.not-a-function", "late-IR item is not a function"),
        /// `NativeStdDecline::NotAStandardOperation`.
        NotAStandardOperation => ("body.not-a-standard-operation", "late-IR body has no canonical standard operation"),
        /// `late_lower.rs`.
        NoNativeSemanticLowering => ("body.no-native-semantic-lowering", "the late-IR body has no native semantic lowering"),
        /// `late_lower.rs`.
        KernelCallerUnavailable => ("body.kernel-caller-unavailable", "native kernel caller has no executable late-IR body"),
        /// `late_lower.rs`.
        PlannerForbidsStructuralDrop => ("body.planner-forbids-structural-drop", "the representation planner did not permit structural drop glue"),
        /// `late_lower.rs`.
        PlannerForbidsStandardBody => ("body.planner-forbids-standard-body", "the representation planner did not permit an optimized standard body"),
    }
}

reasons! {
    /// A type-level legality fact (`BaselineReason` type facts).
    pub enum TypeDecline {
        /// `BaselineReason::UserDefinedDrop`.
        UserDefinedDrop => ("type.user-defined-drop", "user-defined drop behavior must retain the baseline representation"),
        /// `BaselineReason::Unsized`.
        Unsized => ("type.unsized", "the represented value is unsized"),
        /// `BaselineReason::ZeroSized`.
        ZeroSized => ("type.zero-sized", "zero-sized values retain their baseline representation"),
        /// `BaselineReason::UnknownLayout`.
        UnknownLayout => ("type.unknown-layout", "element size or alignment is unavailable"),
        /// `BaselineReason::MixedRepresentations`.
        MixedRepresentations => ("type.mixed-representations", "one value-flow component contains incompatible candidates"),
    }
}

reasons! {
    /// Something can observe the baseline representation (`BaselineReason` observers).
    pub enum ObserveDecline {
        /// `BaselineReason::ForeignCall`.
        ForeignCall => ("observe.foreign-call", "a foreign call can observe the baseline representation"),
        /// `BaselineReason::UnknownCall`.
        UnknownCall => ("observe.unknown-call", "an indirect or unresolved call can observe the representation"),
        /// `BaselineReason::RawProvenance`.
        RawProvenance => ("observe.raw-provenance", "raw provenance or byte access can observe the representation"),
        /// `BaselineReason::StaticEscape`.
        StaticEscape => ("observe.static-escape", "the value escapes into a static"),
    }
}

reasons! {
    /// A pinned boundary (`BaselineReason` boundaries).
    pub enum BoundaryDecline {
        /// `BaselineReason::HostBoundary`.
        Host => ("boundary.host", "the value crosses a pinned host-value boundary"),
        /// `BaselineReason::ViewAbiBoundary`.
        ViewAbi => ("boundary.view-abi", "the value crosses the pinned view ABI"),
        /// `BaselineReason::CRepresentation`.
        CRepresentation => ("boundary.repr-c", "repr(C) fixes the aggregate representation"),
        /// `BaselineReason::CustomAllocator`.
        CustomAllocator => ("boundary.custom-allocator", "a custom allocator can observe the allocation"),
        /// `BaselineReason::UnknownAllocator`.
        UnknownAllocator => ("boundary.unknown-allocator", "the artifact allocator could not be proven to be the default"),
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

/// An unsupported semantic operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OpDecline {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// `BaselineReason::UnsupportedOperation`.
    Unsupported,
    /// A `native_kernel.rs` decline.
    Kernel(KernelOp),
    /// A `native_std_lower.rs` decline. The fields exist in v1 but are never rendered.
    Std(StdOp),
    /// A `native_structural_drop.rs` decline.
    StructuralDrop(StructuralDropOp),
    /// A `late_ir::decode_default_format_template` failure.
    FormatTemplate(FormatTemplateOp),
    /// An `opportunity.rs` iterator-bounds miss.
    Iterator(IteratorOp),
    /// An `opportunity.rs` formatting miss.
    Format(FormatOp),
    /// `optimizer.rs` `ast-switch`.
    AstSwitch,
    /// New in v2: a callee has no interprocedural summary.
    MissingSummary,
}

impl OpDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported => "op.unsupported",
            Self::Kernel(inner) => inner.code(),
            Self::Std(inner) => inner.code(),
            Self::StructuralDrop(inner) => inner.code(),
            Self::FormatTemplate(inner) => inner.code(),
            Self::Iterator(inner) => inner.code(),
            Self::Format(inner) => inner.code(),
            Self::AstSwitch => "op.ast-switch",
            Self::MissingSummary => "op.missing-summary",
        }
    }

    pub fn catalog() -> Vec<OpDecline> {
        let mut all = vec![OpDecline::Unsupported];
        all.extend(KernelOp::ALL.iter().copied().map(OpDecline::Kernel));
        all.extend(StdOp::catalog().into_iter().map(OpDecline::Std));
        all.extend(
            StructuralDropOp::ALL
                .iter()
                .copied()
                .map(OpDecline::StructuralDrop),
        );
        all.extend(
            FormatTemplateOp::ALL
                .iter()
                .copied()
                .map(OpDecline::FormatTemplate),
        );
        all.extend(IteratorOp::ALL.iter().copied().map(OpDecline::Iterator));
        all.extend(FormatOp::ALL.iter().copied().map(OpDecline::Format));
        all.push(OpDecline::AstSwitch);
        all.push(OpDecline::MissingSummary);
        all
    }
}

impl fmt::Display for OpDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => {
                f.write_str("the component contains an unsupported semantic operation")
            }
            Self::Kernel(inner) => inner.fmt(f),
            Self::Std(inner) => inner.fmt(f),
            Self::StructuralDrop(inner) => inner.fmt(f),
            Self::FormatTemplate(inner) => inner.fmt(f),
            Self::Iterator(inner) => inner.fmt(f),
            Self::Format(inner) => inner.fmt(f),
            Self::AstSwitch => f.write_str(
                "switch selection requires a literal discriminant and literal case tests",
            ),
            Self::MissingSummary => f.write_str("the callee has no interprocedural summary"),
        }
    }
}

reasons! {
    /// `native_kernel.rs` (`NativeKernelDecline`), one variant per distinct string.
    pub enum KernelOp {
        StaticMultipleTargets => ("kernel.static-multiple-targets", "native static definition has multiple linked targets"),
        StaticNoTarget => ("kernel.static-no-target", "native static definition has no linked target"),
        FunctionsOnly => ("kernel.functions-only", "native kernel accepts functions only"),
        ExpandedAbi => ("kernel.expanded-abi", "native kernel does not support expanded function ABIs"),
        InvalidTables => ("kernel.invalid-tables", "native kernel received an invalid local or block table"),
        SignatureLocals => ("kernel.signature-locals", "native kernel signature does not match its locals"),
        CleanupBlocks => ("kernel.cleanup-blocks", "native kernel does not support cleanup blocks"),
        CloneNotPermitted => ("kernel.clone-not-permitted", "native kernel plan does not permit an optimized clone"),
        ArrayViewMutable => ("kernel.array-view-mutable", "native fixed-array metadata view is mutable"),
        StructuralOwnDrop => ("kernel.structural-own-drop", "native structural aggregate has its own destructor"),
        StructuralPinned => ("kernel.structural-pinned", "native structural aggregate has a pinned representation boundary"),
        RecursiveType => ("kernel.recursive-type", "native kernel does not support recursive value types"),
        StructuralNoNativeElement => ("kernel.structural-no-native-element", "native structural aggregate has no planned native element"),
        StructuralDropShape => ("kernel.structural-drop-shape", "native structural drop requires an ordinary struct, tuple, or array"),
        StructuralNoNativeField => ("kernel.structural-no-native-field", "native structural aggregate has no planned native field"),
        VecElementUnavailable => ("kernel.vec-element-unavailable", "native Vec element type is unavailable"),
        LocalBaseline => ("kernel.local-baseline", "native kernel local retained its baseline representation"),
        TypePinned => ("kernel.type-pinned", "native kernel type has a pinned representation boundary"),
        PlanTypeMismatch => ("kernel.plan-type-mismatch", "native kernel plan does not match the native Rust type"),
        TypePinnedLayoutOrDrop => ("kernel.type-pinned-layout-or-drop", "native kernel type has pinned layout or drop behavior"),
        ArrayViewNotTemporary => ("kernel.array-view-not-temporary", "native fixed-array metadata view is not temporary"),
        ArrayViewMultipleDefinitions => ("kernel.array-view-multiple-definitions", "native fixed-array metadata view has multiple definitions"),
        ArrayViewProducer => ("kernel.array-view-producer", "native fixed-array metadata view has an unsupported producer"),
        ArrayViewResultType => ("kernel.array-view-result-type", "native fixed-array metadata view has inconsistent result type"),
        ArrayViewSource => ("kernel.array-view-source", "native fixed-array metadata view has an unsupported source"),
        ArrayViewProjected => ("kernel.array-view-projected", "native fixed-array metadata view is projected"),
        ArrayViewEscapes => ("kernel.array-view-escapes", "native fixed-array metadata view escapes"),
        ArrayViewNoProducer => ("kernel.array-view-no-producer", "native fixed-array metadata view has no exact producer"),
        ArrayViewUses => ("kernel.array-view-uses", "native fixed-array metadata view has unsupported uses"),
        ArraySourceNotImmutable => ("kernel.array-source-not-immutable", "native fixed-array metadata source is not immutable"),
        ArraySourceNotArray => ("kernel.array-source-not-array", "native fixed-array metadata source is not an array"),
        ArraySourceMismatch => ("kernel.array-source-mismatch", "native fixed-array metadata element or length does not match"),
        StaticReferenceType => ("kernel.static-reference-type", "native static reference does not match its linked target type"),
        StaticArrayInvalidLocal => ("kernel.static-array-invalid-local", "native static array reference has an invalid local"),
        StaticArrayMultipleDefinitions => ("kernel.static-array-multiple-definitions", "native static array reference has multiple definitions"),
        StaticArrayProducer => ("kernel.static-array-producer", "native static array reference has an unsupported producer"),
        StaticArrayTypeMetadata => ("kernel.static-array-type-metadata", "native static array reference has inconsistent type metadata"),
        PointerConstant => ("kernel.pointer-constant", "native kernel does not support this pointer constant"),
        StaticArrayNoProducer => ("kernel.static-array-no-producer", "native static array reference has no exact producer"),
        StringViewNotSharedStr => ("kernel.string-view-not-shared-str", "native string view is not an exact baseline shared str"),
        SliceViewNotMutable => ("kernel.slice-view-not-mutable", "native slice view is not a mutable slice reference"),
        SliceViewPinned => ("kernel.slice-view-pinned", "native slice view has a pinned reference ABI"),
        SliceViewNotSlice => ("kernel.slice-view-not-slice", "native slice view is not a slice reference"),
        SliceViewMetadata => ("kernel.slice-view-metadata", "native slice view has non-slice representation metadata"),
        SliceViewNoSlice => ("kernel.slice-view-no-slice", "native slice view does not reference a slice"),
        ResultShape => ("kernel.result-shape", "native kernel canonical Result shape is incomplete"),
        ArrayTooLong => ("kernel.array-too-long", "native kernel array exceeds JavaScript's length limit"),
        NoScalar => ("kernel.no-scalar", "native kernel type has no native scalar representation"),
        SetDiscriminant => ("kernel.set-discriminant", "native kernel does not yet initialize discriminants independently"),
        MemoryCopy => ("kernel.memory-copy", "native kernel does not support memory copies"),
        TerminatorUnwinds => ("kernel.terminator-unwinds", "native kernel terminator can unwind or panic"),
        AssertTargetCleanup => ("kernel.assert-target-cleanup", "native bounds assert target is not a normal block"),
        AssertUnwind => ("kernel.assert-unwind", "native bounds assert has observable unwind behavior"),
        AssertNotBounds => ("kernel.assert-not-bounds", "native kernel assert is not an exact bounds check"),
        AssertOperandTypes => ("kernel.assert-operand-types", "native bounds assert has inconsistent operand types"),
        AssertBoolConstant => ("kernel.assert-bool-constant", "native static assert has an invalid boolean constant"),
        DropTargetCleanup => ("kernel.drop-target-cleanup", "native drop target is not a normal block"),
        AsyncDrop => ("kernel.async-drop", "native kernel does not support asynchronous drop glue"),
        DropUnwindNotCleanup => ("kernel.drop-unwind-not-cleanup", "native drop unwind edge is not a cleanup block"),
        DirectDropUnplanned => ("kernel.direct-drop-unplanned", "native direct drop requires a planned representation"),
        DirectDropNoProof => ("kernel.direct-drop-no-proof", "direct drop target lacks a prepared nonthrowing native proof"),
        DirectDropOwnDrop => ("kernel.direct-drop-own-drop", "native direct drop place has its own destructor"),
        DirectDropNoNativeValue => ("kernel.direct-drop-no-native-value", "native direct drop place has no planned native value"),
        DynamicDrop => ("kernel.dynamic-drop", "native kernel does not support dynamic drop glue"),
        DirectDropProjection => ("kernel.direct-drop-projection", "native direct drop place uses an unsupported projection"),
        FormatCallNoProof => ("kernel.format-call-no-proof", "native format machinery call has no inline proof"),
        FormatUnprovenCall => ("kernel.format-unproven-call", "native format pipeline captured an unproven call"),
        FormatCallNoReturn => ("kernel.format-call-no-return", "native format call does not return"),
        FormatNoArguments => ("kernel.format-no-arguments", "native format! has no Arguments value"),
        FormatDestinationProjected => ("kernel.format-destination-projected", "native format! destination is projected"),
        FormatDestinationUnplanned => ("kernel.format-destination-unplanned", "native format! destination has no planned representation"),
        FormatResultNotString => ("kernel.format-result-not-string", "native format! result is not a native String"),
        FormatNoReturn => ("kernel.format-no-return", "native format! does not return"),
        WriteFmtShape => ("kernel.write-fmt-shape", "native write_fmt does not have a String receiver and Arguments"),
        WriteFmtReceiverNotPlace => ("kernel.write-fmt-receiver-not-place", "native write_fmt receiver is not a place"),
        WriteFmtReceiverNotString => ("kernel.write-fmt-receiver-not-string", "native write_fmt receiver is not a mutable native String"),
        WriteFmtDestinationProjected => ("kernel.write-fmt-destination-projected", "native write_fmt destination is projected"),
        WriteFmtNoReturn => ("kernel.write-fmt-no-return", "native write_fmt does not return"),
        ArgumentsNotLocal => ("kernel.arguments-not-local", "native Arguments value is not a local"),
        ArgumentsProjected => ("kernel.arguments-projected", "native Arguments value is projected"),
        ArgumentsNotNew => ("kernel.arguments-not-new", "native Arguments value is not built by Arguments::new"),
        ArgumentsNewShape => ("kernel.arguments-new-shape", "native Arguments::new does not have a template and argument list"),
        FormatArity => ("kernel.format-arity", "native format template and argument list have different arities"),
        FormatTemplateNotConstant => ("kernel.format-template-not-constant", "native format template is not a decoded byte constant"),
        FormatArgumentsNotAggregate => ("kernel.format-arguments-not-aggregate", "native format arguments are not one compiler-generated aggregate"),
        FormatArgumentNotLocal => ("kernel.format-argument-not-local", "native format argument is not a local"),
        FormatArgumentProjected => ("kernel.format-argument-projected", "native format argument is projected"),
        FormatArgumentNotNewDisplay => ("kernel.format-argument-not-new-display", "native format argument is not built by Argument::new_display"),
        NewDisplayNoValue => ("kernel.new-display-no-value", "native Argument::new_display has no value"),
        DisplayNotPlace => ("kernel.display-not-place", "native Display argument is not a place"),
        DisplayNotDefault => ("kernel.display-not-default", "native format argument is not a proven default Display type"),
        DisplayStringUnproven => ("kernel.display-string-unproven", "native format String argument has no proven native representation"),
        DirectCallsOnly => ("kernel.direct-calls-only", "native kernel supports direct calls only"),
        CallBoundary => ("kernel.call-boundary", "native kernel call crosses an unsupported boundary"),
        CallUnwind => ("kernel.call-unwind", "native kernel call has observable unwind behavior"),
        CallNoReturn => ("kernel.call-no-return", "native kernel call does not return"),
        AsStrNoProof => ("kernel.as-str-no-proof", "native String::as_str call has no exact string-view proof"),
        AppendNoProof => ("kernel.append-no-proof", "native String append call has no exact string-view proof"),
        StringViewCall => ("kernel.string-view-call", "native string view crossed an unsupported call"),
        ViewCallProjected => ("kernel.view-call-projected", "native view call has a projected destination"),
        SliceViewOnlySwap => ("kernel.slice-view-only-swap", "native slice view may only enter its exact slice-swap operation"),
        ReferenceKind => ("kernel.reference-kind", "native kernel does not support this reference kind"),
        RvalueAddress => ("kernel.rvalue-address", "native kernel rvalue observes an address or allocation"),
        ArrayMetadataNotUsize => ("kernel.array-metadata-not-usize", "native fixed-array metadata result is not target usize"),
        PointerMetadataUnproven => ("kernel.pointer-metadata-unproven", "native pointer metadata operand is not a proven fixed-array view"),
        RuntimeCheckNotBool => ("kernel.runtime-check-not-bool", "native runtime check is not boolean"),
        StringConstantNotSharedStr => ("kernel.string-constant-not-shared-str", "native string constant is not an exact shared str"),
        MemoryConstant => ("kernel.memory-constant", "native kernel does not lower decoded memory constants"),
        ZeroSizedConstantType => ("kernel.zero-sized-constant-type", "native zero-sized constant has a non-zero-sized type"),
        EnumConstantNoDiscriminant => ("kernel.enum-constant-no-discriminant", "native scalar enum constant has no matching discriminant"),
        EnumConstantAmbiguous => ("kernel.enum-constant-ambiguous", "native scalar enum constant has an ambiguous discriminant"),
        EnumConstantPayload => ("kernel.enum-constant-payload", "native scalar enum constant has a non-zero-sized payload"),
        ResultConstantVariant => ("kernel.result-constant-variant", "native scalar Result constant has an invalid variant"),
        ResultConstantPayload => ("kernel.result-constant-payload", "native scalar Result constant has an invalid payload"),
        StaticTargetNotImmutable => ("kernel.static-target-not-immutable", "native static target is not an immutable reference"),
        StaticTargetNotArray => ("kernel.static-target-not-array", "native static target is not a fixed array"),
        StaticTargetNotStringArray => ("kernel.static-target-not-string-array", "native static target is not a fixed shared-string array"),
        CharConstant => ("kernel.char-constant", "native char constant is invalid"),
        ScalarConstantType => ("kernel.scalar-constant-type", "native scalar constant has a non-scalar type"),
        OverflowNotInteger => ("kernel.overflow-not-integer", "native overflowing binary operation is not integer arithmetic"),
        OverflowResult => ("kernel.overflow-result", "native overflowing binary operation needs an exact `(integer, bool)` result"),
        BinaryPanic => ("kernel.binary-panic", "native kernel binary operation needs panic or overflow support"),
        Unary => ("kernel.unary", "native kernel unary operation has unsupported semantics"),
        Cast => ("kernel.cast", "native kernel cast observes a pointer or needs saturating conversion"),
        ArithmeticNotNumeric => ("kernel.arithmetic-not-numeric", "native arithmetic result is not numeric"),
        ResultAggregateVariant => ("kernel.result-aggregate-variant", "native Result aggregate has an invalid variant"),
        ResultAggregatePayload => ("kernel.result-aggregate-payload", "native Result aggregate has an invalid payload"),
        AggregateForm => ("kernel.aggregate-form", "native aggregate has an unsupported nominal form"),
        AggregateKindMismatch => ("kernel.aggregate-kind-mismatch", "native aggregate kind does not match its result type"),
        StringViewWhole => ("kernel.string-view-whole", "native string view may only be transported as a whole value"),
        SliceViewForward => ("kernel.slice-view-forward", "native slice view may only be forwarded to slice swap"),
        PointerViewDeref => ("kernel.pointer-view-deref", "native pointer view must start with one dereference"),
        PointerViewInvalidLocal => ("kernel.pointer-view-invalid-local", "native pointer view has an invalid local"),
        PointerViewNotReference => ("kernel.pointer-view-not-reference", "native pointer view does not have a reference type"),
        PointerViewProjection => ("kernel.pointer-view-projection", "native pointer view only supports typed structural field projections"),
        FieldProjectionType => ("kernel.field-projection-type", "native field projection does not match its type"),
        PointerViewMetadata => ("kernel.pointer-view-metadata", "native pointer-view projection has inconsistent type metadata"),
        PlaceInvalidLocal => ("kernel.place-invalid-local", "native place has an invalid local"),
        DerefNotReference => ("kernel.deref-not-reference", "native dereference is not applied to a reference"),
        ConsecutiveDowncasts => ("kernel.consecutive-downcasts", "native enum place has consecutive downcasts"),
        DowncastNotEnum => ("kernel.downcast-not-enum", "native downcast is not applied to an enum"),
        DowncastVariant => ("kernel.downcast-variant", "native enum downcast has an invalid variant"),
        ResultPayloadField => ("kernel.result-payload-field", "native Result payload has an invalid field"),
        IndexInvalidLocal => ("kernel.index-invalid-local", "native index has an invalid local"),
        IndexNotUsize => ("kernel.index-not-usize", "native array index is not a target usize"),
        ConstantIndexNotArray => ("kernel.constant-index-not-array", "native constant index is not applied to an array"),
        ConstantIndexMinLength => ("kernel.constant-index-min-length", "native constant index has an invalid minimum length"),
        ConstantIndexBounds => ("kernel.constant-index-bounds", "native constant index is out of bounds"),
        ProjectionSliceLayout => ("kernel.projection-slice-layout", "native place projection observes a slice layout"),
        ProjectionMetadata => ("kernel.projection-metadata", "native place projection has inconsistent type metadata"),
        DiscriminantNotEnum => ("kernel.discriminant-not-enum", "native discriminant operation is not applied to an enum"),
        TypeUnavailable => ("kernel.type-unavailable", "native type definition is unavailable"),
        ReferenceResult => ("kernel.reference-result", "native reference rvalue has a non-reference result"),
        ReferenceMetadata => ("kernel.reference-metadata", "native reference rvalue has inconsistent type metadata"),
        PrimitiveCarrierSource => ("kernel.primitive-carrier-source", "projected primitive reference carrier has an invalid source"),
        ReferenceNoIdentity => ("kernel.reference-no-identity", "native reference pointee has no stable JavaScript identity"),
        FieldProjectionAggregate => ("kernel.field-projection-aggregate", "native field projection does not match its aggregate"),
        IndexNotArray => ("kernel.index-not-array", "native index is not applied to an array"),
        CopyForDerefMetadata => ("kernel.copy-for-deref-metadata", "native CopyForDeref has inconsistent type metadata"),
        CopyForDerefNotIdentity => ("kernel.copy-for-deref-not-identity", "native CopyForDeref is not a planner-selected identity operation"),
        AggregateCopy => ("kernel.aggregate-copy", "native kernel does not yet clone aggregate Copy operands"),
        BitCopyZeroSizedLeaf => ("kernel.bit-copy-zero-sized-leaf", "native bit copy plan does not match its zero-sized leaf"),
        BitCopyOwnDrop => ("kernel.bit-copy-own-drop", "native bit copy encountered a type with its own destructor"),
        BitCopyPinned => ("kernel.bit-copy-pinned", "native bit copy encountered a pinned representation boundary"),
        BitCopyUnplannedLeaf => ("kernel.bit-copy-unplanned-leaf", "native bit copy encountered an unplanned native leaf"),
        BitCopyScalarLeaf => ("kernel.bit-copy-scalar-leaf", "native bit copy plan does not match its scalar leaf"),
        BitCopyRecursive => ("kernel.bit-copy-recursive", "native bit copy encountered a recursive value type"),
        BitCopyArrayLength => ("kernel.bit-copy-array-length", "native bit copy array length does not fit the host"),
        BitCopyPointer => ("kernel.bit-copy-pointer", "native bit copy encountered an unsupported reference or pointer"),
        BitCopyEnumOrUnion => ("kernel.bit-copy-enum-or-union", "native bit copy encountered an enum or union"),
        BitCopyLeaf => ("kernel.bit-copy-leaf", "native bit copy encountered an unsupported leaf type"),
    }
}

/// A signature slot of a standard operation (`native_std_lower.rs::SignatureSlot`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SignatureSlot {
    /// Append-only: postcard serializes this enum by index.
    Result,
    Argument(u16),
}

/// `native_std_lower.rs::NativeStdDecline`. Fields are carried but, exactly as in v1, never
/// rendered.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StdOp {
    /// Append-only: postcard serializes this enum by index.
    UnsupportedOperation(StandardOperation),
    SpreadAbi,
    CallerLocationAbi,
    ParameterCount {
        expected: u16,
        actual: u16,
    },
    SignatureArgumentCount {
        expected: u16,
        actual: u16,
    },
    MissingSignatureLocal(SignatureSlot),
    MissingType(TypeId),
    WrongReferenceKind(SignatureSlot),
    WrongNativeType(SignatureSlot),
    WrongArgumentType(u16),
    MissingRepresentation(SignatureSlot),
    WrongRepresentation(SignatureSlot),
    WrongReturnType,
    UnknownElementLayout,
    ZeroSizedElement,
    ElementDropRequired,
    ElementHasOwnDrop,
    NoncanonicalStringDrop,
    UnsupportedPointerWidth(u32),
}

impl StdOp {
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedOperation(_) => "std.unsupported-operation",
            Self::SpreadAbi => "std.spread-abi",
            Self::CallerLocationAbi => "std.caller-location-abi",
            Self::ParameterCount { .. } => "std.parameter-count",
            Self::SignatureArgumentCount { .. } => "std.signature-argument-count",
            Self::MissingSignatureLocal(_) => "std.missing-signature-local",
            Self::MissingType(_) => "std.missing-type",
            Self::WrongReferenceKind(_) => "std.wrong-reference-kind",
            Self::WrongNativeType(_) => "std.wrong-native-type",
            Self::WrongArgumentType(_) => "std.wrong-argument-type",
            Self::MissingRepresentation(_) => "std.missing-representation",
            Self::WrongRepresentation(_) => "std.wrong-representation",
            Self::WrongReturnType => "std.wrong-return-type",
            Self::UnknownElementLayout => "std.unknown-element-layout",
            Self::ZeroSizedElement => "std.zero-sized-element",
            Self::ElementDropRequired => "std.element-drop-required",
            Self::ElementHasOwnDrop => "std.element-has-own-drop",
            Self::NoncanonicalStringDrop => "std.noncanonical-string-drop",
            Self::UnsupportedPointerWidth(_) => "std.unsupported-pointer-width",
        }
    }

    /// The exact v1 `reason()` text.
    pub const fn text(self) -> &'static str {
        match self {
            Self::UnsupportedOperation(_) => {
                "canonical standard operation has no exact native body lowering"
            }
            Self::SpreadAbi => "standard operation uses an unsupported spread ABI",
            Self::CallerLocationAbi => "standard operation uses an unsupported caller-location ABI",
            Self::ParameterCount { .. } => {
                "JavaScript ABI parameter count does not match the canonical operation"
            }
            Self::SignatureArgumentCount { .. } => {
                "Rust signature argument count does not match the canonical operation"
            }
            Self::MissingSignatureLocal(_) => "late-IR signature slot has no matching local",
            Self::MissingType(_) => "late-IR signature references a missing type definition",
            Self::WrongReferenceKind(_) => {
                "standard operation receiver has the wrong reference mutability"
            }
            Self::WrongNativeType(_) => {
                "standard operation signature slot has the wrong native Rust type"
            }
            Self::WrongArgumentType(_) => "standard operation argument has an unexpected Rust type",
            Self::MissingRepresentation(_) => {
                "standard operation signature slot has no planned representation"
            }
            Self::WrongRepresentation(_) => {
                "standard operation signature slot did not select the required representation"
            }
            Self::WrongReturnType => "standard operation has an unexpected Rust return type",
            Self::UnknownElementLayout => "native Vec element layout is unavailable",
            Self::ZeroSizedElement => "native Vec does not support zero-sized elements",
            Self::ElementDropRequired => {
                "native Vec operation requires executable element drop glue"
            }
            Self::ElementHasOwnDrop => {
                "native Vec element has a custom destructor and stays on baseline"
            }
            Self::NoncanonicalStringDrop => {
                "compiler String drop does not have the exact baseline Vec<u8> fallback"
            }
            Self::UnsupportedPointerWidth(_) => {
                "native capacity lowering requires a JavaScript-number pointer width"
            }
        }
    }

    pub fn catalog() -> Vec<StdOp> {
        vec![
            StdOp::UnsupportedOperation(StandardOperation::VecNew),
            StdOp::SpreadAbi,
            StdOp::CallerLocationAbi,
            StdOp::ParameterCount {
                expected: 0,
                actual: 0,
            },
            StdOp::SignatureArgumentCount {
                expected: 0,
                actual: 0,
            },
            StdOp::MissingSignatureLocal(SignatureSlot::Result),
            StdOp::MissingType(TypeId(0)),
            StdOp::WrongReferenceKind(SignatureSlot::Result),
            StdOp::WrongNativeType(SignatureSlot::Result),
            StdOp::WrongArgumentType(0),
            StdOp::MissingRepresentation(SignatureSlot::Result),
            StdOp::WrongRepresentation(SignatureSlot::Result),
            StdOp::WrongReturnType,
            StdOp::UnknownElementLayout,
            StdOp::ZeroSizedElement,
            StdOp::ElementDropRequired,
            StdOp::ElementHasOwnDrop,
            StdOp::NoncanonicalStringDrop,
            StdOp::UnsupportedPointerWidth(0),
        ]
    }
}

impl fmt::Display for StdOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

reasons! {
    /// `native_structural_drop.rs` (`NativeStructuralDropDecline`).
    pub enum StructuralDropOp {
        CloneNotPermitted => ("structural-drop.clone-not-permitted", "native structural drop plan does not permit an optimized clone"),
        ArgumentNoNativeField => ("structural-drop.argument-no-native-field", "structural drop argument has no planned native field"),
        FunctionsOnly => ("structural-drop.functions-only", "native structural drop accepts functions only"),
        NotDropGlue => ("structural-drop.not-drop-glue", "late-IR body is not compiler-generated drop glue"),
        InvalidTypeTable => ("structural-drop.invalid-type-table", "native structural drop received an invalid type table"),
        NoDrop => ("structural-drop.no-drop", "structural drop type does not need drop"),
        OwnDrop => ("structural-drop.own-drop", "structural drop type has its own destructor"),
        Cycle => ("structural-drop.cycle", "structural drop normal CFG contains a cycle"),
        EntersCleanup => ("structural-drop.enters-cleanup", "structural drop normal path enters cleanup code"),
        Statement => ("structural-drop.statement", "structural drop contains an executable statement"),
        Replace => ("structural-drop.replace", "structural drop uses replace semantics"),
        AsyncGlue => ("structural-drop.async-glue", "structural drop uses asynchronous drop glue"),
        DirectDropNoProof => ("structural-drop.direct-drop-no-proof", "direct drop target lacks a prepared nonthrowing native proof"),
        DirectDropNoNativeValue => ("structural-drop.direct-drop-no-native-value", "direct drop place has no planned native value"),
        DynamicGlue => ("structural-drop.dynamic-glue", "structural drop uses dynamic drop glue"),
        NotOneChain => ("structural-drop.not-one-chain", "structural drop normal CFG is not one drop chain"),
        UnvisitedBlock => ("structural-drop.unvisited-block", "structural drop has an unvisited normal block"),
        PlaceNotArgument => ("structural-drop.place-not-argument", "structural drop place is not rooted at its argument"),
        PlaceDeref => ("structural-drop.place-deref", "structural drop place has an unexpected dereference"),
        FieldOutOfBounds => ("structural-drop.field-out-of-bounds", "structural drop field projection is out of bounds"),
        FieldWrongType => ("structural-drop.field-wrong-type", "structural drop field projection has the wrong type"),
        PlaceProjection => ("structural-drop.place-projection", "structural drop place uses an unsupported projection"),
        PlaceNotField => ("structural-drop.place-not-field", "structural drop place is not an encoded field projection"),
        PlaceFinalType => ("structural-drop.place-final-type", "structural drop place has the wrong final type"),
        FunctionAbi => ("structural-drop.function-abi", "structural drop has an unsupported Rust function ABI"),
        ReturnNotUnit => ("structural-drop.return-not-unit", "structural drop does not return unit"),
        ArgumentNotMutRef => ("structural-drop.argument-not-mut-ref", "structural drop argument is not an exact mutable reference"),
        SignatureLocals => ("structural-drop.signature-locals", "structural drop signature does not match its local table"),
        InvalidBlockTable => ("structural-drop.invalid-block-table", "structural drop received an invalid block table"),
        StandardCandidate => ("structural-drop.standard-candidate", "structural drop type is itself a native standard candidate"),
        NotRustAbi => ("structural-drop.not-rust-abi", "structural drop type does not use the Rust ABI"),
        Pinned => ("structural-drop.pinned", "structural drop type crosses a pinned boundary"),
        NoLayout => ("structural-drop.no-layout", "structural drop type has no supported aggregate layout"),
        NotStructOrTuple => ("structural-drop.not-struct-or-tuple", "structural drop type is not a one-variant struct or tuple"),
        MissingType => ("structural-drop.missing-type", "structural drop references a missing type"),
        MissingLocal => ("structural-drop.missing-local", "structural drop references a missing local"),
        MissingBlock => ("structural-drop.missing-block", "structural drop references a missing block"),
        UnwindNotCleanup => ("structural-drop.unwind-not-cleanup", "structural drop unwind edge does not target cleanup code"),
    }
}

reasons! {
    /// `late_ir::decode_default_format_template` failures.
    pub enum FormatTemplateOp {
        NoTerminator => ("format-template.no-terminator", "format template byte sequence has no terminator"),
        TrailingBytes => ("format-template.trailing-bytes", "format template has bytes after its terminator"),
        PieceOverrun => ("format-template.piece-overrun", "format literal piece runs past the template"),
        PieceNotUtf8 => ("format-template.piece-not-utf8", "format literal piece is not UTF-8"),
        LongLiteralNoLength => ("format-template.long-literal-no-length", "long format literal has no encoded length"),
        UsesFlags => ("format-template.uses-flags", "format template uses flags, width, precision, or explicit argument selection"),
    }
}

reasons! {
    /// `opportunity.rs` iterator-bounds misses.
    pub enum IteratorOp {
        NoConcreteReceiver => ("iterator.no-concrete-receiver", "Iterator::next has no concrete receiver"),
        NotOptionDispatch => ("iterator.not-option-dispatch", "Iterator::next result does not feed the canonical Option loop dispatch"),
        NotLoop => ("iterator.not-loop", "Iterator::next dispatch is not part of a canonical loop"),
        NotConcrete => ("iterator.not-concrete", "iterator type is not a concrete Range, slice, array, or simple Enumerate graph"),
        RangeNoElement => ("iterator.range-no-element", "Range has no concrete element type"),
        RangeNotInteger => ("iterator.range-not-integer", "range element is not a fixed-width Rust integer"),
        ArrayNoElement => ("iterator.array-no-element", "array iterator has no concrete element type"),
        ArrayElementObservable => ("iterator.array-element-observable", "array iterator element move or drop behavior is observable"),
        NestedAdaptors => ("iterator.nested-adaptors", "nested or dynamic iterator adaptors are not eligible for scalarization"),
        EnumerateNoChild => ("iterator.enumerate-no-child", "Enumerate has no concrete child iterator type"),
    }
}

reasons! {
    /// `opportunity.rs` formatting misses.
    pub enum FormatOp {
        TemplateNotConstant => ("format.template-not-constant", "format template is not a compiler-generated constant byte sequence"),
        ArgumentsNotAggregate => ("format.arguments-not-aggregate", "format arguments do not come from one compiler-generated aggregate"),
        Arity => ("format.arity", "format template and argument aggregate have different arities"),
        UnrecognizedSink => ("format.unrecognized-sink", "format arguments are consumed by an unrecognized formatter sink"),
        Float => ("format.float", "default Display for floats remains on Rust formatting"),
        UnsupportedDisplay => ("format.unsupported-display", "default Display implementation is not one of str, String, char, bool, or integers"),
        NotDisplay => ("format.not-display", "format argument uses Debug or another non-Display formatter"),
    }
}

/// A cost gate declined.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CostDecline {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// `late_lower.rs::REGION_NO_SHRINK`.
    RegionNoShrink,
    /// `late_lower.rs`: `"native region [{members}] does not shrink the complete reachable item
    /// graph ({old_bytes} -> {new_bytes} bytes)"`.
    RegionNoShrinkBytes {
        members: Vec<String>,
        old_bytes: u64,
        new_bytes: u64,
    },
    /// `late_lower.rs`.
    KernelNotSmaller,
    /// `optimizer.rs` `ast-dce`.
    AstDce,
    /// New in v2: a candidate whose measured cost did not improve. `before == after` is a
    /// regression-test failure (finding on 40/46 v1 declines).
    NoImprovement { before: u64, after: u64 },
    /// New in v2: an optimizer budget (inline bytes, versions, PEA rounds, wall time) was
    /// exhausted; `budget` names which.
    Budget { budget: String },
}

impl CostDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RegionNoShrink => "cost.region-no-shrink",
            Self::RegionNoShrinkBytes { .. } => "cost.region-no-shrink-bytes",
            Self::KernelNotSmaller => "cost.kernel-not-smaller",
            Self::AstDce => "cost.ast-dce",
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
            CostDecline::KernelNotSmaller,
            CostDecline::AstDce,
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
            Self::KernelNotSmaller => f.write_str(
                "native language kernel is not smaller and removes no linked dependency",
            ),
            Self::AstDce => {
                f.write_str("a rewrite was declined because it did not reduce the AST")
            }
            Self::NoImprovement { before, after } => {
                write!(f, "does not improve ({before} -> {after})")
            }
            Self::Budget { budget } => write!(f, "the `{budget}` budget is exhausted"),
        }
    }
}

/// Schema, clone, sidecar, carrier-bridge, and ABI versioning.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VersionDecline {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// `"late-IR analysis summary has format {found}, but this backend reads format {expected}"`.
    Schema { found: u16, expected: u16 },
    /// `"baseline shadow summary has format {found}, but this backend reads format {expected}"`.
    BaselineShadowSchema { found: u16, expected: u16 },
    /// `late_lower.rs::NATIVE_CLONE_COLLISION`.
    NativeCloneCollision,
    /// `late_lower.rs::NATIVE_CLONE_NON_FUNCTION`.
    NativeCloneNonFunction,
    /// `late_lower.rs::NATIVE_CLONE_MIXED_REFERENCE`.
    NativeCloneMixedReference,
    /// `late_lower.rs::NATIVE_EXPORT_INCOMPLETE_INCOMING`.
    NativeExportIncompleteIncoming,
    /// `late_lower.rs::BASELINE_SIDECAR_COLLISION`.
    BaselineSidecarCollision,
    /// `late_lower.rs::BASELINE_SIDECAR_NON_FUNCTION`.
    BaselineSidecarNonFunction,
    /// `late_lower.rs::BASELINE_SIDECAR_NOT_IN_PLACE`.
    BaselineSidecarNotInPlace,
    /// `late_lower.rs::BASELINE_SIDECAR_NONCANONICAL`.
    BaselineSidecarNoncanonical,
    /// `late_lower.rs::CARRIER_BRIDGE_TARGET`.
    CarrierBridgeTarget,
    /// `late_lower.rs::CARRIER_BRIDGE_PRODUCER`.
    CarrierBridgeProducer,
    /// `late_lower.rs::CARRIER_BRIDGE_PRODUCER_ABI`.
    CarrierBridgeProducerAbi,
    /// `late_lower.rs::CARRIER_BRIDGE_AS_STR`.
    CarrierBridgeAsStr,
    /// `late_lower.rs::CARRIER_BRIDGE_DROP`.
    CarrierBridgeDrop,
    /// `late_lower.rs::CARRIER_BRIDGE_NOT_IN_PLACE`.
    CarrierBridgeNotInPlace,
    /// New in v2: the bounded ABI-version search declined another version; `versions` is the
    /// count already present.
    AbiVersions { versions: u32 },
    /// New in v2 (Phase 4): a representation commit failed `Func::verify` after the carriers were
    /// applied, so the whole strongly-connected component was rolled back and its items kept their
    /// originals. `reason` is the verifier's message, which names the function and the value.
    CarrierCommitRollback { reason: String },
}

impl VersionDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Schema { .. } => "version.schema",
            Self::BaselineShadowSchema { .. } => "version.baseline-shadow-schema",
            Self::NativeCloneCollision => "version.native-clone-collision",
            Self::NativeCloneNonFunction => "version.native-clone-non-function",
            Self::NativeCloneMixedReference => "version.native-clone-mixed-reference",
            Self::NativeExportIncompleteIncoming => "version.native-export-incomplete-incoming",
            Self::BaselineSidecarCollision => "version.baseline-sidecar-collision",
            Self::BaselineSidecarNonFunction => "version.baseline-sidecar-non-function",
            Self::BaselineSidecarNotInPlace => "version.baseline-sidecar-not-in-place",
            Self::BaselineSidecarNoncanonical => "version.baseline-sidecar-noncanonical",
            Self::CarrierBridgeTarget => "version.carrier-bridge-target",
            Self::CarrierBridgeProducer => "version.carrier-bridge-producer",
            Self::CarrierBridgeProducerAbi => "version.carrier-bridge-producer-abi",
            Self::CarrierBridgeAsStr => "version.carrier-bridge-as-str",
            Self::CarrierBridgeDrop => "version.carrier-bridge-drop",
            Self::CarrierBridgeNotInPlace => "version.carrier-bridge-not-in-place",
            Self::AbiVersions { .. } => "version.abi-versions",
            Self::CarrierCommitRollback { .. } => "version.carrier-commit-rollback",
        }
    }

    pub fn catalog() -> Vec<VersionDecline> {
        vec![
            VersionDecline::Schema {
                found: 0,
                expected: 1,
            },
            VersionDecline::BaselineShadowSchema {
                found: 0,
                expected: 1,
            },
            VersionDecline::NativeCloneCollision,
            VersionDecline::NativeCloneNonFunction,
            VersionDecline::NativeCloneMixedReference,
            VersionDecline::NativeExportIncompleteIncoming,
            VersionDecline::BaselineSidecarCollision,
            VersionDecline::BaselineSidecarNonFunction,
            VersionDecline::BaselineSidecarNotInPlace,
            VersionDecline::BaselineSidecarNoncanonical,
            VersionDecline::CarrierBridgeTarget,
            VersionDecline::CarrierBridgeProducer,
            VersionDecline::CarrierBridgeProducerAbi,
            VersionDecline::CarrierBridgeAsStr,
            VersionDecline::CarrierBridgeDrop,
            VersionDecline::CarrierBridgeNotInPlace,
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
            Self::Schema { found, expected } => write!(
                f,
                "late-IR analysis summary has format {found}, but this backend reads format {expected}"
            ),
            Self::BaselineShadowSchema { found, expected } => write!(
                f,
                "baseline shadow summary has format {found}, but this backend reads format {expected}"
            ),
            Self::NativeCloneCollision => {
                f.write_str("native clone name collides with an existing identifier")
            }
            Self::NativeCloneNonFunction => {
                f.write_str("native clone first slice supports functions only")
            }
            Self::NativeCloneMixedReference => f.write_str(
                "one prepared body needs both native and baseline variants of the same target",
            ),
            Self::NativeExportIncompleteIncoming => {
                f.write_str("native export has an unstaged incoming baseline reference")
            }
            Self::BaselineSidecarCollision => {
                f.write_str("baseline sidecar name collides with an existing identifier")
            }
            Self::BaselineSidecarNonFunction => {
                f.write_str("baseline sidecar supports functions only")
            }
            Self::BaselineSidecarNotInPlace => {
                f.write_str("compiler String adapter must keep its original linker identity")
            }
            Self::BaselineSidecarNoncanonical => {
                f.write_str("compiler String adapter has a noncanonical baseline sidecar name")
            }
            Self::CarrierBridgeTarget => {
                f.write_str("native String carrier bridge has no exact reachable target")
            }
            Self::CarrierBridgeProducer => f.write_str(
                "native String carrier bridge producer has no prepared native String result",
            ),
            Self::CarrierBridgeProducerAbi => f.write_str(
                "native String carrier bridge producer has no exact baseline-compatible argument ABI",
            ),
            Self::CarrierBridgeAsStr => f.write_str(
                "native String carrier bridge has no exact compiler StringAsStr adapter",
            ),
            Self::CarrierBridgeDrop => f.write_str(
                "native String carrier bridge has no exact compiler String drop adapter",
            ),
            Self::CarrierBridgeNotInPlace => f.write_str(
                "native String carrier bridge targets must keep their original linker identities",
            ),
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
        /// `late_lower.rs::UNVERIFIED_BASELINE`.
        UnverifiedBaseline => ("proof.unverified-baseline", "native replacement requires an identity-gated baseline shadow"),
        /// `opportunity.rs`.
        BoundsCheck => ("proof.bounds-check", "bounds check lacks a dominating same-length iterator proof"),
        /// New in v2: an optimization's witness failed its independent check.
        Witness => ("proof.witness", "the optimization witness failed its independent check"),
    }
}

reasons! {
    /// Source maps are not reproduced through the transform.
    pub enum SourceMapDecline {
        /// `late_lower.rs::REGION_SOURCE_MAP`.
        Region => ("source-map.region", "native regions do not yet reproduce source-map entries"),
        /// `late_lower.rs`.
        Kernel => ("source-map.kernel", "the native language kernel does not yet reproduce source-map entries"),
        /// `link_dedup.rs`.
        Dedup => ("source-map.dedup", "duplicate item merge does not yet rebase source-map entries"),
    }
}

/// The producer could not encode the item, or the encoding failed to verify.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EncodeDecline {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// `late_ir.rs::SYNTHESIZED_ITEM`: allocator shims, `global_asm`, and other items without a
    /// typed MIR body.
    Synthesized,
    /// The body failed structural verification; the message is the verifier's.
    Invalid {
        message: String,
    },
    /// A postcard codec failure; the message is postcard's.
    Codec {
        message: String,
    },
    /// `"block {block} statement {index}: {reason}"`.
    AtStatement {
        block: u32,
        index: u32,
        reason: Box<EncodeDecline>,
    },
    /// `"block {block} terminator: {reason}"`.
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
    /// `"no layout for `{ty}` in a constant"` (both the element and the value site).
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
    /// New in v2 (append-only, so it lands at the end): `-Cllvm-args=js-jir=off`. The producer can
    /// encode the item and was told not to. Not a property of the item, which is why it is not
    /// [`EncodeDecline::Synthesized`]: W2's writer records `Synthesized` for this today, and W3's
    /// driver must switch to `Disabled` when it reads the flag, so that a coverage report can tell
    /// "the lane is off" from "this item has no typed MIR body".
    Disabled,
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
        }
    }

    pub fn catalog() -> Vec<EncodeDecline> {
        let ty = || "T".to_owned();
        vec![
            EncodeDecline::Synthesized,
            EncodeDecline::Invalid {
                message: "late-IR function has no entry block".to_owned(),
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
        }
    }
}

/// Merging duplicate definitions across objects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MergeDecline {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// `late_ir.rs::DUPLICATE_CONFLICT`. v1 declined availability on any body difference; v2
    /// records [`MergeDecline::BodyChoice`] instead and keeps one body.
    Conflict,
    /// `item.rs` `late-ir-analysis` miss.
    SummaryConflict,
    /// `item.rs` `baseline-shadow` miss.
    ShadowConflict,
    /// `baseline_shadow.rs::DUPLICATE_CONFLICT`.
    BaselineShadowConflict,
    /// New in v2: two verified bodies for one semantic key differed after normalization; the
    /// linker deterministically kept `chosen` (the smaller normalized hash) and dropped
    /// `rejected`. Never causes unavailability.
    BodyChoice { chosen: u128, rejected: u128 },
    /// New in v2: two objects in one program disagree on something every object has to agree on
    /// (`Header`: the JIR schema, the producing rustc, the target data layout, the emit-affecting
    /// options). `field` names it, `left`/`right` are the two values. A hard link error, never a
    /// degrade: type ids and definition ids are only comparable within one of each.
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
            Self::SummaryConflict => "merge.summary-conflict",
            Self::ShadowConflict => "merge.shadow-conflict",
            Self::BaselineShadowConflict => "merge.baseline-shadow-conflict",
            Self::BodyChoice { .. } => "merge.body-choice",
            Self::Header { .. } => "merge.header",
        }
    }

    pub fn catalog() -> Vec<MergeDecline> {
        vec![
            MergeDecline::Conflict,
            MergeDecline::SummaryConflict,
            MergeDecline::ShadowConflict,
            MergeDecline::BaselineShadowConflict,
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
            Self::Conflict => f.write_str(
                "identical JavaScript definitions carried conflicting late-IR bodies",
            ),
            Self::SummaryConflict => f.write_str(
                "identical JavaScript definitions carried conflicting late-IR summaries; optimized lowering must use the baseline representation",
            ),
            Self::ShadowConflict => f.write_str(
                "identical JavaScript definitions carried conflicting current-codegen replay checks; late-IR executable lowering remains unavailable",
            ),
            Self::BaselineShadowConflict => f.write_str(
                "duplicate definitions carried conflicting baseline shadow results",
            ),
            Self::BodyChoice { chosen, rejected } => write!(
                f,
                "duplicate late-IR bodies differ; kept {chosen:032x} over {rejected:032x}"
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
    /// Append-only: postcard serializes this enum by index.
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
    Coroutine,
    Region,
    Kernel,
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
        Self::Coroutine,
        Self::Region,
        Self::Kernel,
        Self::Other,
    ];

    /// The stable code of a `LowerDecline::Other` about this class: `lower.jir-<class>`, so a
    /// lane histogram groups declines by what stopped the lowering (`lower.other` for `Other`).
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
            Self::Coroutine => "lower.jir-coroutine",
            Self::Region => "lower.jir-region",
            Self::Kernel => "lower.jir-kernel",
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
            Self::Coroutine => "coroutine",
            Self::Region => "region",
            Self::Kernel => "kernel",
            Self::Other => "other",
        }
    }
}

/// The lowering transaction vocabulary: v1's `late_lower.rs` inline reasons and region wrappers,
/// plus the escape hatch the Phase 2 lowerer reports through.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LowerDecline {
    /// Append-only: postcard serializes this enum by index.
    ///
    /// `"native language kernel: {reason}"`.
    Kernel(KernelOp),
    /// `"native language kernel: {reason}"` for a symbol-hygiene failure (`late_lower.rs`).
    KernelHygiene {
        reason: String,
    },
    /// `"region member {declined} declined: {reason}"`.
    RegionMember {
        declined: String,
        reason: Box<Decline>,
    },
    /// `"native region [{members}] declined at {declined}: {reason}"`.
    RegionAt {
        members: Vec<String>,
        declined: String,
        reason: Box<Decline>,
    },
    /// `"native region [{members}] declined: {reason}"`.
    Region {
        members: Vec<String>,
        reason: Box<Decline>,
    },
    /// `late_lower.rs::UNPREPARED_VTABLE_REFERENCE`.
    UnpreparedVtableReference,
    /// `late_lower.rs::REGION_REPRESENTATION_DISAGREEMENT`.
    RegionRepresentationDisagreement,
    /// `late_lower.rs::DECLINE_BASELINE_IDENTICAL`.
    BaselineIdentical,
    /// `late_lower.rs::DECLINE_OUTSIDE_CHANGING_REGION`.
    OutsideChangingRegion,
    /// `late_lower.rs::DECLINE_UNREACHABLE`.
    Unreachable,
    VecElementDropMultipleTargets,
    VecElementDropNoTarget,
    GeneratedBindingShadowed,
    RangeErrorShadowed,
    ObjectShadowed,
    WriteFmtNoUniqueTarget,
    SpecWriteFmtNoTargets,
    KernelAbiNotPreserved,
    KernelCallerIndirect,
    KernelCallerNoAbi,
    KernelStandardCallNoTarget,
    VecElementDropWrongType,
    VecElementDropNoReplacement,
    StructuralDropNoTarget,
    KernelCallerAbiMismatch,
    SliceViewNoSwapTarget,
    IdentityArity,
    IdentityReturnUnplanned,
    IdentityArgumentUnplanned,
    IdentityComponents,
    IdentityControlFlow,
    IdentityUnsupportedOperation,
    IdentityNotOneMove,
    /// New in v2: the JIR lowerer declined an item; `detail` says what, `op` which class of
    /// operation. The item keeps its baseline text.
    Other {
        op: OpClass,
        detail: String,
    },
}

impl LowerDecline {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Kernel(_) => "lower.kernel",
            Self::KernelHygiene { .. } => "lower.kernel-hygiene",
            Self::RegionMember { .. } => "lower.region-member",
            Self::RegionAt { .. } => "lower.region-at",
            Self::Region { .. } => "lower.region",
            Self::UnpreparedVtableReference => "lower.unprepared-vtable-reference",
            Self::RegionRepresentationDisagreement => "lower.region-representation-disagreement",
            Self::BaselineIdentical => "lower.baseline-identical",
            Self::OutsideChangingRegion => "lower.outside-changing-region",
            Self::Unreachable => "lower.unreachable",
            Self::VecElementDropMultipleTargets => "lower.vec-element-drop-multiple-targets",
            Self::VecElementDropNoTarget => "lower.vec-element-drop-no-target",
            Self::GeneratedBindingShadowed => "lower.generated-binding-shadowed",
            Self::RangeErrorShadowed => "lower.range-error-shadowed",
            Self::ObjectShadowed => "lower.object-shadowed",
            Self::WriteFmtNoUniqueTarget => "lower.write-fmt-no-unique-target",
            Self::SpecWriteFmtNoTargets => "lower.spec-write-fmt-no-targets",
            Self::KernelAbiNotPreserved => "lower.kernel-abi-not-preserved",
            Self::KernelCallerIndirect => "lower.kernel-caller-indirect",
            Self::KernelCallerNoAbi => "lower.kernel-caller-no-abi",
            Self::KernelStandardCallNoTarget => "lower.kernel-standard-call-no-target",
            Self::VecElementDropWrongType => "lower.vec-element-drop-wrong-type",
            Self::VecElementDropNoReplacement => "lower.vec-element-drop-no-replacement",
            Self::StructuralDropNoTarget => "lower.structural-drop-no-target",
            Self::KernelCallerAbiMismatch => "lower.kernel-caller-abi-mismatch",
            Self::SliceViewNoSwapTarget => "lower.slice-view-no-swap-target",
            Self::IdentityArity => "lower.identity-arity",
            Self::IdentityReturnUnplanned => "lower.identity-return-unplanned",
            Self::IdentityArgumentUnplanned => "lower.identity-argument-unplanned",
            Self::IdentityComponents => "lower.identity-components",
            Self::IdentityControlFlow => "lower.identity-control-flow",
            Self::IdentityUnsupportedOperation => "lower.identity-unsupported-operation",
            Self::IdentityNotOneMove => "lower.identity-not-one-move",
            Self::Other { op, .. } => op.lower_code(),
        }
    }

    pub fn catalog() -> Vec<LowerDecline> {
        let mut all = vec![
            LowerDecline::Kernel(KernelOp::FunctionsOnly),
            LowerDecline::KernelHygiene {
                reason: "native kernel binding shadows a linked item".to_owned(),
            },
            LowerDecline::RegionMember {
                declined: "item".to_owned(),
                reason: Box::new(Decline::Body(BodyDecline::Unavailable)),
            },
            LowerDecline::RegionAt {
                members: vec!["a".to_owned(), "b".to_owned()],
                declined: "a".to_owned(),
                reason: Box::new(Decline::Body(BodyDecline::Unavailable)),
            },
            LowerDecline::Region {
                members: vec!["a".to_owned()],
                reason: Box::new(Decline::Body(BodyDecline::Unavailable)),
            },
            LowerDecline::UnpreparedVtableReference,
            LowerDecline::RegionRepresentationDisagreement,
            LowerDecline::BaselineIdentical,
            LowerDecline::OutsideChangingRegion,
            LowerDecline::Unreachable,
            LowerDecline::VecElementDropMultipleTargets,
            LowerDecline::VecElementDropNoTarget,
            LowerDecline::GeneratedBindingShadowed,
            LowerDecline::RangeErrorShadowed,
            LowerDecline::ObjectShadowed,
            LowerDecline::WriteFmtNoUniqueTarget,
            LowerDecline::SpecWriteFmtNoTargets,
            LowerDecline::KernelAbiNotPreserved,
            LowerDecline::KernelCallerIndirect,
            LowerDecline::KernelCallerNoAbi,
            LowerDecline::KernelStandardCallNoTarget,
            LowerDecline::VecElementDropWrongType,
            LowerDecline::VecElementDropNoReplacement,
            LowerDecline::StructuralDropNoTarget,
            LowerDecline::KernelCallerAbiMismatch,
            LowerDecline::SliceViewNoSwapTarget,
            LowerDecline::IdentityArity,
            LowerDecline::IdentityReturnUnplanned,
            LowerDecline::IdentityArgumentUnplanned,
            LowerDecline::IdentityComponents,
            LowerDecline::IdentityControlFlow,
            LowerDecline::IdentityUnsupportedOperation,
            LowerDecline::IdentityNotOneMove,
            LowerDecline::Other {
                op: OpClass::Call,
                detail: "unsupported callee".to_owned(),
            },
        ];
        // One `Other` per class beyond `Call`, so the per-class codes are covered.
        all.extend(
            OpClass::ALL
                .iter()
                .copied()
                .filter(|op| *op != OpClass::Call)
                .map(|op| LowerDecline::Other {
                    op,
                    detail: "unsupported".to_owned(),
                }),
        );
        all
    }
}

impl fmt::Display for LowerDecline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(reason) => write!(f, "native language kernel: {reason}"),
            Self::KernelHygiene { reason } => write!(f, "native language kernel: {reason}"),
            Self::RegionMember { declined, reason } => {
                write!(f, "region member {declined} declined: {reason}")
            }
            Self::RegionAt {
                members,
                declined,
                reason,
            } => write!(
                f,
                "native region [{}] declined at {declined}: {reason}",
                members.join(", ")
            ),
            Self::Region { members, reason } => {
                write!(
                    f,
                    "native region [{}] declined: {reason}",
                    members.join(", ")
                )
            }
            Self::UnpreparedVtableReference => f.write_str(
                "vtable has a reachable linker reference without a prepared native replacement",
            ),
            Self::RegionRepresentationDisagreement => f.write_str(
                "a native callee slot and a region caller operand disagree on representation",
            ),
            Self::BaselineIdentical => {
                f.write_str("planned variant keeps its printed baseline body")
            }
            Self::OutsideChangingRegion => {
                f.write_str("planned variant did not join a representation-changing native region")
            }
            Self::Unreachable => {
                f.write_str("committed replacement was removed by final reachability")
            }
            Self::VecElementDropMultipleTargets => {
                f.write_str("native Vec element drop glue has multiple reachable semantic targets")
            }
            Self::VecElementDropNoTarget => {
                f.write_str("native Vec element drop glue has no reachable semantic target")
            }
            Self::GeneratedBindingShadowed => {
                f.write_str("native generated binding shadows a linked item")
            }
            Self::RangeErrorShadowed => {
                f.write_str("native RangeError global is shadowed by a linked item")
            }
            Self::ObjectShadowed => {
                f.write_str("native Object global is shadowed by a linked item")
            }
            Self::WriteFmtNoUniqueTarget => f.write_str(
                "transparent String write_fmt has no unique exact specialization target",
            ),
            Self::SpecWriteFmtNoTargets => f.write_str(
                "transparent String spec_write_fmt has no exact method and vtable targets",
            ),
            Self::KernelAbiNotPreserved => f.write_str(
                "unplanned native kernel does not preserve its fixed public function ABI",
            ),
            Self::KernelCallerIndirect => {
                f.write_str("native kernel caller has an indirect or foreign call")
            }
            Self::KernelCallerNoAbi => {
                f.write_str("native kernel caller has no available direct-callee ABI")
            }
            Self::KernelStandardCallNoTarget => {
                f.write_str("native kernel standard call has no exact staged target")
            }
            Self::VecElementDropWrongType => {
                f.write_str("native Vec element drop target has the wrong semantic type")
            }
            Self::VecElementDropNoReplacement => {
                f.write_str("native Vec element drop glue has no compatible staged replacement")
            }
            Self::StructuralDropNoTarget => {
                f.write_str("native structural drop has no compatible staged target")
            }
            Self::KernelCallerAbiMismatch => {
                f.write_str("native kernel caller does not match the direct callee JavaScript ABI")
            }
            Self::SliceViewNoSwapTarget => {
                f.write_str("native slice view requires a staged native SliceSwap target")
            }
            Self::IdentityArity => {
                f.write_str("native lowering currently accepts only one-argument move identities")
            }
            Self::IdentityReturnUnplanned => {
                f.write_str("the return place has no selected representation")
            }
            Self::IdentityArgumentUnplanned => {
                f.write_str("the argument has no selected representation")
            }
            Self::IdentityComponents => {
                f.write_str("argument and return must share one native representation component")
            }
            Self::IdentityControlFlow => {
                f.write_str("the native round trip has observable control flow")
            }
            Self::IdentityUnsupportedOperation => {
                f.write_str("the native round trip contains an unsupported operation")
            }
            Self::IdentityNotOneMove => {
                f.write_str("the native round trip must contain one move into the return place")
            }
            Self::Other { op, detail } => write!(f, "{}: {detail}", op.as_str()),
        }
    }
}

reasons! {
    /// `async_analysis.rs::Failure`, in its declaration (priority) order.
    pub enum AsyncDecline {
        OwnerAware => ("async.owner-aware", "owner-aware Topcoat scheduling is a pinned async boundary"),
        NotConcreteCoroutine => ("async.not-concrete-coroutine", "terminal future is not one concrete coroutine"),
        FutureLangItemUnavailable => ("async.future-lang-item-unavailable", "the Rust Future::poll semantic identity is unavailable"),
        MirUnavailable => ("async.mir-unavailable", "a concrete coroutine has no post-StateTransform MIR"),
        LayoutUnavailable => ("async.layout-unavailable", "a concrete coroutine has no saved-state layout"),
        RecursiveGraph => ("async.recursive-graph", "the concrete coroutine graph is recursive"),
        DynamicPoll => ("async.dynamic-poll", "a future poll target or receiver is not statically known"),
        CustomPoll => ("async.custom-poll", "an awaited child uses a custom Future::poll implementation"),
        NonCanonicalAwait => ("async.non-canonical-await", "a Future::poll call is not in a canonical post-StateTransform await loop"),
        SuspensionMismatch => ("async.suspension-mismatch", "coroutine suspension points do not match canonical await loops"),
        ManualTaskState => ("async.manual-task-state", "the coroutine observes Context or Waker outside canonical Future::poll"),
        PinnedState => ("async.pinned-state", "pinned state remains live across a suspension point"),
        BorrowedState => ("async.borrowed-state", "a Rust reference remains live across a suspension point"),
        PointerState => ("async.pointer-state", "a raw pointer remains live across a suspension point"),
        DropState => ("async.drop-state", "observable drop state remains live across a suspension point"),
        UnsupportedState => ("async.unsupported-state", "unsupported state remains live across a suspension point"),
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
        assert!(catalog.len() > 300, "catalog has {} entries", catalog.len());
    }

    #[test]
    fn display_matches_v1_text() {
        assert_eq!(
            Decline::Body(BodyDecline::Unavailable).to_string(),
            "the complete typed body is unavailable"
        );
        assert_eq!(
            Decline::Merge(MergeDecline::Conflict).to_string(),
            "identical JavaScript definitions carried conflicting late-IR bodies"
        );
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
        assert_eq!(
            Decline::Lower(LowerDecline::Kernel(KernelOp::CleanupBlocks)).to_string(),
            "native language kernel: native kernel does not support cleanup blocks"
        );
        assert_eq!(
            Decline::Op(OpDecline::Std(StdOp::ParameterCount {
                expected: 2,
                actual: 3
            }))
            .to_string(),
            "JavaScript ABI parameter count does not match the canonical operation"
        );
        assert_eq!(
            Decline::Version(VersionDecline::Schema {
                found: 10,
                expected: 11
            })
            .to_string(),
            "late-IR analysis summary has format 10, but this backend reads format 11"
        );
        assert_eq!(
            Decline::Async(AsyncDecline::BorrowedState).to_string(),
            "a Rust reference remains live across a suspension point"
        );
        assert_eq!(
            Decline::Op(OpDecline::StructuralDrop(StructuralDropOp::Cycle)).to_string(),
            "structural drop normal CFG contains a cycle"
        );
        assert_eq!(
            Decline::Op(OpDecline::FormatTemplate(FormatTemplateOp::UsesFlags)).to_string(),
            "format template uses flags, width, precision, or explicit argument selection"
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

    #[test]
    fn kernel_vocabulary_is_complete() {
        // 171 distinct strings across 191 construction sites in v1's native_kernel.rs.
        assert_eq!(KernelOp::ALL.len(), 171, "kernel op count");
        assert_eq!(StructuralDropOp::ALL.len(), 38);
        assert_eq!(AsyncDecline::ALL.len(), 16);
        assert_eq!(StdOp::catalog().len(), 19);
    }
}
