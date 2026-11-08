//! Interned identifiers and the crate-scope tables they index.
//!
//! Every string, byte blob, function signature, source location, and type a body mentions lives in
//! one of these tables once, and the body carries a small integer id. That is what keeps a body's
//! semantic content free of paths and text (two crates describing the same monomorphic item through
//! different paths still produce equal bodies), and what lets the object format store the tables in
//! one section (S1) and decode bodies lazily.
//!
//! Ids are stable within one [`Interns`]: interning the same value twice returns the same id, and
//! ids are dense in insertion order. Ids from two different [`Interns`] are unrelated; a consumer
//! merging objects re-interns.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::origin::SourceFact;
use super::types::{SigDescriptor, SigTable, TypeDescriptor, TypeId, TypeTable};

/// An interned string: an index into [`Interns::strings`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct StrId(pub u32);

/// An interned byte blob: an index into [`Interns::bytes`].
///
/// Host payloads (`js!{}` block sections, `#[js_extern]` descriptors) are bytes rather than
/// strings so a malformed section is a decline at lowering time, never a decode failure.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BytesId(pub u32);

/// An interned function signature: an index into [`Interns::sigs`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SigId(pub u32);

/// The crate-scope tables every body of one object refers into.
///
/// Built once per codegen unit by the encoder and written as object section S1. Read accessors
/// return `Option` so a stale or foreign id is an error the caller reports rather than a panic.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Interns {
    /// Indexed by [`StrId`].
    pub strings: Vec<String>,
    /// Indexed by [`BytesId`].
    pub bytes: Vec<Vec<u8>>,
    /// Indexed by [`SigId`].
    pub sigs: SigTable,
    /// Indexed by [`super::origin::SourceFactId`].
    pub source_facts: Vec<SourceFact>,
    /// Keyed by [`TypeId`], sorted.
    pub types: TypeTable,
    /// Generic source suspension facts, keyed by stable coroutine definition identity.
    pub coroutine_sources:
        BTreeMap<super::semantic::DefinitionId, super::coroutine::CoroutineSource>,
    #[serde(skip)]
    string_index: BTreeMap<String, StrId>,
    #[serde(skip)]
    bytes_index: BTreeMap<Vec<u8>, BytesId>,
    #[serde(skip)]
    sig_index: BTreeMap<SigDescriptor, SigId>,
    #[serde(skip)]
    source_fact_index: BTreeMap<SourceFact, super::origin::SourceFactId>,
}

impl Interns {
    /// Empty tables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the dedup indices after deserialization, so interning can continue on a table read
    /// back from an object.
    ///
    /// The tables themselves are authoritative; the indices are a cache. A table with duplicate
    /// entries (which the writer never produces) keeps the first occurrence as the canonical id.
    pub fn rebuild_indices(&mut self) {
        self.string_index.clear();
        for (index, string) in self.strings.iter().enumerate() {
            self.string_index
                .entry(string.clone())
                .or_insert(StrId(index as u32));
        }
        self.bytes_index.clear();
        for (index, bytes) in self.bytes.iter().enumerate() {
            self.bytes_index
                .entry(bytes.clone())
                .or_insert(BytesId(index as u32));
        }
        self.sig_index.clear();
        for (index, sig) in self.sigs.descriptors.iter().enumerate() {
            self.sig_index
                .entry(sig.clone())
                .or_insert(SigId(index as u32));
        }
        self.source_fact_index.clear();
        for (index, fact) in self.source_facts.iter().enumerate() {
            self.source_fact_index
                .entry(fact.clone())
                .or_insert(super::origin::SourceFactId(index as u32));
        }
    }

    /// The id of `value`, interning it on first use.
    pub fn intern_str(&mut self, value: &str) -> StrId {
        if let Some(id) = self.string_index.get(value) {
            return *id;
        }
        let id = StrId(self.strings.len() as u32);
        self.strings.push(value.to_owned());
        self.string_index.insert(value.to_owned(), id);
        id
    }

    /// The id of `value`, interning it on first use.
    pub fn intern_bytes(&mut self, value: &[u8]) -> BytesId {
        if let Some(id) = self.bytes_index.get(value) {
            return *id;
        }
        let id = BytesId(self.bytes.len() as u32);
        self.bytes.push(value.to_vec());
        self.bytes_index.insert(value.to_vec(), id);
        id
    }

    /// The id of `sig`, interning it on first use.
    pub fn intern_sig(&mut self, sig: SigDescriptor) -> SigId {
        if let Some(id) = self.sig_index.get(&sig) {
            return *id;
        }
        let id = SigId(self.sigs.descriptors.len() as u32);
        self.sigs.descriptors.push(sig.clone());
        self.sig_index.insert(sig, id);
        id
    }

    /// The id of `fact`, interning it on first use.
    pub fn intern_source_fact(&mut self, fact: SourceFact) -> super::origin::SourceFactId {
        if let Some(id) = self.source_fact_index.get(&fact) {
            return *id;
        }
        let id = super::origin::SourceFactId(self.source_facts.len() as u32);
        self.source_facts.push(fact.clone());
        self.source_fact_index.insert(fact, id);
        id
    }

    /// Records a type descriptor under its own id.
    ///
    /// A descriptor already present is left alone: two encodings of one type are equal by
    /// construction, and the first one wins. Returns whether the descriptor was new.
    pub fn intern_type(&mut self, descriptor: TypeDescriptor) -> bool {
        self.types.insert(descriptor)
    }

    /// Whether the type table already describes `id`.
    ///
    /// The encoder inserts a placeholder before walking a type's components so a recursive
    /// nominal type terminates; this is the check it makes first.
    pub fn has_type(&self, id: TypeId) -> bool {
        self.types.get(id).is_some()
    }

    pub fn str(&self, id: StrId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(String::as_str)
    }

    pub fn bytes(&self, id: BytesId) -> Option<&[u8]> {
        self.bytes.get(id.0 as usize).map(Vec::as_slice)
    }

    pub fn sig(&self, id: SigId) -> Option<&SigDescriptor> {
        self.sigs.get(id)
    }

    pub fn source_fact(&self, id: super::origin::SourceFactId) -> Option<&SourceFact> {
        self.source_facts.get(id.0 as usize)
    }

    pub fn ty(&self, id: TypeId) -> Option<&TypeDescriptor> {
        self.types.get(id)
    }
}
