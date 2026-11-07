//! Shared header for the executor suite: the host interface every fixture drives.
//!
//! Each fixture is an ordinary `#![no_std]` crate compiled against the sysroot, linked with
//! `view-abi` and `view-async`, and driven by `scripts/async-check.mjs`, which installs the
//! globals declared here and asserts what the executor did with them.
//!
//! The interface is declared with `#[js_extern]` rather than as `extern "C"` shim members, so a
//! fixture reaches the host the way an island does and the driver can define the globals in plain
//! JavaScript.

#![allow(dead_code)]

use js_extern_macro::js_extern;
use view_abi::JsValue;

#[js_extern]
unsafe extern "C" {
    /// Records one step, in order. The whole assertion surface: a fixture says what happened and
    /// the driver checks the sequence.
    #[js(call = "note")]
    pub fn note(step: &str);

    /// A Promise that resolves to `value`.
    #[js(call = "resolved")]
    pub fn resolved(value: &str) -> JsValue;

    /// A Promise that rejects with `reason`.
    #[js(call = "rejected")]
    pub fn rejected(reason: &str) -> JsValue;

    /// A Promise nothing resolves until the driver calls `settle`.
    #[js(call = "pending")]
    pub fn pending() -> JsValue;

    /// A plain value that is not a thenable at all.
    #[js(call = "plain")]
    pub fn plain(value: &str) -> JsValue;

    /// The owner the caller is running under, recorded under `tag` so the driver can compare two
    /// of them. `view_abi::owner` answers the same thing; this records it.
    #[js(call = "noteOwner")]
    pub fn note_owner(tag: &str, owner: view_abi::Owner);
}

pub fn say(step: &str) {
    note(step)
}
