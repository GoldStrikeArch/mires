#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;
use alloc::vec::Vec;
use mires_js::{
    JsValue as V, await_promise, future_to_promise, js_bindings, js_export, throw_value, try_host,
};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    throw_value(s("Workbox port panicked"))
}

js_bindings! {
    #[js(global,get="globalThis")] fn global()->V;
    #[js(call="Reflect.get")] fn prop(v:V,k:V)->V;
    #[js(call="Reflect.set")] fn assign(v:V,k:V,x:V)->bool;
    #[js(index)] fn at(v:V,i:usize)->V;
    #[js(get="length")] fn len(v:V)->usize;
    #[js(new="Array")] fn array()->V;
    #[js(new="Object")] fn object()->V;
    #[js(new="Request")] fn new_request(path:V,init:V)->V;
    #[js(new="URL")] fn url(v:V,base:V)->V;
    #[js(call="Array.from")] fn collect(v:V)->V;
    #[js(method="keys")] fn keys(v:V)->V;
    #[js(method="push")] fn push(v:V,x:V)->usize;
    #[js(method="slice")] fn suffix(v:V,i:usize)->V;
    #[js(method="set")] fn param(v:V,k:V,w:V);
    #[js(method="delete")] fn delete_param(v:V,k:V);
    #[js(method="sort")] fn sort(v:V)->V;
    #[js(method="randomUUID")] fn uuid(v:V)->V;
    #[js(method="open")] fn open_cache(v:V,k:V)->V;
    #[js(method="delete")] fn delete(v:V,k:V)->V;
    #[js(method="match")] fn cached(v:V,k:V)->V;
    #[js(method="put")] fn store(v:V,k:V,r:V)->V;
    #[js(method="text")] fn body(v:V)->V;
    #[js(method="clone")] fn copy(v:V)->V;
    #[js(method="fetch")] fn fetch(v:V,request:V)->V;
    #[js(method="skipWaiting")] fn skip(v:V)->V;
}

