//! Typed byte and compression operations provided by the JavaScript host.

use alloc::vec::Vec;

use crate::{js_bindings, HostHandle, HostValue, JsString, JsValue};

/// A native JavaScript `Uint8Array`.
///
/// Conversion from an opaque value checks its public constructor against the current realm's
/// `Uint8Array` and verifies the intrinsic element kind. Detached storage raises the host's
/// `TypeError`. These checks apply at conversion time; another alias can later detach the buffer.
/// Copies between two values use the host's overlap-safe `TypedArray.prototype.set` operation.
///
/// # Bulk byte crossings
///
/// Bytes cross between Rust and the host only by copy, in bulk whatever the length.
/// [`from_slice`](Self::from_slice) (also `From<&[u8]>`) is one host operation that hands the host a
/// fresh `Uint8Array`. [`to_vec`](Self::to_vec) (also `From<JsUint8Array> for Vec<u8>`) and
/// [`copy_to_slice`](Self::copy_to_slice) are two: the length read that sizes the destination, then
/// one copy. The two sides never share storage: a later write to the Rust buffer is not visible
/// through the `Uint8Array`, and a later write to the `Uint8Array` is not visible to Rust.
///
/// A length is read from the view's internal slots, never from its `length` property, so an own
/// `length` a host defines on the instance changes nothing here. A view whose buffer was detached
/// after the conversion has length zero: [`len`](Self::len) answers 0, `to_vec` returns an empty
/// `Vec`, and `copy_to_slice` panics unless its destination is empty.
///
/// The export raises nothing but a compiler abort: a value that is not a Rust byte slice and an
/// allocation the engine refuses both abort. The length read runs no host code, and refuses a
/// `JsUint8Array` that does not hold a `Uint8Array` with a `TypeError`, as a mismatched
/// [`JsValue::read`] does. The import's copy checks its source the same way and aborts before it
/// writes if the lengths disagree.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsUint8Array {
    #[allow(dead_code, reason = "opaque host handle lowered by backend")]
    handle: HostHandle,
}

impl crate::host::sealed::Sealed for JsUint8Array {}
impl HostValue for JsUint8Array {
    fn accepts(value: JsValue) -> bool {
        if !array_buffer_is_view(value)
            || !value_constructor(value).strictly_equals(uint8_array_constructor())
        {
            return false;
        }
        let prototype = prototype_of(constructor_prototype(uint8_array_constructor()));
        let descriptor = own_property_descriptor(prototype, to_string_tag(symbol_constructor()));
        let tag = call_getter(descriptor_getter(descriptor), value);
        if !tag.strictly_equals(JsValue::erase("Uint8Array")) {
            return false;
        }
        // indexOf validates attachment before its Infinity fast exit, without reading or
        // writing an element. Live empty buffers pass; detached empty buffers throw.
        let _ = uint8_array_index_of(value, 0, f64::INFINITY);
        true
    }
}

impl JsUint8Array {
    /// Allocates zero-filled host byte storage of exactly `length` bytes.
    #[must_use]
    pub fn new(length: usize) -> Self {
        uint8_array_new(length)
    }

    /// Copies a Rust byte slice into a fresh `Uint8Array` of exactly `bytes.len()` bytes.
    ///
    /// The whole range is copied by one host operation, which raises nothing but a compiler abort.
    /// The result shares no storage with `bytes`.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        crate::host::host_u8array_from_bytes(bytes)
    }

    /// Copies every byte of this view into `destination`: a length read, then one copy.
    ///
    /// # Panics
    ///
    /// When `destination.len()` is not this view's [`len`](Self::len), as
    /// `<[u8]>::copy_from_slice` does, before anything is written. A detached view has length
    /// zero.
    pub fn copy_to_slice(&self, destination: &mut [u8]) {
        assert!(
            self.len() == destination.len(),
            "JsUint8Array::copy_to_slice: destination length differs from the view's"
        );
        crate::host::host_u8array_into_bytes(*self, destination);
    }

    /// Copies this view's bytes into a new `Vec<u8>` of exactly [`len`](Self::len) bytes: a length
    /// read, then one copy. A detached view gives an empty `Vec`.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = alloc::vec![0; self.len()];
        crate::host::host_u8array_into_bytes(*self, &mut bytes);
        bytes
    }

    /// Returns the number of bytes in this view, from its internal slots: an own `length`
    /// property does not shadow it, and a detached view has length zero.
    #[must_use]
    pub fn len(&self) -> usize {
        crate::host::host_u8array_length(*self)
    }

    /// Whether this view contains no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads one byte, or returns `None` when `index` is out of bounds.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<u8> {
        (index < self.len()).then(|| uint8_array_get(*self, index))
    }

    /// Writes one byte. An out-of-bounds JavaScript typed-array write has no effect.
    pub fn set(&mut self, index: usize, value: u8) {
        uint8_array_set_index(*self, index, value);
    }

    /// Returns a view over the checked half-open byte range.
    #[must_use]
    pub fn subarray(&self, start: usize, end: usize) -> Option<Self> {
        (start <= end && end <= self.len()).then(|| uint8_array_subarray(*self, start, end))
    }

    /// Copies all of `source` at `offset` through native typed-array `.set()` semantics.
    ///
    /// The operation is valid for disjoint arrays and overlapping views of the same buffer. A
    /// detached buffer or insufficient destination range is returned as the original host error.
    pub fn copy_from(&mut self, source: Self, offset: usize) -> Result<(), JsValue> {
        uint8_array_set(*self, source, offset)
    }

    /// Encodes this view as unpadded Base64URL.
    ///
    /// Browsers with `Uint8Array.prototype.toBase64` use it directly. Other targets use the
    /// compiler runtime's byte-equivalent chunked `btoa` fallback.
    #[must_use]
    pub fn to_base64_url(&self) -> JsString {
        crate::host::host_base64_url(*self)
    }

    /// Erases the typed view to an opaque host value.
    #[must_use]
    pub fn as_value(self) -> JsValue {
        JsValue::erase(self)
    }
}

