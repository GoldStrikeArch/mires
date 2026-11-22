//! `view!` templates and the rest of the `view-abi`: the payload the encoder handed over, and the
//! dom-expressions calls it becomes (v1 `backend/src/template.rs` and the marker half of
//! `backend/src/abi.rs`).
//!
//! Three steps turn one `rcgjs.tc.template` call into JavaScript:
//!
//! 1. [`Template::read`] decodes the `HostPayload::Marker::template` [`ConstTree`] the encoder
//!    pre-read out of the `static view_abi::TemplateData`, checking [`ABI_VERSION`];
//! 2. [`Template::tree`] rebuilds the node tree the template's HTML describes, which is what says
//!    whether a hole's walk lands on an anchor comment or on the element itself;
//! 3. [`lower_template`] emits the cloner, the walk, and the record the holes are filled through.
//!
//! # Statements as expressions
//!
//! v1's marker lowerings produced statement lists. The seam this module sits behind produces one
//! expression per call (`host.rs`), so a lowering that has several things to do spells them as a
//! comma sequence over hoisted `$tN` temporaries: `($t0 = tmpl$1(), $t1 = $t0.firstChild, $t0)`.
//! The evaluation order, and so the emitted DOM, is the one v1's statements had.
//!
//! # What is already in the program
//!
//! The cloner (`tmpl$…`) and every dom-expressions import (`_$insert`, `_$effect`, …) are items
//! the **baseline** lowering of this same call already interned into the object. A link-time lane
//! may not add items, and does not need to: it names them exactly as `naming.rs` did, and the
//! driver's `refs` check is what proves the two lanes reach the same set.

use std::collections::BTreeMap;

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::interns::Interns;
use rcgjs_object::jir::repr::{Carrier, Op, OpKind, ValueId};
use rcgjs_object::jir::semantic::{
    ConstTree, DropGlue, HostPayload, KeyedListCallable, KeyedListPayload, TypeId,
    KEYED_LIST_FORMAT_V1,
};
use rcgjs_object::jir::types::{AllocatorIdentity, RepresentationCandidate, TypeKind};
use rcgjs_object::jsast::{self, BinOp, Expr};
use rcgjs_object::ptr;

use super::host::{readable, KeyedRowCapture};
use super::{decline, value, JsCx, Res};

/// The `view_abi::ABI_VERSION` this lowering decodes.
pub const ABI_VERSION: u32 = 8;

/// `view_abi::NO_EFFECT_GROUP`: the group of a hole no shared effect drives.
const NO_EFFECT_GROUP: u32 = u32::MAX;

/// The local a dom-expressions runtime name is imported under (v1 `template.rs::dom_import`).
pub(super) fn dom(name: &str) -> Expr {
    jsast::id(format!("_${name}"))
}

// ---------------------------------------------------------------------------------------------
// The payload
// ---------------------------------------------------------------------------------------------

/// One step of a hole's walk from the template root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    FirstChild,
    NextSibling,
}

impl Step {
    fn property(self) -> &'static str {
        match self {
            Step::FirstChild => "firstChild",
            Step::NextSibling => "nextSibling",
        }
    }
}

/// What a hole holds. Mirrors `view_abi::HoleKind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HoleKind {
    Child,
    Attribute,
    Property,
    DelegatedEvent,
    Event,
    ClassList,
    Style,
    Spread,
    Component,
}

impl HoleKind {
    /// Whether this kind puts a node into the tree, rather than writing something on a node it
    /// reached.
    fn inserts_a_node(self) -> bool {
        matches!(self, HoleKind::Child | HoleKind::Component)
    }

    /// The kind by its `view_abi::HoleKind` variant index (the payload is a fieldless enum, so
    /// the encoder recorded the index rather than the name).
    fn from_variant(index: u32) -> Option<HoleKind> {
        Some(match index {
            0 => HoleKind::Child,
            1 => HoleKind::Attribute,
            2 => HoleKind::Property,
            3 => HoleKind::DelegatedEvent,
            4 => HoleKind::Event,
            5 => HoleKind::ClassList,
            6 => HoleKind::Style,
            7 => HoleKind::Spread,
            8 => HoleKind::Component,
            _ => return None,
        })
    }
}

/// One dynamic position in a template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hole {
    pub path: Vec<Step>,
    pub kind: HoleKind,
    pub name: String,
    pub reactive: bool,
    pub effect_group: Option<u32>,
}

impl Hole {
    /// The walk to the element a [`HoleKind::Child`] hole inserts into, given that its own walk
    /// reaches an anchor.
    fn anchor_parent(&self) -> Option<&[Step]> {
        let last = self
            .path
            .iter()
            .rposition(|step| *step == Step::FirstChild)?;
        Some(&self.path[..last])
    }
}

/// One `view!` template: its HTML skeleton, its holes in source order, and the delegated events
/// it needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Template {
    pub html: String,
    pub hydratable: bool,
    pub is_import_node: bool,
    pub is_svg: bool,
    pub holes: Vec<Hole>,
    pub events: Vec<String>,
}

/// A `ConstTree` field reader that peels the `Reference` wrapper a `&T` field decodes to.
fn peel(tree: &ConstTree) -> &ConstTree {
    match tree {
        ConstTree::Reference(inner) => peel(inner),
        other => other,
    }
}

fn field<'a>(fields: &'a [ConstTree], index: usize, what: &str) -> Result<&'a ConstTree, String> {
    fields
        .get(index)
        .map(peel)
        .ok_or_else(|| format!("`{what}` is not in the payload"))
}

fn as_u32(tree: &ConstTree, what: &str) -> Result<u32, String> {
    match tree {
        ConstTree::Scalar { bits, .. } => Ok(*bits as u32),
        _ => Err(format!("`{what}` is not a number")),
    }
}

fn as_bool(tree: &ConstTree, what: &str) -> Result<bool, String> {
    match tree {
        ConstTree::Scalar { bits, .. } => Ok(*bits != 0),
        _ => Err(format!("`{what}` is not a boolean")),
    }
}

fn as_str<'a>(interns: &'a Interns, tree: &ConstTree, what: &str) -> Result<&'a str, String> {
    match tree {
        ConstTree::Str(id) => interns
            .str(*id)
            .ok_or_else(|| format!("`{what}` names a string that is not interned")),
        _ => Err(format!("`{what}` is not a string")),
    }
}

fn as_seq<'a>(tree: &'a ConstTree, what: &str) -> Result<&'a [ConstTree], String> {
    match tree {
        ConstTree::Slice(elements) | ConstTree::Aggregate(elements) => Ok(elements),
        _ => Err(format!("`{what}` is not a sequence")),
    }
}

impl Template {
    /// Decodes the `static view_abi::TemplateData` the encoder pre-read into a [`ConstTree`].
    ///
    /// The fields are positional, in `TemplateData`'s declaration order: `abi`, `html`,
    /// `hydratable`, `is_import_node`, `is_svg`, `holes`, `events`.
    pub fn read(interns: &Interns, tree: &ConstTree) -> Result<Template, String> {
        let ConstTree::Aggregate(fields) = peel(tree) else {
            return Err("the `view!` payload is not a struct".to_owned());
        };
        let abi = as_u32(field(fields, 0, "abi")?, "abi")?;
        if abi != ABI_VERSION {
            return Err(format!(
                "it announces view-abi version {abi}, and this backend reads version \
                 {ABI_VERSION}: rebuild the crate and the backend against one `view-abi`"
            ));
        }
        let html = as_str(interns, field(fields, 1, "html")?, "html")?.to_owned();
        let hydratable = as_bool(field(fields, 2, "hydratable")?, "hydratable")?;
        let is_import_node = as_bool(field(fields, 3, "is_import_node")?, "is_import_node")?;
        let is_svg = as_bool(field(fields, 4, "is_svg")?, "is_svg")?;

        let mut holes = Vec::new();
        for (index, hole) in as_seq(field(fields, 5, "holes")?, "holes")?
            .iter()
            .enumerate()
        {
            holes.push(
                Hole::read(interns, peel(hole))
                    .map_err(|err| format!("in `holes[{index}]`, {err}"))?,
            );
        }
        let mut events = Vec::new();
        for (index, event) in as_seq(field(fields, 6, "events")?, "events")?
            .iter()
            .enumerate()
        {
            events.push(as_str(interns, peel(event), &format!("events[{index}]"))?.to_owned());
        }
        Ok(Template {
            html,
            hydratable,
            is_import_node,
            is_svg,
            holes,
            events,
        })
    }

    /// The node tree the template's HTML describes, rooted where a hole's walk starts.
    pub fn tree(&self) -> Tree {
        let tree = Tree::parse(&self.html);
        match self.is_svg {
            true => tree.inside_svg_wrapper(),
            false => tree,
        }
    }

