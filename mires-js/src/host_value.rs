//! Native JavaScript strings and arrays at a typed host boundary.

use core::marker::PhantomData;

use crate::{js_bindings, HostHandle, HostValue, JsValue};

/// A JavaScript string.
///
/// This is not a Rust [`str`] or [`alloc::string::String`]. Its indices and length are UTF-16
/// code units, matching JavaScript's native string operations.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsString {
    #[allow(dead_code)]
    handle: HostHandle,
}

impl crate::host::sealed::Sealed for JsString {}
impl HostValue for JsString {
    fn accepts(value: JsValue) -> bool {
        value.is_string()
    }
}

impl JsString {
    /// Copies a Rust string into a native JavaScript string.
    #[must_use]
    pub fn new(value: &str) -> Self {
        JsValue::erase(value).read()
    }

    /// Returns the number of UTF-16 code units.
    #[must_use]
    pub fn len(&self) -> usize {
        string_len(*self)
    }

    /// Whether the string has no UTF-16 code units.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns one UTF-16 code unit, or `None` when `index` is out of bounds.
    #[must_use]
    pub fn code_unit_at(&self, index: usize) -> Option<u16> {
        (index < self.len()).then(|| string_char_code_at(*self, index))
    }

    /// Returns the native substring between two UTF-16 indices.
    #[must_use]
    pub fn slice(&self, start: usize, end: usize) -> Self {
        string_slice(*self, start, end)
    }

    /// Returns the concatenation of two native strings.
    #[must_use]
    pub fn concat(&self, other: Self) -> Self {
        string_concat(*self, other)
    }

    /// Erases the string to an opaque host value.
    #[must_use]
    pub fn as_value(self) -> JsValue {
        JsValue::erase(self)
    }
}

impl From<&str> for JsString {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<JsString> for JsValue {
    fn from(value: JsString) -> Self {
        value.as_value()
    }
}

impl TryFrom<JsValue> for JsString {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        value.try_read()
    }
}

/// A JavaScript array whose elements have the host representation of `T`.
///
/// Only `JsArray<JsValue>` exposes construction and element access. This prevents a caller from
/// reinterpreting one native array as unrelated Rust element types.
#[repr(transparent)]
pub struct JsArray<T> {
    #[allow(dead_code)]
    handle: HostHandle,
    marker: PhantomData<fn(T) -> T>,
}

impl<T> Clone for JsArray<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for JsArray<T> {}

impl crate::host::sealed::Sealed for JsArray<JsValue> {}
impl HostValue for JsArray<JsValue> {
    fn accepts(value: JsValue) -> bool {
        array_is_array(value)
    }
}

impl JsArray<JsValue> {
    /// Creates an empty native array.
    #[must_use]
    pub fn new() -> Self {
        array_new()
    }

    /// Returns the array length.
    #[must_use]
    pub fn len(&self) -> usize {
        array_len(*self)
    }

    /// Whether the array has no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads an index. A missing index returns JavaScript `undefined`.
    #[must_use]
    pub fn get(&self, index: usize) -> JsValue {
        array_get(*self, index)
    }

    /// Writes an index using JavaScript array semantics.
    pub fn set(&mut self, index: usize, value: JsValue) {
        array_set(*self, index, value);
    }

    /// Appends one value and returns the new length.
    pub fn push(&mut self, value: JsValue) -> usize {
        array_push(*self, value)
    }

    /// Removes and returns the final value. An empty array returns JavaScript `undefined`.
    pub fn pop(&mut self) -> JsValue {
        array_pop(*self)
    }

    /// Erases the array to an opaque host value.
    #[must_use]
    pub fn as_value(self) -> JsValue {
        JsValue::erase(self)
    }
}

impl Default for JsArray<JsValue> {
    fn default() -> Self {
        Self::new()
    }
}

impl From<JsArray<JsValue>> for JsValue {
    fn from(value: JsArray<JsValue>) -> Self {
        value.as_value()
    }
}

impl TryFrom<JsValue> for JsArray<JsValue> {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        value.try_read()
    }
}

js_bindings! {
    #[js(get = "length")]
    fn string_len(value: JsString) -> usize;

    #[js(method = "charCodeAt")]
    fn string_char_code_at(value: JsString, index: usize) -> u16;

    #[js(method = "slice")]
    fn string_slice(value: JsString, start: usize, end: usize) -> JsString;

    #[js(method = "concat")]
    fn string_concat(value: JsString, other: JsString) -> JsString;

    #[js(new = "Array")]
    fn array_new() -> JsArray<JsValue>;

    #[js(call = "Array.isArray")]
    fn array_is_array(value: JsValue) -> bool;

    #[js(get = "length")]
    fn array_len(value: JsArray<JsValue>) -> usize;

    #[js(index)]
    fn array_get(value: JsArray<JsValue>, index: usize) -> JsValue;

    #[js(index_set)]
    fn array_set(value: JsArray<JsValue>, index: usize, item: JsValue);

    #[js(method = "push")]
    fn array_push(value: JsArray<JsValue>, item: JsValue) -> usize;

    #[js(method = "pop")]
    fn array_pop(value: JsArray<JsValue>) -> JsValue;
}
