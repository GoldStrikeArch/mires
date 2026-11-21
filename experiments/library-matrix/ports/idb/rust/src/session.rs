//! Plays a script through the idb port as `reference.mjs` drives the npm package, then dumps the
//! databases it left and deletes every database it touched, also after a failure.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use mires_js::{JsArray, JsValue};

use crate::host::{self, list};
use crate::idb::{self, settle, Cursor, Database, DomError, Index, Method, ObjectStore, OpenCallbacks, Request, Source, Transaction, Wrapped};
use crate::script::{self, Action, Argument, Args, Blocked, Blocking, Op, OpenDb, Query, Step, Target, Upgrade, Walk, WalkKind};

fn s(text: &str) -> JsValue {
    JsValue::erase(text)
}

fn version(value: Option<f64>) -> JsValue {
    value.map_or(JsValue::null(), JsValue::erase)
}

/// A failure that the step where it happened has already logged.
struct Failed;

/// The output log. It is a host array, so the open callbacks can append to it too.
#[derive(Clone, Copy)]
struct Log(JsArray<JsValue>);

impl Log {
    fn push(self, entry: &[JsValue]) {
        let mut entries = self.0;
        entries.push(list(entry));
    }

    fn error(self, label: &str, error: DomError) {
        self.push(&[s(label), s("error"), error.name()]);
    }

    /// `[label, "ok"]`, `[label, "ok", value]` or `[label, "error", name]`.
    fn outcome(self, label: &str, outcome: Result<Option<JsValue>, DomError>) -> Result<(), Failed> {
        match outcome {
            Ok(Some(value)) => self.push(&[s(label), s("ok"), value]),
            Ok(None) => self.push(&[s(label), s("ok")]),
            Err(error) => {
                self.error(label, error);
                return Err(Failed);
            }
        }
        Ok(())
    }

    /// Logs only a failure.
    fn check<T>(self, label: &str, result: Result<T, DomError>) -> Result<T, Failed> {
        result.map_err(|error| {
            self.error(label, error);
            Failed
        })
    }
}

/// An object store or one of its indexes.
#[derive(Clone, Copy)]
enum Endpoint {
    Store(ObjectStore),
    Index(Index),
}

impl Wrapped for Endpoint {
    fn raw(&self) -> JsValue {
        match self {
            Endpoint::Store(store) => store.raw(),
            Endpoint::Index(index) => index.raw(),
        }
    }
}

impl Source for Endpoint {}

fn endpoint(tx: Transaction, target: &Target) -> Result<Endpoint, DomError> {
    let store = match &target.store {
        Some(name) => tx.object_store(name)?,
        None => tx.store()?.expect("`store: null` is used only in one-store transactions"),
    };
    Ok(match &target.index {
        Some(name) => Endpoint::Index(store.index(name)?),
        None => Endpoint::Store(store),
    })
}

fn key_range(query: &Query) -> Result<JsValue, DomError> {
    match *query {
        Query::Only(key) => host::only(key),
        Query::Lower(lower, open) => host::lower_bound(lower, open),
        Query::Upper(upper, open) => host::upper_bound(upper, open),
        Query::Bound(lower, upper, lower_open, upper_open) => host::bound(lower, upper, lower_open, upper_open),
    }
    .map_err(DomError)
}

fn arguments(args: &Args) -> Result<(JsValue, JsValue), DomError> {
    let first = match &args.first {
        None => JsValue::undefined(),
        Some(Argument::Value(value)) => *value,
        Some(Argument::Query(query)) => key_range(query)?,
    };
    Ok((first, args.second))
}

fn request(tx: Transaction, method: Method, target: &Target, args: &Args) -> Result<Request<Option<JsValue>>, DomError> {
    let source = endpoint(tx, target)?;
    let (first, second) = arguments(args)?;
    source.request(method, first, second)
}

/// `wrap(unwrap(source)[method](...))`: a raw IndexedDB request, promisified.
fn raw_request(tx: Transaction, step: &script::Request) -> Result<Request<Option<JsValue>>, DomError> {
    let raw = idb::unwrap(&endpoint(tx, &step.target)?);
    let (first, second) = arguments(&step.args)?;
    Ok(idb::wrap(step.method.call(raw, first, second).map_err(DomError)?))
}

fn remember(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|known| known == name) {
        names.push(String::from(name));
    }
}

struct Session {
    prefix: String,
    log: Log,
    connections: Vec<(String, Database)>,
    opened: Vec<Database>,
    attempted: Vec<String>,
    live: Vec<String>,
}

