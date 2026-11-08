//! Fetch retry and timeout workload inspired by Ky.

use alloc::string::{String, ToString};

use mires_js::{await_promise, callback0, construct, future_to_promise, invoke, try_host, JsValue};

use crate::host::{call, clear_timeout, get, global_this, object, set, set_timeout, wait};

/// Fetches a URL with timeout, retry, and per-attempt headers.
#[must_use]
pub fn get_with_retry(options: JsValue) -> JsValue {
    future_to_promise(async move {
        let url = get(options, "url");
        let attempts = get(options, "attempts");
        let timeout = get(options, "timeout");
        let attempts = if attempts.is_number() { attempts.read::<f64>() as u32 } else { 3 };
        let timeout = if timeout.is_number() { timeout.read() } else { 2_000.0 };
        fetch_loop(url, attempts.max(1), timeout)
            .await
            .map(|result| JsValue::erase(result.as_str()))
    })
}

async fn fetch_loop(url: JsValue, attempts: u32, timeout: f64) -> Result<String, JsValue> {
    let mut last_error = JsValue::erase("fetch failed");
    for attempt in 1..=attempts {
        match fetch_once(url, attempt, timeout).await {
            Ok((status, body)) if status < 500 || attempt == attempts => {
                let mut result = status.to_string();
                result.push('|');
                result.push_str(&body);
                result.push('|');
                result.push_str(&attempt.to_string());
                return Ok(result);
            }
            Ok((_status, _body)) => {}
            Err(error) if attempt == attempts => return Err(error),
            Err(error) => last_error = error,
        }
        let _ = await_promise(wait(f64::from(attempt * 8))).await?;
    }
    Err(last_error)
}

async fn fetch_once(url: JsValue, attempt: u32, timeout: f64) -> Result<(u32, String), JsValue> {
    let controller = construct(get(global_this(), "AbortController"), &[]);
    let abort = callback0(move || {
        let _ = call(controller, "abort", &[]);
        JsValue::undefined()
    });
    let timer = set_timeout(abort, timeout);
    let headers = object();
    set(headers, "accept", JsValue::erase("application/json, text/plain, */*"));
    set(headers, "x-workload-attempt", JsValue::erase(f64::from(attempt)));
    let init = object();
    set(init, "method", JsValue::erase("GET"));
    set(init, "headers", headers);
    set(init, "signal", get(controller, "signal"));
    let fetch = get(global_this(), "fetch");
    let pending = try_host(move || invoke(fetch, global_this(), &[url, init]));
    let response = match pending {
        Ok(value) => await_promise(value).await,
        Err(error) => Err(error),
    };
    clear_timeout(timer);
    let response = response?;
    let status = get(response, "status").read::<f64>() as u32;
    let text = await_promise(call(response, "text", &[])).await?;
    Ok((status, if text.is_string() { text.read::<&str>().to_string() } else { String::new() }))
}