    /// The interning key of this template's cloner: the HTML text and the flags it is built with
    /// (v1 `template.rs::cloner_key`).
    fn cloner_key(&self) -> String {
        match (self.is_import_node, self.is_svg) {
            (false, false) => format!("tmpl\u{1}{}", self.html),
            (import, svg) => format!(
                "tmpl\u{1}{}{}\u{1}{}",
                u8::from(import),
                u8::from(svg),
                self.html
            ),
        }
    }

    /// The module-level `const` holding this template's cloner, which the baseline lowering of
    /// this same call already interned into the program.
    fn cloner(&self) -> Expr {
        jsast::id(readable("tmpl", &self.cloner_key()))
    }
}

impl Hole {
    fn read(interns: &Interns, tree: &ConstTree) -> Result<Hole, String> {
        let ConstTree::Aggregate(fields) = tree else {
            return Err("a hole is not a struct".to_owned());
        };
        let path = parse_path(as_str(interns, field(fields, 0, "path")?, "path")?)?;
        let kind = match field(fields, 1, "kind")? {
            ConstTree::Variant { variant, .. } => *variant,
            ConstTree::Scalar { bits, .. } => *bits as u32,
            _ => return Err("`kind` names no `HoleKind`".to_owned()),
        };
        let kind =
            HoleKind::from_variant(kind).ok_or_else(|| format!("`{kind}` names no `HoleKind`"))?;
        let name = as_str(interns, field(fields, 2, "name")?, "name")?.to_owned();
        let reactive = as_bool(field(fields, 3, "reactive")?, "reactive")?;
        let group = as_u32(field(fields, 4, "effect_group")?, "effect_group")?;
        Ok(Hole {
            path,
            kind,
            name,
            reactive,
            effect_group: (group != NO_EFFECT_GROUP).then_some(group),
        })
    }
}

/// Parses a hole's walk: `c` for `firstChild`, `n` for `nextSibling`.
fn parse_path(path: &str) -> Result<Vec<Step>, String> {
    path.bytes()
        .map(|byte| match byte {
            b'c' => Ok(Step::FirstChild),
            b'n' => Ok(Step::NextSibling),
            other => Err(format!(
                "`{}` is not a walk step (`c` or `n`)",
                char::from(other).escape_debug()
            )),
        })
        .collect()
}

/// Renders a walk as the property chain it reads, for a diagnostic.
pub fn render_path(path: &[Step]) -> String {
    let mut out = String::from("root");
    for step in path {
        out.push('.');
        out.push_str(step.property());
    }
    out
}

/// The `delegateEvents` names of a set of templates, deduplicated and ordered.
pub fn delegated_events<'a>(templates: impl Iterator<Item = &'a Template>) -> Vec<String> {
    let mut events: Vec<String> = Vec::new();
    for template in templates {
        for event in &template.events {
            if !events.iter().any(|known| known == event) {
                events.push(event.clone());
            }
        }
    }
    events.sort();
    events
}

// ---------------------------------------------------------------------------------------------
// The template's node tree
// ---------------------------------------------------------------------------------------------

/// What kind of node a walk reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Element,
    Text,
    Comment,
    MarkerOpen,
    MarkerClose,
}

impl NodeKind {
    fn is_comment(self) -> bool {
        matches!(
            self,
            NodeKind::Comment | NodeKind::MarkerOpen | NodeKind::MarkerClose
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Node {
    kind: NodeKind,
    children: Vec<Node>,
}

/// The node tree of one template's HTML (v1 `template.rs::Tree`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tree {
    roots: Vec<Node>,
}

/// The elements an HTML parser closes without a closing tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

impl Tree {
    fn parse(html: &str) -> Tree {
        let bytes = html.as_bytes();
        let mut at = 0;
        let mut stack: Vec<(String, Vec<Node>)> = vec![(String::new(), Vec::new())];

        while at < bytes.len() {
            if bytes[at] != b'<' {
                let end = match html[at..].find('<') {
                    Some(offset) => at + offset,
                    None => bytes.len(),
                };
                push(
                    &mut stack,
                    Node {
                        kind: NodeKind::Text,
                        children: Vec::new(),
                    },
                );
                at = end;
                continue;
            }
            if html[at..].starts_with("<!") {
                let end = match html[at..].starts_with("<!--") {
                    true => html[at..].find("-->").map(|offset| at + offset + 3),
                    false => html[at..].find('>').map(|offset| at + offset + 1),
                };
                let kind = match &html[at..end.unwrap_or(bytes.len())] {
                    "<!$>" => NodeKind::MarkerOpen,
                    "<!/>" => NodeKind::MarkerClose,
                    _ => NodeKind::Comment,
                };
                push(
                    &mut stack,
                    Node {
                        kind,
                        children: Vec::new(),
                    },
                );
                at = end.unwrap_or(bytes.len());
                continue;
            }
            if html[at..].starts_with("</") {
                let end = html[at..].find('>').map(|offset| at + offset + 1);
                let name = tag_name(&html[at + 2..]);
                if let Some(depth) = stack.iter().rposition(|(open, _)| *open == name) {
                    while stack.len() > depth {
                        close(&mut stack);
                    }
                }
                at = end.unwrap_or(bytes.len());
                continue;
            }
            let name = tag_name(&html[at + 1..]);
            let end = tag_end(html, at);
            let self_closing = html[at..end]
                .trim_end_matches('>')
                .trim_end()
                .ends_with('/');
            if VOID_ELEMENTS.contains(&name.as_str()) || self_closing {
                push(
                    &mut stack,
                    Node {
                        kind: NodeKind::Element,
                        children: Vec::new(),
                    },
                );
            } else {
                stack.push((name, Vec::new()));
            }
            at = end;
        }

        while stack.len() > 1 {
            close(&mut stack);
        }
        Tree {
            roots: stack.pop().map(|(_, roots)| roots).unwrap_or_default(),
        }
    }

    /// The same tree with the synthetic `<svg>` wrapper stripped (CONTRACT-DOM 2.2).
    fn inside_svg_wrapper(self) -> Tree {
        match self.roots.into_iter().next() {
            Some(wrapper) => Tree {
                roots: wrapper.children,
            },
            None => Tree { roots: Vec::new() },
        }
    }

    /// The kind of the node `path` reaches from the template root, if it reaches one.
    pub fn kind_at(&self, path: &[Step]) -> Option<NodeKind> {
        self.node_at(path).map(|node| node.kind)
    }

    fn node_at(&self, path: &[Step]) -> Option<&Node> {
        let mut node = self.roots.first()?;
        let mut siblings: &[Node] = &self.roots;
        let mut index = 0usize;
        for step in path {
            match step {
                Step::FirstChild => {
                    siblings = &node.children;
                    index = 0;
                    node = siblings.first()?;
                }
                Step::NextSibling => {
                    index += 1;
                    node = siblings.get(index)?;
                }
            }
        }
        Some(node)
    }
}

fn push(stack: &mut [(String, Vec<Node>)], node: Node) {
    if let Some((_, children)) = stack.last_mut() {
        children.push(node);
    }
}

fn close(stack: &mut Vec<(String, Vec<Node>)>) {
    let Some((_, children)) = stack.pop() else {
        return;
    };
    push(
        stack,
        Node {
            kind: NodeKind::Element,
            children,
        },
    );
}

fn tag_name(rest: &str) -> String {
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == ':')
        .flat_map(char::to_lowercase)
        .collect()
}

fn tag_end(html: &str, at: usize) -> usize {
    let bytes = html.as_bytes();
    let mut quote = None;
    let mut index = at;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(open) if byte == open => quote = None,
            Some(_) => {}
            None if byte == b'"' || byte == b'\'' => quote = Some(byte),
            None if byte == b'>' => return index + 1,
            None => {}
        }
        index += 1;
    }
    bytes.len()
}

// ---------------------------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------------------------

/// The walks one instantiation needs, lowered to temporaries that share their prefixes
/// (v1 `template.rs::Walk`).
pub struct Walk {
    nodes: Vec<(Vec<Step>, Expr)>,
    pairs: Vec<(Vec<Step>, Expr)>,
}

