//! `view!` templates: the payload `view-abi` hands over, and the dom-expressions calls it becomes.
//!
//! A view compiled for the client reaches the backend as marker calls (see [`crate::abi`]) whose
//! first argument is a `static view_abi::TemplateData`. This module owns the three steps that turn
//! one of those into JavaScript:
//!
//! 1. [`Template::read`] decodes the static with [`crate::constread`], checking [`ABI_VERSION`].
//! 2. [`Template::tree`] rebuilds the node tree the template's HTML describes, which is what says
//!    whether a hole's walk lands on an anchor comment or on the element itself.
//! 3. [`crate::base::FnCx::instantiate_template`] emits the cloner, the walk, and the holes.
//!
//! # Why the HTML is parsed
//!
//! A [`HoleKind::Child`] hole's walk reaches one of two things, and the emitted call differs: the
//! element the value is the sole child of (`_$insert(el, value)`), or an anchor comment before
//! which it goes (`_$insert(parent, value, anchor)`). The two are indistinguishable from the walk
//! alone — `c` is both "the first child element" and "the first child, which is an anchor" — so
//! the node it reaches has to be looked at. [`Tree`] is the smallest parser that answers it, over
//! the closed grammar the emitter writes rather than over HTML in general.
//!
//! # Memo hoisting
//!
//! A conditional's test is hoisted into `_$memo` (CONTRACT-DOM 6.4), which needs the test as a
//! value of its own rather than buried in a closure that also builds the branches. `view_abi::cond`
//! is what hands it over, and [`crate::abi::FnCx::codegen_cond`] is what emits the memo. A `match`,
//! and an `if` whose test binds, still arrive as one closure and get no memo; see that ABI marker
//! for why.

use std::fmt::Write as _;

use rustc_hir::def_id::DefId;
use rustc_middle::ty::TyCtxt;

use crate::cgu::CguCx;
use crate::constread::{self, ConstVal};
use crate::item::{ItemKind, JsItem, JsName, Linkage, SourceOrder};
use crate::jsast::{self, Expr};

/// The `view_abi::ABI_VERSION` this backend decodes. A payload announcing anything else is refused
/// by name rather than read as if it had this shape.
pub(crate) const ABI_VERSION: u32 = 6;

/// `view_abi::NO_EFFECT_GROUP`: the group of a hole no shared effect drives.
const NO_EFFECT_GROUP: u32 = u32::MAX;

/// One step of a hole's walk from the template root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    FirstChild,
    NextSibling,
}

impl Step {
    /// The DOM property this step reads.
    fn property(self) -> &'static str {
        match self {
            Step::FirstChild => "firstChild",
            Step::NextSibling => "nextSibling",
        }
    }
}

/// What a hole holds. Mirrors `view_abi::HoleKind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HoleKind {
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
    ///
    /// The two that do are a dynamic child and a component, and they are anchored identically: the
    /// walk reaches either the element the value is the sole child of, the `<!>` it is inserted
    /// before, or the opening `<!$>` of a hydratable pair. So the walk planner and the target
    /// locator ask this rather than naming [`HoleKind::Child`] twice.
    pub(crate) fn inserts_a_node(self) -> bool {
        matches!(self, HoleKind::Child | HoleKind::Component)
    }

    /// The kind named by a `view_abi::HoleKind` variant.
    fn from_variant(name: &str) -> Option<HoleKind> {
        Some(match name {
            "Child" => HoleKind::Child,
            "Attribute" => HoleKind::Attribute,
            "Property" => HoleKind::Property,
            "DelegatedEvent" => HoleKind::DelegatedEvent,
            "Event" => HoleKind::Event,
            "ClassList" => HoleKind::ClassList,
            "Style" => HoleKind::Style,
            "Spread" => HoleKind::Spread,
            "Component" => HoleKind::Component,
            _ => return None,
        })
    }
}

/// One dynamic position in a template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Hole {
    /// The walk from the template root to this hole's node.
    pub(crate) path: Vec<Step>,
    pub(crate) kind: HoleKind,
    /// The attribute, property or event name. Empty where the kind takes none.
    pub(crate) name: String,
    pub(crate) reactive: bool,
    /// The effect this hole shares, or `None` for one no shared effect drives.
    pub(crate) effect_group: Option<u32>,
}

