//! Opaque host values and the small dynamic boundary that typed bindings cannot express.

use core::task::Waker;

/// A Rust value whose in-memory view is also a complete JavaScript host value.
///
/// This trait is sealed. It includes primitives, native host handles, and `&str`, but deliberately
/// excludes owning Rust values such as [`alloc::string::String`] and [`alloc::vec::Vec`]. Erasing
/// an owning value would bypass its Rust representation and `Drop` contract.
pub trait HostValue: sealed::Sealed + Copy {
    /// Whether `value` can be read without violating this Rust type's value contract.
    #[doc(hidden)]
    fn accepts(value: JsValue) -> bool;
}

pub(crate) mod sealed {
    pub trait Sealed {}
}

macro_rules! number_values {
    ($($ty:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl HostValue for $ty {
                fn accepts(value: JsValue) -> bool {
                    value.is_number()
                }
            }
        )*
    };
}

number_values!(f32, f64);

macro_rules! integer_values {
    ($($ty:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl HostValue for $ty {
                fn accepts(value: JsValue) -> bool {
                    if !value.is_number() || !host_is_integer(value) {
                        return false;
                    }
                    let number = host_read::<f64>(value);
                    number >= <$ty>::MIN as f64 && number <= <$ty>::MAX as f64
                }
            }
        )*
    };
}

integer_values!(i8, i16, i32, isize, u8, u16, u32, usize);

macro_rules! bigint_values {
    ($($ty:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl HostValue for $ty {
                fn accepts(value: JsValue) -> bool {
                    host_is_bigint(value)
                        && host_bigint_range(
                            value,
                            JsValue::erase(<$ty>::MIN),
                            JsValue::erase(<$ty>::MAX),
                        )
                }
            }
        )*
    };
}

bigint_values!(i64, i128, u64, u128);

impl sealed::Sealed for bool {}

impl HostValue for bool {
    fn accepts(value: JsValue) -> bool {
        value.is_boolean()
    }
}

impl sealed::Sealed for char {}

impl HostValue for char {
    fn accepts(value: JsValue) -> bool {
        if !value.is_number() || !host_is_integer(value) {
            return false;
        }
        let code = host_read::<f64>(value);
        (0.0..=0x10ffff as f64).contains(&code)
            && !((0xd800 as f64)..=(0xdfff as f64)).contains(&code)
    }
}

impl sealed::Sealed for &str {}

impl HostValue for &str {
    fn accepts(value: JsValue) -> bool {
        value.is_string()
    }
}

/// An opaque JavaScript value.
///
/// This is one transparent machine word because the backend represents a host object as the
/// object itself. Rust code can inspect or retype one only through the safe operations below.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsValue {
    #[allow(dead_code)]
    handle: u32,
}

impl sealed::Sealed for JsValue {}
impl HostValue for JsValue {
    fn accepts(_: JsValue) -> bool {
        true
    }
}

/// Uses JavaScript addition, including string concatenation and primitive conversion.
impl core::ops::Add for JsValue {
    type Output = Self;

    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.add")]
    #[inline(never)]
    fn add(self, other: Self) -> Self {
        host_add(self, other)
    }
}

impl JsValue {
    /// JavaScript `undefined`.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.undefined")]
    #[inline(never)]
    pub fn undefined() -> Self {
        host_undefined()
    }

    /// JavaScript `null`.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.null")]
    #[inline(never)]
    pub fn null() -> Self {
        host_null()
    }

    /// Whether this value is `undefined`.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isundefined")]
    #[inline(never)]
    pub fn is_undefined(self) -> bool {
        host_is_undefined(self)
    }

    /// Whether this value is `null`.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isnull")]
    #[inline(never)]
    pub fn is_null(self) -> bool {
        host_is_null(self)
    }

    /// Whether JavaScript considers this value truthy.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.truthy")]
    #[inline(never)]
    pub fn is_truthy(self) -> bool {
        host_is_truthy(self)
    }

    /// Whether this value and `other` are strictly equal.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.stricteq")]
    #[inline(never)]
    pub fn strictly_equals(self, other: Self) -> bool {
        host_strictly_equals(self, other)
    }

    /// JavaScript's `typeof` predicate for booleans.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isboolean")]
    #[inline(never)]
    pub fn is_boolean(self) -> bool {
        host_is_boolean(self)
    }

    /// JavaScript's `typeof` predicate for numbers.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isnumber")]
    #[inline(never)]
    pub fn is_number(self) -> bool {
        host_is_number(self)
    }

    /// JavaScript's `typeof` predicate for big integers.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isbigint")]
    #[inline(never)]
    pub fn is_bigint(self) -> bool {
        host_is_bigint(self)
    }

    /// JavaScript's `typeof` predicate for strings.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isstring")]
    #[inline(never)]
    pub fn is_string(self) -> bool {
        host_is_string(self)
    }

    /// JavaScript's `typeof` predicate for functions.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isfunction")]
    #[inline(never)]
    pub fn is_function(self) -> bool {
        host_is_function(self)
    }

    /// Whether the value is a non-null object.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.isobject")]
    #[inline(never)]
    pub fn is_object(self) -> bool {
        host_is_object(self)
    }