impl Walk {
    /// Plans the walks reaching every path in `paths`, rooted at `root`.
    pub fn plan(
        root: Expr,
        tree: &Tree,
        paths: &[&[Step]],
        mut bind: impl FnMut(Expr) -> Expr,
        mut marker_pair: impl FnMut(Expr) -> Expr,
    ) -> Walk {
        let mut wanted: Vec<Vec<Step>> = Vec::new();
        for path in paths {
            for length in 1..=path.len() {
                let prefix = path[..length].to_vec();
                if !wanted.contains(&prefix) {
                    wanted.push(prefix);
                }
            }
        }
        wanted.sort_by_key(Vec::len);

        let mut nodes: Vec<(Vec<Step>, Expr)> = vec![(Vec::new(), root)];
        let mut pairs: Vec<(Vec<Step>, Expr)> = Vec::new();
        for path in wanted {
            let (parent, step) = path.split_at(path.len() - 1);
            let Some(base) = nodes
                .iter()
                .find(|(known, _)| known == parent)
                .map(|(_, expr)| expr.clone())
            else {
                continue;
            };
            let closes_a_pair = step[0] == Step::NextSibling
                && tree.kind_at(&path) == Some(NodeKind::MarkerClose)
                && tree.kind_at(parent) == Some(NodeKind::MarkerOpen);
            if closes_a_pair {
                let pair = marker_pair(jsast::member(base, Step::NextSibling.property()));
                pairs.push((parent.to_vec(), pair.clone()));
                nodes.push((path, jsast::index(pair, jsast::num(0))));
                continue;
            }
            let expr = bind(jsast::member(base, step[0].property()));
            nodes.push((path, expr));
        }
        Walk { nodes, pairs }
    }

    /// The expression naming the node `path` reaches.
    pub fn at(&self, path: &[Step]) -> Option<Expr> {
        self.nodes
            .iter()
            .find(|(known, _)| known.as_slice() == path)
            .map(|(_, expr)| expr.clone())
    }

    /// The `_$getNextMarker` pair of the `<!$>` at `path`.
    pub fn marker_pair(&self, path: &[Step]) -> Option<Expr> {
        self.pairs
            .iter()
            .find(|(known, _)| known.as_slice() == path)
            .map(|(_, expr)| expr.clone())
    }
}

// ---------------------------------------------------------------------------------------------
// One instantiated template
// ---------------------------------------------------------------------------------------------

/// Where one hole of an instantiated template writes.
#[derive(Clone)]
struct HoleTarget {
    node: Expr,
    parent: Option<Expr>,
    pair: Option<Expr>,
}

/// One hole's write, held until the rest of its shared effect arrives.
struct Write {
    hole: Hole,
    target: HoleTarget,
    fill: Fill,
}

/// What a hole is filled with: a value written once, or a closure the runtime re-runs.
enum Fill {
    Once(Expr),
    Tracked { accessor: Expr, value: Expr },
}

impl Fill {
    fn accessor(&self) -> Expr {
        match self {
            Fill::Once(value)
            | Fill::Tracked {
                accessor: value, ..
            } => value.clone(),
        }
    }

    fn value(&self) -> Expr {
        match self {
            Fill::Once(value) | Fill::Tracked { value, .. } => value.clone(),
        }
    }

    fn is_tracked(&self) -> bool {
        matches!(self, Fill::Tracked { .. })
    }
}

/// One template instantiated in this function, kept until its holes have been filled.
pub struct Instantiation {
    template: Template,
    targets: Vec<Result<HoleTarget, String>>,
    awaited: BTreeMap<u32, Vec<usize>>,
    arrived: BTreeMap<u32, Vec<Write>>,
}