/// One `view!` template: its HTML skeleton, its holes in source order, and the delegated events it
/// needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Template {
    pub(crate) html: String,
    /// Whether the template claims server-rendered nodes rather than cloning.
    pub(crate) hydratable: bool,
    /// Whether the cloner imports the prototype node rather than cloning it, which is what upgrades
    /// a custom element on insertion. The second argument of `_$template`.
    pub(crate) is_import_node: bool,
    /// Whether the template's root is an SVG-only element. The third argument of `_$template`.
    ///
    /// [`Template::html`] already carries the literal `<svg>` wrapper the flag expects, and every
    /// hole's walk is still rooted at the wrapped element, so [`Template::tree`] descends past the
    /// wrapper to match. See that method.
    pub(crate) is_svg: bool,
    pub(crate) holes: Vec<Hole>,
    /// Delegated event names, deduplicated and sorted by the emitter that wrote them.
    pub(crate) events: Vec<String>,
}

impl Template {
    /// Decodes the `static view_abi::TemplateData` that `def_id` names.
    ///
    /// # Errors
    ///
    /// Returns a message naming the field that could not be read, or the version that was found
    /// where [`ABI_VERSION`] was expected.
    pub(crate) fn read(tcx: TyCtxt<'_>, def_id: DefId) -> Result<Template, String> {
        let data = constread::read_static(tcx, def_id)?;

        let abi = data.field("abi")?.as_u32("abi")?;
        if abi != ABI_VERSION {
            return Err(format!(
                "it announces view-abi version {abi}, and this backend reads version \
                 {ABI_VERSION}: rebuild the crate and the backend against one `view-abi`"
            ));
        }

        let html = data.field("html")?.as_str("html")?.to_owned();
        let hydratable = data.field("hydratable")?.as_bool("hydratable")?;
        let is_import_node = data.field("is_import_node")?.as_bool("is_import_node")?;
        let is_svg = data.field("is_svg")?.as_bool("is_svg")?;

        let mut holes = Vec::new();
        for (index, hole) in data.field("holes")?.as_seq("holes")?.iter().enumerate() {
            holes.push(Hole::read(hole).map_err(|err| format!("in `holes[{index}]`, {err}"))?);
        }

        let mut events = Vec::new();
        for (index, event) in data.field("events")?.as_seq("events")?.iter().enumerate() {
            events.push(event.as_str(&format!("events[{index}]"))?.to_owned());
        }

        Ok(Template { html, hydratable, is_import_node, is_svg, holes, events })
    }

    /// The node tree the template's HTML describes, rooted where a hole's walk starts.
    ///
    /// Under [`Template::is_svg`] that is not the first node of the HTML. The runtime unwraps two
    /// levels for an SVG template (`t.content.firstChild.firstChild`, CONTRACT-DOM 2.2) because the
    /// HTML carries a literal `<svg>` wrapper the template's own root sits inside, so the walk is
    /// rooted at the wrapper's first child. Rooting it at the wrapper instead lands every walk in
    /// the template one level too high, and nothing else would report it.
    pub(crate) fn tree(&self) -> Tree {
        let tree = Tree::parse(&self.html);
        match self.is_svg {
            true => tree.inside_svg_wrapper(),
            false => tree,
        }
    }

    /// The interning key of this template's cloner: the HTML text and the flags it is built with.
    ///
    /// Never a def path. Two crates that build the same template must name one cloner, and a def
    /// path is spelled relative to whoever printed it (see `naming.rs`).
    ///
    /// The flags are part of the key because they are part of the cloner: two templates with one
    /// HTML text and different flags are two different construction routines, and interning them
    /// together would give whichever was emitted second the first one's `importNode` or unwrap
    /// depth.
    ///
    /// A template with neither flag keys on its HTML alone, which is the majority and keeps their
    /// names where they were. The flagged form cannot collide with it: a template's HTML always
    /// opens with `<`, so no unflagged key can spell the flag prefix.
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

