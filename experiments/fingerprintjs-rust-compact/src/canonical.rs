use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::x64hash128;

/// The canonical part of a collected component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalValue<'a> {
    /// A component with an own or inherited `error` property.
    Error,
    /// The exact result of the host's `JSON.stringify` call.
    Json(Option<&'a str>),
}

/// A component prepared for deterministic hashing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanonicalComponent<'a> {
    pub key: &'a str,
    pub value: CanonicalValue<'a>,
}

/// Produces the exact string consumed by FingerprintJS's hash function.
///
/// JSON conversion stays with the host because JavaScript values such as
/// `undefined`, sparse arrays, and custom `toJSON` methods have no exact Rust
/// equivalent. Pass the unchanged `JSON.stringify` result in `Json`.
pub fn components_to_canonical_string(components: &[CanonicalComponent<'_>]) -> String {
    let mut sorted = components.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| compare_js_strings(left.key, right.key));

    let mut output = String::new();
    for component in sorted {
        if !output.is_empty() {
            output.push('|');
        }
        push_escaped_key(&mut output, component.key);
        output.push(':');
        match component.value {
            CanonicalValue::Error => output.push_str("error"),
            CanonicalValue::Json(Some(json)) => output.push_str(json),
            CanonicalValue::Json(None) => output.push_str("undefined"),
        }
    }
    output
}

/// Hashes host-canonicalized components with FingerprintJS's MurmurHash alias.
pub fn hash_components(components: &[CanonicalComponent<'_>]) -> String {
    x64hash128(&components_to_canonical_string(components))
}

fn compare_js_strings(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

fn push_escaped_key(output: &mut String, key: &str) {
    for character in key.chars() {
        if matches!(character, ':' | '|' | '\\') {
            output.push('\\');
        }
        output.push(character);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_keys_values_and_errors() {
        let components = [
            CanonicalComponent {
                key: "z",
                value: CanonicalValue::Json(None),
            },
            CanonicalComponent {
                key: "b|c:d\\e",
                value: CanonicalValue::Json(Some("[1,true]")),
            },
            CanonicalComponent {
                key: "a",
                value: CanonicalValue::Error,
            },
        ];
        assert_eq!(
            components_to_canonical_string(&components),
            "a:error|b\\|c\\:d\\\\e:[1,true]|z:undefined"
        );
    }

    #[test]
    fn sorts_like_javascript_utf16_strings() {
        let components = [
            CanonicalComponent {
                key: "\u{e000}",
                value: CanonicalValue::Json(Some("1")),
            },
            CanonicalComponent {
                key: "\u{10000}",
                value: CanonicalValue::Json(Some("2")),
            },
        ];
        assert_eq!(
            components_to_canonical_string(&components),
            "\u{10000}:2|\u{e000}:1"
        );
    }
}