pub async fn run(input: JsValue) -> Result<JsValue, JsValue> {
    let script = script::decode(input);
    let mut prefix = String::from("mires-matrix-idb-");
    prefix.push_str(host::random_uuid().as_value().read::<&str>());
    prefix.push('-');
    let mut session = Session {
        prefix,
        log: Log(JsArray::new()),
        connections: Vec::new(),
        opened: Vec::new(),
        attempted: Vec::new(),
        live: Vec::new(),
    };
    // A failure ends the script where it happened; that step logged it.
    let _ = session.steps(&script, None).await;
    let output = session.output().await;
    // As the reference's `finally`: a cleanup failure replaces the result.
    session.cleanup().await.map_err(|error| error.0)?;
    output.map_err(|error| error.0)
}

impl Session {
    fn database(&self, logical: &str) -> String {
        let mut name = self.prefix.clone();
        name.push_str(logical);
        name
    }

    fn connection(&self, conn: &str) -> Database {
        let (_, db) = self.connections.iter().find(|(name, _)| name == conn).expect("a step names a connection opened earlier");
        *db
    }

    fn steps<'a>(&'a mut self, steps: &'a [Step], tx: Option<Transaction>) -> Pin<Box<dyn Future<Output = Result<(), Failed>> + 'a>> {
        Box::pin(async move {
            for step in steps {
                if self.step(&step.op, tx).await.is_err() && !step.catch {
                    return Err(Failed);
                }
            }
            Ok(())
        })
    }

    async fn step(&mut self, op: &Op, tx: Option<Transaction>) -> Result<(), Failed> {
        let log = self.log;
        let tx = || tx.expect("a transaction step runs inside a transaction");
        match op {
            Op::OpenDb(step) => self.open(step).await,
            Op::Close { conn } => {
                self.connection(conn).close();
                log.outcome("close", Ok(None))
            }
            Op::DeleteDb { db, blocked } => self.delete(db, blocked.as_ref()).await,
            Op::Transaction { conn, stores, mode, steps } => {
                let tx = log.check("tx", self.connection(conn).transaction(*stores, *mode))?;
                self.steps(steps, Some(tx)).await
            }
            Op::Shortcut { conn, label, method, store, index, args } => {
                let outcome = match arguments(args) {
                    Ok((first, second)) => self.connection(conn).shortcut(*method, store, index.as_deref(), first, second).await,
                    Err(error) => Err(error),
                };
                log.outcome(label, outcome)
            }
            Op::Request(step) => log.outcome(step.method.name(), settle(request(tx(), step.method, &step.target, &step.args)).await),
            Op::Batch { requests, abort, done } => {
                batch(log, tx(), requests, *abort, *done).await;
                Ok(())
            }
            Op::Walk(walk) if walk.kind == WalkKind::Iterate => iterate(log, tx(), walk).await,
            Op::Walk(walk) => cursor_walk(log, tx(), walk).await,
            Op::Abort => log.outcome("abort", tx().abort().map(|()| None)),
            Op::Done => log.outcome("done", tx().done().await.map(|()| None)),
            Op::Store => log.outcome("store", tx().store().map(|store| store.map(|store| store.name().as_value()))),
            Op::Wrap(step) => log.outcome("wrap", settle(raw_request(tx(), step)).await),
        }
    }

    async fn open(&mut self, step: &OpenDb) -> Result<(), Failed> {
        remember(&mut self.attempted, &step.db);
        let log = self.log;
        let closing = match &step.blocked {
            Some(Blocked::Close(conn)) => Some(self.connection(conn)),
            _ => None,
        };
        let mut callbacks = OpenCallbacks::default();
        if let Some(ops) = &step.upgrade {
            let ops = ops.clone();
            callbacks.upgrade = Some(Box::new(move |db, old, new, tx| {
                log.push(&[s("upgrade"), JsValue::erase(old), version(new)]);
                for op in &ops {
                    upgrade(log, op, db, tx);
                }
            }));
        }
        if step.blocked.is_some() {
            callbacks.blocked = Some(Box::new(move |current, blocked, _| {
                log.push(&[s("blocked"), JsValue::erase(current), version(blocked)]);
                if let Some(db) = closing {
                    db.close();
                }
            }));
        }
        if let Some(blocking) = step.blocking {
            let conn = step.conn.clone();
            callbacks.blocking = Some(Box::new(move |current, blocked, event| {
                log.push(&[s("blocking"), s(&conn), JsValue::erase(current), version(blocked)]);
                if let Blocking::Close = blocking {
                    host::close(host::event_target(event));
                }
            }));
        }
        let name = self.database(&step.db);
        let opened = match idb::open_db(&name, step.version, callbacks) {
            Ok(open) => open.await,
            Err(error) => Err(error),
        };
        let db = log.check("openDB", opened)?;
        log.push(&[s("openDB"), s("ok"), list(&[JsValue::erase(db.version()), db.object_store_names().as_value()])]);
        match self.connections.iter_mut().find(|(conn, _)| *conn == step.conn) {
            Some(entry) => entry.1 = db,
            None => self.connections.push((step.conn.clone(), db)),
        }
        self.opened.push(db);
        remember(&mut self.live, &step.db);
        Ok(())
    }

