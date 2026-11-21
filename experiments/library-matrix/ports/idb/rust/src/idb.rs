//! idb 8.0.4 (`build/index.js`) for Rust: IndexedDB requests become futures, transactions get
//! a `done` future, cursors move by returning the next cursor, and `iterate` walks a source.
//!
//! idb wraps each IDB object in a Proxy that forwards every property and wraps what comes back.
//! Here each wrapped object is a typed handle over the host object, and its methods call the
//! host method and wrap the result the same way. Request futures are host Promises, created and
//! settled by idb's own listener logic (`wrap`, `cache_done`).

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::future::{Future, IntoFuture};
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use mires_js::{await_promise, callback1, callback2, JsArray, JsString, JsValue, PromiseFuture};

use crate::host::{self, Listeners};

/// A rejection: the DOMException (or TypeError) a request, transaction or call produced.
#[derive(Clone, Copy)]
pub struct DomError(pub JsValue);

impl DomError {
    pub fn name(self) -> JsValue {
        host::field(self.0, "name")
    }
}

fn fallible(result: Result<JsValue, JsValue>) -> Result<JsValue, DomError> {
    result.map_err(DomError)
}

/// How a settled request's `result` is viewed, as idb's `wrap(request.result)` decides by type.
pub trait FromResult: Sized {
    fn from_result(value: JsValue) -> Self;
}

impl FromResult for JsValue {
    fn from_result(value: JsValue) -> Self {
        value
    }
}

/// Any result: `None` is `undefined` (a miss), which is not a stored `null`.
impl FromResult for Option<JsValue> {
    fn from_result(value: JsValue) -> Self {
        (!value.is_undefined()).then_some(value)
    }
}

impl FromResult for () {
    fn from_result(_: JsValue) -> Self {}
}

impl FromResult for Database {
    fn from_result(value: JsValue) -> Self {
        Database(value)
    }
}

/// A cursor request yields the cursor again, or `null` past the end.
impl FromResult for Option<Cursor> {
    fn from_result(value: JsValue) -> Self {
        (!value.is_null()).then_some(Cursor(value))
    }
}

/// Awaits a request that may have failed to issue, as `await` on idb's call would.
pub async fn settle<T: FromResult>(issued: Result<Request<T>, DomError>) -> Result<T, DomError> {
    issued?.await
}

/// A promisified request: a host Promise that settles once, with the request's next outcome.
pub struct Request<T> {
    promise: JsValue,
    result: PhantomData<fn() -> T>,
}

impl<T> Request<T> {
    fn new(promise: JsValue) -> Self {
        Self { promise, result: PhantomData }
    }

    /// Handles a rejection nobody awaits, as `promise.catch(() => {})` does.
    pub fn ignore(self) {
        host::ignore_rejection(self.promise);
    }

    pub(crate) fn promise(&self) -> JsValue {
        self.promise
    }
}

impl<T: FromResult> IntoFuture for Request<T> {
    type Output = Result<T, DomError>;
    type IntoFuture = Settled<T>;

    fn into_future(self) -> Settled<T> {
        Settled { promise: await_promise(self.promise), result: PhantomData }
    }
}

pub struct Settled<T> {
    promise: PromiseFuture,
    result: PhantomData<fn() -> T>,
}

impl<T: FromResult> Future for Settled<T> {
    type Output = Result<T, DomError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().promise).poll(context).map(|settled| settled.map(T::from_result).map_err(DomError))
    }
}

/// idb's `promisifyRequest`: resolves with the request's result on `success`, rejects with its
/// error on `error`, then removes both listeners. A cursor request settles once per call, so each
/// cursor move wraps the same request again.
pub fn wrap<T: FromResult>(request: JsValue) -> Request<T> {
    Request::new(host::promise(callback2(move |resolve, reject| {
        let listeners = Listeners::new();
        listeners.on(request, "success", callback1(move |_| {
            host::settle(resolve, host::result(request));
            listeners.remove();
            JsValue::undefined()
        }));
        listeners.on(request, "error", callback1(move |_| {
            host::settle(reject, host::error(request));
            listeners.remove();
            JsValue::undefined()
        }));
        JsValue::undefined()
    })))
}

/// The host object behind a wrapped value.
pub trait Wrapped {
    fn raw(&self) -> JsValue;
}

/// idb's `unwrap`.
pub fn unwrap(value: &impl Wrapped) -> JsValue {
    value.raw()
}

