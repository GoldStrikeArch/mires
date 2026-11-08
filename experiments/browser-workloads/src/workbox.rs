//! Service-worker lifecycle workload inspired by Workbox.

use mires_js::{await_promise, callback1, future_to_promise, invoke, JsValue};

use crate::host::{array, box_value, call, get, global_this, index, length, push};

const CACHE_PREFIX: &str = "browser-workloads";
const PRECACHE: [&str; 2] = ["/asset-a.txt", "/asset-b.txt"];

/// Registers install, activate, fetch, and message handlers in a service worker.
pub fn register() {
    let scope = global_this();
    let install = callback1(move |event| {
        let pending = future_to_promise(install_cache());
        let _ = call(event, "waitUntil", &[pending]);
        JsValue::undefined()
    });
    let activate = callback1(move |event| {
        let pending = future_to_promise(activate_cache());
        let _ = call(event, "waitUntil", &[pending]);
        JsValue::undefined()
    });
    let fetch = callback1(move |event| {
        let request = get(event, "request");
        if get(request, "method").is_string() && get(request, "method").read::<&str>() == "GET" {
            let pending = future_to_promise(cache_first(request));
            let _ = call(event, "respondWith", &[pending]);
        }
        JsValue::undefined()
    });
    let message = callback1(move |event| {
        let data = get(event, "data");
        if get(data, "type").is_string() && get(data, "type").read::<&str>() == "SKIP_WAITING" {
            let _ = call(scope, "skipWaiting", &[]);
        }
        JsValue::undefined()
    });
    let _ = call(
        scope,
        "addEventListener",
        &[JsValue::erase("install"), install],
    );
    let _ = call(
        scope,
        "addEventListener",
        &[JsValue::erase("activate"), activate],
    );
    let _ = call(scope, "addEventListener", &[JsValue::erase("fetch"), fetch]);
    let _ = call(
        scope,
        "addEventListener",
        &[JsValue::erase("message"), message],
    );
}

async fn install_cache() -> Result<JsValue, JsValue> {
    let cache = await_promise(call(get(global_this(), "caches"), "open", &[cache_name()])).await?;
    let urls = array();
    for url in PRECACHE {
        push(urls, JsValue::erase(url));
    }
    await_promise(call(cache, "addAll", &[urls])).await
}

async fn activate_cache() -> Result<JsValue, JsValue> {
    let caches = get(global_this(), "caches");
    let current = cache_name();
    let names = await_promise(call(caches, "keys", &[])).await?;
    for name_index in 0..length(names) {
        let name = index(names, name_index);
        let managed = name.is_string()
            && call(
                box_value(name),
                "startsWith",
                &[JsValue::erase(CACHE_PREFIX)],
            )
            .is_truthy();
        let same = name.is_string()
            && current.is_string()
            && name.read::<&str>() == current.read::<&str>();
        if managed && !same {
            let _ = await_promise(call(caches, "delete", &[name])).await?;
        }
    }
    let clients = get(global_this(), "clients");
    await_promise(call(clients, "claim", &[])).await
}

async fn cache_first(request: JsValue) -> Result<JsValue, JsValue> {
    let caches = get(global_this(), "caches");
    let cached = await_promise(call(caches, "match", &[request])).await?;
    if !cached.is_undefined() {
        return Ok(cached);
    }
    let fetch = get(global_this(), "fetch");
    let response = await_promise(invoke(fetch, global_this(), &[request])).await?;
    if get(response, "ok").is_truthy() {
        let cache = await_promise(call(caches, "open", &[cache_name()])).await?;
        let clone = call(response, "clone", &[]);
        let _ = await_promise(call(cache, "put", &[request, clone])).await?;
    }
    Ok(response)
}

fn cache_name() -> JsValue {
    let location = get(global_this(), "location");
    let search = get(location, "search");
    if search.is_string() {
        call(box_value(JsValue::erase(CACHE_PREFIX)), "concat", &[search])
    } else {
        JsValue::erase(CACHE_PREFIX)
    }
}