/// The record key of the `index`th write of a shared effect (CONTRACT-DOM 6.3).
fn record_key(index: usize) -> String {
    const ALPHABET: &[u8] = b"etaoinshrdlucwmfygpbTAOISWCBvkxjqzPHFMDRELNGUKVYJQZX_$";
    let mut out = Vec::new();
    let mut left = index;
    loop {
        out.push(ALPHABET[left % ALPHABET.len()]);
        left /= ALPHABET.len();
        if left == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// A hoisted temporary holding `value`: the assignment as an expression, and the name.
fn temp(cx: &JsCx<'_>, value: Expr) -> (Expr, Expr) {
    let (_, name) = cx.temp(value.clone());
    (jsast::assign(name.clone(), value), name)
}

/// `effects` in order, answering `value` (a comma sequence, or `value` alone).
fn sequence(mut effects: Vec<Expr>, value: Expr) -> Expr {
    if effects.is_empty() {
        return value;
    }
    effects.push(value);
    jsast::seq(effects)
}

// ---------------------------------------------------------------------------------------------
// The markers
// ---------------------------------------------------------------------------------------------

/// Lowers a call to one of the `view-abi` markers (v1 `abi.rs::codegen_marker`).
pub fn lower_marker(cx: &JsCx<'_>, op: &Op, marker: &str, args: &[ValueId]) -> Res<Expr> {
    match marker {
        "template" => lower_template(cx, op, args),
        "hole" | "handler" | "component" => lower_fill(cx, marker, args, false),
        "effect" => lower_fill(cx, marker, args, true),
        "signal" => lower_signal(cx, args),
        "selector" => lower_selector(cx, args),
        "selis" => lower_selector_read(cx, args),
        "sget" => lower_signal_read(cx, args),
        "sset" => lower_signal_write(cx, args),
        "etv" => lower_event_target_value(cx, args),
        "epd" => lower_event_prevent_default(cx, args),
        "content" => lower_content(cx, args),
        "text" => lower_text(cx, op, args),
        "cond" => lower_cond(cx, args),
        "list" => Ok(jsast::array(Vec::new())),
        "push" => lower_push(cx, args),
        "pushkeyed" => lower_push_keyed(cx, op, args),
        "keyedlist.v1" => Err(decline(
            OpClass::Template,
            "keyed.legacy-payload: the version-one keyed-list marker has no dedicated JIR payload",
        )),
        "micro" => lower_microtask(cx, args),
        "cleanup" => lower_cleanup(cx, args),
        "settled" => lower_settled(cx, args),
        "owner" => Ok(jsast::call(dom("getOwner"), Vec::new())),
        "withowner" => lower_with_owner(cx, args),
        "hostval" => lower_host_value(cx, op, args),
        other => Err(decline(
            OpClass::Template,
            format!("`rcgjs.tc.{other}` is not a `view-abi` marker this backend knows"),
        )),
    }
}

/// Validates the dedicated keyed-list object contract before an executable lane may use it.
pub fn lower_keyed_list(cx: &JsCx<'_>, payload: &KeyedListPayload, args: &[ValueId]) -> Res<Expr> {
    let keyed_decline = |reason: &str, detail: &str| {
        Err(decline(
            OpClass::Template,
            format!("keyed.{reason}: {detail}"),
        ))
    };
    if payload.format != KEYED_LIST_FORMAT_V1 {
        return keyed_decline(
            "schema-version",
            &format!(
                "keyed-list payload format {} is unsupported (expected {})",
                payload.format, KEYED_LIST_FORMAT_V1
            ),
        );
    }
    if args.len() != 3 {
        return keyed_decline(
            "marker-arity",
            &format!("keyed-list marker has {} arguments, expected 3", args.len()),
        );
    }
    for (role, argument, callable, arity, result_ty) in [
        (
            "collection",
            args[0],
            &payload.collection,
            0,
            payload.collection_ty,
        ),
        ("key", args[1], &payload.key, 1, payload.key_ty),
        ("row", args[2], &payload.row, 2, payload.row_ty),
    ] {
        let argument_ty = cx.ty_of(argument)?;
        if argument_ty != callable.ty {
            return keyed_decline(
                "callable-type-mismatch",
                &format!("{role} callable type does not match its argument"),
            );
        }
        validate_keyed_callable(cx, role, callable, arity, result_ty)?;
    }
    if !supported_key_type(cx, payload.key_ty) {
        return keyed_decline(
            "unsupported-key-type",
            "the key type has no exact native integer, string, character, or boolean representation",
        );
    }
    let collection_lane = keyed_collection_lane(cx, payload.collection_ty, payload.item_ty)
        .map_err(|(reason, detail)| {
            decline(OpClass::Template, format!("keyed.{reason}: {detail}"))
        })?;
    for (role, callable) in [
        ("collection", &payload.collection),
        ("key", &payload.key),
        ("row", &payload.row),
    ] {
        if !matches!(callable.cleanup, DropGlue::None) {
            return keyed_decline(
                "callable-cleanup-not-ready",
                &format!("the initial executable lane cannot retain a dropful {role} callable"),
            );
        }
    }
    for (role, ty) in [("item", payload.item_ty), ("key", payload.key_ty)] {
        if keyed_value_needs_drop(cx, ty) {
            return keyed_decline(
                "dropful-value-not-ready",
                &format!("the initial executable lane cannot retain a dropful {role} value"),
            );
        }
    }
    let row_capture = payload
        .row
        .body
        .and_then(|body| cx.items.get(&body.item))
        .and_then(|entry| entry.keyed_row_capture);
    let (live, live_index) = match row_capture {
        Some(KeyedRowCapture::Static) => (false, false),
        Some(KeyedRowCapture::Live) => {
            if !supported_live_item(cx, payload.item_ty) {
                return keyed_decline(
                    "row-live-item-not-ready",
                    "the first live row lane requires an immutable item reference",
                );
            }
            let Some(row_body) = payload.row.body.and_then(|body| cx.items.get(&body.item)) else {
                return keyed_decline(
                    "row-capture-unproven",
                    "the linker could not inspect the live row body",
                );
            };
            let index_parameter = match cx
                .table
                .get(payload.row.ty)
                .map(|descriptor| &descriptor.kind)
            {
                Some(TypeKind::Closure { .. }) => 2,
                Some(TypeKind::FnDef { .. }) => 1,
                _ => {
                    return keyed_decline(
                        "row-callable-type",
                        "the live row callable is neither a closure nor a function item",
                    );
                }
            };
            let item_parameter = index_parameter - 1;
            let proxy_safe = row_body.keyed_proxy_safe_params.as_ref();
            if !proxy_safe.is_some_and(|params| params.contains(&item_parameter)) {
                return keyed_decline(
                    "row-live-item-use-not-ready",
                    "the live item reaches an operation the callable-proxy ABI cannot preserve",
                );
            }
            let live_index = row_body.used_params.contains(&index_parameter);
            if live_index && !proxy_safe.is_some_and(|params| params.contains(&index_parameter)) {
                return keyed_decline(
                    "row-live-index-not-ready",
                    "the live index reaches an operation the callable-proxy ABI cannot preserve",
                );
            }
            (true, live_index)
        }
        None => {
            return keyed_decline(
                "row-capture-unproven",
                "the linker could not prove a creation-only row body",
            );
        }
    };

    let mut setup = Vec::new();
    let mut callables = Vec::new();
    for (index, argument) in args.iter().enumerate() {
        let (materialize, callable, _) =
            super::host::closure_thunk_with(cx, *argument, false, false)?;
        setup.extend(materialize);
        callables.push(if index == 0 {
            collection_lane.adapt(cx, callable)?
        } else {
            callable
        });
    }
    let hooks = jsast::object(vec![
        ("batch".to_owned(), dom("batch")),
        (
            "items".to_owned(),
            jsast::string(collection_lane.runtime_mode()),
        ),
        ("cleanup".to_owned(), dom("onCleanup")),
        ("effect".to_owned(), dom("effect")),
        ("live".to_owned(), jsast::boolean(live)),
        ("liveIndex".to_owned(), jsast::boolean(live_index)),
        ("root".to_owned(), dom("createRoot")),
        ("signal".to_owned(), dom("createSignal")),
        ("untrack".to_owned(), dom("untrack")),
    ]);
    callables.push(hooks);
    Ok(sequence(setup, jsast::rt_call("keyed_dom_view", callables)))
}

fn validate_keyed_callable(
    cx: &JsCx<'_>,
    role: &str,
    callable: &KeyedListCallable,
    arity: usize,
    result_ty: TypeId,
) -> Res<()> {
    let Some(descriptor) = cx.table.get(callable.ty) else {
        return Err(decline(
            OpClass::Template,
            format!("keyed.{role}-type-missing: the callable type is absent from the type table"),
        ));
    };
    let Some(body) = callable.body else {
        return Err(decline(
            OpClass::Template,
            format!("keyed.{role}-callable-indirect: rustc resolved no direct callable body"),
        ));
    };
    let (body_matches_type, has_environment) = match &descriptor.kind {
        TypeKind::Closure { def, .. } => (
            body.item.definition == *def && body.item.instantiation == callable.ty.0,
            true,
        ),
        TypeKind::FnDef {
            instance: Some(instance),
            ..
        } => (body == *instance, false),
        TypeKind::FnDef { instance: None, .. } | TypeKind::FnPtr { .. } => (false, false),
        _ => {
            return Err(decline(
                OpClass::Template,
                format!("keyed.{role}-callable-type: the value is not a closure or function item"),
            ));
        }
    };
    if !body_matches_type {
        return Err(decline(
            OpClass::Template,
            format!("keyed.{role}-body-mismatch: the callable type resolves to another body"),
        ));
    }
    let Some(entry) = cx.items.get(&body.item) else {
        return Err(decline(
            OpClass::Template,
            format!("keyed.{role}-body-unavailable: the direct callable body is not linked"),
        ));
    };
    if cx.interns.str(body.link_name) != Some(entry.name.as_str()) {
        return Err(decline(
            OpClass::Template,
            format!("keyed.{role}-body-mismatch: callable identity and linked body disagree"),
        ));
    }
    let actual_arity = entry
        .params
        .saturating_sub(usize::from(has_environment))
        .saturating_sub(usize::from(entry.caller_location));
    if actual_arity != arity {
        return Err(decline(
            OpClass::Template,
            format!(
                "keyed.{role}-arity: callable body takes {actual_arity} value arguments, expected {arity}"
            ),
        ));
    }
    if entry.result != Some(result_ty) {
        return Err(decline(
            OpClass::Template,
            format!(
                "keyed.{role}-result: callable body returns {:?}, expected {result_ty:?}",
                entry.result
            ),
        ));
    }
    if role == "key" {
        let item_parameter = u16::try_from(usize::from(has_environment)).map_err(|_| {
            decline(
                OpClass::Template,
                "keyed.key-parameter-overflow: the item parameter cannot be represented",
            )
        })?;
        match entry.keyed_purity {
            Some(super::host::KeyedPurity::Const) => {}
            Some(super::host::KeyedPurity::Read)
                if entry.keyed_read_params.as_ref().is_some_and(|parameters| {
                    parameters.len() == 1 && parameters.contains(&item_parameter)
                }) => {}
            Some(super::host::KeyedPurity::Read) => {
                return Err(decline(
                    OpClass::Template,
                    "keyed.key-extractor-reads-state: the key is not a function of its item alone",
                ));
            }
            Some(super::host::KeyedPurity::Effect) => {
                return Err(decline(
                    OpClass::Template,
                    "keyed.key-extractor-effectful: the key extractor has observable effects",
                ));
            }
            None => {
                return Err(decline(
                    OpClass::Template,
                    "keyed.key-purity-unproven: no validated effects summary exists for the key extractor",
                ));
            }
        }
        match entry.keyed_may_unwind {
            Some(false) => {}
            Some(true) => {
                return Err(decline(
                    OpClass::Template,
                    "keyed.key-extractor-fallible: the key extractor may panic or throw",
                ));
            }
            None => {
                return Err(decline(
                    OpClass::Template,
                    "keyed.key-fallibility-unproven: no validated unwind summary exists for the key extractor",
                ));
            }
        }
    }
    match callable.cleanup {
        DropGlue::None if !descriptor.layout.needs_drop => {}
        DropGlue::None => {
            return Err(decline(
                OpClass::Template,
                format!(
                    "keyed.{role}-cleanup-missing: the callable needs cleanup but records none"
                ),
            ));
        }
        DropGlue::Dynamic => {
            return Err(decline(
                OpClass::Template,
                format!("keyed.{role}-cleanup-dynamic: callable cleanup is dynamically dispatched"),
            ));
        }
        DropGlue::Direct { item, link_name } => {
            if !descriptor.layout.needs_drop {
                return Err(decline(
                    OpClass::Template,
                    format!(
                        "keyed.{role}-cleanup-unexpected: a drop-free callable records cleanup"
                    ),
                ));
            }
            let Some(entry) = cx.items.get(&item) else {
                return Err(decline(
                    OpClass::Template,
                    format!("keyed.{role}-cleanup-unavailable: callable cleanup is not linked"),
                ));
            };
            if cx.interns.str(link_name) != Some(entry.name.as_str()) {
                return Err(decline(
                    OpClass::Template,
                    format!(
                        "keyed.{role}-cleanup-mismatch: cleanup identity and linked body disagree"
                    ),
                ));
            }
            if cx.drop_glue.get(&callable.ty).map(String::as_str) != Some(entry.name.as_str()) {
                return Err(decline(
                    OpClass::Template,
                    format!(
                        "keyed.{role}-cleanup-mismatch: callable type and cleanup body disagree"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn supported_key_type(cx: &JsCx<'_>, ty: TypeId) -> bool {
    let ty = cx.table.peeled(ty);
    let Some(descriptor) = cx.table.get(ty) else {
        return false;
    };
    match &descriptor.kind {
        TypeKind::Bool | TypeKind::Char | TypeKind::Integer { .. } | TypeKind::Str => true,
        TypeKind::Reference {
            pointee,
            mutable: false,
        } => supported_key_type(cx, *pointee),
        _ => descriptor.meta.candidate == RepresentationCandidate::String,
    }
}

#[derive(Clone, Copy)]
enum KeyedCollectionLane {
    Owned,
    OwnedVec(TypeId),
    BorrowedSlot,
    BorrowedValue,
    BorrowedVecSlot(TypeId),
    BorrowedVecValue(TypeId),
}

impl KeyedCollectionLane {
    fn runtime_mode(self) -> &'static str {
        match self {
            Self::Owned | Self::OwnedVec(_) => "owned",
            Self::BorrowedSlot | Self::BorrowedVecSlot(_) => "shared-slot",
            Self::BorrowedValue | Self::BorrowedVecValue(_) => "shared-value",
        }
    }

    fn adapt(self, cx: &JsCx<'_>, callable: Expr) -> Res<Expr> {
        let vector = match self {
            Self::OwnedVec(vector)
            | Self::BorrowedVecSlot(vector)
            | Self::BorrowedVecValue(vector) => vector,
            _ => return Ok(callable),
        };
        let value = jsast::call(callable, Vec::new());
        let view = super::adapters::convert(
            cx.table,
            cx.interns,
            value,
            vector,
            Carrier::Baseline,
            Carrier::SliceView,
        )?;
        Ok(jsast::arrow(Vec::new(), view))
    }
}

fn keyed_collection_lane(
    cx: &JsCx<'_>,
    collection: TypeId,
    item: TypeId,
) -> Result<KeyedCollectionLane, (&'static str, &'static str)> {
    let collection = cx.table.peeled(collection);
    let descriptor = cx.table.get(collection).ok_or((
        "source-type-missing",
        "the collection type is absent from the post-transform type table",
    ))?;
    match &descriptor.kind {
        TypeKind::Array { element, .. } => {
            require_keyed_item(cx, *element, item, false)?;
            Ok(KeyedCollectionLane::Owned)
        }
        TypeKind::Adt { args, .. } if descriptor.meta.candidate == RepresentationCandidate::Vec => {
            require_keyed_vec(cx, descriptor, args, item, false)?;
            Ok(KeyedCollectionLane::OwnedVec(descriptor.id))
        }
        TypeKind::Reference { pointee, mutable } => {
            if *mutable {
                return Err((
                    "mutable-source-borrow",
                    "a keyed collection cannot retain items yielded through a mutable borrow",
                ));
            }
            let pointee = cx.table.peeled(*pointee);
            let pointee = cx.table.get(pointee).ok_or((
                "source-pointee-missing",
                "the borrowed collection's pointee is absent from the post-transform type table",
            ))?;
            match &pointee.kind {
                TypeKind::Array { element, .. } | TypeKind::Slice(element) => {
                    require_keyed_item(cx, *element, item, true)?;
                    let element = cx.table.peeled(*element);
                    let by_value = cx.table.is_indirect(element)
                        || matches!(
                            cx.table.get(element).map(|descriptor| &descriptor.kind),
                            Some(TypeKind::Str)
                        );
                    Ok(if by_value {
                        KeyedCollectionLane::BorrowedValue
                    } else {
                        KeyedCollectionLane::BorrowedSlot
                    })
                }
                TypeKind::Adt { args, .. }
                    if pointee.meta.candidate == RepresentationCandidate::Vec =>
                {
                    let element = require_keyed_vec(cx, pointee, args, item, true)?;
                    let by_value = cx.table.is_indirect(cx.table.peeled(element));
                    Ok(if by_value {
                        KeyedCollectionLane::BorrowedVecValue(pointee.id)
                    } else {
                        KeyedCollectionLane::BorrowedVecSlot(pointee.id)
                    })
                }
                _ => Err((
                    "unsupported-source-type",
                    "the borrowed source is not an array, slice, or canonical Vec",
                )),
            }
        }
        _ => Err((
            "unsupported-source-type",
            "the source is not an array, shared array or slice, or canonical Vec",
        )),
    }
}

fn require_keyed_vec(
    cx: &JsCx<'_>,
    vector: &rcgjs_object::jir::types::TypeDescriptor,
    args: &[TypeId],
    item: TypeId,
    borrowed: bool,
) -> Result<TypeId, (&'static str, &'static str)> {
    if vector.meta.allocator != AllocatorIdentity::Global {
        return Err((
            "unsupported-source-allocator",
            "the Vec source does not use the proven global allocator",
        ));
    }
    let element = *args.first().ok_or((
        "source-element-type-missing",
        "the Vec source has no recorded element type",
    ))?;
    require_keyed_item(cx, element, item, borrowed)?;
    if !cx
        .table
        .get(cx.table.peeled(element))
        .and_then(|element| element.layout.size)
        .is_some_and(|size| size > 0)
    {
        return Err((
            "unsupported-source-element-layout",
            "the Vec source element has no nonzero proven layout",
        ));
    }
    Ok(element)
}

fn require_keyed_item(
    cx: &JsCx<'_>,
    element: TypeId,
    item: TypeId,
    borrowed: bool,
) -> Result<(), (&'static str, &'static str)> {
    if keyed_item_matches(cx, element, item, borrowed) {
        Ok(())
    } else {
        Err((
            "source-item-mismatch",
            "the collection's yielded item does not match the keyed-list item type",
        ))
    }
}

fn supported_live_item(cx: &JsCx<'_>, item: TypeId) -> bool {
    matches!(
        cx.table
            .get(cx.table.peeled(item))
            .map(|descriptor| &descriptor.kind),
        Some(TypeKind::Reference { mutable: false, .. })
    )
}

fn keyed_value_needs_drop(cx: &JsCx<'_>, ty: TypeId) -> bool {
    let ty = cx.table.peeled(ty);
    let Some(descriptor) = cx.table.get(ty) else {
        return false;
    };
    let value = match &descriptor.kind {
        TypeKind::Reference {
            pointee,
            mutable: false,
        } => cx.table.peeled(*pointee),
        _ => ty,
    };
    cx.table
        .get(value)
        .is_some_and(|descriptor| descriptor.layout.needs_drop)
}

fn keyed_item_matches(cx: &JsCx<'_>, element: TypeId, item: TypeId, borrowed: bool) -> bool {
    let element = cx.table.peeled(element);
    let item = cx.table.peeled(item);
    if !borrowed {
        return element == item;
    }
    matches!(
        cx.table.get(item).map(|descriptor| &descriptor.kind),
        Some(TypeKind::Reference { pointee, mutable: false })
            if cx.table.peeled(*pointee) == element
    )
}

/// `template(&DATA)`: the cloner, the walk, and the record the holes are filled through
/// (v1 `abi.rs::codegen_template`).
fn lower_template(cx: &JsCx<'_>, op: &Op, args: &[ValueId]) -> Res<Expr> {
    if !matches!(cx.opts.modules, rcgjs_object::opts::Modules::Esm) {
        return Err(decline(
            OpClass::Template,
            "a `view!` template needs the dom-expressions runtime, which is imported: compile \
             with `-Cllvm-args=js-modules=esm`",
        ));
    }
    let OpKind::Call {
        host: Some(HostPayload::Marker {
            template: Some(tree),
            ..
        }),
        ..
    } = &op.kind
    else {
        return Err(decline(
            OpClass::Template,
            "the encoder recorded no `view!` payload for this `rcgjs.tc.template` call (its \
             first argument is not a reference to a `static`)",
        ));
    };
    let _ = args;
    let template = Template::read(cx.interns, tree).map_err(|err| {
        decline(
            OpClass::Template,
            format!("the `view!` template cannot be read: {err}"),
        )
    })?;

    let mut effects = Vec::new();
    let cloner = template.cloner();
    let root_call = match template.hydratable {
        true => jsast::call(dom("getNextElement"), vec![cloner]),
        false => jsast::call(cloner, Vec::new()),
    };
    let (bind_root, root) = temp(cx, root_call);
    effects.push(bind_root);

    let tree = template.tree();
    let mut wanted: Vec<&[Step]> = Vec::new();
    let closing: Vec<Vec<Step>> = template
        .holes
        .iter()
        .filter(|hole| {
            hole.kind.inserts_a_node() && tree.kind_at(&hole.path) == Some(NodeKind::MarkerOpen)
        })
        .map(|hole| {
            let mut path = hole.path.clone();
            path.push(Step::NextSibling);
            path
        })
        .collect();
    for hole in &template.holes {
        wanted.push(&hole.path);
        if hole.kind.inserts_a_node() && tree.kind_at(&hole.path).is_some_and(NodeKind::is_comment)
        {
            if let Some(parent) = hole.anchor_parent() {
                wanted.push(parent);
            }
        }
    }
    wanted.extend(closing.iter().map(Vec::as_slice));

    let steps = std::cell::RefCell::new(Vec::new());
    let walk = Walk::plan(
        root.clone(),
        &tree,
        &wanted,
        |expr| {
            let (bind, name) = temp(cx, expr);
            steps.borrow_mut().push(bind);
            name
        },
        |expr| {
            let (bind, name) = temp(cx, jsast::call(dom("getNextMarker"), vec![expr]));
            steps.borrow_mut().push(bind);
            name
        },
    );
    effects.extend(steps.into_inner());

    let targets: Vec<Result<HoleTarget, String>> = template
        .holes
        .iter()
        .map(|hole| {
            let rendered = render_path(&hole.path);
            let Some(kind) = tree.kind_at(&hole.path) else {
                return Err(format!(
                    "its walk `{rendered}` leaves the template `{}`",
                    template.html
                ));
            };
            let node = walk
                .at(&hole.path)
                .ok_or_else(|| format!("its walk `{rendered}` was not planned"))?;
            let anchored = hole.kind.inserts_a_node() && kind.is_comment();
            let parent = match anchored {
                true => Some(
                    hole.anchor_parent()
                        .and_then(|path| walk.at(path))
                        .ok_or_else(|| {
                            format!(
                                "the anchor its walk `{rendered}` reaches is not inside an \
                                 element of the template `{}`",
                                template.html
                            )
                        })?,
                ),
                false => None,
            };
            Ok(HoleTarget {
                node,
                parent,
                pair: walk.marker_pair(&hole.path),
            })
        })
        .collect();

    let events = delegated_events(std::iter::once(&template));
    if !events.is_empty() {
        let names = events.into_iter().map(jsast::string).collect();
        effects.push(jsast::call(
            dom("delegateEvents"),
            vec![jsast::array(names)],
        ));
    }

    let mut awaited: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (index, hole) in template.holes.iter().enumerate() {
        if let Some(group) = hole.effect_group {
            awaited.entry(group).or_default().push(index);
        }
    }
    awaited.retain(|_, holes| holes.len() > 1);

    let Some(result) = op.result else {
        return Err(decline(
            OpClass::Template,
            "a `rcgjs.tc.template` call whose root is never named",
        ));
    };
    cx.instantiations.borrow_mut().insert(
        result,
        Instantiation {
            template,
            targets,
            awaited,
            arrived: BTreeMap::new(),
        },
    );
    Ok(sequence(effects, root))
}

/// `hole(&root, index, value)` and `effect(&root, index, closure)` (v1 `abi.rs::codegen_fill`).
fn lower_fill(cx: &JsCx<'_>, marker: &str, args: &[ValueId], tracked: bool) -> Res<Expr> {
    let [root, index, value] = args else {
        return Err(decline(
            OpClass::Template,
            "a `view-abi` hole marker takes three arguments",
        ));
    };
    let index = constant_u32(cx, *index).ok_or_else(|| {
        decline(
            OpClass::Template,
            "the index of a `view-abi` hole marker is not a constant",
        )
    })?;

    let Some(resolved_root) = resolve_instantiation_root(cx, *root) else {
        return Err(decline(
            OpClass::Template,
            "the `root` of a `view-abi` hole marker does not name a template this body \
             instantiated",
        ));
    };

    let (hole, target, shared) = {
        let instantiations = cx.instantiations.borrow();
        let Some(instantiation) = instantiations.get(&resolved_root) else {
            return Err(decline(
                OpClass::Template,
                "the `root` of a `view-abi` hole marker does not name a template this body \
                 instantiated",
            ));
        };
        let Some(hole) = instantiation.template.holes.get(index as usize) else {
            return Err(decline(
                OpClass::Template,
                format!(
                    "hole {index} is past the end of a template with {} holes",
                    instantiation.template.holes.len()
                ),
            ));
        };
        let target = match &instantiation.targets[index as usize] {
            Ok(target) => target.clone(),
            Err(err) => {
                return Err(decline(
                    OpClass::Template,
                    format!("hole {index} cannot be filled: {err}"),
                ));
            }
        };
        let hole = hole.clone();
        let shared = hole
            .effect_group
            .filter(|group| instantiation.awaited.contains_key(group));
        (hole, target, shared)
    };

    let as_function = tracked
        || matches!(
            hole.kind,
            HoleKind::DelegatedEvent | HoleKind::Event | HoleKind::Component
        );
    // Inline closure environments are materialized here, at the marker call. A closure created in
    // a loop then snapshots that value through `host::closure_thunk`'s IIFE.
    let (setup, fill) = match as_function {
        true => {
            let (convert_params_in, convert_result_out) = if tracked {
                (false, true)
            } else if marker == "component" || matches!(hole.kind, HoleKind::Component) {
                (false, false)
            } else {
                (true, false)
            };
            let (setup, accessor, call) =
                super::host::closure_thunk_with(cx, *value, convert_params_in, convert_result_out)?;
            let fill = match tracked {
                true => Fill::Tracked {
                    accessor,
                    value: call,
                },
                false => Fill::Once(accessor),
            };
            (setup, fill)
        }
        false => {
            let expr = cx.expr_of(*value)?;
            let expr = if crate::lower::bstr::enabled() {
                let ty = cx.ty_of(*value)?;
                if cx.is_known_ascii(*value) && crate::lower::bstr::is_str_pointer(cx.table, ty) {
                    expr
                } else {
                    super::host::crossing(cx, ty, expr, crate::lower::bstr::Crossing::Out)?
                }
            } else {
                expr
            };
            (None, Fill::Once(expr))
        }
    };
    let setup: Vec<Expr> = setup.into_iter().collect();

    let Some(group) = shared else {
        return Ok(sequence(setup, write_hole(&hole, &target, fill)));
    };
    let writes = {
        let mut instantiations = cx.instantiations.borrow_mut();
        let Some(instantiation) = instantiations.get_mut(&resolved_root) else {
            return Ok(sequence(setup, jsast::undefined()));
        };
        instantiation
            .awaited
            .entry(group)
            .or_default()
            .retain(|waiting| *waiting != index as usize);
        instantiation
            .arrived
            .entry(group)
            .or_default()
            .push(Write { hole, target, fill });
        if !instantiation.awaited[&group].is_empty() {
            return Ok(jsast::undefined());
        }
        instantiation.arrived.remove(&group).unwrap_or_default()
    };
    Ok(sequence(setup, write_group(writes)))
}

/// One `_$effect` carrying every write of a shared effect group, with its `_p$` record
/// (v1 `abi.rs::write_group`).
fn write_group(writes: Vec<Write>) -> Expr {
    const RECORD: &str = "_p$";

    let value_name = |index: usize| match index {
        0 => "_v$".to_owned(),
        n => format!("_v${}", n + 1),
    };

    let mut body = Vec::new();
    let decls: Vec<(String, Option<Expr>)> = writes
        .iter()
        .enumerate()
        .map(|(index, write)| (value_name(index), Some(write.fill.value())))
        .collect();
    body.push(jsast::lets(decls));

    for (index, write) in writes.iter().enumerate() {
        let key = record_key(index);
        let slot = jsast::member(jsast::id(RECORD), key);
        let value = jsast::id(value_name(index));
        let node = write.target.node.clone();
        let name = jsast::string(write.hole.name.clone());
        match write.hole.kind {
            HoleKind::ClassList | HoleKind::Style => {
                let helper = match write.hole.kind {
                    HoleKind::ClassList => "classList",
                    _ => "style",
                };
                let call = jsast::call(dom(helper), vec![node, value, slot.clone()]);
                body.push(jsast::assign_stmt(slot, call));
            }
            _ => {
                let stored = jsast::assign(slot.clone(), value.clone());
                let write_call = match write.hole.kind {
                    HoleKind::Property => jsast::assign(jsast::index(node, name), stored),
                    _ => jsast::call(dom("setAttribute"), vec![node, name, stored]),
                };
                body.push(jsast::expr_stmt(jsast::binary(
                    BinOp::And,
                    jsast::binary(BinOp::StrictNe, value, slot),
                    write_call,
                )));
            }
        }
    }
    body.push(jsast::ret(jsast::id(RECORD)));

    let seed = (0..writes.len())
        .map(|index| (record_key(index), jsast::undefined()))
        .collect();
    jsast::call(
        dom("effect"),
        vec![
            jsast::arrow_block(vec![RECORD.to_owned()], body),
            jsast::object(seed),
        ],
    )
}

/// The expression that fills one located hole (v1 `abi.rs::write_hole`).
fn write_hole(hole: &Hole, target: &HoleTarget, fill: Fill) -> Expr {
    let node = target.node.clone();
    let name = || jsast::string(hole.name.clone());
    let reactive = fill.is_tracked();
    let wrap = |body: Expr| match reactive {
        true => jsast::call(dom("effect"), vec![jsast::arrow(Vec::new(), body)]),
        false => body,
    };
    let wrap_diffing = |helper: &str| {
        let previous = "_$p";
        let mut arguments = vec![node.clone(), fill.value()];
        if reactive {
            arguments.push(jsast::id(previous));
        }
        let call = jsast::call(dom(helper), arguments);
        match reactive {
            true => jsast::call(
                dom("effect"),
                vec![jsast::arrow(vec![previous.to_owned()], call)],
            ),
            false => call,
        }
    };

    match hole.kind {
        HoleKind::Child | HoleKind::Component => {
            let value = match hole.kind {
                HoleKind::Component => jsast::call(dom("createComponent"), vec![fill.accessor()]),
                _ => fill.accessor(),
            };
            let Some(parent) = target.parent.clone() else {
                return jsast::call(dom("insert"), vec![node, value]);
            };
            let Some(pair) = target.pair.clone() else {
                return jsast::call(dom("insert"), vec![parent, value, node]);
            };
            jsast::call(
                dom("insert"),
                vec![
                    parent,
                    value,
                    jsast::index(pair.clone(), jsast::num(0)),
                    jsast::index(pair, jsast::num(1)),
                ],
            )
        }
        HoleKind::Attribute => wrap(jsast::call(
            dom("setAttribute"),
            vec![node.clone(), name(), fill.value()],
        )),
        HoleKind::Property => wrap(jsast::assign(
            jsast::index(node.clone(), name()),
            fill.value(),
        )),
        HoleKind::DelegatedEvent => jsast::assign(
            jsast::index(node, jsast::string(format!("$${}", hole.name))),
            fill.accessor(),
        ),
        HoleKind::Event => {
            jsast::method_call(node, "addEventListener", vec![name(), fill.accessor()])
        }
        HoleKind::ClassList => wrap_diffing("classList"),
        HoleKind::Style => wrap_diffing("style"),
        HoleKind::Spread => jsast::call(
            dom("spread"),
            vec![
                node,
                fill.accessor(),
                jsast::boolean(false),
                jsast::boolean(false),
            ],
        ),
    }
}

/// `signal(ordinal, init)`: the runtime's `createSignal`. The ordinal is dropped.
fn lower_signal(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [_ordinal, init] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::signal` takes two arguments",
        ));
    };
    Ok(jsast::call(dom("createSignal"), vec![cx.expr_of(*init)?]))
}

/// `selector(sig)`: the runtime's key-specific selector over the signal's getter.
fn lower_selector(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [signal] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::selector` takes one signal",
        ));
    };
    Ok(jsast::call(
        dom("createSelector"),
        vec![jsast::index(cx.expr_of(*signal)?, jsast::num(0))],
    ))
}

/// `selector_is(selector, value)`: subscribe the current effect to one exact key.
fn lower_selector_read(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [selector, value] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::selector_is` takes a selector and a value",
        ));
    };
    if !supported_key_type(cx, cx.ty_of(*value)?) {
        return Err(decline(
            OpClass::Template,
            "keyed.unsupported-key-type: a selector value must use an exact keyed-list scalar representation",
        ));
    }
    Ok(jsast::call(
        cx.expr_of(*selector)?,
        vec![cx.expr_of(*value)?],
    ))
}

/// `sget(&sig)`: the read half of the pair, called.
fn lower_signal_read(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [sig] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::sig_get` takes one argument",
        ));
    };
    Ok(jsast::call(
        jsast::index(cx.expr_of(*sig)?, jsast::num(0)),
        Vec::new(),
    ))
}

/// `sset(&sig, value)`: the write half of the pair, called with the new value.
fn lower_signal_write(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [sig, value] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::sig_set` takes two arguments",
        ));
    };
    Ok(jsast::call(
        jsast::index(cx.expr_of(*sig)?, jsast::num(1)),
        vec![cx.expr_of(*value)?],
    ))
}

/// `etv(event)`: `event.target.value || ""`.
fn lower_event_target_value(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [event] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::event_target_value` takes one argument",
        ));
    };
    let value = jsast::member(jsast::member(cx.expr_of(*event)?, "target"), "value");
    let text = jsast::binary(BinOp::Or, value, jsast::string(""));
    if crate::lower::bstr::enabled() {
        Ok(crate::lower::bstr::host_in(text))
    } else {
        Ok(text)
    }
}

