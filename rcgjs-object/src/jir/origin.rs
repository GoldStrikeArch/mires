//! Origins: where each part of a body came from.
//!
//! Origins are **diagnostic**: they feed source maps, warnings, reports, and the debugger; they
//! never influence lowering, and they take **no part in semantic equality or hashing**. Two bodies
//! from two crates that describe one monomorphic item through different paths are equal bodies
//! with two origin tables; the linker merges the tables and keeps one body. That is why the
//! [`OriginTable`] is a separate value beside the [`super::semantic::Body`] (object section S5)
//! and not a field of it.
//!
//! An origin table is positional: `statements[i]` is the origin of the `i`-th statement of the
//! body in block-major order (block 0's statements, then block 1's, ...), `terminators[b]` the
//! origin of block `b`'s terminator, `locals[l]` the origin of local `l`. `Body::verify_origins`
//! checks the lengths.

use serde::{Deserialize, Serialize};

use super::interns::StrId;

/// An index into [`OriginTable::origins`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct OriginId(pub u32);

/// An index into [`super::interns::Interns::source_facts`], or one of the two `Call::track_caller`
/// ABI sentinels ([`SourceFactId::FORWARD`] / [`SourceFactId::NOT_TRACKED`]).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceFactId(pub u32);

impl SourceFactId {
    /// Sentinel on `Call::track_caller` meaning the callee is `#[track_caller]` and the enclosing
    /// `#[track_caller]` body forwards its incoming `$loc` parameter directly.
    pub const FORWARD: Self = Self(u32::MAX);

    /// Sentinel on `Call::track_caller` meaning the encoder proved the callee is NOT
    /// `#[track_caller]`, so even inside a `#[track_caller]` body no trailing `$loc` argument is
    /// passed and link-time lowering never needs to inspect `ItemIndex` for the callee's ABI.
    pub const NOT_TRACKED: Self = Self(u32::MAX - 1);

    /// Whether this id is a `Call::track_caller` ABI sentinel rather than an index into
    /// [`super::interns::Interns::source_facts`].
    pub const fn is_sentinel(self) -> bool {
        self.0 >= Self::NOT_TRACKED.0
    }

    /// Whether this id explicitly forwards the enclosing body's incoming `$loc`.
    pub const fn is_forward(self) -> bool {
        self.0 == Self::FORWARD.0
    }

    /// Whether the encoder proved the callee does not take a caller location.
    pub const fn is_not_tracked(self) -> bool {
        self.0 == Self::NOT_TRACKED.0
    }
}

/// One source position, already rendered: the file as rustc names it, 1-based line, 0-based
/// column (as `SourceMap::lookup_char_pos` reports).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SourceFact {
    pub file: StrId,
    pub line: u32,
    pub col: u32,
}

/// The origin of one statement, terminator, or local.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Origin {
    pub kind: OriginKind,
    /// The span's position, when it has one that maps to a real file.
    pub source: Option<SourceFactId>,
    /// The user's name for a local (`local_decls[l]` debug info), for readable output and the
    /// debugger's variable mapping. `None` for compiler temporaries.
    pub local_name: Option<StrId>,
    /// The chain of inlining that placed this element here, innermost first: the origin of each
    /// call site MIR inlining replaced (`SourceScope::inlined`). Empty when not inlined.
    pub inline_stack: Vec<OriginId>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum OriginKind {
    /// Append-only: postcard serializes this enum by index.
    Statement,
    Terminator,
    Local,
    /// A local that is a function argument.
    Argument,
    /// A promoted or anonymous constant.
    Constant,
    /// Something the compiler produced with no user source (drop glue, shims, adapters); the
    /// string says what.
    Generated(StrId),
}

/// The origins of one body.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OriginTable {
    /// The origin values `statements`/`terminators`/`locals`/`inline_stack` index into.
    pub origins: Vec<Origin>,
    /// One per statement, block-major.
    pub statements: Vec<OriginId>,
    /// One per block.
    pub terminators: Vec<OriginId>,
    /// One per local.
    pub locals: Vec<OriginId>,
    /// The item's path as rustc prints it (`def_path_str` with no trimmed paths).
    pub item_path: StrId,
    /// The item's own definition span.
    pub root: Option<SourceFactId>,
}

impl OriginTable {
    pub fn origin(&self, id: OriginId) -> Option<&Origin> {
        self.origins.get(id.0 as usize)
    }

    /// The origin of the `index`-th statement in block-major order.
    pub fn statement(&self, index: usize) -> Option<&Origin> {
        self.statements.get(index).and_then(|id| self.origin(*id))
    }

    pub fn terminator(&self, block: usize) -> Option<&Origin> {
        self.terminators.get(block).and_then(|id| self.origin(*id))
    }

    pub fn local(&self, local: usize) -> Option<&Origin> {
        self.locals.get(local).and_then(|id| self.origin(*id))
    }
}
