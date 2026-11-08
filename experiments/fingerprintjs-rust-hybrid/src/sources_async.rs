//! FingerprintJS v5.2.0 sources that depend on Promises or browser callbacks.

use alloc::rc::Rc;
#[cfg(test)]
use alloc::string::{String, ToString};
#[cfg(test)]
use alloc::vec::Vec;
use core::cell::RefCell;

use mires_js::{
    await_promise, callback0, callback1, construct, future_to_promise, invoke, throw_value,
    try_host, JsValue,
};

use crate::host_async::{instance_of, promise, settle};
use crate::{
    array, call, date_now, document, get, global_this, index, is_chromium,
    is_chromium_122_or_newer, is_desktop_webkit, is_promise, is_safari_webkit, is_samsung_internet,
    is_webkit, is_webkit_606_or_newer, is_webkit_616_or_newer, length, navigator, new_error,
    object, push, set, set_timeout, suppress_unhandled_rejection, window,
};

const AUDIO_KNOWN_FOR_SUSPENDING: i32 = -1;
const AUDIO_NOT_SUPPORTED: i32 = -2;
const AUDIO_TIMEOUT: i32 = -3;
const AUDIO_KNOWN_FOR_ANTIFINGERPRINTING: i32 = -4;

const HASH_FROM_INDEX: usize = 4_500;
const HASH_TO_INDEX: usize = 5_000;
const RENDER_TRY_MAX_COUNT: u8 = 3;
const RENDER_RETRY_DELAY: f64 = 500.0;
const RUNNING_MAX_AWAIT_TIME: f64 = 500.0;
const RUNNING_SUFFICIENT_TIME: f64 = 5_000.0;

/// Loads the User-Agent Client Hints source.
#[must_use]
pub fn source_user_agent_data(options: JsValue) -> JsValue {
    let _ = options;
    future_to_promise(collect_user_agent_data())
}

async fn collect_user_agent_data() -> Result<JsValue, JsValue> {
    let ua_data = get(navigator(), "userAgentData");
    if !ua_data.is_truthy() {
        return Ok(JsValue::undefined());
    }

    let raw_brands = get(ua_data, "brands");
    let not_pattern = construct(
        get(global_this(), "RegExp"),
        &[JsValue::erase("not"), JsValue::erase("i")],
    );
    let filtered_brands = array();
    for brand_index in 0..length(raw_brands) {
        let brand = get(index(raw_brands, brand_index), "brand");
        if !call(not_pattern, "test", &[brand]).read::<bool>() {
            push(filtered_brands, brand);
        }
    }

    let brands = if length(filtered_brands) > 1 {
        let without_chromium = array();
        for brand_index in 0..length(filtered_brands) {
            let brand = index(filtered_brands, brand_index);
            if !brand.strictly_equals(JsValue::erase("Chromium")) {
                push(without_chromium, brand);
            }
        }
        without_chromium
    } else {
        filtered_brands
    };

    let result = object();
    set(result, "brands", brands);
    set(result, "mobile", get(ua_data, "mobile"));
    set(result, "platform", get(ua_data, "platform"));

    let high_entropy_function = get(ua_data, "getHighEntropyValues");
    if !high_entropy_function.is_truthy() {
        return Ok(result);
    }

    let hints = array();
    for hint in ["architecture", "bitness", "model", "platformVersion"] {
        push(hints, JsValue::erase(hint));
    }
    let called = try_host(move || invoke(high_entropy_function, ua_data, &[hints]));
    let high_entropy = match called {
        Ok(value) => await_promise(value).await,
        Err(error) => Err(error),
    };

    match high_entropy {
        Ok(high_entropy) => {
            let applied = try_host(move || {
                for name in ["architecture", "bitness", "model", "platformVersion"] {
                    set(result, name, get(high_entropy, name));
                }
                JsValue::undefined()
            });
            if let Err(error) = applied {
                handle_high_entropy_error(result, error)?;
            }
        }
        Err(error) => handle_high_entropy_error(result, error)?,
    }

    Ok(result)
}

fn handle_high_entropy_error(result: JsValue, error: JsValue) -> Result<(), JsValue> {
    let is_not_allowed = try_host(move || {
        let constructor = get(window(), "DOMException");
        let matches = instance_of(error, constructor)
            && get(error, "name").read::<&str>() == "NotAllowedError";
        JsValue::erase(matches)
    })?;
    if !is_not_allowed.read::<bool>() {
        return Err(error);
    }

    try_host(move || {
        set(result, "highEntropyStatus", JsValue::erase("not_allowed"));
        JsValue::undefined()
    })?;
    Ok(())
}