/// `epd(event)`: `event.preventDefault()`.
fn lower_event_prevent_default(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [event] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::event_prevent_default` takes one argument",
        ));
    };
    Ok(jsast::method_call(
        cx.expr_of(*event)?,
        "preventDefault",
        Vec::new(),
    ))
}

/// `content(value)`: the value a branch or a row contributes, erased to one type.
fn lower_content(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [value] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::content` takes one argument",
        ));
    };
    let ty = cx.ty_of(*value)?;
    if matches!(value::kind(cx.table, cx.table.peeled(ty)), Some(TypeKind::Tuple(elements)) if elements.is_empty())
    {
        return Ok(jsast::null());
    }
    let expr = cx.expr_of(*value)?;
    if !crate::lower::bstr::enabled()
        || (cx.is_known_ascii(*value) && crate::lower::bstr::is_str_pointer(cx.table, ty))
    {
        return Ok(expr);
    }
    super::host::crossing(cx, ty, expr, crate::lower::bstr::Crossing::Out)
}

/// `text(v)`: the string a hole's value renders as (v1 `abi.rs::codegen_text`).
fn lower_text(cx: &JsCx<'_>, op: &Op, args: &[ValueId]) -> Res<Expr> {
    let [value] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::text` takes one argument",
        ));
    };
    text_value(cx, op, *value)
}

/// The JavaScript string a `Display` value renders as: the shared half of `text` and `pushkeyed`.
fn text_value(cx: &JsCx<'_>, op: &Op, value: ValueId) -> Res<Expr> {
    let ty = cx.ty_of(value)?;
    let operand = cx.expr_of(value)?;
    let elide_bstr_out =
        cx.is_known_ascii(value) && crate::lower::bstr::is_str_pointer(cx.table, ty);
    if let Some(converted) = text_conversion(cx, ty, operand.clone(), elide_bstr_out) {
        return Ok(converted);
    }
    // The general path: `T`'s own `Display`, through `view_abi::display_to_str::<T>`, which the
    // marker declares beside itself and the producer resolved at this call's own type
    // (`HostPayload::helpers`).
    let helper = match &op.kind {
        OpKind::Call {
            host: Some(payload),
            ..
        } => payload.helper(cx.interns, "display_to_str"),
        _ => None,
    };
    let Some(helper) = helper else {
        return Err(decline(
            OpClass::Template,
            "there is no in-place conversion for this `view_abi::text` value, and the call \
             records no `view_abi::display_to_str::<T>` instance to call instead",
        ));
    };
    let name = cx.interns.str(helper.link_name).unwrap_or("").to_owned();
    let call = jsast::call(jsast::id(name), vec![operand]);
    Ok(if crate::lower::bstr::enabled() {
        crate::lower::bstr::host_out(call)
    } else {
        call
    })
}

/// How a `view_abi::text` value of type `ty` becomes a JavaScript string, or `None` for a type
/// that needs its own `Display` (v1 `abi.rs::text_conversion`).
fn text_conversion(cx: &JsCx<'_>, ty: TypeId, value: Expr, elide_bstr_out: bool) -> Option<Expr> {
    let peeled = cx.table.peeled(ty);
    match value::kind(cx.table, peeled)? {
        TypeKind::Str => Some(if crate::lower::bstr::enabled() && !elide_bstr_out {
            crate::lower::bstr::host_out(value)
        } else {
            value
        }),
        TypeKind::Reference { pointee, mutable } => {
            if *mutable && value::is_str(cx.table, *pointee) {
                return None;
            }
            let pointee = *pointee;
            let read = match value::is_slot(cx.table, peeled) {
                true => ptr::slot_element(value),
                false => value,
            };
            text_conversion(cx, pointee, read, elide_bstr_out)
        }
        TypeKind::Char => Some(jsast::call(jsast::id("String.fromCodePoint"), vec![value])),
        TypeKind::Bool | TypeKind::Integer { .. } | TypeKind::Float { .. } => {
            Some(jsast::call(jsast::id("String"), vec![value]))
        }
        _ => None,
    }
}

/// `cond(test, then, els)`: a conditional whose test is hoisted into `_$memo`
/// (v1 `abi.rs::codegen_cond`).
fn lower_cond(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [test, then, els] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::cond` takes three closures",
        ));
    };
    let mut calls = Vec::with_capacity(3);
    let mut effects = Vec::new();
    for closure in [test, then, els] {
        // The call, not the arrow: each closure is called from inside the one accessor this
        // builds, rather than being handed over as a function of its own. Its environment still
        // belongs to that closure's call.
        let (setup, _, call) = super::host::closure_thunk_with(cx, *closure, false, false)?;
        effects.extend(setup);
        calls.push(call);
    }
    let [test, then, els] = <[Expr; 3]>::try_from(calls)
        .map_err(|_| decline(OpClass::Template, "`view_abi::cond` takes three closures"))?;
    let (bind, memo) = temp(
        cx,
        jsast::call(dom("memo"), vec![jsast::arrow(Vec::new(), test)]),
    );
    let dispatch = jsast::arrow(
        Vec::new(),
        jsast::cond(jsast::call(memo, Vec::new()), then, els),
    );
    effects.push(bind);
    Ok(sequence(effects, dispatch))
}