    /// The module level `const` holding this template's cloner, interned into `cgu`.
    ///
    /// One `_$template(html)` per distinct HTML text in the whole program, which is what makes the
    /// runtime's per-cloner memoisation per template rather than per render (CONTRACT-DOM 2.1).
    ///
    /// The call takes its three boolean arguments only when one of them is true, which is what the
    /// reference compiler emits (CONTRACT-DOM 2.4). `isMathML` is always false: the emitter knows
    /// its own namespaces and has no MathML lowering.
    ///
    /// `order` is where the `view!` that built this template was written, which is the order the
    /// cloners have to be declared in; see [`SourceOrder`]. The first instantiation to reach a
    /// given HTML text decides it, exactly as first use does in the reference compiler's output.
    pub(crate) fn cloner(&self, cgu: &CguCx<'_>, order: Option<SourceOrder>) -> Expr {
        let name = cgu.namer.synthetic_name("tmpl", &self.cloner_key());
        // Every use offers its position and the earliest wins, so a template several views build is
        // declared where the first of them is written rather than where codegen happened to reach it
        // first.
        if let Some(order) = order.clone() {
            cgu.order_at_most(&name, order);
        }
        cgu.intern(name.clone(), || {
            let mut arguments = vec![jsast::string(self.html.clone())];
            if self.is_import_node || self.is_svg {
                arguments.push(jsast::boolean(self.is_import_node));
                arguments.push(jsast::boolean(self.is_svg));
                arguments.push(jsast::boolean(false));
            }
            let call = jsast::call(dom_import(cgu, "template"), arguments);
            let item = JsItem::new(
                name.clone(),
                ItemKind::Const,
                jsast::const_(name.as_str(), call),
                Vec::new(),
                Linkage::internal(),
                format!("view! template {}", abbreviated(&self.html)),
            );
            match order {
                Some(order) => item.with_order(order),
                None => item,
            }
        });
        jsast::id(name.into_string())
    }
}

impl Hole {
    /// Decodes one `view_abi::Hole`.
    fn read(value: &ConstVal) -> Result<Hole, String> {
        let path = parse_path(value.field("path")?.as_str("path")?)?;
        let variant = value.field("kind")?.variant("kind")?;
        let kind = HoleKind::from_variant(variant)
            .ok_or_else(|| format!("`{variant}` names no `HoleKind`"))?;
        let name = value.field("name")?.as_str("name")?.to_owned();
        let reactive = value.field("reactive")?.as_bool("reactive")?;
        let group = value.field("effect_group")?.as_u32("effect_group")?;
        let effect_group = (group != NO_EFFECT_GROUP).then_some(group);
        Ok(Hole { path, kind, name, reactive, effect_group })
    }

    /// The walk to the element a [`HoleKind::Child`] hole inserts into, given that its own walk
    /// reaches an anchor.
    ///
    /// An anchor is a child position of its element, so its walk is the element's walk followed by
    /// one `firstChild` and a run of `nextSibling`s. Dropping that tail is the element's walk.
    pub(crate) fn anchor_parent(&self) -> Option<&[Step]> {
        let last = self.path.iter().rposition(|step| *step == Step::FirstChild)?;
        Some(&self.path[..last])
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

/// A string as a one line header comment: escaped, and cut short if it is long.
fn abbreviated(text: &str) -> String {
    const LIMIT: usize = 48;
    let mut end = LIMIT.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    match end == text.len() {
        true => format!("{text:?}"),
        false => format!("{:?}...", &text[..end]),
    }
}

/// The local binding a dom-expressions runtime name is imported under.
///
/// One item per name, whose own name *is* the binding it introduces, so the reachability pass
/// keeps exactly the imports something mentions and two crates importing the same name collapse to
/// one item. The `_$` prefix is the convention the reference compiler's output uses, which keeps a
/// diff against it about the code rather than about spelling.
pub(crate) fn dom_import(cgu: &CguCx<'_>, name: &str) -> Expr {
    let local = format!("_${name}");
    let js_name = JsName::new(local.clone());
    let source = crate::opts::get().dom_module.clone();
    cgu.intern(js_name.clone(), || {
        JsItem::new(
            js_name.clone(),
            ItemKind::Import,
            jsast::import_named(vec![(name.to_owned(), local.clone())], source.clone()),
            Vec::new(),
            Linkage { fixed_name: Some(local.clone()), root: false, exported: false },
            source.clone(),
        )
    });
    jsast::id(local)
}

// -------------------------------------------------------------------------------------------
// The template's node tree
// -------------------------------------------------------------------------------------------

/// What kind of node a walk reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeKind {
    Element,
    Text,
    /// A `<!>` anchor, or any comment that is not one of the two markers below.
    Comment,
    /// The `<!$>` opening a hydratable marker pair.
    MarkerOpen,
    /// The `<!/>` closing one.
    MarkerClose,
}

impl NodeKind {
    /// Whether the node is a comment of any of the three forms, which is what makes a child hole
    /// *anchored*: the value goes before the comment rather than into the node the walk reached.
    pub(crate) fn is_comment(self) -> bool {
        matches!(self, NodeKind::Comment | NodeKind::MarkerOpen | NodeKind::MarkerClose)
    }
}

/// One node of a parsed template.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Node {
    kind: NodeKind,
    children: Vec<Node>,
}