#[cfg(test)]
fn normalize_brands(brands: &[String]) -> Vec<String> {
    let mut filtered = brands
        .iter()
        .filter(|brand| !brand.to_ascii_lowercase().contains("not"))
        .cloned()
        .collect::<Vec<_>>();
    if filtered.len() > 1 {
        filtered.retain(|brand| brand != "Chromium");
    }
    filtered
}

/// Loads the stabilized audio source.
#[must_use]
pub fn source_audio(options: JsValue) -> JsValue {
    let _ = options;
    if current_browser_performs_antifingerprinting() {
        JsValue::erase(AUDIO_KNOWN_FOR_ANTIFINGERPRINTING)
    } else {
        get_unstable_audio_fingerprint()
    }
}

/// Returns the unstabilized two-stage audio source value.
#[must_use]
pub fn get_unstable_audio_fingerprint() -> JsValue {
    let host_window = window();
    let audio_context = get(host_window, "OfflineAudioContext");
    let webkit_audio_context = get(host_window, "webkitOfflineAudioContext");
    if !audio_context.is_truthy() && !webkit_audio_context.is_truthy() {
        return JsValue::erase(AUDIO_NOT_SUPPORTED);
    }
    if current_browser_suspends_audio_context() {
        return JsValue::erase(AUDIO_KNOWN_FOR_SUSPENDING);
    }

    let context = crate::create_offline_audio_context(1.0, HASH_TO_INDEX as f64, 44_100.0);
    crate::configure_audio_graph(context);

    let (render_promise, finish_rendering) = start_rendering_audio(context);
    let on_rendered = callback1(move |buffer| {
        let channel = call(buffer, "getChannelData", &[JsValue::erase(0.0_f64)]);
        let signal = call(channel, "subarray", &[JsValue::erase(HASH_FROM_INDEX)]);
        JsValue::erase(crate::hash_audio_signal(signal))
    });
    let on_render_error = callback1(move |error| {
        let name = get(error, "name");
        if name.is_string() && matches!(name.read::<&str>(), "timeout" | "suspended") {
            JsValue::erase(AUDIO_TIMEOUT)
        } else {
            throw_value(error)
        }
    });
    let fingerprint_promise = call(render_promise, "then", &[on_rendered, on_render_error]);
    let fingerprint_promise = suppress_unhandled_rejection(fingerprint_promise);

    callback0(move || {
        let _ = invoke(finish_rendering, JsValue::undefined(), &[]);
        fingerprint_promise
    })
}

struct AudioRenderState {
    context: JsValue,
    resolve: JsValue,
    reject: JsValue,
    finalized: bool,
    render_try_count: u8,
    started_running_at: f64,
}

fn start_rendering_audio(context: JsValue) -> (JsValue, JsValue) {
    let state = Rc::new(RefCell::new(AudioRenderState {
        context,
        resolve: JsValue::undefined(),
        reject: JsValue::undefined(),
        finalized: false,
        render_try_count: 0,
        started_running_at: 0.0,
    }));

    let executor_state = Rc::clone(&state);
    let result = promise(move |resolve, reject| {
        {
            let mut state = executor_state.borrow_mut();
            state.resolve = resolve;
            state.reject = reject;
        }

        let completion_state = Rc::clone(&executor_state);
        let complete = callback1(move |event| {
            let resolve = completion_state.borrow().resolve;
            settle(resolve, get(event, "renderedBuffer"));
            JsValue::undefined()
        });
        set(context, "oncomplete", complete);
        try_render(&executor_state);
        JsValue::undefined()
    });

    let finalize_state = Rc::clone(&state);
    let finalize = callback0(move || {
        let should_start_timeout = {
            let mut state = finalize_state.borrow_mut();
            if state.finalized {
                false
            } else {
                state.finalized = true;
                state.started_running_at > 0.0
            }
        };
        if should_start_timeout {
            start_running_timeout(&finalize_state);
        }
        JsValue::undefined()
    });

    (result, finalize)
}