/// Resolves a `view_abi::push` / `push_keyed` list argument to its JS array expression,
/// dereferencing `&Node` when unoptimized MIR keeps the `list()` temporary in a local cell.
fn list_expr(cx: &JsCx<'_>, list: ValueId) -> Res<Expr> {
    if super::value::pointee_of(cx.table, cx.ty_of(list)?).is_some() {
        let place = cx.pointee_place(list)?;
        Ok(cx.read_place(&place.place))
    } else {
        cx.expr_of(list)
    }
}

/// `push(&list, row)`: one row appended to the list the loop is filling.
fn lower_push(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [list, row] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::push` takes a list and a row",
        ));
    };
    Ok(jsast::method_call(
        list_expr(cx, *list)?,
        "push",
        vec![cx.expr_of(*row)?],
    ))
}

/// `pushkeyed(&list, key, row)`: one row appended, matched to the row of the same key from the
/// previous render (v1 `abi.rs::codegen_push_keyed`).
fn lower_push_keyed(cx: &JsCx<'_>, op: &Op, args: &[ValueId]) -> Res<Expr> {
    let [list, key, row] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::push_keyed` takes a list, a key and a row",
        ));
    };
    let key = text_value(cx, op, *key)?;
    let row = cx.expr_of(*row)?;
    // The site only has to tell one loop's row cache from another's. v1 numbered them per codegen
    // unit; a link-time lowering numbers them per item and per origin, which is stable under
    // recompilation and unique for the same reason: a `for` has exactly one `push_keyed` in it.
    let site = jsast::string(format!("{}#{}", cx.summary.key.link_name, op.origin.0));
    let cached = jsast::rt_call("keyed_row", vec![site, key, row]);
    Ok(jsast::method_call(
        list_expr(cx, *list)?,
        "push",
        vec![cached],
    ))
}

