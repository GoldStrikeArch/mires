//! Typed byte and compression operations provided by the JavaScript host.

use crate::{js_bindings, HostValue, JsString, JsValue};

/// A native JavaScript `Uint8Array`.
///
/// Conversion from an opaque value checks its public constructor against the current realm's
/// `Uint8Array` and verifies the intrinsic element kind. Detached storage raises the host's
/// `TypeError`. These checks apply at conversion time; another alias can later detach the buffer.
/// Construction from Rust bytes copies once into host-owned contiguous storage. Copies between
/// two values use the host's overlap-safe `TypedArray.prototype.set` operation.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsUint8Array {
    #[allow(dead_code)]
    handle: u32,
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

    /// Copies a Rust byte slice into exact-length host storage.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        let mut output = Self::new(bytes.len());
        for (index, byte) in bytes.iter().copied().enumerate() {
            output.set(index, byte);
        }
        output
    }

    /// Returns the number of bytes in this view.
    #[must_use]
    pub fn len(&self) -> usize {
        uint8_array_len(*self)
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

impl TryFrom<JsValue> for JsUint8Array {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        match Self::accepts(value) {
            true => Ok(value.read()),
            false => Err(value),
        }
    }
}

/// Proof that the current realm provides native typed-array Base64.
///
/// Obtain this zero-sized token through [`typed_array_base64_capability`]. Requiring it at the
/// direct operation keeps a fallback-free package explicit instead of silently raising the
/// target contract.
#[derive(Clone, Copy, Debug)]
pub struct TypedArrayBase64 {
    #[allow(dead_code)]
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

/// A native browser `CompressionStream` transform.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsCompressionStream {
    #[allow(dead_code)]
    handle: u32,
}

impl crate::host::sealed::Sealed for JsCompressionStream {}
impl HostValue for JsCompressionStream {
    fn accepts(value: JsValue) -> bool {
        compression_stream_supported()
            && value.is_object()
            && value_constructor(value).strictly_equals(compression_stream_constructor())
    }
}

impl JsCompressionStream {
    /// Constructs the requested native compression transform.
    ///
    /// Unsupported browsers and unsupported formats return their original JavaScript exception.
    pub fn new(format: CompressionFormat) -> Result<Self, JsValue> {
        compression_stream_new(format.host_name())
    }

    /// The readable side of the transform.
    #[must_use]
    pub fn readable(&self) -> JsValue {
        compression_stream_readable(*self)
    }

    /// The writable side of the transform.
    #[must_use]
    pub fn writable(&self) -> JsValue {
        compression_stream_writable(*self)
    }

    /// Erases the transform to an opaque host value.
    #[must_use]
    pub fn as_value(self) -> JsValue {
        JsValue::erase(self)
    }
}

impl From<JsCompressionStream> for JsValue {
    fn from(value: JsCompressionStream) -> Self {
        value.as_value()
    }
}

impl TryFrom<JsValue> for JsCompressionStream {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        match Self::accepts(value) {
            true => Ok(value.read()),
            false => Err(value),
        }
    }
}

/// A native browser `DecompressionStream` transform.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct JsDecompressionStream {
    #[allow(dead_code)]
    handle: u32,
}

impl crate::host::sealed::Sealed for JsDecompressionStream {}
impl HostValue for JsDecompressionStream {
    fn accepts(value: JsValue) -> bool {
        compression_stream_supported()
            && value.is_object()
            && value_constructor(value).strictly_equals(decompression_stream_constructor())
    }
}

impl JsDecompressionStream {
    /// Constructs the requested native decompression transform.
    ///
    /// Unsupported browsers and unsupported formats return their original JavaScript exception.
    pub fn new(format: CompressionFormat) -> Result<Self, JsValue> {
        decompression_stream_new(format.host_name())
    }

    /// The readable side of the transform.
    #[must_use]
    pub fn readable(&self) -> JsValue {
        decompression_stream_readable(*self)
    }

    /// The writable side of the transform.
    #[must_use]
    pub fn writable(&self) -> JsValue {
        decompression_stream_writable(*self)
    }

    /// Erases the transform to an opaque host value.
    #[must_use]
    pub fn as_value(self) -> JsValue {
        JsValue::erase(self)
    }
}

impl From<JsDecompressionStream> for JsValue {
    fn from(value: JsDecompressionStream) -> Self {
        value.as_value()
    }
}

impl TryFrom<JsValue> for JsDecompressionStream {
    type Error = JsValue;

    fn try_from(value: JsValue) -> Result<Self, Self::Error> {
        match Self::accepts(value) {
            true => Ok(value.read()),
            false => Err(value),
        }
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

    #[js(get = "length")]
    fn uint8_array_len(value: JsUint8Array) -> usize;

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