/// The node tree of one template's HTML.
///
/// The grammar is the closed one the `view!` emitter writes: elements with attributes, text runs,
/// and the three comment forms. It is not an HTML parser and does not try to be one; a construct
/// outside that grammar makes a walk unresolvable, which is reported at the call site rather than
/// guessed at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Tree {
    roots: Vec<Node>,
}

/// The elements an HTML parser closes without a closing tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

impl Tree {
    /// Parses `html` into the tree a browser's fragment parser would build from it.
    fn parse(html: &str) -> Tree {
        let bytes = html.as_bytes();
        let mut at = 0;
        // A stack of the open elements' child lists; the bottom one is the roots.
        let mut stack: Vec<(String, Vec<Node>)> = vec![(String::new(), Vec::new())];

        while at < bytes.len() {
            if bytes[at] != b'<' {
                let end = match html[at..].find('<') {
                    Some(offset) => at + offset,
                    None => bytes.len(),
                };
                push(&mut stack, Node { kind: NodeKind::Text, children: Vec::new() });
                at = end;
                continue;
            }

            // A comment: `<!>`, `<!$>`, `<!/>`, or a full `<!-- .. -->`. The two markers are told
            // apart from an anchor because a hydratable walk re-bases on the closing one; see
            // [`Walk::plan`].
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
                push(&mut stack, Node { kind, children: Vec::new() });
                at = end.unwrap_or(bytes.len());
                continue;
            }

            // A closing tag: pop back to the matching open element, if there is one.
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

            // An opening tag. Attribute values may hold `>`, so the scan honours quoting.
            let name = tag_name(&html[at + 1..]);
            let end = tag_end(html, at);
            let self_closing = html[at..end].trim_end_matches('>').trim_end().ends_with('/');
            if VOID_ELEMENTS.contains(&name.as_str()) || self_closing {
                push(&mut stack, Node { kind: NodeKind::Element, children: Vec::new() });
            } else {
                stack.push((name, Vec::new()));
            }
            at = end;
        }

        while stack.len() > 1 {
            close(&mut stack);
        }
        Tree { roots: stack.pop().map(|(_, roots)| roots).unwrap_or_default() }
    }

    /// The same tree with the synthetic `<svg>` wrapper stripped: the wrapper's children become the
    /// roots.
    ///
    /// This is the parse-side half of the pair CONTRACT-DOM 2.2 describes. The runtime unwraps one
    /// extra level for an SVG template, so the node a walk starts from is the wrapper's first child
    /// and not the wrapper. A tree left rooted at the wrapper answers every `kind_at` one level too
    /// high, which turns a sole-child insert into an anchored one and back, with no diagnostic.
    fn inside_svg_wrapper(self) -> Tree {
        match self.roots.into_iter().next() {
            Some(wrapper) => Tree { roots: wrapper.children },
            None => Tree { roots: Vec::new() },
        }
    }

    /// The kind of the node `path` reaches from the template root, if it reaches one.
    ///
    /// The root of the walk is the template's own root node, which is the first of `roots`: a
    /// template with several roots is instantiated one root at a time, and each carries its own
    /// walk.
    pub(crate) fn kind_at(&self, path: &[Step]) -> Option<NodeKind> {
        self.node_at(path).map(|node| node.kind)
    }

    fn node_at(&self, path: &[Step]) -> Option<&Node> {
        let mut node = self.roots.first()?;
        // `siblings` is the list `node` lives in, which is what a `nextSibling` step moves along.
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

/// Appends a node to the innermost open element.
fn push(stack: &mut [(String, Vec<Node>)], node: Node) {
    if let Some((_, children)) = stack.last_mut() {
        children.push(node);
    }
}

/// Closes the innermost open element, appending it to its parent.
fn close(stack: &mut Vec<(String, Vec<Node>)>) {
    let Some((_, children)) = stack.pop() else { return };
    push(stack, Node { kind: NodeKind::Element, children });
}

/// The tag name at the start of `rest`, lowercased.
fn tag_name(rest: &str) -> String {
    rest.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == ':')
        .flat_map(char::to_lowercase)
        .collect()
}

/// The byte just past the `>` closing the tag that starts at `at`.
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