/// idb's `cacheDonePromiseForTransaction`: settles on the first `complete`, `error` or `abort`.
/// A request error bubbles to the transaction before the transaction records it, so `done`
/// then rejects with a new AbortError.
fn cache_done(tx: JsValue) -> JsValue {
    host::promise(callback2(move |resolve, reject| {
        let listeners = Listeners::new();
        listeners.on(tx, "complete", callback1(move |_| {
            host::settle(resolve, JsValue::undefined());
            listeners.remove();
            JsValue::undefined()
        }));
        let failed = callback1(move |_| {
            let error = host::error(tx);
            host::settle(reject, if error.is_truthy() { error } else { host::dom_exception("AbortError", "AbortError") });
            listeners.remove();
            JsValue::undefined()
        });
        listeners.on(tx, "error", failed);
        listeners.on(tx, "abort", failed);
        JsValue::undefined()
    }))
}

/// The request methods of IDBObjectStore and IDBIndex that idb promisifies (and, for the
/// database shortcuts, the `readMethods` and `writeMethods` lists).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    GetKey,
    GetAll,
    GetAllKeys,
    Count,
    Put,
    Add,
    Delete,
    Clear,
}

impl Method {
    pub fn is_write(self) -> bool {
        matches!(self, Method::Put | Method::Add | Method::Delete | Method::Clear)
    }

    pub fn name(self) -> &'static str {
        match self {
            Method::Get => "get",
            Method::GetKey => "getKey",
            Method::GetAll => "getAll",
            Method::GetAllKeys => "getAllKeys",
            Method::Count => "count",
            Method::Put => "put",
            Method::Add => "add",
            Method::Delete => "delete",
            Method::Clear => "clear",
        }
    }

    /// Calls the host method, unwrapped, forwarding the exact positional arguments given.
    pub fn call(self, source: JsValue, args: &[JsValue]) -> Result<JsValue, JsValue> {
        host::call_method(source, self.name(), args)
    }
}

/// Methods an object store and an index share.
pub trait Source: Wrapped {
    /// A wrapped request method: issued now, settled later.
    fn request<T: FromResult>(&self, method: Method, args: &[JsValue]) -> Result<Request<T>, DomError> {
        Ok(wrap(fallible(method.call(self.raw(), args))?))
    }

    fn open_cursor(&self, args: &[JsValue]) -> Result<Request<Option<Cursor>>, DomError> {
        Ok(wrap(fallible(host::call_method(self.raw(), "openCursor", args))?))
    }

    fn open_key_cursor(&self, args: &[JsValue]) -> Result<Request<Option<Cursor>>, DomError> {
        Ok(wrap(fallible(host::call_method(self.raw(), "openKeyCursor", args))?))
    }

    /// idb's async iterator over `openCursor(...args)`.
    fn iterate(&self, args: Vec<JsValue>) -> Iterate {
        Iterate { source: self.raw(), args, state: IterState::Unopened, advanced: None }
    }

    fn name(&self) -> JsString {
        host::name(self.raw())
    }

    fn key_path(&self) -> JsValue {
        host::key_path(self.raw())
    }
}

#[derive(Clone, Copy)]
pub struct Database(JsValue);

impl Wrapped for Database {
    fn raw(&self) -> JsValue {
        self.0
    }
}

impl Database {
    pub fn version(&self) -> f64 {
        host::version(self.0)
    }

    pub fn object_store_names(&self) -> JsArray<JsValue> {
        host::names(host::object_store_names(self.0))
    }

    pub fn create_object_store(&self, name: &str, options: JsValue) -> Result<ObjectStore, DomError> {
        fallible(host::create_object_store(self.0, name, options)).map(ObjectStore)
    }

    pub fn delete_object_store(&self, name: &str) -> Result<(), DomError> {
        fallible(host::delete_object_store(self.0, name)).map(drop)
    }

    /// `stores` is a name or an array of names; an undefined `mode` is `readonly`.
    pub fn transaction(&self, stores: JsValue, mode: JsValue) -> Result<Transaction, DomError> {
        fallible(host::transaction(self.0, stores, mode)).map(Transaction::wrap)
    }

    pub fn close(&self) {
        host::close(self.0);
    }

    /// The database shortcuts (`db.get(store, ...)`, `db.getAllFromIndex(store, index, ...)`, ...),
    /// database-extras' `getMethod`: one transaction per call on `tx.store` (or its index), and
    /// `Promise.all([request, isWrite && tx.done])`, so a write resolves only after `done`, the
    /// request's error comes first, and both promises are handled. Everything up to the request is
    /// done now; a synchronous failure rejects the call.
    pub fn shortcut(&self, method: Method, store: &str, index: Option<&str>, args: &[JsValue])
        -> impl Future<Output = Result<Option<JsValue>, DomError>> {
        let issued = (|| {
            let tx = self.transaction(JsValue::erase(store), JsValue::erase(if method.is_write() { "readwrite" } else { "readonly" }))?;
            let target = tx.store()?.expect("a one-store transaction has a store");
            let request: Request<JsValue> = match index {
                Some(index) => target.index(index)?.request(method, args)?,
                None => target.request(method, args)?,
            };
            let done = if method.is_write() { tx.done } else { JsValue::erase(false) };
            Ok(Request::<JsValue>::new(host::promise_all(host::list(&[request.promise, done]))))
        })();
        async move { Ok(Option::<JsValue>::from_result(host::at(settle(issued).await?, 0))) }
    }
}

