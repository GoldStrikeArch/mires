//! Ergonomic user-facing facade crate for `mires` Rust-to-JavaScript programs.
//!
//! `mires` re-exports the export/binding macros, inline `js!{}` macro, host value handles,
//! host-backed collections, web APIs, and [`JsError`] from a single dependency that works in both
//! `#![no_std]` + `alloc` crates and `std` crates (via `mires-std`), as well as native host
//! `cargo test` runs.
//!
//! # Example
//!
//! ```
//! use mires::prelude::*;
//!
//! #[js_export(name = "greetUser")]
//! pub fn greet_user(name: &str) -> Result<String, JsError> {
//!     if name.is_empty() {
//!         return Err(JsError::new("name must not be empty"));
//!     }
//!     Ok(format!("hello, {name}!"))
//! }
//!
//! assert_eq!(greet_user("mires").unwrap(), "hello, mires!");
//! assert_eq!(greet_user("").unwrap_err().message(), "name must not be empty");
//! ```

#![no_std]

extern crate alloc;

mod error;
pub mod prelude;

pub use error::JsError;
pub use js_macro::js;
pub use mires_js::{
    abort, await_promise, callback0, callback1, callback2, callback3, callback4, class_handle,
    collections, construct, future_to_promise, invoke, js_bindings, js_export, js_extern,
    spawn_local, throw_value, try_host, web, yield_task, HostValue, JsArray, JsBTreeMapEntries,
    JsMapEntries, JsString, JsUint8Array, JsValue, Promise, PromiseFuture, TypedPromise, YieldTask,
};

/// Internal runtime support items referenced by `#[js_export]` macro expansions.
#[doc(hidden)]
pub use mires_js::__export;

