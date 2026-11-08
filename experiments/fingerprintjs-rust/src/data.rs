use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// Returns whether a slice contains a value.
pub fn includes<T: PartialEq>(haystack: &[T], needle: &T) -> bool {
    haystack.contains(needle)
}

/// Returns whether a slice does not contain a value.
pub fn excludes<T: PartialEq>(haystack: &[T], needle: &T) -> bool {
    !includes(haystack, needle)
}

/// Applies JavaScript `parseInt` without an explicit radix.
pub fn to_int(value: &str) -> f64 {
    let value = trim_js_whitespace(value);
    let (negative, value) = strip_sign(value);
    let (radix, digits) = if value.starts_with("0x") || value.starts_with("0X") {
        (16, &value[2..])
    } else {
        (10, value)
    };

    let mut result = 0.0;
    let mut count = 0;
    for byte in digits.bytes() {
        let Some(digit) = digit_value(byte, radix) else {
            break;
        };
        result = result * f64::from(radix) + f64::from(digit);
        count += 1;
    }
    if count == 0 {
        f64::NAN
    } else if negative {
        -result
    } else {
        result
    }
}

/// Applies JavaScript `parseFloat` to a string value.
pub fn to_float(value: &str) -> f64 {
    let value = trim_js_whitespace(value);
    let (negative, unsigned) = strip_sign(value);
    if unsigned.starts_with("Infinity") {
        return if negative {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }

    let bytes = unsigned.as_bytes();
    let mut end = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let mut has_digits = end > 0;
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        let fraction_start = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        has_digits |= end > fraction_start;
    }
    if !has_digits {
        return f64::NAN;
    }

    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let exponent_start = end;
        end += 1;
        if matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 1;
        }
        let digits_start = end;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == digits_start {
            end = exponent_start;
        }
    }

    let parsed = unsigned[..end].parse::<f64>().unwrap_or(f64::NAN);
    if negative {
        -parsed
    } else {
        parsed
    }
}

/// Turns NaN into `None`, preserving all other numbers.
pub fn finite_or_none(value: f64) -> Option<f64> {
    (!value.is_nan()).then_some(value)
}

/// Counts `true` values.
pub fn count_truthy(values: &[bool]) -> usize {
    values.iter().filter(|&&value| value).count()
}

/// Rounds to a multiple using FingerprintJS's JavaScript rounding semantics.
pub fn round(value: f64, base: f64) -> f64 {
    if base.abs() >= 1.0 {
        js_round(value / base) * base
    } else {
        let counter_base = 1.0 / base;
        js_round(value * counter_base) / counter_base
    }
}

fn js_round(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    if value >= i64::MAX as f64 || value <= i64::MIN as f64 {
        return value;
    }
    let truncated = value as i64 as f64;
    let floor = if truncated > value {
        truncated - 1.0
    } else {
        truncated
    };
    let result = if value - floor < 0.5 {
        floor
    } else {
        floor + 1.0
    };
    if result == 0.0 && value.is_sign_negative() {
        -0.0
    } else {
        result
    }
}

/// The output of parsing a supported single-element CSS selector.
#[derive(Debug, PartialEq, Eq)]
pub struct SimpleCssSelector {
    pub tag: Option<String>,
    pub attributes: BTreeMap<String, Vec<String>>,
}

/// An unsupported simple selector.
#[derive(Debug, PartialEq, Eq)]
pub struct SelectorError {
    selector: String,
}

impl fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Unexpected syntax '{}'", self.selector)
    }
}

/// Parses the single-element selector grammar used by the blocker source.
pub fn parse_simple_css_selector(selector: &str) -> Result<SimpleCssSelector, SelectorError> {
    let error = || SelectorError {
        selector: selector.to_string(),
    };
    let trimmed = selector.trim_start();
    let tag_end = trimmed
        .char_indices()
        .find_map(|(index, character)| (!is_tag_character(character)).then_some(index))
        .unwrap_or(trimmed.len());
    let tag = (tag_end > 0).then(|| trimmed[..tag_end].to_string());
    let mut attributes = BTreeMap::<String, Vec<String>>::new();
    let rest = &trimmed[tag_end..];
    let bytes = rest.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'.' | b':' | b'#' if bytes.get(index + 1).is_some_and(|byte| is_word(*byte)) => {
                let prefix = bytes[index];
                let start = index + 1;
                index = start;
                while index < bytes.len() && is_word(bytes[index]) {
                    index += 1;
                }
                let name = match prefix {
                    b'.' => "class",
                    b'#' => "id",
                    _ => return Err(error()),
                };
                attributes
                    .entry(name.to_string())
                    .or_default()
                    .push(rest[start..index].to_string());
            }
            b'[' => {
                let Some(relative_end) = rest[index + 1..].find(']') else {
                    break;
                };
                let end = index + relative_end + 1;
                let (name, value) = parse_attribute(&rest[index + 1..end]).ok_or_else(error)?;
                attributes.entry(name).or_default().push(value);
                index = end + 1;
            }
            _ => index += 1,
        }
    }

    Ok(SimpleCssSelector { tag, attributes })
}

