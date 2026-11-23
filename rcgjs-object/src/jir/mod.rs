//! The JIR-S schema: semantic bodies, type descriptors, origins, interns, declines, verification.

pub mod codec;
pub mod conflict;
pub mod coroutine;
pub mod decline;
mod dependencies;
pub mod hash;
pub mod interns;
pub mod interp;
pub mod origin;
pub mod repr;
pub mod semantic;
pub mod summary;
pub mod types;
pub mod verify;

#[cfg(test)]
mod tests;

/// The independent schema of every type in this module.
///
/// The containing object format is versioned too. A clean rebuild is required when this changes;
/// see the crate documentation for the append-only rule that keeps it stable.
///
/// | schema | change |
/// | --- | --- |
/// | 1 | W1/W2: the initial JIR-S schema and the format 20 summary index. |
/// | 2 | W4: `ItemSummary::semantic_hash`, the normalized body hash the dedup rule compares. A struct field, which postcard reads positionally, so an object written under schema 1 is rejected rather than misread. |
/// | 6 | Phase 4 W4d: the `#[js_extern]` descriptor's version 3, whose per-slot host contract an object's `HostPayload::Extern` bytes now carry, and the appended `VersionDecline::CarrierCommitRollback`. Both are read by the producer this schema keys, so an object written by an older one is rejected rather than misread. |
/// | 7 | Phase 6A: versioned physical-call facts in the eager item summary. |
/// | 8 | Phase 12 K1: the append-only `HostPayload::KeyedList` operation with typed callable, cleanup, and source facts. |
/// | 9 | Phase 12 iterator-state lowering: exact standard iterator identity in `TypeMeta`. |
/// | 10 | Source coroutine suspension facts in the object intern table. |
/// | 16 | `AbiSignature::inline_never` preserves `#[inline(never)]` across JIR. |
/// | 17 | `Callee::Runtime { member, sysroot }` records rustc sysroot provenance for foreign runtime shims. |
/// | 18 | `AbiSignature::{inline_hint, inline_always}` preserves `#[inline]` hints and `Terminator::Assert::track_caller` records resolved MIR-inlined `#[track_caller]` locations. |
pub const JIR_SCHEMA: u16 = 19;

use serde::{Deserialize, Serialize};

/// One item's JIR-S: the semantic body and, beside it, its diagnostic origins.
///
/// What the encoder produces and what an object stores. The two travel together everywhere except
/// in the object itself, where the body is content addressed in S3 and the origins live in S5.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JirItem {
    pub body: Body,
    pub origins: OriginTable,
}

pub use decline::Decline;
pub use hash::Hash128;
pub use interns::{BytesId, Interns, SigId, StrId};
pub use origin::{Origin, OriginId, OriginKind, OriginTable, SourceFact, SourceFactId};
pub use semantic::{Body, BodyKind, Call, Callee, DefinitionId, DirectItemRef, ItemId, TypeId};
pub use summary::{
    Availability, ItemClass, ItemSummary, PhysicalSummary, PhysicalSummaryValidation, SemanticKey,
};
pub use types::{SigDescriptor, SigTable, TypeDescriptor, TypeTable};
pub use verify::VerifyError;
