//! The browser's IndexedDB, DOM event and Promise APIs as typed bindings.
//!
//! Every IndexedDB call that can throw is `fallible`: its exception comes back as `Err`, so the
//! caller's cleanup still runs.

use mires_js::{callback1, invoke, js_bindings, JsArray, JsString, JsValue};

js_bindings! {
    #[js(global, get = "indexedDB")]
    pub fn factory() -> JsValue;
    #[js(method = "open", fallible)]
    pub fn open(factory: JsValue, name: &str, version: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "deleteDatabase", fallible)]
    pub fn delete_database(factory: JsValue, name: &str) -> Result<JsValue, JsValue>;

    #[js(call = "IDBKeyRange.only", fallible)]
    pub fn only(value: JsValue) -> Result<JsValue, JsValue>;
    #[js(call = "IDBKeyRange.lowerBound", fallible)]
    pub fn lower_bound(lower: JsValue, open: JsValue) -> Result<JsValue, JsValue>;
    #[js(call = "IDBKeyRange.upperBound", fallible)]
    pub fn upper_bound(upper: JsValue, open: JsValue) -> Result<JsValue, JsValue>;
    #[js(call = "IDBKeyRange.bound", fallible)]
    pub fn bound(lower: JsValue, upper: JsValue, lower_open: JsValue, upper_open: JsValue) -> Result<JsValue, JsValue>;

    // IDBRequest, IDBOpenDBRequest and IDBVersionChangeEvent.
    #[js(get = "result")]
    pub fn result(request: JsValue) -> JsValue;
    #[js(get = "error")]
    pub fn error(target: JsValue) -> JsValue;
    #[js(get = "transaction")]
    pub fn request_transaction(request: JsValue) -> JsValue;
    #[js(get = "oldVersion")]
    pub fn old_version(event: JsValue) -> f64;
    #[js(get = "newVersion", nullable)]
    pub fn new_version(event: JsValue) -> Option<f64>;
    #[js(get = "target")]
    pub fn event_target(event: JsValue) -> JsValue;

    // IDBDatabase.
    #[js(get = "version")]
    pub fn version(db: JsValue) -> f64;
    #[js(get = "objectStoreNames")]
    pub fn object_store_names(target: JsValue) -> JsValue;
    #[js(method = "createObjectStore", fallible)]
    pub fn create_object_store(db: JsValue, name: &str, options: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "deleteObjectStore", fallible)]
    pub fn delete_object_store(db: JsValue, name: &str) -> Result<JsValue, JsValue>;
    #[js(method = "transaction", fallible)]
    pub fn transaction(db: JsValue, stores: JsValue, mode: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "close")]
    pub fn close(db: JsValue);

    // DOMStringList.
    #[js(get = "length")]
    pub fn length(list: JsValue) -> usize;
    #[js(method = "item")]
    pub fn item(list: JsValue, index: usize) -> JsString;

    // IDBTransaction.
    #[js(method = "objectStore", fallible)]
    pub fn object_store(tx: JsValue, name: &str) -> Result<JsValue, JsValue>;
    #[js(method = "abort", fallible)]
    pub fn abort(tx: JsValue) -> Result<JsValue, JsValue>;

    // IDBObjectStore and IDBIndex.
    #[js(get = "name")]
    pub fn name(target: JsValue) -> JsString;
    #[js(get = "keyPath")]
    pub fn key_path(target: JsValue) -> JsValue;
    #[js(get = "autoIncrement")]
    pub fn auto_increment(store: JsValue) -> bool;
    #[js(get = "indexNames")]
    pub fn index_names(store: JsValue) -> JsValue;
    #[js(get = "unique")]
    pub fn unique(index: JsValue) -> bool;
    #[js(get = "multiEntry")]
    pub fn multi_entry(index: JsValue) -> bool;
    #[js(method = "index", fallible)]
    pub fn index(store: JsValue, name: &str) -> Result<JsValue, JsValue>;
    #[js(method = "createIndex", fallible)]
    pub fn create_index(store: JsValue, name: &str, key_path: JsValue, options: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "deleteIndex", fallible)]
    pub fn delete_index(store: JsValue, name: &str) -> Result<JsValue, JsValue>;
    #[js(method = "get", fallible)]
    pub fn get(source: JsValue, query: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "getKey", fallible)]
    pub fn get_key(source: JsValue, query: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "getAll", fallible)]
    pub fn get_all(source: JsValue, query: JsValue, count: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "getAllKeys", fallible)]
    pub fn get_all_keys(source: JsValue, query: JsValue, count: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "count", fallible)]
    pub fn count(source: JsValue, query: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "put", fallible)]
    pub fn put(store: JsValue, value: JsValue, key: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "add", fallible)]
    pub fn add(store: JsValue, value: JsValue, key: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "delete", fallible)]
    pub fn delete(store: JsValue, query: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "clear", fallible)]
    pub fn clear(store: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "openCursor", fallible)]
    pub fn open_cursor(source: JsValue, query: JsValue, direction: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "openKeyCursor", fallible)]
    pub fn open_key_cursor(source: JsValue, query: JsValue, direction: JsValue) -> Result<JsValue, JsValue>;

    // IDBCursor and IDBCursorWithValue.
    #[js(get = "key")]
    pub fn key(cursor: JsValue) -> JsValue;
    #[js(get = "primaryKey")]
    pub fn primary_key(cursor: JsValue) -> JsValue;
    #[js(get = "value")]
    pub fn value(cursor: JsValue) -> JsValue;
    #[js(get = "request")]
    pub fn cursor_request(cursor: JsValue) -> JsValue;
    #[js(method = "advance", fallible)]
    pub fn advance(cursor: JsValue, count: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "continue", fallible)]
    pub fn continue_to(cursor: JsValue, key: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "continuePrimaryKey", fallible)]
    pub fn continue_primary_key(cursor: JsValue, key: JsValue, primary_key: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "update", fallible)]
    pub fn update(cursor: JsValue, value: JsValue) -> Result<JsValue, JsValue>;
    #[js(method = "delete", fallible)]
    pub fn delete_record(cursor: JsValue) -> Result<JsValue, JsValue>;

    // Events, promises and errors.
    #[js(method = "addEventListener")]
    pub fn listen(target: JsValue, kind: &str, listener: JsValue);
    #[js(method = "addEventListener")]
    fn listen_with(target: JsValue, kind: &str, listener: JsValue, options: JsValue);
    #[js(new = "AbortController")]
    fn abort_controller() -> JsValue;
    #[js(get = "signal")]
    fn signal(controller: JsValue) -> JsValue;
    #[js(method = "abort")]
    fn abort_controller_abort(controller: JsValue);
    #[js(new = "Object")]
    fn object() -> JsValue;
    #[js(set = "signal")]
    fn set_signal(options: JsValue, signal: JsValue);
    #[js(new = "Promise")]
    pub fn promise(executor: JsValue) -> JsValue;
    #[js(call = "Promise.all")]
    pub fn promise_all(promises: JsValue) -> JsValue;
    #[js(method = "catch")]
    fn catch(promise: JsValue, handler: JsValue) -> JsValue;
    #[js(new = "DOMException")]
    pub fn dom_exception(message: &str, name: &str) -> JsValue;
    #[js(call = "crypto.randomUUID")]
    pub fn random_uuid() -> JsString;

    // Plain data.
    #[js(index)]
    pub fn field(object: JsValue, key: &str) -> JsValue;
    #[js(has)]
    pub fn has(object: JsValue, key: &str) -> bool;
    #[js(index)]
    pub fn at(array: JsValue, index: usize) -> JsValue;
    #[js(call = "Array.isArray")]
    pub fn is_array(value: JsValue) -> bool;
    #[js(index_set)]
    fn set_field(object: JsValue, key: &str, value: JsValue);
}

