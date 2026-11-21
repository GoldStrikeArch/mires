//! The corpus input (CONTRACT.md, "Input") as Rust types. Names become Rust strings; keys,
//! values and options stay host values, passed to IndexedDB unchanged.

use alloc::string::String;
use alloc::vec::Vec;

use mires_js::JsValue;

use crate::host::{at, field, has, is_array, length};
use crate::idb::Method;

pub struct Step {
    pub op: Op,
    /// A failure of this step (or inside it) is logged and the script goes on after it.
    pub catch: bool,
}

pub enum Op {
    OpenDb(OpenDb),
    Close { conn: String },
    DeleteDb { db: String, blocked: Option<Blocked> },
    Transaction { conn: String, stores: JsValue, mode: JsValue, steps: Vec<Step> },
    Shortcut { conn: String, label: String, method: Method, store: String, index: Option<String>, args: Args },
    Request(Request),
    Batch { requests: Vec<Request>, abort: bool, done: bool },
    Walk(Walk),
    Abort,
    Done,
    Store,
    Wrap(Request),
}

pub struct OpenDb {
    pub db: String,
    pub conn: String,
    pub version: Option<JsValue>,
    pub upgrade: Option<Vec<Upgrade>>,
    pub blocking: Option<Blocking>,
    pub blocked: Option<Blocked>,
}

#[derive(Clone, Copy)]
pub enum Blocking {
    Log,
    Close,
}

pub enum Blocked {
    Log,
    Close(String),
}

#[derive(Clone)]
pub enum Upgrade {
    CreateStore { name: String, options: JsValue },
    DeleteStore { name: String },
    CreateIndex { store: String, name: String, key_path: JsValue, options: JsValue },
    DeleteIndex { store: String, name: String },
    Write { method: Method, store: String, args: Args },
    Abort,
}

/// `store: None` is `tx.store`.
pub struct Target {
    pub store: Option<String>,
    pub index: Option<String>,
}

pub struct Request {
    pub method: Method,
    pub target: Target,
    pub args: Args,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WalkKind {
    Cursor,
    KeyCursor,
    Iterate,
}

pub struct Walk {
    pub kind: WalkKind,
    pub target: Target,
    pub args: Args,
    pub positions: Vec<Vec<Action>>,
    pub rest: Vec<Action>,
}

impl Walk {
    pub fn label(&self) -> &'static str {
        match self.kind {
            WalkKind::Cursor => "openCursor",
            WalkKind::KeyCursor => "openKeyCursor",
            WalkKind::Iterate => "iterate",
        }
    }

    pub fn actions(&self, position: usize) -> &[Action] {
        match self.positions.get(position) {
            Some(actions) => actions,
            None => &self.rest,
        }
    }
}

pub enum Action {
    /// `continue()` when the key is undefined.
    Continue(JsValue),
    Advance(JsValue),
    ContinuePrimaryKey(JsValue, JsValue),
    Update(JsValue),
    Delete,
    Break,
}

/// A call's arguments; an absent one is `undefined`. A query method's first argument is a key
/// or a key range.
#[derive(Clone)]
pub struct Args {
    pub first: Option<Argument>,
    pub second: JsValue,
}

#[derive(Clone)]
pub enum Argument {
    Value(JsValue),
    Query(Query),
}

/// A key range written as an object; open flags stay host values, as IDBKeyRange converts them.
#[derive(Clone)]
pub enum Query {
    Only(JsValue),
    Lower(JsValue, JsValue),
    Upper(JsValue, JsValue),
    Bound(JsValue, JsValue, JsValue, JsValue),
}

pub fn decode(input: JsValue) -> Vec<Step> {
    steps(field(input, "steps"))
}

fn text(value: JsValue) -> String {
    String::from(value.read::<&str>())
}

fn optional(value: JsValue) -> Option<JsValue> {
    (!value.is_undefined()).then_some(value)
}

fn name_or_null(value: JsValue) -> Option<String> {
    (!value.is_null()).then(|| text(value))
}

fn items<T>(array: JsValue, item: impl Fn(JsValue) -> T) -> Vec<T> {
    (0..length(array)).map(|i| item(at(array, i))).collect()
}

fn steps(array: JsValue) -> Vec<Step> {
    items(array, step)
}

