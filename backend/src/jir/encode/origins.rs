//! Source-origin recording, inline-stack tracking, and debug local-name extraction.

use std::collections::BTreeMap;

use rustc_index::IndexVec;
use rustc_middle::mir;
use rustc_middle::ty::print::with_no_trimmed_paths;
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_span::Span;

use rcgjs_object::jir::interns::StrId;
use rcgjs_object::jir::origin::{
    Origin, OriginId, OriginKind, OriginTable, SourceFact, SourceFactId,
};
use rcgjs_object::jsast::Loc;

use crate::jir::types::TypeCx;

/// Collects deduplicated [`Origin`] entries and positional tables for one item.
pub(super) struct OriginRecorder {
    origins: Vec<Origin>,
    origin_index: BTreeMap<Origin, OriginId>,
    statement_origins: Vec<OriginId>,
    terminator_origins: Vec<OriginId>,
    local_origins: Vec<OriginId>,
    scope_stacks: IndexVec<mir::SourceScope, Option<Vec<OriginId>>>,
    item_path: Option<StrId>,
    root: Option<SourceFactId>,
}

impl OriginRecorder {
    pub(super) fn new() -> Self {
        Self {
            origins: Vec::new(),
            origin_index: BTreeMap::new(),
            statement_origins: Vec::new(),
            terminator_origins: Vec::new(),
            local_origins: Vec::new(),
            scope_stacks: IndexVec::new(),
            item_path: None,
            root: None,
        }
    }

    pub(super) fn set_item_path(&mut self, path: StrId) {
        self.item_path = Some(path);
    }

    pub(super) fn set_root(&mut self, root: Option<SourceFactId>) {
        self.root = root;
    }

    pub(super) fn push_local(&mut self, id: OriginId) {
        self.local_origins.push(id);
    }

    pub(super) fn push_statement(&mut self, id: OriginId) {
        self.statement_origins.push(id);
    }

    pub(super) fn push_terminator(&mut self, id: OriginId) {
        self.terminator_origins.push(id);
    }

    pub(super) fn extend_terminators(&mut self, ids: impl IntoIterator<Item = OriginId>) {
        self.terminator_origins.extend(ids);
    }

    /// The origin of one element: its kind, its span's fact, and the inline stack of its scope.
    pub(super) fn record<'tcx>(
        &mut self,
        types: &mut TypeCx<'_, 'tcx>,
        mir: &mir::Body<'tcx>,
        kind: OriginKind,
        info: mir::SourceInfo,
        local_name: Option<StrId>,
    ) -> OriginId {
        let source = source_fact(types, info.span, Some(mir.span));
        let inline_stack = self.scope_stack(types, mir, info.scope);
        self.intern_origin(Origin {
            kind,
            source,
            local_name,
            inline_stack,
        })
    }

    fn intern_origin(&mut self, origin: Origin) -> OriginId {
        if let Some(&id) = self.origin_index.get(&origin) {
            return id;
        }
        let id = OriginId(self.origins.len() as u32);
        self.origins.push(origin.clone());
        self.origin_index.insert(origin, id);
        id
    }

    /// The chain of inlined call sites that placed `scope`'s elements here, innermost first.
    fn scope_stack<'tcx>(
        &mut self,
        types: &mut TypeCx<'_, 'tcx>,
        mir: &mir::Body<'tcx>,
        scope: mir::SourceScope,
    ) -> Vec<OriginId> {
        if let Some(Some(stack)) = self.scope_stacks.get(scope) {
            return stack.clone();
        }
        if self.scope_stacks.len() < mir.source_scopes.len() {
            self.scope_stacks.resize(mir.source_scopes.len(), None);
        }
        let tcx = types.tcx;
        let mut stack = Vec::new();
        let mut current = scope;
        for _ in 0..mir.source_scopes.len() {
            let data = &mir.source_scopes[current];
            let next = match data.inlined {
                Some((instance, span)) => {
                    let what = types.str(&with_no_trimmed_paths!(format!(
                        "inlined {}",
                        tcx.def_path_str(instance.def_id())
                    )));
                    let source = source_fact(types, span, Some(mir.span));
                    let origin = self.intern_origin(Origin {
                        kind: OriginKind::Generated(what),
                        source,
                        local_name: None,
                        inline_stack: Vec::new(),
                    });
                    stack.push(origin);
                    data.inlined_parent_scope
                }
                None => data.parent_scope,
            };
            match next {
                Some(next) => current = next,
                None => break,
            }
        }
        self.scope_stacks[scope] = Some(stack.clone());
        stack
    }

    pub(super) fn finish(self, types: &mut TypeCx<'_, '_>) -> OriginTable {
        OriginTable {
            origins: self.origins,
            statements: self.statement_origins,
            terminators: self.terminator_origins,
            locals: self.local_origins,
            item_path: self.item_path.unwrap_or_else(|| types.str("")),
            root: self.root,
        }
    }
}