#[cfg(test)]
extern crate self as mires;

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use alloc::format;
    use alloc::string::String;
    use core::fmt::Write as _;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use crate::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[js_export]
    fn add_points(a: Point, b: Point) -> Point {
        Point {
            x: a.x + b.x,
            y: a.y + b.y,
        }
    }

    #[js_export(name = "parseMarkdown")]
    fn parse_markdown(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(format!("<p>{trimmed}</p>"))
        }
    }

    #[js_export(raw)]
    fn raw_double(x: u32) -> u32 {
        x * 2
    }

    #[js_export(name = "rawSum", raw)]
    fn raw_sum(a: i32, b: i32) -> i32 {
        a + b
    }

    #[js_export(raw, name = "rawDiff")]
    fn raw_diff(a: i32, b: i32) -> i32 {
        a - b
    }

    #[js_export(name = "divideChecked")]
    fn divide_checked(a: i32, b: i32) -> Result<i32, JsError> {
        if b == 0 {
            return Err("division by zero".into());
        }
        Ok(a / b)
    }

    #[js_export(name = "asyncGreet")]
    async fn async_greet(name: &str) -> Result<String, JsError> {
        if name.is_empty() {
            return Err(JsError::new("empty name"));
        }
        Ok(format!("hello async, {name}!"))
    }

    #[js_export]
    async fn async_unit_ping() {}

    #[js_export(name = "sumBTreeScores")]
    fn sum_btree_scores(scores: BTreeMap<String, i32>, bonus: i32) -> i32 {
        scores.values().copied().sum::<i32>() + bonus
    }

    #[js_export(name = "sumHostMap")]
    fn sum_host_map(scores: crate::collections::HashMap<String, i32>) -> i32 {
        scores.iter().map(|(_, v)| *v).sum()
    }

    #[js_export]
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Counter {
        value: i32,
        label: String,
    }

    #[js_export]
    impl Counter {
        pub fn new(start: i32, label: &str) -> Self {
            Self {
                value: start,
                label: String::from(label),
            }
        }

        pub fn try_positive(start: i32) -> Result<Self, JsError> {
            if start < 0 {
                return Err(JsError::new("negative start"));
            }
            Ok(Self::new(start, "positive"))
        }

        #[js_export(name = "getValue")]
        pub fn value(&self) -> i32 {
            self.value
        }

        pub fn add(&mut self, delta: i32) -> i32 {
            self.value += delta;
            self.value
        }

        pub fn add_all(&mut self, deltas: BTreeMap<String, i32>) -> i32 {
            for delta in deltas.values() {
                self.value += *delta;
            }
            self.value
        }

        pub fn into_summary(self) -> String {
            format!("{}: {}", self.label, self.value)
        }

        pub fn default_step() -> i32 {
            1
        }

        fn helper_double(&self) -> i32 {
            self.value * 2
        }
    }

    // Two separate modules exporting identical symbol names verify that native host `cargo test`
    // builds emit no `no_mangle` / `export_name` attributes and therefore never collide.
    #[cfg(not(target_arch = "wasm32"))]
    mod first_module {
        use crate::js_export;

        #[js_export]
        pub fn collide() -> u32 {
            1
        }

        #[js_export(name = "sharedExportName")]
        pub fn named_collide() -> u32 {
            10
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod second_module {
        use crate::js_export;

        #[js_export]
        pub fn collide() -> u32 {
            2
        }

        #[js_export(name = "sharedExportName")]
        pub fn named_collide() -> u32 {
            20
        }
    }

    js_bindings! {
        #[js(call = "Math.max")]
        fn host_only_max(a: f64, b: f64) -> f64;
    }

    #[js_extern]
    unsafe extern "C" {
        #[js(call = "Math.min")]
        fn host_only_min(a: f64, b: f64) -> f64;
    }

    #[test]
    fn js_error_construction_display_and_conversions() {
        let from_new = JsError::new("something went wrong");
        assert_eq!(from_new.message(), "something went wrong");
        assert_eq!(format!("{from_new}"), "something went wrong");

        let from_str: JsError = "bad input".into();
        assert_eq!(from_str.message(), "bad input");

        let owned = String::from("owned error");
        let from_string: JsError = owned.into();
        assert_eq!(from_string.message(), "owned error");

        let as_dyn_error: &dyn core::error::Error = &from_string;
        assert_eq!(format!("{as_dyn_error}"), "owned error");

        let cloned = from_string.clone();
        assert_eq!(cloned, from_string);
        assert_eq!(String::from(cloned), "owned error");

        struct FailWriter;
        impl core::fmt::Write for FailWriter {
            fn write_str(&mut self, _: &str) -> core::fmt::Result {
                Err(core::fmt::Error)
            }
        }
        let fmt_err: Result<(), JsError> = write!(FailWriter, "x").map_err(JsError::from);
        let err = fmt_err.unwrap_err();
        assert!(!err.message().is_empty());
    }

    #[test]
    #[should_panic(expected = "thrown from host test")]
    fn js_error_throw_panics_with_message_on_host() {
        JsError::new("thrown from host test").throw();
    }

    #[test]
    fn js_export_functions_run_natively_in_cargo_test() {
        assert_eq!(
            add_points(Point { x: 1, y: 2 }, Point { x: 3, y: 4 }),
            Point { x: 4, y: 6 }
        );
        assert_eq!(parse_markdown("  hi  "), Some(String::from("<p>hi</p>")));
        assert_eq!(parse_markdown("   "), None);
        assert_eq!(raw_double(21), 42);
        assert_eq!(raw_sum(10, 5), 15);
        assert_eq!(raw_diff(10, 5), 5);
        assert_eq!(divide_checked(10, 2), Ok(5));
        assert_eq!(divide_checked(10, 0), Err(JsError::new("division by zero")));
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert_eq!(first_module::collide(), 1);
            assert_eq!(second_module::collide(), 2);
            assert_eq!(first_module::named_collide(), 10);
            assert_eq!(second_module::named_collide(), 20);
        }
    }

    #[test]
    #[should_panic(
        expected = "mires JS binding 'host_only_max' is only available on the mires JS target"
    )]
    fn js_bindings_panic_cleanly_when_called_on_host() {
        let _ = host_only_max(1.0, 2.0);
    }

    #[test]
    #[should_panic(
        expected = "mires JS binding 'host_only_min' is only available on the mires JS target"
    )]
    fn js_extern_panics_cleanly_when_called_on_host() {
        let _ = host_only_min(1.0, 2.0);
    }

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("expected test future to resolve immediately"),
        }
    }

    #[test]
    fn js_export_v3_async_map_and_class_run_natively_on_host() {
        assert_eq!(
            poll_ready(async_greet("mires")),
            Ok(String::from("hello async, mires!"))
        );
        assert_eq!(poll_ready(async_greet("")), Err(JsError::new("empty name")));
        poll_ready(async_unit_ping());

        let mut scores = BTreeMap::new();
        scores.insert(String::from("alice"), 10);
        scores.insert(String::from("bob"), 25);
        assert_eq!(sum_btree_scores(scores, 5), 40);

        let mut counter = Counter::new(10, "hits");
        assert_eq!(counter.value(), 10);
        assert_eq!(counter.helper_double(), 20);
        assert_eq!(counter.add(5), 15);
        let mut deltas = BTreeMap::new();
        deltas.insert(String::from("bonus"), 7);
        assert_eq!(counter.add_all(deltas), 22);
        assert_eq!(Counter::default_step(), 1);
        assert_eq!(counter.into_summary(), "hits: 22");

        assert!(Counter::try_positive(-1).is_err());
        assert_eq!(Counter::try_positive(3).unwrap().value(), 3);

        // Keep `sum_host_map` referenced as a type-checked function item on host.
        let _check_fn: fn(crate::collections::HashMap<String, i32>) -> i32 = sum_host_map;

        let mut safe_box = forbid_unsafe_v3::SafeBox::new(42);
        assert_eq!(safe_box.get(), 42);
        safe_box.set(99);
        assert_eq!(safe_box.take(), 99);
        assert_eq!(
            poll_ready(forbid_unsafe_v3::forbid_unsafe_async("hi", BTreeMap::new())),
            2
        );
    }

    #[forbid(unsafe_code)]
    mod forbid_unsafe_v3 {
        use alloc::collections::BTreeMap;
        use alloc::string::String;

        use crate::js_export;

        #[js_export]
        pub struct SafeBox {
            inner: i32,
        }

        #[js_export]
        impl SafeBox {
            pub fn new(inner: i32) -> Self {
                Self { inner }
            }

            pub fn get(&self) -> i32 {
                self.inner
            }

            pub fn set(&mut self, next: i32) {
                self.inner = next;
            }

            pub fn take(self) -> i32 {
                self.inner
            }
        }

        #[js_export(name = "forbidUnsafeAsync")]
        pub async fn forbid_unsafe_async(msg: &str, tags: BTreeMap<String, u32>) -> usize {
            msg.len() + tags.len()
        }
    }
}
