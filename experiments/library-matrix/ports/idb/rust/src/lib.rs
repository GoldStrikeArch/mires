#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::string::String;
use mires_js::{
    await_promise, callback1, callback2, construct, future_to_promise, invoke, js_bindings,
    js_export, throw_value, JsValue,
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(JsValue::erase("idb port panicked"))
}

js_bindings! {
    #[js(global, get = "globalThis")]
    fn global() -> JsValue;
    #[js(call = "Reflect.get")]
    fn property(target:JsValue,key:JsValue)->JsValue;
    #[js(call = "Reflect.set")]
    fn assign(target:JsValue,key:JsValue,value:JsValue)->bool;
    #[js(new = "Array")]
    fn array()->JsValue;
    #[js(new = "Object")]
    fn object()->JsValue;
    #[js(index)]
    fn at(target:JsValue,i:usize)->JsValue;
    #[js(get = "length")]
    fn len(target:JsValue)->usize;
}

fn s(value: &str) -> JsValue {
    JsValue::erase(value)
}
fn get(target: JsValue, key: &str) -> JsValue {
    property(target, s(key))
}
fn set(target: JsValue, key: &str, value: JsValue) {
    let _ = assign(target, s(key), value);
}
fn call(target: JsValue, method: &str, args: &[JsValue]) -> JsValue {
    invoke(get(target, method), target, args)
}
fn push(target: JsValue, value: JsValue) {
    call(target, "push", &[value]);
}
fn settle(f: JsValue, value: JsValue) {
    invoke(f, JsValue::undefined(), &[value]);
}

fn request(req: JsValue) -> JsValue {
    let executor = callback2(move |resolve, reject| {
        set(
            req,
            "onsuccess",
            callback1(move |_| {
                set(req, "onsuccess", JsValue::null());
                set(req, "onerror", JsValue::null());
                settle(resolve, get(req, "result"));
                JsValue::undefined()
            }),
        );
        set(
            req,
            "onerror",
            callback1(move |_| {
                set(req, "onsuccess", JsValue::null());
                set(req, "onerror", JsValue::null());
                settle(reject, get(req, "error"));
                JsValue::undefined()
            }),
        );
        JsValue::undefined()
    });
    construct(get(global(), "Promise"), &[executor])
}

fn transaction_done(tx: JsValue) -> JsValue {
    fn clear(tx: JsValue) {
        for key in ["oncomplete", "onerror", "onabort"] {
            set(tx, key, JsValue::null());
        }
    }
    let executor = callback2(move |resolve, reject| {
        set(
            tx,
            "oncomplete",
            callback1(move |_| {
                clear(tx);
                settle(resolve, JsValue::undefined());
                JsValue::undefined()
            }),
        );
        let failure = callback1(move |_| {
            let reason = get(tx, "error");
            let reason = if reason.is_truthy() {
                reason
            } else {
                construct(
                    get(global(), "DOMException"),
                    &[s("AbortError"), s("AbortError")],
                )
            };
            clear(tx);
            settle(reject, reason);
            JsValue::undefined()
        });
        set(tx, "onerror", failure);
        set(tx, "onabort", failure);
        JsValue::undefined()
    });
    construct(get(global(), "Promise"), &[executor])
}

fn open(name: JsValue) -> JsValue {
    let req = call(
        get(global(), "indexedDB"),
        "open",
        &[name, JsValue::erase(1)],
    );
    set(
        req,
        "onupgradeneeded",
        callback1(move |_| {
            let options = object();
            set(options, "keyPath", s("id"));
            let store = call(
                get(req, "result"),
                "createObjectStore",
                &[s("rows"), options],
            );
            call(store, "createIndex", &[s("byGroup"), s("group")]);
            JsValue::undefined()
        }),
    );
    request(req)
}