pub(super) fn source_fact<'tcx>(
    types: &mut TypeCx<'_, 'tcx>,
    span: Span,
    mir_span: Option<Span>,
) -> Option<SourceFactId> {
    // Match rustc's debug locations: collapse only opted-in macro expansions,
    // and stop at the containing function rather than its generating macro.
    let span = mir_span.map_or(span, |mir_span| {
        rustc_span::hygiene::walk_chain_collapsed(span, mir_span)
    });
    let location = location_of(types.tcx, span)?;
    let file = types.str(&location.file);
    Some(types.interns.intern_source_fact(SourceFact {
        file,
        line: location.line,
        col: location.col,
    }))
}

/// The user's name for each local (`names.rs`'s rule: the first `var_debug_info` entry naming the
/// whole local, never the return place, never `_`), or `None` for a compiler temporary.
pub(super) fn local_names(mir: &mir::Body<'_>) -> Vec<Option<String>> {
    let mut names: Vec<Option<String>> = vec![None; mir.local_decls.len()];
    for info in &mir.var_debug_info {
        let mir::VarDebugInfoContents::Place(place) = info.value else {
            continue;
        };
        if !place.projection.is_empty() {
            continue;
        }
        let index = place.local.as_usize();
        if place.local == mir::RETURN_PLACE || index >= names.len() {
            continue;
        }
        if names[index].is_none() {
            let raw = info.name.as_str();
            if !raw.is_empty() && raw != "_" {
                names[index] = Some(raw.to_owned());
            }
        }
    }
    names
}

/// Resolves the `#[track_caller]` location for `source_info` inside `mir`.
pub(super) fn resolve_caller_location_span<'tcx>(
    tcx: TyCtxt<'tcx>,
    mir: &mir::Body<'tcx>,
    outer_instance: Instance<'tcx>,
    source_info: mir::SourceInfo,
) -> Option<Span> {
    let mut span = source_info.span;
    let mut scope = source_info.scope;
    for _ in 0..mir.source_scopes.len() {
        let data = &mir.source_scopes[scope];
        if let Some((inlined_instance, call_span)) = data.inlined {
            if !inlined_instance.def.requires_caller_location(tcx) {
                return Some(span);
            }
            span = call_span;
            match data.inlined_parent_scope {
                Some(parent) => scope = parent,
                None => break,
            }
        } else {
            match data.parent_scope {
                Some(parent) => scope = parent,
                None => break,
            }
        }
    }
    if outer_instance.def.requires_caller_location(tcx) {
        None
    } else {
        Some(span)
    }
}

/// Where a span starts, spelled as rustc would spell it in a diagnostic.
pub(crate) fn location_of(tcx: TyCtxt<'_>, span: Span) -> Option<Loc> {
    if span.is_dummy() {
        return None;
    }
    let position = tcx.sess.source_map().lookup_char_pos(span.lo());
    let file = position
        .file
        .name
        .prefer_remapped_unconditionally()
        .to_string();
    if file.is_empty() {
        return None;
    }
    Some(Loc {
        file,
        line: position.line as u32,
        col: position.col.0 as u32,
    })
}
