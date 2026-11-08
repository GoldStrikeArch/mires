//! Source suspension facts captured before rustc lowers a coroutine to a state machine.
//!
//! These describe the promoted analysis MIR, not the monomorphic JIR body. Block and local
//! numbers belong only to that source body. They do not establish await relationships, wake
//! behavior or executable initialized cleanup and cannot alone authorize lowering.

use serde::{Deserialize, Serialize};

use super::interns::Interns;
use super::origin::SourceFactId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoroutineSource {
    pub initialization: Option<SourceInitialization>,
    /// Exact saved-field identities carried by preserved rustc debug places, when available.
    pub runtime_frame: Option<RuntimeFrame>,
    pub block_count: u32,
    pub local_count: u32,
    /// Promoted MIR blocks in index order. Imaginary borrow-check edges are not executable.
    pub blocks: Vec<SourceBlock>,
    /// Yield terminators in source block order, including unreachable analysis MIR blocks.
    pub yields: Vec<SourceYield>,
}

impl CoroutineSource {
    /// Checks the source body's own index space and the separate diagnostic source references.
    pub fn valid(&self, interns: &Interns) -> bool {
        self.initialization
            .as_ref()
            .is_none_or(|init| init.valid(self.local_count, self.block_count))
            && self
                .runtime_frame
                .as_ref()
                .is_none_or(|frame| frame.valid(self.local_count))
            && self.block_count > 0
            && self.local_count > 0
            && self.blocks.len() == self.block_count as usize
            && self
                .blocks
                .iter()
                .all(|block| block.valid(self.block_count, self.local_count))
            && self
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, block)| block.kind == SourceTerminator::Yield)
                .map(|(index, _)| index as u32)
                .eq(self.yields.iter().map(|point| point.block))
            && self
                .yields
                .windows(2)
                .all(|pair| pair[0].block < pair[1].block)
            && self.yields.iter().all(|point| {
                point.block < self.block_count
                    && self.blocks[point.block as usize]
                        .successors
                        .iter()
                        .copied()
                        .eq(std::iter::once(SourceEdge {
                            kind: SourceEdgeKind::Resume,
                            target: point.resume,
                        })
                        .chain(point.drop.map(|target| SourceEdge {
                            kind: SourceEdgeKind::Cancel,
                            target,
                        })))
                    && point.resume < self.block_count
                    && point.drop.is_none_or(|block| block < self.block_count)
                    && point
                        .resume_local
                        .is_none_or(|local| local < self.local_count)
                    && point
                        .liveness
                        .as_ref()
                        .is_none_or(|live| live.valid(self.local_count))
                    && point.source.is_none_or(|source| {
                        interns
                            .source_facts
                            .get(source.0 as usize)
                            .is_some_and(|source| {
                                source.line > 0 && interns.str(source.file).is_some()
                            })
                    })
            })
    }
}

/// The control-flow operation only. Executable operands and a runtime drop plan are not encoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceTerminator {
    Goto,
    Switch,
    Call,
    TailCall,
    Drop,
    Assert,
    Yield,
    Return,
    Unreachable,
    CoroutineDrop,
    UnwindResume,
    UnwindTerminate,
    FalseEdge,
    FalseUnwind,
    InlineAsm,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceBlock {
    pub kind: SourceTerminator,
    pub successors: Vec<SourceEdge>,
    /// A bare local dropped by this terminator. Projected drop places remain unavailable.
    pub drop_local: Option<u32>,
    /// The source function identity of a direct call, before MIR inlining. Dynamic calls are None.
    pub callee: Option<super::semantic::DefinitionId>,
}