impl From<JsUint8Array> for JsValue {
    fn from(value: JsUint8Array) -> Self {
        value.as_value()
    }
}

/// [`JsUint8Array::from_slice`].
impl From<&[u8]> for JsUint8Array {
    fn from(bytes: &[u8]) -> Self {
        Self::from_slice(bytes)
    }
}

/// [`JsUint8Array::to_vec`].
impl From<JsUint8Array> for Vec<u8> {
    fn from(value: JsUint8Array) -> Self {
        value.to_vec()
    }
}

impl TryFrom<JsValue> for JsUint8Array {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        value.try_read()
    }
}

/// Proof that the current realm provides native typed-array Base64.
///
/// Obtain this zero-sized token through [`typed_array_base64_capability`]. Requiring it at the
/// direct operation keeps a fallback-free package explicit instead of silently raising the
/// target contract.
#[derive(Clone, Copy, Debug)]
pub struct TypedArrayBase64 {
    #[allow(
        dead_code,
        reason = "private zero-sized field prevents external struct construction"
    )]
    private: (),
}

impl TypedArrayBase64 {
    /// Encodes `bytes` as unpadded Base64URL through the native capability.
    #[must_use]
    pub fn encode_url(self, bytes: JsUint8Array) -> JsString {
        let _ = self;
        crate::host::host_base64_url_native(bytes)
    }
}

/// Returns the native typed-array Base64 capability when the current realm provides it.
#[must_use]
pub fn typed_array_base64_capability() -> Option<TypedArrayBase64> {
    typed_array_base64_supported().then_some(TypedArrayBase64 { private: () })
}

/// Whether the current realm provides native typed-array Base64.
#[must_use]
pub fn typed_array_base64_supported() -> bool {
    crate::host::host_base64_url_supported()
}

/// A format accepted by `CompressionStream` and `DecompressionStream`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompressionFormat {
    Deflate,
    DeflateRaw,
    Gzip,
}

impl CompressionFormat {
    fn host_name(self) -> JsString {
        JsString::new(match self {
            Self::Deflate => "deflate",
            Self::DeflateRaw => "deflate-raw",
            Self::Gzip => "gzip",
        })
    }
}

macro_rules! compression_stream_type {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            js_name: $js_name:literal,
            new_doc: $new_doc:literal,
            ctor: $ctor:ident,
            new: $new_fn:ident,
            readable: $readable_fn:ident,
            writable: $writable_fn:ident,
        }
    ) => {
        $(#[$meta])*
        #[repr(transparent)]
        #[derive(Clone, Copy)]
        pub struct $name {
            #[allow(dead_code, reason = "opaque host handle lowered by backend")]
            handle: HostHandle,
        }

        impl crate::host::sealed::Sealed for $name {}
        impl HostValue for $name {
            fn accepts(value: JsValue) -> bool {
                value.is_object()
                    && reflect_has(global_this(), JsString::new($js_name))
                    && value_constructor(value).strictly_equals($ctor())
            }
        }

        impl $name {
            #[doc = $new_doc]
            ///
            /// Unsupported browsers and unsupported formats return their original JavaScript
            /// exception.
            pub fn new(format: CompressionFormat) -> Result<Self, JsValue> {
                $new_fn(format.host_name())
            }

            /// The readable side of the transform.
            #[must_use]
            pub fn readable(&self) -> JsValue {
                $readable_fn(*self)
            }

            /// The writable side of the transform.
            #[must_use]
            pub fn writable(&self) -> JsValue {
                $writable_fn(*self)
            }

            /// Erases the transform to an opaque host value.
            #[must_use]
            pub fn as_value(self) -> JsValue {
                JsValue::erase(self)
            }
        }

        impl From<$name> for JsValue {
            fn from(value: $name) -> Self {
                value.as_value()
            }
        }

        impl TryFrom<JsValue> for $name {
            type Error = JsValue;

            fn try_from(value: JsValue) -> Result<Self, Self::Error> {
                value.try_read()
            }
        }
    };
}

