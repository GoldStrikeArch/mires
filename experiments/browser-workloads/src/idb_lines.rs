//! IndexedDB control variant using idiomatic Rust string iterators.

use alloc::string::String;

use mires_js::{JsValue, await_promise, callback1, future_to_promise};

use crate::host::{call, get, global_this, index, length, promise, set, settle};

const STORE: &str = "records";

/// Stores and reads rows while parsing input through `str::lines`.
#[must_use]
pub fn roundtrip(options: JsValue) -> JsValue {
    future_to_promise(async move {
        let database = get(options, "database");
        let records = get(options, "records");
        let name = if database.is_string() {
            database.read()
        } else {
            "browser-workloads-lines"
        };
        let rows = if records.is_string() {
            records.read()
        } else {
            ""
        };
        roundtrip_records(name, rows)
            .await
            .map(|result| JsValue::erase(result.as_str()))
    })
}

async fn roundtrip_records(database: &str, records: &str) -> Result<String, JsValue> {
    let db = await_promise(open_database(database)).await?;
    let transaction = call(
        db,
        "transaction",
        &[JsValue::erase(STORE), JsValue::erase("readwrite")],
    );
    let store = call(transaction, "objectStore", &[JsValue::erase(STORE)]);
    for row in records.lines() {
        let key = row.split_once('=').map_or(row, |(key, _)| key);
        let _ = call(store, "put", &[JsValue::erase(row), JsValue::erase(key)]);
    }
    await_promise(transaction_promise(transaction)).await?;

    let transaction = call(
        db,
        "transaction",
        &[JsValue::erase(STORE), JsValue::erase("readonly")],
    );
    let store = call(transaction, "objectStore", &[JsValue::erase(STORE)]);
    let values = await_promise(request_promise(call(store, "getAll", &[]))).await?;
    let mut result = String::new();
    for index_at in 0..length(values) {
        if index_at > 0 {
            result.push('\n');
        }
        let value = index(values, index_at);
        result.push_str(if value.is_string() { value.read() } else { "" });
    }
    let _ = call(db, "close", &[]);
    Ok(result)
}

fn open_database(name: &str) -> JsValue {
    let factory = get(global_this(), "indexedDB");
    let request = call(
        factory,
        "open",
        &[JsValue::erase(name), JsValue::erase(1.0)],
    );
    let upgrade = callback1(move |_event| {
        let database = get(request, "result");
        let stores = get(database, "objectStoreNames");
        if !call(stores, "contains", &[JsValue::erase(STORE)]).is_truthy() {
            let _ = call(database, "createObjectStore", &[JsValue::erase(STORE)]);
        }
        JsValue::undefined()
    });
    set(request, "onupgradeneeded", upgrade);
    request_promise(request)
}

fn request_promise(request: JsValue) -> JsValue {
    promise(move |resolve, reject| {
        let success = callback1(move |_event| {
            settle(resolve, get(request, "result"));
            JsValue::undefined()
        });
        let failure = callback1(move |_event| {
            settle(reject, get(request, "error"));
            JsValue::undefined()
        });
        set(request, "onsuccess", success);
        set(request, "onerror", failure);
        JsValue::undefined()
    })
}

fn transaction_promise(transaction: JsValue) -> JsValue {
    promise(move |resolve, reject| {
        let complete = callback1(move |_event| {
            settle(resolve, JsValue::undefined());
            JsValue::undefined()
        });
        let failure = callback1(move |_event| {
            settle(reject, get(transaction, "error"));
            JsValue::undefined()
        });
        set(transaction, "oncomplete", complete);
        set(transaction, "onabort", failure);
        set(transaction, "onerror", failure);
        JsValue::undefined()
    })
}