#[derive(Clone, Copy)]
pub struct Transaction {
    raw: JsValue,
    done: JsValue,
}

impl Wrapped for Transaction {
    fn raw(&self) -> JsValue {
        self.raw
    }
}

impl Transaction {
    /// idb creates the `done` promise when it first wraps a transaction, so `done` also settles
    /// for a caller that only asks after the transaction finished.
    fn wrap(raw: JsValue) -> Self {
        Self { raw, done: cache_done(raw) }
    }

    pub fn done(&self) -> Request<()> {
        Request::new(self.done)
    }

    /// idb's `tx.store`: the only store in scope, or `None` when there are several.
    pub fn store(&self) -> Result<Option<ObjectStore>, DomError> {
        let names = host::object_store_names(self.raw);
        if host::length(names) > 1 {
            return Ok(None);
        }
        self.object_store(&host::item(names, 0).as_value().read::<&str>()).map(Some)
    }

    pub fn object_store(&self, name: &str) -> Result<ObjectStore, DomError> {
        fallible(host::object_store(self.raw, name)).map(ObjectStore)
    }

    pub fn abort(&self) -> Result<(), DomError> {
        fallible(host::abort(self.raw)).map(drop)
    }
}

#[derive(Clone, Copy)]
pub struct ObjectStore(JsValue);

impl Wrapped for ObjectStore {
    fn raw(&self) -> JsValue {
        self.0
    }
}

impl Source for ObjectStore {}

impl ObjectStore {
    pub fn index(&self, name: &str) -> Result<Index, DomError> {
        fallible(host::index(self.0, name)).map(Index)
    }

    pub fn create_index(&self, name: &str, key_path: JsValue, options: JsValue) -> Result<Index, DomError> {
        fallible(host::create_index(self.0, name, key_path, options)).map(Index)
    }

    pub fn delete_index(&self, name: &str) -> Result<(), DomError> {
        fallible(host::delete_index(self.0, name)).map(drop)
    }

    pub fn auto_increment(&self) -> bool {
        host::auto_increment(self.0)
    }

    pub fn index_names(&self) -> JsArray<JsValue> {
        host::names(host::index_names(self.0))
    }
}

#[derive(Clone, Copy)]
pub struct Index(JsValue);

impl Wrapped for Index {
    fn raw(&self) -> JsValue {
        self.0
    }
}

impl Source for Index {}

impl Index {
    pub fn unique(&self) -> bool {
        host::unique(self.0)
    }

    pub fn multi_entry(&self) -> bool {
        host::multi_entry(self.0)
    }
}

/// A wrapped IDBCursor or IDBCursorWithValue.
#[derive(Clone, Copy)]
pub struct Cursor(JsValue);

impl Wrapped for Cursor {
    fn raw(&self) -> JsValue {
        self.0
    }
}

impl Cursor {
    pub fn key(&self) -> JsValue {
        host::key(self.0)
    }

    pub fn primary_key(&self) -> JsValue {
        host::primary_key(self.0)
    }

    pub fn value(&self) -> JsValue {
        host::value(self.0)
    }

    /// idb's cursor advance methods call the host method, then wrap the cursor's request again.
    fn moved(&self, called: Result<JsValue, JsValue>) -> Result<Request<Option<Cursor>>, DomError> {
        fallible(called)?;
        Ok(wrap(host::cursor_request(self.0)))
    }

    pub fn advance(&self, count: JsValue) -> Result<Request<Option<Cursor>>, DomError> {
        self.moved(host::advance(self.0, count))
    }

    /// `continue(key)`; an undefined key continues to the next record.
    pub fn continue_to(&self, key: JsValue) -> Result<Request<Option<Cursor>>, DomError> {
        self.moved(host::continue_to(self.0, key))
    }

    pub fn continue_primary_key(&self, key: JsValue, primary_key: JsValue) -> Result<Request<Option<Cursor>>, DomError> {
        self.moved(host::continue_primary_key(self.0, key, primary_key))
    }

    pub fn update(&self, value: JsValue) -> Result<Request<JsValue>, DomError> {
        Ok(wrap(fallible(host::update(self.0, value))?))
    }

    pub fn delete(&self) -> Result<Request<()>, DomError> {
        Ok(wrap(fallible(host::delete_record(self.0))?))
    }
}

enum IterState {
    Unopened,
    At(Cursor),
    Finished,
}