/// Calls a Promise executor's `resolve` or `reject`.
pub fn settle(function: JsValue, value: JsValue) {
    let _ = invoke(function, JsValue::undefined(), &[value]);
}

/// Marks a promise's rejection as handled.
pub fn ignore_rejection(promise: JsValue) {
    let _ = catch(promise, callback1(|_| JsValue::undefined()));
}

/// Listeners that are removed together, as idb's `unlisten` removes both of a request's listeners.
#[derive(Clone, Copy)]
pub struct Listeners {
    controller: JsValue,
    options: JsValue,
}

impl Listeners {
    pub fn new() -> Self {
        let controller = abort_controller();
        let options = object();
        set_signal(options, signal(controller));
        Self { controller, options }
    }

    pub fn on(self, target: JsValue, kind: &str, listener: JsValue) {
        listen_with(target, kind, listener, self.options);
    }

    pub fn remove(self) {
        abort_controller_abort(self.controller);
    }
}

/// Builds a plain object; its keys keep the given order.
pub fn record(fields: &[(&str, JsValue)]) -> JsValue {
    let record = object();
    for &(key, value) in fields {
        set_field(record, key, value);
    }
    record
}

/// Builds a host array from values.
pub fn list(values: &[JsValue]) -> JsValue {
    let mut array = JsArray::new();
    for &value in values {
        array.push(value);
    }
    array.as_value()
}

/// Copies a DOMStringList into a host array.
pub fn names(list: JsValue) -> JsArray<JsValue> {
    let mut array = JsArray::new();
    for i in 0..length(list) {
        array.push(item(list, i).as_value());
    }
    array
}