async fn write(db: JsValue, batch: JsValue) -> Result<JsValue, JsValue> {
    let tx = call(db, "transaction", &[s("rows"), s("readwrite")]);
    let done = transaction_done(tx);
    let store = call(tx, "objectStore", &[s("rows")]);
    let rows = get(batch, "rows");
    let promises = array();
    let method = if get(batch, "add").is_truthy() {
        "add"
    } else {
        "put"
    };
    for i in 0..len(rows) {
        push(promises, request(call(store, method, &[at(rows, i)])));
    }
    push(promises, done);
    let outcomes = call(get(global(), "Promise"), "allSettled", &[promises]);
    if get(batch, "abort").is_truthy() {
        call(tx, "abort", &[]);
    }
    let results = await_promise(outcomes).await?;
    let normalized = array();
    for i in 0..len(results) {
        let r = at(results, i);
        if get(r, "status").read::<&str>() == "rejected" {
            let mut error = String::from("!");
            error.push_str(get(get(r, "reason"), "name").read::<&str>());
            push(normalized, s(&error));
        } else if i == len(rows) {
            push(normalized, s("done"));
        } else {
            push(normalized, get(r, "value"));
        }
    }
    Ok(normalized)
}

fn lookup(source: JsValue, keys: JsValue) -> JsValue {
    let requests = array();
    for i in 0..len(keys) {
        push(requests, request(call(source, "get", &[at(keys, i)])));
    }
    call(get(global(), "Promise"), "all", &[requests])
}

fn values(rows: JsValue) -> JsValue {
    let out = array();
    for i in 0..len(rows) {
        let row = at(rows, i);
        push(
            out,
            if row.is_undefined() {
                JsValue::null()
            } else {
                get(row, "value")
            },
        );
    }
    out
}

async fn exercise(db: JsValue, input: JsValue) -> Result<JsValue, JsValue> {
    let transactions = array();
    let batches = get(input, "batches");
    for i in 0..len(batches) {
        push(transactions, write(db, at(batches, i)).await?);
    }
    let tx = call(db, "transaction", &[s("rows")]);
    let done = transaction_done(tx);
    let store = call(tx, "objectStore", &[s("rows")]);
    let index = call(store, "index", &[s("byGroup")]);
    let reads = lookup(store, get(input, "keys"));
    let groups = lookup(index, get(input, "groups"));
    let source = if get(input, "index").is_truthy() {
        index
    } else {
        store
    };
    let only = get(input, "only");
    let range = if only.is_undefined() {
        JsValue::null()
    } else {
        call(get(global(), "IDBKeyRange"), "only", &[only])
    };
    let direction = if get(input, "reverse").is_truthy() {
        "prev"
    } else {
        "next"
    };
    let req = call(source, "openCursor", &[range, s(direction)]);
    let step = get(input, "step");
    let cursors = array();
    let mut cursor = await_promise(request(req)).await?;
    while cursor.is_truthy() {
        let triple = array();
        push(triple, get(cursor, "key"));
        push(triple, get(cursor, "primaryKey"));
        push(triple, get(get(cursor, "value"), "value"));
        push(cursors, triple);
        if step.read::<f64>() == 1.0 {
            call(cursor, "continue", &[]);
        } else {
            call(cursor, "advance", &[step]);
        }
        cursor = await_promise(request(req)).await?;
    }
    let out = array();
    push(out, transactions);
    push(out, values(await_promise(reads).await?));
    push(out, values(await_promise(groups).await?));
    push(out, cursors);
    await_promise(done).await?;
    Ok(out)
}

#[js_export]
pub fn matrix_run(input: JsValue) -> JsValue {
    future_to_promise(async move {
        let mut prefix = String::from("mires-matrix-idb-");
        prefix.push_str(call(get(global(), "crypto"), "randomUUID", &[]).read::<&str>());
        let name = s(&prefix);
        let result = match await_promise(open(name)).await {
            Ok(db) => {
                let result = exercise(db, input).await;
                call(db, "close", &[]);
                result
            }
            Err(e) => Err(e),
        };
        await_promise(request(call(
            get(global(), "indexedDB"),
            "deleteDatabase",
            &[name],
        )))
        .await?;
        result
    })
}