fn step(value: JsValue) -> Step {
    let op = text(field(value, "op"));
    let conn = || text(field(value, "conn"));
    let op = match op.as_str() {
        "openDB" => Op::OpenDb(OpenDb {
            db: text(field(value, "db")),
            conn: conn(),
            version: optional(field(value, "version")),
            upgrade: optional(field(value, "upgrade")).map(|ops| items(ops, upgrade)),
            blocking: optional(field(value, "blocking")).map(|kind| match kind.read::<&str>() {
                "close" => Blocking::Close,
                _ => Blocking::Log,
            }),
            blocked: blocked(field(value, "blocked")),
        }),
        "close" => Op::Close { conn: conn() },
        "deleteDB" => Op::DeleteDb { db: text(field(value, "db")), blocked: blocked(field(value, "blocked")) },
        "tx" => Op::Transaction {
            conn: conn(),
            stores: field(value, "stores"),
            mode: field(value, "mode"),
            steps: steps(field(value, "steps")),
        },
        "call" => {
            let label = text(field(value, "method"));
            let method = match label.strip_suffix("FromIndex") {
                Some(read) if method(read).is_write() => panic!("only read methods have FromIndex forms"),
                Some(read) => method(read),
                None => method(&label),
            };
            Op::Shortcut {
                conn: conn(),
                method,
                store: text(field(value, "store")),
                index: name_or_null(field(value, "index")),
                args: args(method, field(value, "args")),
                label,
            }
        }
        "batch" => Op::Batch {
            requests: items(field(value, "requests"), request),
            abort: field(value, "abort").read::<bool>(),
            done: field(value, "done").read::<bool>(),
        },
        "openCursor" => Op::Walk(walk(WalkKind::Cursor, value)),
        "openKeyCursor" => Op::Walk(walk(WalkKind::KeyCursor, value)),
        "iterate" => Op::Walk(walk(WalkKind::Iterate, value)),
        "abort" => Op::Abort,
        "done" => Op::Done,
        "store" => Op::Store,
        "wrap" => {
            let method = method(field(value, "method").read::<&str>());
            Op::Wrap(Request { method, target: target(value), args: args(method, field(value, "args")) })
        }
        _ => Op::Request(request(value)),
    };
    Step { op, catch: field(value, "catch").is_truthy() }
}

fn blocked(value: JsValue) -> Option<Blocked> {
    optional(value).map(|value| if value.is_string() { Blocked::Log } else { Blocked::Close(text(field(value, "close"))) })
}

fn upgrade(value: JsValue) -> Upgrade {
    let name = || text(field(value, "name"));
    let store = || text(field(value, "store"));
    match field(value, "op").read::<&str>() {
        "createStore" => Upgrade::CreateStore { name: name(), options: field(value, "options") },
        "deleteStore" => Upgrade::DeleteStore { name: name() },
        "createIndex" => Upgrade::CreateIndex { store: store(), name: name(), key_path: field(value, "keyPath"), options: field(value, "options") },
        "deleteIndex" => Upgrade::DeleteIndex { store: store(), name: name() },
        "abort" => Upgrade::Abort,
        write => {
            let method = method(write);
            Upgrade::Write { method, store: store(), args: args(method, field(value, "args")) }
        }
    }
}

fn method(name: &str) -> Method {
    match name {
        "get" => Method::Get,
        "getKey" => Method::GetKey,
        "getAll" => Method::GetAll,
        "getAllKeys" => Method::GetAllKeys,
        "count" => Method::Count,
        "put" => Method::Put,
        "add" => Method::Add,
        "delete" => Method::Delete,
        "clear" => Method::Clear,
        _ => panic!("not a request method"),
    }
}

fn target(value: JsValue) -> Target {
    Target { store: name_or_null(field(value, "store")), index: name_or_null(field(value, "index")) }
}

fn request(value: JsValue) -> Request {
    let method = method(field(value, "op").read::<&str>());
    Request { method, target: target(value), args: args(method, field(value, "args")) }
}

fn walk(kind: WalkKind, value: JsValue) -> Walk {
    Walk {
        kind,
        target: target(value),
        args: query_args(field(value, "args")),
        positions: items(field(value, "positions"), |list| items(list, action)),
        rest: items(field(value, "rest"), action),
    }
}

fn action(value: JsValue) -> Action {
    match at(value, 0).read::<&str>() {
        "continue" => Action::Continue(at(value, 1)),
        "advance" => Action::Advance(at(value, 1)),
        "continuePrimaryKey" => Action::ContinuePrimaryKey(at(value, 1), at(value, 2)),
        "update" => Action::Update(at(value, 1)),
        "delete" => Action::Delete,
        _ => Action::Break,
    }
}

/// Put and add take a value and a key; every other method takes a query first.
fn args(method: Method, list: JsValue) -> Args {
    match method {
        Method::Put | Method::Add => Args { first: (length(list) > 0).then(|| Argument::Value(at(list, 0))), second: at(list, 1) },
        _ => query_args(list),
    }
}

fn query_args(list: JsValue) -> Args {
    Args { first: (length(list) > 0).then(|| query(at(list, 0))), second: at(list, 1) }
}

/// A key is passed as it is (null included); an object is a key range.
fn query(value: JsValue) -> Argument {
    if !value.is_object() || is_array(value) {
        return Argument::Value(value);
    }
    let lower = has(value, "lower");
    let upper = has(value, "upper");
    Argument::Query(if has(value, "only") {
        Query::Only(field(value, "only"))
    } else if lower && upper {
        Query::Bound(field(value, "lower"), field(value, "upper"), field(value, "lowerOpen"), field(value, "upperOpen"))
    } else if lower {
        Query::Lower(field(value, "lower"), field(value, "lowerOpen"))
    } else {
        Query::Upper(field(value, "upper"), field(value, "upperOpen"))
    })
}