// -------------------------------------------------------------------------------------------
// The walk
// -------------------------------------------------------------------------------------------

/// The walks one instantiation needs, lowered to temporaries that share their prefixes.
///
/// Every prefix of every needed walk gets a name, which is both what shares the work between two
/// holes under one element and what CONTRACT-DOM 3.2 describes: only nodes on the path to a
/// dynamic position are named at all.
///
/// # Re-basing a hydratable walk
///
/// A template's paths describe the *client* tree, where a dynamic child is an empty `<!$><!/>`
/// pair. The nodes a hydratable walk moves over are the *server's*, and there the pair has the
/// rendered value between its two markers. So `<!$>.nextSibling` is the first node of that value
/// rather than the `<!/>`, and every step taken after the pair would land somewhere the client
/// tree does not describe.
///
/// `_$getNextMarker` is what bridges the two: given the node after the `<!$>` it scans forward to
/// the matching `<!/>` and returns it together with the nodes it passed. So the closing marker is
/// named by that call rather than by a `nextSibling`, and the walk continues from there. This is
/// the shape the reference compiler emits, and the reason the call belongs to the walk rather than
/// to the hole that also uses its result.
pub(crate) struct Walk {
    /// `(path, expression)` for every named node, shortest path first, root included.
    nodes: Vec<(Vec<Step>, Expr)>,
    /// `(path of the `<!$>`, the `[closing marker, nodes between]` pair naming it)`.
    pairs: Vec<(Vec<Step>, Expr)>,
}