compression_stream_type! {
    /// A native browser `CompressionStream` transform.
    pub struct JsCompressionStream {
        js_name: "CompressionStream",
        new_doc: "Constructs the requested native compression transform.",
        ctor: compression_stream_constructor,
        new: compression_stream_new,
        readable: compression_stream_readable,
        writable: compression_stream_writable,
    }
}

compression_stream_type! {
    /// A native browser `DecompressionStream` transform.
    pub struct JsDecompressionStream {
        js_name: "DecompressionStream",
        new_doc: "Constructs the requested native decompression transform.",
        ctor: decompression_stream_constructor,
        new: decompression_stream_new,
        readable: decompression_stream_readable,
        writable: decompression_stream_writable,
    }
}

/// Whether the current realm provides both compression transform constructors.
#[must_use]
pub fn compression_stream_supported() -> bool {
    let global = global_this();
    reflect_has(global, JsString::new("CompressionStream"))
        && reflect_has(global, JsString::new("DecompressionStream"))
}

js_bindings! {
    #[js(new = "Uint8Array")]
    fn uint8_array_new(length: usize) -> JsUint8Array;

    #[js(call = "ArrayBuffer.isView")]
    fn array_buffer_is_view(value: JsValue) -> bool;

    #[js(global, get = "Uint8Array")]
    fn uint8_array_constructor() -> JsValue;

    #[js(get = "constructor")]
    fn value_constructor(value: JsValue) -> JsValue;

    #[js(get = "prototype")]
    fn constructor_prototype(value: JsValue) -> JsValue;

    #[js(call = "Object.getPrototypeOf")]
    fn prototype_of(value: JsValue) -> JsValue;

    #[js(global, get = "Symbol")]
    fn symbol_constructor() -> JsValue;

    #[js(get = "toStringTag")]
    fn to_string_tag(value: JsValue) -> JsValue;

    #[js(call = "Object.getOwnPropertyDescriptor")]
    fn own_property_descriptor(value: JsValue, key: JsValue) -> JsValue;

    #[js(get = "get")]
    fn descriptor_getter(value: JsValue) -> JsValue;

    #[js(method = "call")]
    fn call_getter(getter: JsValue, value: JsValue) -> JsValue;

    #[js(call = "Uint8Array.prototype.indexOf.call")]
    fn uint8_array_index_of(value: JsValue, search: u8, start: f64) -> isize;

    #[js(index)]
    fn uint8_array_get(value: JsUint8Array, index: usize) -> u8;

    #[js(index_set)]
    fn uint8_array_set_index(value: JsUint8Array, index: usize, item: u8);

    #[js(method = "subarray")]
    fn uint8_array_subarray(value: JsUint8Array, start: usize, end: usize) -> JsUint8Array;

    #[js(method = "set", fallible)]
    fn uint8_array_set(
        value: JsUint8Array,
        source: JsUint8Array,
        offset: usize,
    ) -> Result<(), JsValue>;

    #[js(global, get = "globalThis")]
    fn global_this() -> JsValue;

    #[js(call = "Reflect.has")]
    fn reflect_has(target: JsValue, property: JsString) -> bool;

    #[js(global, get = "CompressionStream")]
    fn compression_stream_constructor() -> JsValue;

    #[js(global, get = "DecompressionStream")]
    fn decompression_stream_constructor() -> JsValue;

    #[js(new = "CompressionStream", fallible)]
    fn compression_stream_new(
        format: JsString,
    ) -> Result<JsCompressionStream, JsValue>;

    #[js(get = "readable")]
    fn compression_stream_readable(value: JsCompressionStream) -> JsValue;

    #[js(get = "writable")]
    fn compression_stream_writable(value: JsCompressionStream) -> JsValue;

    #[js(new = "DecompressionStream", fallible)]
    fn decompression_stream_new(
        format: JsString,
    ) -> Result<JsDecompressionStream, JsValue>;

    #[js(get = "readable")]
    fn decompression_stream_readable(value: JsDecompressionStream) -> JsValue;

    #[js(get = "writable")]
    fn decompression_stream_writable(value: JsDecompressionStream) -> JsValue;
}