    async fn delete(&mut self, db: &str, blocked: Option<&Blocked>) -> Result<(), Failed> {
        remember(&mut self.attempted, db);
        let log = self.log;
        let closing = match blocked {
            Some(Blocked::Close(conn)) => Some(self.connection(conn)),
            _ => None,
        };
        let blocked = blocked.map(|_| -> idb::DeleteBlocked {
            Box::new(move |current, _| {
                log.push(&[s("blocked"), JsValue::erase(current)]);
                if let Some(db) = closing {
                    db.close();
                }
            })
        });
        let name = self.database(db);
        log.outcome("deleteDB", settle(idb::delete_db(&name, blocked)).await.map(|()| None))?;
        self.live.retain(|live| live != db);
        Ok(())
    }

    async fn output(&self) -> Result<JsValue, DomError> {
        let mut databases = JsArray::new();
        for name in &self.live {
            databases.push(dump(&self.database(name), name).await?);
        }
        Ok(host::record(&[("log", self.log.0.as_value()), ("databases", databases.as_value())]))
    }

    async fn cleanup(&self) -> Result<(), DomError> {
        for db in &self.opened {
            db.close();
        }
        for name in &self.attempted {
            idb::delete_db(&self.database(name), None)?.await?;
        }
        Ok(())
    }
}

/// Runs inside the upgrade callback: synchronous calls, each logged; a failure does not stop it.
fn upgrade(log: Log, op: &Upgrade, db: Database, tx: Transaction) {
    let (label, outcome) = match op {
        Upgrade::CreateStore { name, options } => ("createStore", db.create_object_store(name, *options).map(|store| Some(store.name().as_value()))),
        Upgrade::DeleteStore { name } => ("deleteStore", db.delete_object_store(name).map(|()| None)),
        Upgrade::CreateIndex { store, name, key_path, options } => (
            "createIndex",
            tx.object_store(store).and_then(|store| store.create_index(name, *key_path, *options)).map(|index| Some(index.name().as_value())),
        ),
        Upgrade::DeleteIndex { store, name } => ("deleteIndex", tx.object_store(store).and_then(|store| store.delete_index(name)).map(|()| None)),
        Upgrade::Abort => ("abort", tx.abort().map(|()| None)),
        Upgrade::Write { method, store, args } => {
            // Issued and not awaited: only a synchronous failure is logged.
            let issued = tx.object_store(store).and_then(|store| {
                let (first, second) = arguments(args)?;
                store.request::<()>(*method, first, second)
            });
            match issued {
                Ok(request) => request.ignore(),
                Err(error) => log.error(method.name(), error),
            }
            return;
        }
    };
    let _ = log.outcome(label, outcome);
}

/// Issues every request before awaiting any, then logs each outcome in issue order. Never fails.
async fn batch(log: Log, tx: Transaction, requests: &[script::Request], abort: bool, done: bool) {
    let issued: Vec<_> = requests.iter().map(|step| (step.method.name(), request(tx, step.method, &step.target, &step.args))).collect();
    if abort {
        let _ = log.outcome("abort", tx.abort().map(|()| None));
    }
    for (label, request) in issued {
        let outcome = match request {
            Ok(request) => request.await,
            Err(error) => Err(error),
        };
        let _ = log.outcome(label, outcome);
    }
    if done {
        let _ = log.outcome("done", tx.done().await.map(|()| None));
    }
}

fn position(log: Log, cursor: Cursor, value: bool) {
    let (key, primary_key) = (cursor.key(), cursor.primary_key());
    if value {
        log.push(&[s("cursor"), key, primary_key, cursor.value()]);
    } else {
        log.push(&[s("cursor"), key, primary_key]);
    }
}