/// idb's `iterate`: the first `next` opens the cursor; each later one awaits the move the loop
/// body made (`advance`, `continue_to`, `continue_primary_key` on the yielded cursor), or calls
/// `continue()` when the body made none.
pub struct Iterate {
    source: JsValue,
    args: Vec<JsValue>,
    state: IterState,
    advanced: Option<Request<Option<Cursor>>>,
}

impl Iterate {
    pub async fn next(&mut self) -> Result<Option<IterCursor<'_>>, DomError> {
        let state = core::mem::replace(&mut self.state, IterState::Finished);
        let next = match state {
            IterState::Unopened => wrap(fallible(host::call_method(self.source, "openCursor", &self.args))?).await?,
            IterState::At(cursor) => match self.advanced.take() {
                Some(moved) => moved.await?,
                None => cursor.continue_to(JsValue::undefined())?.await?,
            },
            IterState::Finished => None,
        };
        Ok(next.map(move |cursor| {
            self.state = IterState::At(cursor);
            IterCursor { iterate: self, cursor }
        }))
    }
}

/// The cursor an iteration yields: its moves are recorded for the iterator instead of returned.
pub struct IterCursor<'a> {
    iterate: &'a mut Iterate,
    cursor: Cursor,
}

impl IterCursor<'_> {
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn advance(&mut self, count: JsValue) -> Result<(), DomError> {
        self.iterate.advanced = Some(self.cursor.advance(count)?);
        Ok(())
    }

    pub fn continue_to(&mut self, key: JsValue) -> Result<(), DomError> {
        self.iterate.advanced = Some(self.cursor.continue_to(key)?);
        Ok(())
    }

    pub fn continue_primary_key(&mut self, key: JsValue, primary_key: JsValue) -> Result<(), DomError> {
        self.iterate.advanced = Some(self.cursor.continue_primary_key(key, primary_key)?);
        Ok(())
    }
}

/// openDB's `upgrade(db, oldVersion, newVersion, transaction)`.
pub type Upgrade = Box<dyn Fn(Database, f64, Option<f64>, Transaction)>;
/// openDB's `blocked` and `blocking`: `(currentVersion, blockedVersion, event)`.
pub type VersionChange = Box<dyn Fn(f64, Option<f64>, JsValue)>;
/// deleteDB's `blocked`: `(currentVersion, event)`.
pub type DeleteBlocked = Box<dyn Fn(f64, JsValue)>;

#[derive(Default)]
pub struct OpenCallbacks {
    pub upgrade: Option<Upgrade>,
    pub blocked: Option<VersionChange>,
    pub blocking: Option<VersionChange>,
}

/// A pending `openDB`. `blocking` is registered on the connection once it opens.
pub struct OpenDb {
    opened: Request<Database>,
    blocking: Option<VersionChange>,
}

impl IntoFuture for OpenDb {
    type Output = Result<Database, DomError>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Database, DomError>>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let db = self.opened.await?;
            if let Some(blocking) = self.blocking {
                host::listen(db.0, "versionchange", callback1(move |event| {
                    blocking(host::old_version(event), host::new_version(event), event);
                    JsValue::undefined()
                }));
            }
            Ok(db)
        })
    }
}

/// idb's `openDB(name, version, { upgrade, blocked, blocking })`. A `None` version opens the
/// current version, or version 1 for a new database. An invalid version throws here.
pub fn open_db(name: &str, version: Option<JsValue>, callbacks: OpenCallbacks) -> Result<OpenDb, DomError> {
    let request = fallible(host::open(host::factory(), name, version.unwrap_or(JsValue::undefined())))?;
    let opened = wrap(request);
    if let Some(upgrade) = callbacks.upgrade {
        host::listen(request, "upgradeneeded", callback1(move |event| {
            let db = Database(host::result(request));
            let tx = Transaction::wrap(host::request_transaction(request));
            upgrade(db, host::old_version(event), host::new_version(event), tx);
            JsValue::undefined()
        }));
    }
    if let Some(blocked) = callbacks.blocked {
        host::listen(request, "blocked", callback1(move |event| {
            blocked(host::old_version(event), host::new_version(event), event);
            JsValue::undefined()
        }));
    }
    Ok(OpenDb { opened, blocking: callbacks.blocking })
}

/// idb's `deleteDB(name, { blocked })`.
pub fn delete_db(name: &str, blocked: Option<DeleteBlocked>) -> Result<Request<()>, DomError> {
    let request = fallible(host::delete_database(host::factory(), name))?;
    if let Some(blocked) = blocked {
        host::listen(request, "blocked", callback1(move |event| {
            blocked(host::old_version(event), event);
            JsValue::undefined()
        }));
    }
    Ok(wrap(request))
}