type Answer = Result<V, V>;
fn s(x: &str) -> V {
    V::erase(x)
}
fn field(v: V, k: &str) -> V {
    prop(v, s(k))
}
fn equal(a: V, b: &str) -> bool {
    a.strictly_equals(s(b))
}
fn pair(a: V, b: V) -> V {
    let out = array();
    push(out, a);
    push(out, b);
    out
}
fn fallback(v: V, default: V) -> V {
    if v.is_undefined() { default } else { v }
}
fn base() -> V {
    field(
        url(
            s("experiments/library-matrix/libraries/workbox/"),
            field(field(global(), "location"), "href"),
        ),
        "href",
    )
}
fn absolute(path: V, base: V) -> V {
    field(url(path, base), "href")
}
fn relative(v: V, base: V) -> V {
    suffix(v, len(base))
}
fn caches() -> V {
    field(global(), "caches")
}
struct Key {
    original: V,
    revisioned: V,
}
fn manifest(data: V, base: V) -> Result<Vec<Key>, V> {
    let mut entries: Vec<Key> = Vec::new();
    for i in 0..len(data) {
        let item = at(data, i);
        let original = absolute(field(item, "path"), base);
        let key = url(original, base);
        let rev = field(item, "revision");
        if rev.is_truthy() {
            param(field(key, "searchParams"), s("__WB_REVISION__"), rev);
        }
        let revisioned = field(key, "href");
        if let Some(previous) = entries
            .iter()
            .find(|x| x.original.strictly_equals(original))
        {
            if !previous.revisioned.strictly_equals(revisioned) {
                return Err(s("conflicting revisions"));
            }
        } else {
            entries.push(Key {
                original,
                revisioned,
            });
        }
    }
    Ok(entries)
}
fn lookup(keys: &[Key], url: V) -> Option<V> {
    let value = field(url, "href");
    keys.iter()
        .find(|key| key.original.strictly_equals(value))
        .map(|key| key.revisioned)
}
fn precache_key(entries: &[Key], path: V, base: V) -> Option<V> {
    let u = url(path, base);
    assign(u, s("hash"), s(""));
    if let Some(v) = lookup(entries, u) {
        return Some(v);
    }
    let query = field(u, "searchParams");
    let params = collect(keys(query));
    for i in 0..len(params) {
        let name = at(params, i);
        if equal(name, "fbclid") || name.read::<&str>().starts_with("utm_") {
            delete_param(query, name);
        }
    }
    if let Some(v) = lookup(entries, u) {
        return Some(v);
    }
    let pathname = field(u, "pathname");
    if pathname.read::<&str>().ends_with('/') {
        assign(u, s("pathname"), pathname + s("index.html"));
        if let Some(v) = lookup(entries, u) {
            return Some(v);
        }
    }
    assign(u, s("pathname"), pathname + s(".html"));
    lookup(entries, u)
}
async fn network(source: V, base: V) -> Answer {
    if equal(source, "offline") {
        return Err(s("offline"));
    }
    await_promise(fetch(global(), absolute(source + s(".mjs"), base))).await
}
async fn phase(step: V, name: V, items: V, base: V) -> Answer {
    let entries = manifest(items, base)?;
    let op = field(step, "phase");
    let source = fallback(field(step, "source"), s("alpha"));
    let cache = await_promise(open_cache(caches(), name)).await?;
    if equal(op, "install") {
        let updated = array();
        let unchanged = array();
        for key in &entries {
            let existing = await_promise(cached(cache, key.revisioned)).await?;
            let list = if existing.is_undefined() {
                let res = network(source, base).await?;
                if field(res, "status").read::<f64>() >= 400.0 {
                    return Err(s("bad-precaching-response"));
                }
                await_promise(store(cache, key.revisioned, copy(res))).await?;
                updated
            } else {
                unchanged
            };
            push(list, relative(key.original, base));
        }
        return Ok(pair(updated, unchanged));
    }
    if equal(op, "activate") {
        let dropped = array();
        let requests = await_promise(keys(cache)).await?;
        for i in 0..len(requests) {
            let path = field(at(requests, i), "url");
            if !entries.iter().any(|k| k.revisioned.strictly_equals(path)) {
                await_promise(delete(cache, path)).await?;
                push(dropped, relative(path, base));
            }
        }
        return Ok(sort(dropped));
    }
    if equal(op, "message") {
        let matched = equal(field(step, "message"), "SKIP_WAITING");
        if matched {
            await_promise(skip(global())).await?;
        }
        return Ok(V::erase(matched));
    }
    let request = field(step, "request");
    let request = if request.is_truthy() {
        request
    } else {
        let init = object();
        let method = field(step, "method");
        if method.is_truthy() {
            assign(init, s("method"), method);
        }
        try_host(|| new_request(absolute(field(step, "path"), base), init))?
    };
    let path = field(request, "url");
    let method = field(request, "method");
    if !equal(method, "GET") {
        return Ok(V::null());
    }
    let result = if let Some(key) = precache_key(&entries, path, base) {
        let hit = await_promise(cached(cache, key)).await?;
        if hit.is_undefined() {
            network(source, base).await
        } else {
            Ok(hit)
        }
    } else {
        let rel = relative(path, base).read::<&str>();
        if rel.starts_with("network/") {
            network(source, base).await
        } else if rel.starts_with("only/") || rel.starts_with("first/") {
            let runtime = await_promise(open_cache(caches(), name + s("-runtime"))).await?;
            let hit = await_promise(cached(runtime, path)).await?;
            if !hit.is_undefined() {
                Ok(hit)
            } else if rel.starts_with("only/") {
                Err(s("no-response"))
            } else {
                match network(source, base).await {
                    Ok(res) => {
                        if field(res, "status").read::<f64>() == 200.0 {
                            await_promise(store(runtime, path, copy(res))).await?;
                        }
                        Ok(res)
                    }
                    Err(e) => Err(e),
                }
            }
        } else {
            return Ok(V::null());
        }
    };
    match result {
        Ok(res) => Ok(pair(field(res, "status"), await_promise(body(res)).await?)),
        Err(_) => {
            let failed = array();
            push(failed, s("no-response"));
            Ok(failed)
        }
    }
}
async fn scenario(input: V, name: V, base: V) -> Answer {
    let steps = field(input, "steps");
    let mut items = array();
    let result = array();
    for i in 0..len(steps) {
        let step = at(steps, i);
        items = fallback(field(step, "manifest"), items);
        push(result, phase(step, name, items, base).await?);
    }
    Ok(result)
}
#[js_export]
pub fn matrix_run(input: V) -> V {
    future_to_promise(async move {
        let base = base();
        if !field(input, "phase").is_undefined() {
            return phase(input, field(input, "name"), field(input, "manifest"), base).await;
        }
        let name = s("mires-workbox-") + uuid(field(global(), "crypto"));
        let result = scenario(input, name, base).await;
        await_promise(delete(caches(), name)).await?;
        await_promise(delete(caches(), name + s("-runtime"))).await?;
        result
    })
}
