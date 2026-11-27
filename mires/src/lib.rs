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
    abort, await_promise, callback0, callback1, callback2, callback3, callback4, collections,
    construct, future_to_promise, invoke, js_bindings, js_export, js_extern, spawn_local,
    throw_value, try_host, web, yield_task, HostValue, JsArray, JsString, JsUint8Array, JsValue,
    Promise, PromiseFuture, YieldTask,
};

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;
    use core::fmt::Write as _;

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
}