/// `micro(f)`: `__rt.microtask(() => f())`.
fn lower_microtask(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [f] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::microtask` takes one closure",
        ));
    };
    let (setup, arrow, _) = super::host::closure_thunk_with(cx, *f, false, false)?;
    Ok(sequence(
        setup.into_iter().collect(),
        jsast::rt_call("microtask", vec![arrow]),
    ))
}

/// `cleanup(f)`: `_$onCleanup(() => f())`.
fn lower_cleanup(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [f] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::on_cleanup` takes one closure",
        ));
    };
    let (setup, arrow, _) = super::host::closure_thunk_with(cx, *f, false, false)?;
    Ok(sequence(
        setup.into_iter().collect(),
        jsast::call(dom("onCleanup"), vec![arrow]),
    ))
}

/// `settled(value, f)`: `__rt.settled(value, (x, ok) => f(x, ok))`.
fn lower_settled(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [value, f] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::on_settled` takes a value and a closure",
        ));
    };
    let value = cx.expr_of(*value)?;
    let (setup, arrow, _) = super::host::closure_thunk(cx, *f)?;
    Ok(sequence(
        setup.into_iter().collect(),
        jsast::rt_call("settled", vec![value, arrow]),
    ))
}

/// `withowner(owner, f)`: `_$runWithOwner(owner, () => f())`.
fn lower_with_owner(cx: &JsCx<'_>, args: &[ValueId]) -> Res<Expr> {
    let [owner, f] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::with_owner` takes an owner and a closure",
        ));
    };
    let owner = cx.expr_of(*owner)?;
    let (setup, arrow, _) = super::host::closure_thunk_with(cx, *f, false, false)?;
    Ok(sequence(
        setup.into_iter().collect(),
        jsast::call(dom("runWithOwner"), vec![owner, arrow]),
    ))
}