async fn write(log: Log, cursor: Cursor, action: &Action) -> Result<(), Failed> {
    match *action {
        Action::Update(value) => log.outcome("update", settle(cursor.update(value)).await.map(Some)),
        _ => log.outcome("delete", settle(cursor.delete()).await.map(|()| None)),
    }
}

/// A walk with the promise-returning moves; a position without a move ends it.
async fn cursor_walk(log: Log, tx: Transaction, walk: &Walk) -> Result<(), Failed> {
    let label = walk.label();
    let opened = endpoint(tx, &walk.target).and_then(|source| {
        let (query, direction) = arguments(&walk.args)?;
        match walk.kind {
            WalkKind::KeyCursor => source.open_key_cursor(query, direction),
            _ => source.open_cursor(query, direction),
        }
    });
    let mut cursor = log.check(label, settle(opened).await)?;
    let mut at = 0;
    while let Some(current) = cursor {
        position(log, current, walk.kind != WalkKind::KeyCursor);
        let mut moved = None;
        for action in walk.actions(at) {
            moved = match *action {
                Action::Break => return Ok(()),
                Action::Update(_) | Action::Delete => {
                    write(log, current, action).await?;
                    continue;
                }
                Action::Continue(key) => Some(("continue", log.check("continue", current.continue_to(key))?)),
                Action::Advance(count) => Some(("advance", log.check("advance", current.advance(count))?)),
                Action::ContinuePrimaryKey(key, primary_key) => {
                    Some(("continuePrimaryKey", log.check("continuePrimaryKey", current.continue_primary_key(key, primary_key))?))
                }
            };
        }
        let Some((label, next)) = moved else { return Ok(()) };
        cursor = log.check(label, next.await)?;
        at += 1;
    }
    log.push(&[s("end")]);
    Ok(())
}

/// The async iterator: a move only records the next step, and a position without one continues.
async fn iterate(log: Log, tx: Transaction, walk: &Walk) -> Result<(), Failed> {
    let made = endpoint(tx, &walk.target).and_then(|source| {
        let (query, direction) = arguments(&walk.args)?;
        Ok(source.iterate(query, direction))
    });
    let mut iterator = log.check("iterate", made)?;
    let mut at = 0;
    while let Some(mut cursor) = log.check("iterate", iterator.next().await)? {
        position(log, cursor.cursor(), true);
        for action in walk.actions(at) {
            match *action {
                Action::Break => return Ok(()),
                Action::Update(_) | Action::Delete => write(log, cursor.cursor(), action).await?,
                Action::Continue(key) => log.check("continue", cursor.continue_to(key))?,
                Action::Advance(count) => log.check("advance", cursor.advance(count))?,
                Action::ContinuePrimaryKey(key, primary_key) => log.check("continuePrimaryKey", cursor.continue_primary_key(key, primary_key))?,
            }
        }
        at += 1;
    }
    log.push(&[s("end")]);
    Ok(())
}

/// `[name, version, stores]`, each store `[name, keyPath, autoIncrement, indexes, records]`.
async fn dump(name: &str, label: &str) -> Result<JsValue, DomError> {
    let db = idb::open_db(name, None, OpenCallbacks::default())?.await?;
    let stores = stores(db).await;
    db.close();
    Ok(list(&[s(label), JsValue::erase(db.version()), stores?]))
}

async fn stores(db: Database) -> Result<JsValue, DomError> {
    let names = db.object_store_names();
    let mut stores = JsArray::new();
    if names.is_empty() {
        return Ok(stores.as_value());
    }
    let tx = db.transaction(names.as_value(), JsValue::undefined())?;
    for i in 0..names.len() {
        let store = tx.object_store(names.get(i).read::<&str>())?;
        let index_names = store.index_names();
        let mut indexes = JsArray::new();
        for j in 0..index_names.len() {
            let index = store.index(index_names.get(j).read::<&str>())?;
            indexes.push(list(&[index_names.get(j), index.key_path(), JsValue::erase(index.unique()), JsValue::erase(index.multi_entry())]));
        }
        let keys = store.request::<JsValue>(Method::GetAllKeys, JsValue::undefined(), JsValue::undefined())?.await?;
        let values = store.request::<JsValue>(Method::GetAll, JsValue::undefined(), JsValue::undefined())?.await?;
        let mut records = JsArray::new();
        for k in 0..host::length(keys) {
            records.push(list(&[host::at(keys, k), host::at(values, k)]));
        }
        stores.push(list(&[names.get(i), store.key_path(), JsValue::erase(store.auto_increment()), indexes.as_value(), records.as_value()]));
    }
    tx.done().await?;
    Ok(stores.as_value())
}