fn parse_attribute(attribute: &str) -> Option<(String, String)> {
    let bytes = attribute.as_bytes();
    let mut index = 0;
    while index < bytes.len() && is_word(bytes[index]) {
        index += 1;
    }
    if index == 0 {
        return None;
    }
    let name = attribute[..index].to_string();
    if index == bytes.len() {
        return Some((name, String::new()));
    }

    if matches!(bytes.get(index), Some(b'~' | b'|' | b'^' | b'$' | b'*')) {
        index += 1;
    }
    if bytes.get(index) != Some(&b'=') {
        return None;
    }
    index += 1;

    let value = if bytes.get(index) == Some(&b'"') {
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != b'"' {
            index += 1;
        }
        if index == bytes.len() {
            return None;
        }
        let value = attribute[value_start..index].to_string();
        index += 1;
        value
    } else {
        let value_start = index;
        while index < bytes.len() && is_word(bytes[index]) {
            index += 1;
        }
        if index == value_start {
            return None;
        }
        attribute[value_start..index].to_string()
    };

    if index < bytes.len() {
        if !bytes[index].is_ascii_whitespace() {
            return None;
        }
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if !matches!(bytes.get(index), Some(b'i' | b's')) {
            return None;
        }
        index += 1;
    }
    (index == bytes.len()).then_some((name, value))
}

/// Compares two set-like slices by value.
pub fn are_sets_equal<T: PartialEq>(first: &[T], second: &[T]) -> bool {
    first.len() == second.len() && first.iter().all(|item| second.contains(item))
}

/// Returns the item with the highest score, keeping the first item on ties.
pub fn max_by_score<T, I, F>(items: I, mut score: F) -> Option<T>
where
    I: IntoIterator<Item = T>,
    F: FnMut(&T) -> f64,
{
    let mut maximum: Option<(T, f64)> = None;
    for item in items {
        let item_score = score(&item);
        if maximum
            .as_ref()
            .is_none_or(|(_, maximum_score)| item_score > *maximum_score)
        {
            maximum = Some((item, item_score));
        }
    }
    maximum.map(|(item, _)| item)
}

/// Returns a Rust string's UTF-8 bytes.
pub fn get_utf8_bytes(input: &str) -> Vec<u8> {
    input.as_bytes().to_vec()
}

fn trim_js_whitespace(value: &str) -> &str {
    value.trim_start_matches(|character: char| character.is_whitespace() || character == '\u{feff}')
}

fn strip_sign(value: &str) -> (bool, &str) {
    if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    }
}

fn digit_value(byte: u8, radix: u32) -> Option<u32> {
    let digit = match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a') + 10,
        b'A'..=b'F' => u32::from(byte - b'A') + 10,
        _ => return None,
    };
    (digit < radix).then_some(digit)
}

fn is_tag_character(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '-'
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn parses_numbers_like_javascript() {
        assert_eq!(to_int("  -12.6"), -12.0);
        assert_eq!(to_int("0x10 tail"), 16.0);
        assert!(to_int("foo").is_nan());
        assert_eq!(to_float("12.6px"), 12.6);
        assert_eq!(to_float("-.5e2 tail"), -50.0);
        assert!(to_float("foo").is_nan());
    }

    #[test]
    fn rounds_like_javascript() {
        assert_eq!(round(7.9, 1.0), 8.0);
        assert_eq!(round(29_847.23, 10.0), 29_850.0);
        assert_eq!(round(0.123_432_1, 0.0001), 0.1234);
        assert_eq!(round(-234.65, 2.0), -234.0);
        assert_eq!(round(-1.5, 1.0), -1.0);
    }

    #[test]
    fn parses_blocker_selectors() {
        assert_eq!(
            parse_simple_css_selector("#narrow_ad_unit.top-menu-ads.foo_bar").unwrap(),
            SimpleCssSelector {
                tag: None,
                attributes: BTreeMap::from([
                    (
                        "class".to_string(),
                        vec!["top-menu-ads".to_string(), "foo_bar".to_string()]
                    ),
                    ("id".to_string(), vec!["narrow_ad_unit".to_string()]),
                ]),
            }
        );
        assert_eq!(
            parse_simple_css_selector(" img[width=\"460\"][height=60 i]").unwrap(),
            SimpleCssSelector {
                tag: Some("img".to_string()),
                attributes: BTreeMap::from([
                    ("height".to_string(), vec!["60".to_string()]),
                    ("width".to_string(), vec!["460".to_string()]),
                ]),
            }
        );
    }
}
