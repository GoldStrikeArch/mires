//! Service-worker lifecycle workload built from native promise chains.

use mires_js::{JsValue, callback1};

use crate::host::{array, call, get, global_this, push};

const CACHE_NAME: &str = "browser-workloads-current";
/// Registers the same lifecycle as `workbox`, without Rust async frames.
pub fn register() {
    let scope = global_this();
    let install = callback1(move |event| {
        let caches = get(scope, "caches");
        let opened = call(caches, "open", &[cache_name()]);
        let populate = callback1(move |cache| call(cache, "addAll", &[precache_urls()]));
        let populated = call(opened, "then", &[populate]);
        let advance = callback1(move |_value| call(scope, "skipWaiting", &[]));
        let _ = call(event, "waitUntil", &[call(populated, "then", &[advance])]);
        JsValue::undefined()
    });
    let activate = callback1(move |event| {
        let caches = get(scope, "caches");
        let cleaned = call(
            caches,
            "delete",
            &[JsValue::erase("browser-workloads-stale")],
        );
        let claim = callback1(move |_value| call(get(scope, "clients"), "claim", &[]));
        let _ = call(event, "waitUntil", &[call(cleaned, "then", &[claim])]);
        JsValue::undefined()
    });
    let fetch = callback1(move |event| {
        let request = get(event, "request");
        let caches = get(scope, "caches");
        let resolve = callback1(move |cached| {
            if cached.is_undefined() {
                call(scope, "fetch", &[request])
            } else {
                cached
            }
        });
        let pending = call(call(caches, "match", &[request]), "then", &[resolve]);
        let _ = call(event, "respondWith", &[pending]);
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

fn precache_urls() -> JsValue {
    let urls = array();
    push(urls, JsValue::erase("/asset-a.txt"));
    push(urls, JsValue::erase("/asset-b.txt"));
    urls
}

fn cache_name() -> JsValue {
    JsValue::erase(CACHE_NAME)
}
