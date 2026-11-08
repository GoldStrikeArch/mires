//! Native canvas and WebGL operations shared by the graphics entropy sources.

use mires_js::{construct, JsValue};

use crate::{
    array, call, get, get_key, global_this, index, length, object_keys, push, string_value,
};

/// Concatenates host values after JavaScript `String` conversion.
pub fn join_as_strings(parts: &[JsValue]) -> JsValue {
    let strings = array();
    for part in parts {
        push(strings, string_value(*part));
    }
    call(strings, "join", &[JsValue::erase("")])
}

/// Converts an optional host value like `value?.toString() || ''` for WebGL primitives.
pub fn webgl_string(value: JsValue) -> JsValue {
    if value.is_null() || value.is_undefined() {
        JsValue::erase("")
    } else {
        let value = string_value(value);
        if value.is_truthy() {
            value
        } else {
            JsValue::erase("")
        }
    }
}

/// Returns the uppercase constant-like own keys on an object's immediate prototype.
pub fn constants_from_prototype(value: JsValue) -> JsValue {
    let keys = object_keys(get(value, "__proto__"));
    let constants = array();
    let invalid_character = construct(
        get(global_this(), "RegExp"),
        &[JsValue::erase("[^A-Z0-9_x]")],
    );
    for key_index in 0..length(keys) {
        let key = index(keys, key_index);
        if key.is_string() && !call(invalid_character, "test", &[key]).is_truthy() {
            push(constants, key);
        }
    }
    constants
}

/// Matches FingerprintJS's `/[^A-Z0-9_x]/` constant-name filter.
pub fn is_constant_like(key: &str) -> bool {
    key.bytes().all(|byte| {
        byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'x')
    })
}

/// Reads a computed property without converting its key to Rust.
pub fn computed(value: JsValue, key: JsValue) -> JsValue {
    get_key(value, key)
}

#[cfg(test)]
mod tests {
    use super::is_constant_like;

    #[test]
    fn matches_upstream_constant_filter() {
        for accepted in [
            "",
            "VERSION",
            "TEXTURE_2D",
            "MAX_3D_TEXTURE_SIZE",
            "UNPACK_COLORSPACE_CONVERSION_WEBGL",
            "x",
        ] {
            assert!(is_constant_like(accepted), "{accepted}");
        }

        for rejected in ["getParameter", "drawingBufferWidth", "WEBGL-debug", "RGB8!"] {
            assert!(!is_constant_like(rejected), "{rejected}");
        }
    }
}