/// `hostval(value)`: the host value interpreted as Rust `V`.
fn lower_host_value(cx: &JsCx<'_>, op: &Op, args: &[ValueId]) -> Res<Expr> {
    let [value] = args else {
        return Err(decline(
            OpClass::Template,
            "`view_abi::host_value` takes one argument",
        ));
    };
    let expr = cx.expr_of(*value)?;
    match op.result {
        Some(result) if crate::lower::bstr::enabled() => super::host::crossing(
            cx,
            cx.ty_of(result)?,
            expr,
            crate::lower::bstr::Crossing::In,
        ),
        _ => Ok(expr),
    }
}

/// Resolves a `view-abi` hole marker's `&root` operand back to the `ValueId` produced by
/// `rcgjs.tc.template`, chasing unoptimized-MIR stack cells and reborrows (`BorrowView` /
/// `CellStore`).
fn resolve_instantiation_root(cx: &JsCx<'_>, value: ValueId) -> Option<ValueId> {
    use rcgjs_object::jir::repr::{CellId, Proj, Root};

    let cell_store_source = |cell: CellId| -> Option<ValueId> {
        let mut found = None;
        for block in &cx.func.blocks {
            for op in &block.ops {
                if let OpKind::CellStore { cell: stored, v } = &op.kind {
                    if *stored == cell {
                        if found.is_some() {
                            return None;
                        }
                        found = Some(*v);
                    }
                }
            }
        }
        found
    };

    let mut cur = value;
    for _ in 0..8 {
        if cx.instantiations.borrow().contains_key(&cur) {
            return Some(cur);
        }
        let op = cx.def_of(cur)?;
        cur = match &op.kind {
            OpKind::Copy(src) | OpKind::Move(src) | OpKind::Deref(src) => *src,
            OpKind::BorrowView {
                root: Root::Value(src),
                path,
                ..
            } if path.is_empty() || path.as_slice() == [Proj::Deref] => *src,
            OpKind::BorrowView {
                root: Root::Cell(cell),
                path,
                ..
            } if path.is_empty() || path.as_slice() == [Proj::Deref] => cell_store_source(*cell)?,
            OpKind::CellLoad(cell) => cell_store_source(*cell)?,
            _ => return None,
        };
    }
    cx.instantiations.borrow().contains_key(&cur).then_some(cur)
}

/// The `u32` a constant operand names, for a hole index.
fn constant_u32(cx: &JsCx<'_>, value: ValueId) -> Option<u32> {
    match &cx.def_of(value)?.kind {
        OpKind::Const(ConstTree::Scalar { bits, .. }) => u32::try_from(*bits).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> Vec<Step> {
        parse_path(text).expect("the test paths are well formed")
    }

    #[test]
    fn a_walk_reaches_the_node_the_html_describes() {
        let tree = Tree::parse("<div><span></span><!></div>");
        assert_eq!(tree.kind_at(&path("")), Some(NodeKind::Element));
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Element));
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::Comment));
        assert_eq!(tree.kind_at(&path("cnn")), None);
    }

    #[test]
    fn a_hydratable_pair_is_told_apart_from_an_anchor() {
        let tree = Tree::parse("<div><!$><!/></div>");
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::MarkerOpen));
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::MarkerClose));
    }

    #[test]
    fn an_svg_template_is_rooted_inside_its_wrapper() {
        let template = Template {
            html: "<svg><path></path></svg>".to_owned(),
            hydratable: false,
            is_import_node: false,
            is_svg: true,
            holes: Vec::new(),
            events: Vec::new(),
        };
        assert_eq!(template.tree().kind_at(&path("")), Some(NodeKind::Element));
        assert_eq!(template.tree().kind_at(&path("c")), None);
    }

    #[test]
    fn record_keys_are_upstreams_numerals() {
        assert_eq!(record_key(0), "e");
        assert_eq!(record_key(1), "t");
        assert_eq!(record_key(2), "a");
        assert_eq!(record_key(53), "$");
        assert_eq!(record_key(54), "te");
    }

    #[test]
    fn a_void_element_needs_no_closing_tag() {
        let tree = Tree::parse("<div><br><span></span></div>");
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::Element));
    }
}