impl SourceBlock {
    fn valid(&self, block_count: u32, local_count: u32) -> bool {
        use SourceEdgeKind::{Cancel, Imaginary, Normal, Resume, Unwind};
        use SourceTerminator::*;
        if self.callee.is_some() && !matches!(self.kind, Call | TailCall) {
            return false;
        }
        if self
            .drop_local
            .is_some_and(|local| local >= local_count || self.kind != Drop)
        {
            return false;
        }
        if self
            .successors
            .iter()
            .any(|edge| edge.target >= block_count)
        {
            return false;
        }
        let kinds: Vec<_> = self.successors.iter().map(|edge| edge.kind).collect();
        match self.kind {
            Goto => kinds == [Normal],
            Switch => !kinds.is_empty() && kinds.iter().all(|kind| *kind == Normal),
            Call => matches!(
                kinds.as_slice(),
                [] | [Normal] | [Unwind] | [Normal, Unwind]
            ),
            Drop => matches!(
                kinds.as_slice(),
                [Normal] | [Normal, Unwind] | [Normal, Cancel] | [Normal, Unwind, Cancel]
            ),
            Assert => matches!(kinds.as_slice(), [Normal] | [Normal, Unwind]),
            Yield => matches!(kinds.as_slice(), [Resume] | [Resume, Cancel]),
            FalseEdge => kinds == [Normal, Imaginary],
            FalseUnwind => matches!(kinds.as_slice(), [Normal] | [Normal, Imaginary]),
            InlineAsm => {
                let normal = kinds.strip_suffix(&[Unwind]).unwrap_or(&kinds);
                normal.iter().all(|kind| *kind == Normal)
            }
            TailCall | Return | Unreachable | CoroutineDrop | UnwindResume | UnwindTerminate => {
                kinds.is_empty()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceEdge {
    pub kind: SourceEdgeKind,
    pub target: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceEdgeKind {
    Normal,
    Unwind,
    Resume,
    Cancel,
    Imaginary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceYield {
    pub block: u32,
    pub resume: u32,
    pub drop: Option<u32>,
    /// The resume destination when it is a bare local. A projected place is unavailable.
    pub resume_local: Option<u32>,
    pub source: Option<SourceFactId>,
    /// Source-local liveness, independently checked against rustc's pre-cleanup witness layout.
    pub liveness: Option<SourceLiveness>,
}

/// Promoted MIR locals needed at one yield. These are not post-transform frame field indices.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceLiveness {
    /// Locals that rustc's coroutine analysis must retain, in ascending source-local order.
    pub live: Vec<u32>,
    /// Retained locals previously borrowed. This does not prove pinning or alias legality.
    pub borrowed: Vec<u32>,
    /// Locals whose storage is live immediately before the yield.
    pub storage: Vec<u32>,
}

impl SourceLiveness {
    fn valid(&self, local_count: u32) -> bool {
        [&self.live, &self.borrowed, &self.storage]
            .into_iter()
            .all(|locals| {
                locals.iter().all(|local| *local < local_count)
                    && locals.windows(2).all(|pair| pair[0] < pair[1])
            })
            && !self.live.contains(&1)
            && self
                .borrowed
                .iter()
                .all(|local| self.live.binary_search(local).is_ok())
    }
}

/// Runtime field indices are separate from promoted source locals. An empty source list means
/// that no preserved bare-local debug record proves the field's source identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeFrame {
    pub saved_local_sources: Vec<Vec<u32>>,
    pub states: Vec<Vec<u32>>,
}

impl RuntimeFrame {
    fn valid(&self, local_count: u32) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        self.states.len() >= 3
            && self.states[..3].iter().all(Vec::is_empty)
            && self.saved_local_sources.iter().all(|sources| {
                sources.windows(2).all(|pair| pair[0] < pair[1])
                    && sources
                        .iter()
                        .all(|local| *local < local_count && *local != 1 && seen.insert(*local))
            })
            && self.states.iter().all(|fields| {
                fields.windows(2).all(|pair| pair[0] < pair[1])
                    && fields
                        .iter()
                        .all(|field| (*field as usize) < self.saved_local_sources.len())
            })
            && (0..self.saved_local_sources.len()).all(|field| {
                self.states
                    .iter()
                    .any(|state| state.contains(&(field as u32)))
            })
    }
}

/// Conservative move-path facts in the promoted source body. Each point describes a block's
/// terminator before and after its primary effect, excluding normal-return edge effects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceInitialization {
    pub paths: Vec<SourceMovePath>,
    pub points: Vec<SourceInitPoint>,
}

impl SourceInitialization {
    fn valid(&self, local_count: u32, block_count: u32) -> bool {
        self.points.len() == block_count as usize
            && self.paths.iter().enumerate().all(|(index, path)| {
                path.local < local_count
                    && path.projection.as_ref().is_none_or(|projection| {
                        projection.iter().all(|part| match part {
                            SourceProjection::Index(local) => *local < local_count,
                            _ => true,
                        })
                    })
                    && match path.parent {
                        None => path.projection.as_ref().is_some_and(Vec::is_empty),
                        Some(parent) => {
                            (parent as usize) < index && {
                                let parent = &self.paths[parent as usize];
                                path.local == parent.local
                                    && match (&path.projection, &parent.projection) {
                                        (Some(child), Some(parent)) => {
                                            child.len() == parent.len() + 1
                                                && child.starts_with(parent)
                                        }
                                        _ => true,
                                    }
                            }
                        }
                    }
            })
            && self.points.iter().all(|point| {
                point.before.is_some() == point.after.is_some()
                    && [&point.before, &point.after]
                        .into_iter()
                        .flatten()
                        .all(|state| {
                            [&state.maybe_initialized, &state.maybe_uninitialized]
                                .into_iter()
                                .all(|paths| {
                                    paths.iter().all(|path| (*path as usize) < self.paths.len())
                                        && paths.windows(2).all(|pair| pair[0] < pair[1])
                                })
                        })
            })
    }
}

/// One node of rustc's move-path tree. Projection indices belong to the source body, not a
/// runtime layout. A type-changing projection is unavailable until its types can be preserved.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMovePath {
    pub parent: Option<u32>,
    pub local: u32,
    pub projection: Option<Vec<SourceProjection>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceProjection {
    Deref,
    Field(u32),
    Downcast(u32),
    Index(u32),
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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceInitPoint {
    /// None denotes an unreachable analysis point, not an empty initialized set.
    pub before: Option<SourceInitState>,
    pub after: Option<SourceInitState>,
}

/// A path in both sets is conditional. Aggregate descendants must be considered individually;
/// these sets do not prove a runtime drop flag or an executable cleanup plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceInitState {
    pub maybe_initialized: Vec<u32>,
    pub maybe_uninitialized: Vec<u32>,
}