    /// Whether the value is an `Error` in the current realm.
    #[must_use]
    pub fn is_error(self) -> bool {
        host_is_error(self)
    }

    /// Erases the Rust view of a host-compatible value.
    #[must_use]
    #[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.erase")]
    #[inline(never)]
    pub fn erase<T: HostValue>(value: T) -> Self {
        host_erase(value)
    }

    /// Reads this host value as `T`.
    ///
    /// The caller chooses one of the sealed [`HostValue`] representations; typed
    /// [`js_bindings`](crate::js_bindings) should be preferred when the host shape is known
    /// statically.
    #[must_use]
    pub fn read<T: HostValue>(self) -> T {
        match T::accepts(self) {
            true => host_read(self),
            false => host_bad_read(),
        }
    }
}

/// Converts a Rust closure with no arguments into a JavaScript callback.
///
/// The callback transfers its capture environment to the host for the rest of the JavaScript
/// realm. Captured Rust destructors are intentionally not run. Capture primitives and host-owned
/// values only; use a separate explicit cleanup operation for resources that need deterministic
/// release.
#[must_use]
pub fn callback0<F>(f: F) -> JsValue
where
    F: Fn() -> JsValue + 'static,
{
    host_callback0(f)
}

/// Converts a Rust closure with one argument into a JavaScript callback.
///
/// The callback has the same host-owned capture contract as [`callback0`].
#[must_use]
pub fn callback1<F>(f: F) -> JsValue
where
    F: Fn(JsValue) -> JsValue + 'static,
{
    host_callback1(f)
}

/// Converts a Rust closure with two arguments into a JavaScript callback.
///
/// The callback has the same host-owned capture contract as [`callback0`].
#[must_use]
pub fn callback2<F>(f: F) -> JsValue
where
    F: Fn(JsValue, JsValue) -> JsValue + 'static,
{
    host_callback2(f)
}

/// Converts a Rust closure with three arguments into a JavaScript callback.
///
/// The callback has the same host-owned capture contract as [`callback0`].
#[must_use]
pub fn callback3<F>(f: F) -> JsValue
where
    F: Fn(JsValue, JsValue, JsValue) -> JsValue + 'static,
{
    host_callback3(f)
}

/// Converts a Rust closure with four arguments into a JavaScript callback.
///
/// The callback has the same host-owned capture contract as [`callback0`].
#[must_use]
pub fn callback4<F>(f: F) -> JsValue
where
    F: Fn(JsValue, JsValue, JsValue, JsValue) -> JsValue + 'static,
{
    host_callback4(f)
}

/// Calls `function` with an explicit `this` value and positional arguments.
#[must_use]
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.invoke")]
#[inline(never)]
pub fn invoke(function: JsValue, this: JsValue, arguments: &[JsValue]) -> JsValue {
    host_invoke(function, this, arguments)
}

/// Constructs a value dynamically, using `constructor` and positional arguments.
#[must_use]
#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.construct")]
#[inline(never)]
pub fn construct(constructor: JsValue, arguments: &[JsValue]) -> JsValue {
    host_construct(constructor, arguments)
}

/// Runs one host operation and catches a synchronous JavaScript exception.
pub fn try_host<F>(f: F) -> Result<JsValue, JsValue>
where
    F: Fn() -> JsValue,
{
    host_try(f)
}

/// Throws `value` as a synchronous JavaScript exception.
pub fn throw_value(value: JsValue) -> ! {
    host_throw(value)
}

macro_rules! marker {
    ($name:ident($($arg:ident: $ty:ty),* $(,)?) -> $result:ty, $section:literal, $body:block) => {
        #[cfg_attr(target_arch = "wasm32", link_section = $section)]
        #[inline(never)]
        fn $name($($arg: $ty),*) -> $result $body
    };
}