impl Walk {
    /// Plans the walks reaching every path in `paths`, rooted at `root`.
    ///
    /// `bind` is asked for a name for each step past the root, in the order the statements must
    /// run; it is [`crate::base::FnCx::temp`] at a call site and a plain counter in a test.
    /// `marker_pair` is handed the node after a `<!$>` and answers with the name of the
    /// `_$getNextMarker` pair for it; a walk over a template with no marker pair never calls it.
    ///
    /// `tree` is the template's own node tree, which is what says where those pairs are.
    pub(crate) fn plan(
        root: Expr,
        tree: &Tree,
        paths: &[&[Step]],
        mut bind: impl FnMut(Expr) -> Expr,
        mut marker_pair: impl FnMut(Expr) -> Expr,
    ) -> Walk {
        // Every prefix, so that a walk is one step past a node that already has a name.
        let mut wanted: Vec<Vec<Step>> = Vec::new();
        for path in paths {
            for length in 1..=path.len() {
                let prefix = path[..length].to_vec();
                if !wanted.contains(&prefix) {
                    wanted.push(prefix);
                }
            }
        }
        // Shortest first, so a prefix is always named before what extends it. Ties keep discovery
        // order, which is the order the holes are written in.
        wanted.sort_by_key(Vec::len);

        let mut nodes: Vec<(Vec<Step>, Expr)> = vec![(Vec::new(), root)];
        let mut pairs: Vec<(Vec<Step>, Expr)> = Vec::new();
        for path in wanted {
            let (parent, step) = path.split_at(path.len() - 1);
            let base = nodes
                .iter()
                .find(|(known, _)| known == parent)
                .map(|(_, expr)| expr.clone())
                .expect("a prefix is planned before what extends it");
            // The closing marker of a pair is what `_$getNextMarker` returns, not a sibling of the
            // opening one: on the server there is rendered content in between.
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
    pub(crate) fn at(&self, path: &[Step]) -> Option<Expr> {
        self.nodes
            .iter()
            .find(|(known, _)| known.as_slice() == path)
            .map(|(_, expr)| expr.clone())
    }

    /// The `_$getNextMarker` pair of the `<!$>` at `path`: `[closing marker, nodes between]`.
    ///
    /// `_$insert` takes both, which is what lets a hydrating insert adopt the nodes the server
    /// wrote instead of replacing them.
    pub(crate) fn marker_pair(&self, path: &[Step]) -> Option<Expr> {
        self.pairs
            .iter()
            .find(|(known, _)| known.as_slice() == path)
            .map(|(_, expr)| expr.clone())
    }

    /// How many nodes the walk names, the root included.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.nodes.len()
    }
}

/// The `delegateEvents` names of a set of templates, deduplicated and ordered.
pub(crate) fn delegated_events<'a>(templates: impl Iterator<Item = &'a Template>) -> Vec<String> {
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

/// Renders a walk as the property chain it reads, for a diagnostic.
pub(crate) fn render_path(path: &[Step]) -> String {
    let mut out = String::from("root");
    for step in path {
        let _ = write!(out, ".{}", step.property());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(text: &str) -> Vec<Step> {
        parse_path(text).expect("the test paths are well formed")
    }

    fn hole(path_text: &str, kind: HoleKind) -> Hole {
        Hole {
            path: path(path_text),
            kind,
            name: String::new(),
            reactive: false,
            effect_group: None,
        }
    }

    fn template(html: &str, holes: Vec<Hole>, events: &[&str]) -> Template {
        Template {
            html: html.to_owned(),
            hydratable: false,
            is_import_node: false,
            is_svg: false,
            holes,
            events: events.iter().map(|event| (*event).to_owned()).collect(),
        }
    }

    // ------------------------------------------------------------------ paths

    #[test]
    fn a_path_is_a_run_of_walk_steps() {
        assert_eq!(path(""), []);
        assert_eq!(path("c"), [Step::FirstChild]);
        assert_eq!(path("cnn"), [Step::FirstChild, Step::NextSibling, Step::NextSibling]);
        assert_eq!(
            parse_path("cx").unwrap_err(),
            "`x` is not a walk step (`c` or `n`)"
        );
    }

    #[test]
    fn an_anchors_parent_is_its_walk_without_the_last_descent() {
        // `<div><span></span><!><span></span>`: the anchor is the root's second child, and the
        // element it is inserted into is the root itself.
        assert_eq!(hole("cn", HoleKind::Child).anchor_parent(), Some(&[][..]));
        // A nested element's anchor: `root.firstChild` is the element, and the anchor is two
        // children into it.
        assert_eq!(
            hole("ccn", HoleKind::Child).anchor_parent(),
            Some(&[Step::FirstChild][..])
        );
        // A walk that never descends reaches a sibling of the root, which no element encloses.
        assert_eq!(hole("n", HoleKind::Child).anchor_parent(), None);
    }

    // ------------------------------------------------------------------- tree

    #[test]
    fn the_tree_tells_an_anchor_apart_from_an_element() {
        // The sole-child case: the walk reaches the element the value goes into.
        let tree = Tree::parse("<div><p></p></div>");
        assert_eq!(tree.kind_at(&path("")), Some(NodeKind::Element));
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Element));

        // The anchored case: the same walk, and it reaches a comment instead.
        let tree = Tree::parse("<div><!></div>");
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Comment));

        // Text is neither, and a walk off the end of the tree reaches nothing.
        let tree = Tree::parse("<div>Hello <!></div>");
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Text));
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::Comment));
        assert_eq!(tree.kind_at(&path("cnn")), None);
    }

    #[test]
    fn the_tree_follows_the_shapes_the_emitter_writes() {
        // A hydratable marker pair takes two child positions, and the two markers are told
        // apart: only the closing one is reached through `_$getNextMarker`.
        let tree = Tree::parse("<span>a<!$><!/>b<!$><!/></span>");
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Text));
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::MarkerOpen));
        assert_eq!(tree.kind_at(&path("cnn")), Some(NodeKind::MarkerClose));
        assert_eq!(tree.kind_at(&path("cnnn")), Some(NodeKind::Text));
        assert_eq!(tree.kind_at(&path("cnnnn")), Some(NodeKind::MarkerOpen));
        // All three comment forms anchor a child hole, which is the question the emitter asks.
        assert!(NodeKind::Comment.is_comment());
        assert!(NodeKind::MarkerOpen.is_comment());
        assert!(NodeKind::MarkerClose.is_comment());
        assert!(!NodeKind::Element.is_comment());
        assert!(!NodeKind::Text.is_comment());

        // A void element takes one position and has no children.
        let tree = Tree::parse(r#"<div id="main"><input type="text"><span></span></div>"#);
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Element));
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::Element));
        assert_eq!(tree.kind_at(&path("cc")), None);

        // An unclosed tag is closed by the end of the fragment, as the parser would.
        let tree = Tree::parse("<div>Hello ");
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Text));

        // A `>` inside an attribute value does not end the tag.
        let tree = Tree::parse(r#"<div title="a > b"><span></span></div>"#);
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Element));
    }

    #[test]
    fn adjacent_text_is_one_node() {
        // Two literal runs with nothing between them merge, so `nextSibling` skips past both.
        let tree = Tree::parse("<p>ab<!></p>");
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Text));
        assert_eq!(tree.kind_at(&path("cn")), Some(NodeKind::Comment));
    }

    // ------------------------------------------------------------------- walk

    /// Plans a walk over `html` with numbered stand-ins for the temporaries a call site would
    /// allocate. A `_$getNextMarker` pair is spelled as the call, so the statements show which
    /// node it was handed.
    fn plan_over(html: &str, paths: &[&str]) -> (Walk, Vec<String>) {
        let tree = Tree::parse(html);
        let owned: Vec<Vec<Step>> = paths.iter().map(|text| path(text)).collect();
        let borrowed: Vec<&[Step]> = owned.iter().map(Vec::as_slice).collect();
        let bound = std::cell::RefCell::new(Vec::new());
        let name = |expr: Expr, wrap: fn(String) -> String| {
            let mut bound = bound.borrow_mut();
            let name = format!("$t{}", bound.len());
            bound.push(format!("{name} = {}", wrap(jsast::expr_to_string(&expr))));
            jsast::id(name)
        };
        let walk = Walk::plan(
            jsast::id("root"),
            &tree,
            &borrowed,
            |expr| name(expr, |text| text),
            |expr| name(expr, |text| format!("getNextMarker({text})")),
        );
        (walk, bound.into_inner())
    }

    /// Plans a walk over a tree with no marker pair in it, where only the paths matter.
    fn plan(paths: &[&str]) -> (Walk, Vec<String>) {
        plan_over("<div><span></span><!><!></div>", paths)
    }

    #[test]
    fn a_walk_names_every_prefix_once() {
        let (walk, bound) = plan(&["cn", "cnn"]);
        assert_eq!(
            bound,
            [
                "$t0 = root.firstChild",
                "$t1 = $t0.nextSibling",
                "$t2 = $t1.nextSibling",
            ]
        );
        // The root, plus the three named steps.
        assert_eq!(walk.len(), 4);
        assert_eq!(jsast::expr_to_string(&walk.at(&path("")).unwrap()), "root");
        assert_eq!(jsast::expr_to_string(&walk.at(&path("cnn")).unwrap()), "$t2");
    }

    #[test]
    fn two_holes_under_one_element_share_their_prefix() {
        // `<div><span></span><!><!></div>`: both anchors walk through the same first child.
        let (walk, bound) = plan(&["cn", "cnn"]);
        assert_eq!(bound.len(), 3, "a shared prefix is walked once");
        assert_eq!(jsast::expr_to_string(&walk.at(&path("c")).unwrap()), "$t0");

        // Two separate branches share only the root.
        let (_, bound) = plan_over("<div><p><span></span></p></div>", &["c", "cc"]);
        assert_eq!(bound, ["$t0 = root.firstChild", "$t1 = $t0.firstChild"]);
    }

    #[test]
    fn a_marker_pair_is_closed_by_the_node_after_the_opening_marker() {
        // `<span>Hello <!$><!/>`: the hole's own walk is `cn`, the `<!$>`. Its `<!/>` is what
        // `_$getNextMarker` returns, and what it is handed is the node *after* the `<!$>` -- on the
        // server, the first node of the rendered value.
        let (walk, bound) = plan_over("<span>Hello <!$><!/></span>", &["cn", "cnn"]);
        assert_eq!(
            bound,
            [
                "$t0 = root.firstChild",
                "$t1 = $t0.nextSibling",
                "$t2 = getNextMarker($t1.nextSibling)",
            ]
        );
        // The pair belongs to the opening marker, and the closing one is its first element.
        assert_eq!(jsast::expr_to_string(&walk.marker_pair(&path("cn")).unwrap()), "$t2");
        assert_eq!(jsast::expr_to_string(&walk.at(&path("cnn")).unwrap()), "$t2[0]");
        assert!(walk.marker_pair(&path("c")).is_none());
    }

    #[test]
    fn a_walk_past_a_marker_pair_re_bases_on_its_closing_marker() {
        // `<span><!$><!/> John`: the text after the pair is not two siblings past the `<!$>` on the
        // server, because the rendered value sits between the markers. It is one sibling past the
        // `<!/>` the pair names.
        let (walk, bound) = plan_over("<span><!$><!/> John</span>", &["c", "cnn"]);
        assert_eq!(
            bound,
            [
                "$t0 = root.firstChild",
                "$t1 = getNextMarker($t0.nextSibling)",
                "$t2 = $t1[0].nextSibling",
            ]
        );
        assert_eq!(jsast::expr_to_string(&walk.at(&path("cnn")).unwrap()), "$t2");

        // Two pairs in one child list: the second `<!$>` is reached from the first `<!/>`, and its
        // own pair from there again. This is the case a raw walk gets wrong twice over.
        // Both pairs are asked for, exactly as `abi.rs` asks for the `<!/>` of every hydratable
        // child hole.
        let (walk, bound) =
            plan_over("<span><!$><!/> <!$><!/></span>", &["cn", "cnn", "cnnn", "cnnnn"]);
        assert_eq!(
            bound,
            [
                "$t0 = root.firstChild",
                "$t1 = getNextMarker($t0.nextSibling)",
                "$t2 = $t1[0].nextSibling",
                "$t3 = $t2.nextSibling",
                "$t4 = getNextMarker($t3.nextSibling)",
            ]
        );
        assert_eq!(jsast::expr_to_string(&walk.marker_pair(&path("cnnn")).unwrap()), "$t4");
    }

    #[test]
    fn a_walk_of_no_paths_is_the_root_alone() {
        let (walk, bound) = plan(&[]);
        assert!(bound.is_empty(), "a template with no holes walks nowhere");
        assert_eq!(walk.len(), 1);
        assert_eq!(jsast::expr_to_string(&walk.at(&path("")).unwrap()), "root");
        assert!(walk.at(&path("c")).is_none());
    }

    // ----------------------------------------------------------------- events

    #[test]
    fn delegated_events_are_unioned_deduplicated_and_sorted() {
        let first = template("<button>", vec![], &["click"]);
        let second = template("<input>", vec![], &["input", "click"]);
        assert_eq!(delegated_events([&first, &second].into_iter()), ["click", "input"]);
        // A payload that repeats a name registers one listener, not two.
        let repeated = template("<button>", vec![], &["click", "click"]);
        assert_eq!(delegated_events([&repeated].into_iter()), ["click"]);
        assert!(delegated_events([&template("<div>", vec![], &[])].into_iter()).is_empty());
    }

    #[test]
    fn a_cloner_is_keyed_by_its_html_and_its_construction_flags() {
        let plain = template("<div>Hello ", vec![], &[]);
        let same_html_hydratable = Template { hydratable: true, ..plain.clone() };
        assert_eq!(plain.cloner_key(), same_html_hydratable.cloner_key());
        assert_ne!(plain.cloner_key(), template("<div>Hi ", vec![], &[]).cloner_key());
        // The key describes the value, so it never mentions where the template came from, and an
        // unflagged template keys on its HTML alone.
        assert_eq!(plain.cloner_key(), "tmpl\u{1}<div>Hello ");

        // The flags are part of the cloner, not of the text: two templates with one HTML and
        // different flags are two construction routines and must not intern together.
        let imported = Template { is_import_node: true, ..plain.clone() };
        let svg = Template { is_svg: true, ..plain.clone() };
        assert_ne!(plain.cloner_key(), imported.cloner_key());
        assert_ne!(plain.cloner_key(), svg.cloner_key());
        assert_ne!(imported.cloner_key(), svg.cloner_key());
    }

    #[test]
    fn an_svg_walk_starts_inside_the_wrapper() {
        // The HTML the payload carries for corpus 09's `rootRect`, wrapper and all. Rooted at the
        // wrapper, the empty walk would reach the `<svg>`; rooted correctly it reaches the `<rect>`
        // the template is actually of.
        let wrapped = Template {
            is_svg: true,
            ..template(r#"<svg><rect x="50"><!></rect></svg>"#, vec![], &[])
        };
        let tree = wrapped.tree();
        assert_eq!(tree.kind_at(&path("")), Some(NodeKind::Element));
        assert_eq!(tree.kind_at(&path("c")), Some(NodeKind::Comment));
        assert_eq!(tree.kind_at(&path("cc")), None);

        // Without the flag the same text is read as the wrapper being the root, which is what the
        // pair exists to keep from happening by halves.
        let unwrapped = Template { is_svg: false, ..wrapped.clone() };
        assert_eq!(unwrapped.tree().kind_at(&path("c")), Some(NodeKind::Element));
    }

    #[test]
    fn a_path_renders_as_the_properties_it_reads() {
        assert_eq!(render_path(&path("")), "root");
        assert_eq!(render_path(&path("cn")), "root.firstChild.nextSibling");
    }
}