fn try_render(state: &Rc<RefCell<AudioRenderState>>) {
    let attempt_state = Rc::clone(state);
    let attempt = try_host(move || {
        let context = attempt_state.borrow().context;
        let rendering = call(context, "startRendering", &[]);
        if is_promise(rendering) {
            let _ = suppress_unhandled_rejection(rendering);
        }

        let state_name = get(context, "state");
        if !state_name.is_string() {
            return JsValue::undefined();
        }
        match state_name.read::<&str>() {
            "running" => {
                let finalized = {
                    let mut state = attempt_state.borrow_mut();
                    state.started_running_at = date_now();
                    state.finalized
                };
                if finalized {
                    start_running_timeout(&attempt_state);
                }
            }
            "suspended" => {
                if !get(document(), "hidden").is_truthy() {
                    let mut state = attempt_state.borrow_mut();
                    state.render_try_count = state.render_try_count.saturating_add(1);
                }
                let (finalized, render_try_count) = {
                    let state = attempt_state.borrow();
                    (state.finalized, state.render_try_count)
                };
                if finalized && render_try_count >= RENDER_TRY_MAX_COUNT {
                    reject_inner(&attempt_state, "suspended");
                } else {
                    let retry_state = Rc::clone(&attempt_state);
                    let retry = callback0(move || {
                        try_render(&retry_state);
                        JsValue::undefined()
                    });
                    let _ = set_timeout(retry, RENDER_RETRY_DELAY);
                }
            }
            _ => {}
        }
        JsValue::undefined()
    });
    if let Err(error) = attempt {
        let reject = state.borrow().reject;
        settle(reject, error);
    }
}

fn start_running_timeout(state: &Rc<RefCell<AudioRenderState>>) {
    let delay = {
        let state = state.borrow();
        RUNNING_MAX_AWAIT_TIME.min(state.started_running_at + RUNNING_SUFFICIENT_TIME - date_now())
    };
    let timeout_state = Rc::clone(state);
    let timeout = callback0(move || {
        reject_inner(&timeout_state, "timeout");
        JsValue::undefined()
    });
    let _ = set_timeout(timeout, delay);
}

fn reject_inner(state: &Rc<RefCell<AudioRenderState>>, name: &str) {
    let reject = state.borrow().reject;
    settle(reject, new_error(name, name));
}



#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AudioBrowserFacts {
    webkit: bool,
    desktop_webkit: bool,
    webkit_606_or_newer: bool,
    webkit_616_or_newer: bool,
    safari_webkit: bool,
    chromium: bool,
    samsung_internet: bool,
    chromium_122_or_newer: bool,
}

fn current_browser_suspends_audio_context() -> bool {
    is_webkit() && !is_desktop_webkit() && !is_webkit_606_or_newer()
}

fn current_browser_performs_antifingerprinting() -> bool {
    (is_webkit() && is_webkit_616_or_newer() && is_safari_webkit())
        || (is_chromium() && is_samsung_internet() && is_chromium_122_or_newer())
}

#[cfg(test)]
fn browser_suspends_audio_context(browser: AudioBrowserFacts) -> bool {
    browser.webkit && !browser.desktop_webkit && !browser.webkit_606_or_newer
}

#[cfg(test)]
fn browser_performs_antifingerprinting(browser: AudioBrowserFacts) -> bool {
    (browser.webkit && browser.webkit_616_or_newer && browser.safari_webkit)
        || (browser.chromium && browser.samsung_internet && browser.chromium_122_or_newer)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn normalizes_client_hint_brands() {
        assert_eq!(
            normalize_brands(&[
                "Not_A Brand".to_string(),
                "Chromium".to_string(),
                "Google Chrome".to_string(),
            ]),
            vec!["Google Chrome".to_string()]
        );
        assert_eq!(
            normalize_brands(&["Chromium".to_string()]),
            vec!["Chromium".to_string()]
        );
    }

    #[test]
    fn applies_audio_browser_exclusions() {
        assert!(browser_suspends_audio_context(AudioBrowserFacts {
            webkit: true,
            ..AudioBrowserFacts::default()
        }));
        assert!(browser_performs_antifingerprinting(AudioBrowserFacts {
            webkit: true,
            webkit_616_or_newer: true,
            safari_webkit: true,
            ..AudioBrowserFacts::default()
        }));
        assert!(browser_performs_antifingerprinting(AudioBrowserFacts {
            chromium: true,
            samsung_internet: true,
            chromium_122_or_newer: true,
            ..AudioBrowserFacts::default()
        }));
    }

    #[test]
    fn audio_hash_preserves_ordered_absolute_sum() {
        let signal = [-1.5_f64, 2.0, -0.25, 0.0];
        assert_eq!(signal.into_iter().map(f64::abs).sum::<f64>(), 3.75);
    }
}