marker!(host_undefined() -> JsValue, "rcgjs.host.undefined", {
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
});
marker!(host_null() -> JsValue, "rcgjs.host.null", {
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
});

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.base64url")]
#[inline(never)]
pub(crate) fn host_base64_url(value: crate::JsUint8Array) -> crate::JsString {
    let _ = value;
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.base64urlnative")]
#[inline(never)]
pub(crate) fn host_base64_url_native(value: crate::JsUint8Array) -> crate::JsString {
    let _ = value;
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.base64urlsupported")]
#[inline(never)]
pub(crate) fn host_base64_url_supported() -> bool {
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}
marker!(
    host_is_undefined(value: JsValue) -> bool,
    "rcgjs.host.isundefined",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(host_is_null(value: JsValue) -> bool, "rcgjs.host.isnull", {
    let _ = value;
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
});
marker!(
    host_is_truthy(value: JsValue) -> bool,
    "rcgjs.host.truthy",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_add(value: JsValue, other: JsValue) -> JsValue,
    "rcgjs.host.add",
    {
        let _ = (value, other);
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_strictly_equals(value: JsValue, other: JsValue) -> bool,
    "rcgjs.host.stricteq",
    {
        let _ = (value, other);
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_boolean(value: JsValue) -> bool,
    "rcgjs.host.isboolean",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_number(value: JsValue) -> bool,
    "rcgjs.host.isnumber",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_string(value: JsValue) -> bool,
    "rcgjs.host.isstring",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_function(value: JsValue) -> bool,
    "rcgjs.host.isfunction",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_object(value: JsValue) -> bool,
    "rcgjs.host.isobject",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_error(value: JsValue) -> bool,
    "rcgjs.host.iserror",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_bigint(value: JsValue) -> bool,
    "rcgjs.host.isbigint",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_is_integer(value: JsValue) -> bool,
    "rcgjs.host.isinteger",
    {
        let _ = value;
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(
    host_bigint_range(value: JsValue, min: JsValue, max: JsValue) -> bool,
    "rcgjs.host.bigintrange",
    {
        let _ = (value, min, max);
        unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
    }
);
marker!(host_bad_read() -> !, "rcgjs.host.badread", {
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
});

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.erase")]
#[inline(never)]
fn host_erase<T: HostValue>(value: T) -> JsValue {
    let _ = value;
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.read")]
#[inline(never)]
fn host_read<T: HostValue>(value: JsValue) -> T {
    let _ = value;
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback0")]
#[inline(never)]
fn host_callback0<F>(f: F) -> JsValue
where
    F: Fn() -> JsValue,
{
    let result = f();
    core::mem::forget(f);
    result
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback1")]
#[inline(never)]
fn host_callback1<F>(f: F) -> JsValue
where
    F: Fn(JsValue) -> JsValue,
{
    let result = f(JsValue::undefined());
    core::mem::forget(f);
    result
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback2")]
#[inline(never)]
fn host_callback2<F>(f: F) -> JsValue
where
    F: Fn(JsValue, JsValue) -> JsValue,
{
    let result = f(JsValue::undefined(), JsValue::undefined());
    core::mem::forget(f);
    result
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback3")]
#[inline(never)]
fn host_callback3<F>(f: F) -> JsValue
where
    F: Fn(JsValue, JsValue, JsValue) -> JsValue,
{
    let result = f(
        JsValue::undefined(),
        JsValue::undefined(),
        JsValue::undefined(),
    );
    core::mem::forget(f);
    result
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.callback4")]
#[inline(never)]
fn host_callback4<F>(f: F) -> JsValue
where
    F: Fn(JsValue, JsValue, JsValue, JsValue) -> JsValue,
{
    let result = f(
        JsValue::undefined(),
        JsValue::undefined(),
        JsValue::undefined(),
        JsValue::undefined(),
    );
    core::mem::forget(f);
    result
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.invoke")]
#[inline(never)]
fn host_invoke(function: JsValue, this: JsValue, arguments: &[JsValue]) -> JsValue {
    let _ = (function, this, arguments);
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.construct")]
#[inline(never)]
fn host_construct(constructor: JsValue, arguments: &[JsValue]) -> JsValue {
    let _ = (constructor, arguments);
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.try")]
#[inline(never)]
fn host_try<F>(f: F) -> Result<JsValue, JsValue>
where
    F: Fn() -> JsValue,
{
    Ok(f())
}

marker!(host_throw(value: JsValue) -> !, "rcgjs.host.throw", {
    let _ = value;
    unreachable!("mires-js host markers are only meaningful to rustc_codegen_js")
});

/// Creates the executor's callback-backed local waker.
pub(crate) fn local_waker<F>(f: F) -> Waker
where
    F: Fn() + 'static,
{
    host_local_waker(f)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.localwaker")]
#[inline(never)]
fn host_local_waker<F>(f: F) -> Waker
where
    F: Fn(),
{
    f();
    Waker::noop().clone()
}

/// Schedules `f` in a microtask.
pub(crate) fn microtask<F>(f: F)
where
    F: Fn(),
{
    host_microtask(f)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.microtask")]
#[inline(never)]
fn host_microtask<F>(f: F)
where
    F: Fn(),
{
    f();
}

/// Calls `f` with a Promise's value and success flag.
pub(crate) fn on_settled<F>(value: JsValue, f: F)
where
    F: Fn(JsValue, bool),
{
    host_on_settled(value, f)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.settled")]
#[inline(never)]
fn host_on_settled<F>(value: JsValue, f: F)
where
    F: Fn(JsValue, bool),
{
    f(value, true);
}

/// Converts a resolver/rejecter registration closure into a Promise.
pub(crate) fn promise_new<F>(f: F) -> JsValue
where
    F: FnOnce(JsValue, JsValue),
{
    host_promise_new(f)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.promise")]
#[inline(never)]
fn host_promise_new<F>(f: F) -> JsValue
where
    F: FnOnce(JsValue, JsValue),
{
    f(JsValue::undefined(), JsValue::undefined());
    JsValue::undefined()
}

/// Schedules one browser task using `MessageChannel`.
pub(crate) fn task<F>(f: F)
where
    F: Fn(),
{
    host_task(f)
}

#[cfg_attr(target_arch = "wasm32", link_section = "rcgjs.host.task")]
#[inline(never)]
fn host_task<F>(f: F)
where
    F: Fn(),
{
    f();
}
